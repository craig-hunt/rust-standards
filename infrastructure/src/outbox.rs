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

/// The wire names this deployment can turn back into an event.
///
/// Passed to the claim, so a row naming anything else never enters a batch. The
/// list has to match `from_row` exactly, and a test asserts both directions:
/// every name here resolves, and every event the domain can raise has its name
/// here.
const RESOLVABLE_WIRE_NAMES: [&str; 2] = [wire::SIGNUP_RECORDED, wire::TASK_COMPLETED];

/// The wire names a claim asks for.
///
/// Published so a reader and a test can see what the relay will take, rather
/// than inferring it from a statement.
#[must_use]
pub const fn resolvable_wire_names() -> [&'static str; RESOLVABLE_WIRE_NAMES.len()] {
    RESOLVABLE_WIRE_NAMES
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
/// Neither kind of undeliverable message can block the ones behind it, and
/// neither is acknowledged, because marking a message published would claim a
/// consumer saw it. They are kept apart because they differ:
///
/// - **A type this deployment cannot resolve** is excluded by the claim itself,
///   which passes the names this build knows. The row waits, untouched, for a
///   deployment that knows the type. Nothing about it is wrong.
/// - **A known type carrying a body that will not read** can never be delivered
///   by any deployment, so leaving it pending would park it at the head of every
///   ordered batch. The relay quarantines it, which takes it out of the claim and
///   leaves it in the table for an operator.
///
/// Separate from the relay loop, so the delivery path has a seam a test can drive
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
            let now = self.clock.now();
            let mut published = 0;

            for row in claimed {
                let Some(event) = from_row(&row) else {
                    // The claim passed this deployment's own wire names, so a
                    // claimed row naming one of them and still failing to read
                    // carries a body nothing can parse. No later deployment
                    // improves on that, which is what separates it from an
                    // unknown type and what makes quarantine the answer.
                    tracing::error!(
                        event_id = %row.event_id,
                        wire_name = %row.wire_name,
                        "{}",
                        messages::UNREADABLE_PAYLOAD
                    );
                    transaction
                        .execute(sql::QUARANTINE_MESSAGE, &[&now, &row.event_id])
                        .map_err(|failure| persistence::unavailable(&failure))?;
                    continue;
                };
                self.dispatcher.dispatch(&event)?;
                transaction
                    .execute(sql::MARK_PUBLISHED, &[&now, &row.event_id])
                    .map_err(|failure| persistence::unavailable(&failure))?;
                published += 1;
            }

            Ok(published)
        })
    }

    fn claim(transaction: &mut Transaction<'_>) -> StoreResult<Vec<StoredEvent>> {
        let resolvable = RESOLVABLE_WIRE_NAMES.to_vec();
        let rows = transaction
            .query(sql::CLAIM_OUTBOX, &[&OUTBOX_BATCH_SIZE, &resolvable])
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

#[cfg(test)]
mod tests {
    // A test asserts by panicking, so the lints that forbid a panic in a service
    // have to be lifted here.
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{StoredEvent, from_row, resolvable_wire_names, to_row, wire};
    use domain::events::DomainEvent;
    use std::time::SystemTime;
    use uuid::Uuid;

    const SIGNUP_ID: i64 = 7;
    const TASK_ID: i64 = 4;
    const EMAIL: &str = "ada@example.com";
    const PLAN: &str = "Team";
    const SEATS: i32 = 12;
    const TITLE: &str = "Read the ADR";
    const RETIRED_WIRE_NAME: &str = "AccountClosed";
    const NOT_JSON: &str = "{";
    const JSON_MISSING_THE_MEMBERS: &str = "{}";

    fn a_signup() -> DomainEvent {
        DomainEvent::SignupRecorded {
            event_id: Uuid::new_v4(),
            occurred_at: SystemTime::now(),
            signup_id: SIGNUP_ID,
            email: EMAIL.to_owned(),
            plan: PLAN.to_owned(),
            seats: SEATS,
        }
    }

    fn a_completion() -> DomainEvent {
        DomainEvent::TaskCompleted {
            event_id: Uuid::new_v4(),
            occurred_at: SystemTime::now(),
            task_id: TASK_ID,
            title: TITLE.to_owned(),
        }
    }

    #[test]
    fn every_event_survives_the_round_trip_through_a_row() {
        for event in [a_signup(), a_completion()] {
            let row = to_row(&event).unwrap();

            assert_eq!(
                from_row(&row),
                Some(event),
                "a stored event has to read back as what was written, member for member"
            );
        }
    }

    #[test]
    fn the_claim_asks_for_exactly_the_names_this_build_resolves() {
        let written: Vec<String> = [a_signup(), a_completion()]
            .iter()
            .map(|event| to_row(event).unwrap().wire_name)
            .collect();

        for name in resolvable_wire_names() {
            assert!(
                written.contains(&name.to_owned()),
                "{name} is claimed and nothing writes it, so the claim asks for a name from_row cannot resolve"
            );
        }
        for name in &written {
            assert!(
                resolvable_wire_names().contains(&name.as_str()),
                "{name} is written and never claimed, so those events would sit pending forever"
            );
        }
    }

    #[test]
    fn a_name_this_deployment_retired_resolves_to_nothing() {
        let row = StoredEvent {
            event_id: Uuid::new_v4(),
            wire_name: RETIRED_WIRE_NAME.to_owned(),
            payload: JSON_MISSING_THE_MEMBERS.to_owned(),
            occurred_at: SystemTime::now(),
        };

        assert!(from_row(&row).is_none());
        assert!(
            !resolvable_wire_names().contains(&RETIRED_WIRE_NAME),
            "the claim must leave a name this build cannot resolve alone"
        );
    }

    #[test]
    fn a_known_name_carrying_an_unreadable_body_resolves_to_nothing() {
        for body in [NOT_JSON, JSON_MISSING_THE_MEMBERS] {
            let row = StoredEvent {
                event_id: Uuid::new_v4(),
                wire_name: wire::TASK_COMPLETED.to_owned(),
                payload: body.to_owned(),
                occurred_at: SystemTime::now(),
            };

            assert!(
                from_row(&row).is_none(),
                "the relay quarantines what it cannot read, and this is how it finds out"
            );
        }
    }
}
