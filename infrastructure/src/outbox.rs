//! Writing an event beside the state change that caused it, and delivering it
//! later.

use crate::constants::{OUTBOX_BATCH_SIZE, columns, messages, sql};
use crate::persistence::{self, ConnectionPool};
use application::ports::{Clock, EventDispatcher, StoreError, StoreResult};
use domain::events::DomainEvent;
use postgres::Transaction;
use std::collections::BTreeMap;
use std::time::SystemTime;
use uuid::Uuid;

/// The wire names a stored event carries.
///
/// Literals rather than a type name derived at runtime. A derived name would
/// change the moment somebody renamed a variant, and every row written before
/// the rename would stop resolving: a refactor nobody thought of as a migration
/// would silently strand a backlog.
mod wire {
    /// A validated signup reached the database.
    pub(crate) const SIGNUP_RECORDED: &str = "SignupRecorded";
    /// A task was marked completed.
    pub(crate) const TASK_COMPLETED: &str = "TaskCompleted";
}

/// The members a stored payload carries.
mod payload {
    pub(crate) const SIGNUP_ID: &str = "signupId";
    pub(crate) const EMAIL: &str = "email";
    pub(crate) const PLAN: &str = "plan";
    pub(crate) const SEATS: &str = "seats";
    pub(crate) const TASK_ID: &str = "taskId";
    pub(crate) const TITLE: &str = "title";
}

/// The persistence shape of an outbox message.
///
/// The one place a row type earns its keep. Elsewhere the stores map a result
/// straight into a domain type, but a stored event is a wire format: it holds a
/// type name and a JSON string that no domain type should know about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredEvent {
    /// Identifies the event, and keeps a retried insert from duplicating it.
    pub event_id: Uuid,
    /// The wire name a later deployment resolves.
    pub wire_name: String,
    /// The JSON body.
    pub payload: String,
    /// When it happened.
    pub occurred_at: SystemTime,
}

/// The wire name for an event.
///
/// A match with no catch-all, which is what sealing the event enum buys: a new
/// variant cannot reach the outbox without a wire name, because the build stops
/// here until it has one.
const fn wire_name_of(event: &DomainEvent) -> &'static str {
    match event {
        DomainEvent::SignupRecorded { .. } => wire::SIGNUP_RECORDED,
        DomainEvent::TaskCompleted { .. } => wire::TASK_COMPLETED,
    }
}

/// Turns an event into the row that stores it.
///
/// The payload is built member by member rather than derived, so the wire shape
/// is stated here and a field rename in the domain cannot change it silently.
///
/// # Errors
///
/// Returns [`StoreError::Unavailable`] when the payload cannot be written as
/// JSON, which a map of strings and numbers cannot do.
pub fn to_row(event: &DomainEvent) -> StoreResult<StoredEvent> {
    let mut body: BTreeMap<&str, serde_json::Value> = BTreeMap::new();
    match event {
        DomainEvent::SignupRecorded {
            signup_id,
            email,
            plan,
            seats,
            ..
        } => {
            body.insert(payload::SIGNUP_ID, (*signup_id).into());
            body.insert(payload::EMAIL, email.clone().into());
            body.insert(payload::PLAN, plan.clone().into());
            body.insert(payload::SEATS, (*seats).into());
        }
        DomainEvent::TaskCompleted { task_id, title, .. } => {
            body.insert(payload::TASK_ID, (*task_id).into());
            body.insert(payload::TITLE, title.clone().into());
        }
    }

    let payload = serde_json::to_string(&body).map_err(|failure| {
        StoreError::Unavailable(format!("{}: {failure}", messages::SERIALIZE_FAILED))
    })?;

    Ok(StoredEvent {
        event_id: event.event_id(),
        wire_name: wire_name_of(event).to_owned(),
        payload,
        occurred_at: event.occurred_at(),
    })
}

/// Reads a stored row back, answering `None` when this deployment cannot resolve
/// the type.
///
/// Mapping through a table rather than resolving whatever name a row holds. A
/// name resolved dynamically would let anything that can write a row choose what
/// gets constructed.
#[must_use]
pub fn from_row(row: &StoredEvent) -> Option<DomainEvent> {
    let body: BTreeMap<String, serde_json::Value> = serde_json::from_str(&row.payload).ok()?;

    match row.wire_name.as_str() {
        wire::SIGNUP_RECORDED => Some(DomainEvent::SignupRecorded {
            event_id: row.event_id,
            occurred_at: row.occurred_at,
            signup_id: body.get(payload::SIGNUP_ID)?.as_i64()?,
            email: body.get(payload::EMAIL)?.as_str()?.to_owned(),
            plan: body.get(payload::PLAN)?.as_str()?.to_owned(),
            seats: i32::try_from(body.get(payload::SEATS)?.as_i64()?).ok()?,
        }),
        wire::TASK_COMPLETED => Some(DomainEvent::TaskCompleted {
            event_id: row.event_id,
            occurred_at: row.occurred_at,
            task_id: body.get(payload::TASK_ID)?.as_i64()?,
            title: body.get(payload::TITLE)?.as_str()?.to_owned(),
        }),
        _ => None,
    }
}

/// Writes one outbox row on a transaction a caller already owns.
///
/// It takes the transaction rather than the pool on purpose. The whole value of
/// the outbox is that the row commits with the state change that produced it, so
/// a writer opening its own transaction would quietly undo the guarantee while
/// appearing to provide it.
///
/// # Errors
///
/// Returns [`StoreError::Unavailable`] when the insert fails or the event cannot
/// be serialized.
pub fn write(transaction: &mut Transaction<'_>, event: &DomainEvent) -> StoreResult<()> {
    let row = to_row(event)?;
    transaction
        .execute(
            sql::INSERT_OUTBOX,
            &[
                &row.event_id,
                &row.wire_name,
                &row.payload,
                &row.occurred_at,
            ],
        )
        .map_err(|failure| persistence::unavailable(&failure))?;
    Ok(())
}

/// Claims a batch of messages, delivers them, and marks what it delivered.
///
/// A message whose type this deployment cannot resolve stays unpublished.
/// Marking it would acknowledge something no consumer ever saw, which is the one
/// outcome the outbox exists to prevent. It waits for a deployment that knows the
/// type, and a production system would move it aside once a retry budget ran out.
///
/// Separate from the relay, so the delivery path has a seam a test can drive
/// without a background thread and a timer.
pub struct Publisher<D, C> {
    pool: ConnectionPool,
    dispatcher: D,
    clock: C,
}

impl<D: EventDispatcher, C: Clock> Publisher<D, C> {
    /// Wires the publisher to a pool, a dispatcher and a clock.
    pub const fn new(pool: ConnectionPool, dispatcher: D, clock: C) -> Self {
        Self {
            pool,
            dispatcher,
            clock,
        }
    }

    /// Delivers one batch and answers how many it published.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Unavailable`] when a statement fails, or whatever a
    /// consumer reported, which leaves the batch uncommitted and pending.
    pub fn publish_pending(&self) -> StoreResult<usize> {
        persistence::in_transaction(&self.pool, |transaction| {
            let claimed = Self::claim(transaction)?;
            let published_at = self.clock.now();
            let mut published = 0;

            for row in claimed {
                let Some(event) = from_row(&row) else {
                    tracing::error!(
                        event_id = %row.event_id,
                        wire_name = %row.wire_name,
                        "{}",
                        messages::UNKNOWN_TYPE
                    );
                    continue;
                };
                self.dispatcher.dispatch(&event)?;
                transaction
                    .execute(sql::MARK_PUBLISHED, &[&published_at, &row.event_id])
                    .map_err(|failure| persistence::unavailable(&failure))?;
                published += 1;
            }

            Ok(published)
        })
    }

    fn claim(transaction: &mut Transaction<'_>) -> StoreResult<Vec<StoredEvent>> {
        let rows = transaction
            .query(sql::CLAIM_OUTBOX, &[&OUTBOX_BATCH_SIZE])
            .map_err(|failure| persistence::unavailable(&failure))?;

        Ok(rows
            .iter()
            .map(|row| StoredEvent {
                event_id: row.get(columns::EVENT_ID),
                wire_name: row.get(columns::TYPE),
                payload: row.get(columns::PAYLOAD),
                occurred_at: row.get(columns::OCCURRED_AT),
            })
            .collect())
    }
}
