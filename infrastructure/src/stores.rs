//! The ports, implemented against PostgreSQL.
//!
//! No row type sits between the table and the domain. `RETURNING` hands back the
//! assigned key in the same round trip, so the mapping reads straight into the
//! domain type and a middle layer would carry no information. The outbox is the
//! exception, and says why in its own module.

use crate::constants::{columns, messages, sql};
use crate::outbox;
use crate::persistence::{self, ConnectionPool};
use application::ports::{Clock, InventoryStore, SignupStore, StoreError, StoreResult, TaskStore};
use domain::inventory::InventoryItem;
use domain::signups::{Signup, SignupId};
use domain::tasks::{TaskId, TaskItem, TaskTitle, errors};
use postgres::Row;
use uuid::Uuid;

/// Nothing matched, which for a write means the row named does not exist.
const NOTHING_CHANGED: u64 = 0;

/// Tasks, in PostgreSQL.
///
/// Completing a task writes the task row and the event announcing it in one
/// transaction. Announcing it afterwards, outside the transaction, would let a
/// crash between the two leave a completed task nobody was told about.
pub struct PostgresTaskStore<C> {
    pool: ConnectionPool,
    clock: C,
}

impl<C: Clock> PostgresTaskStore<C> {
    /// Wires the store to a pool and a clock.
    pub const fn new(pool: ConnectionPool, clock: C) -> Self {
        Self { pool, clock }
    }
}

/// Reads a task the way every statement returning one must.
fn read_task(row: &Row) -> StoreResult<TaskItem> {
    Ok(TaskItem {
        id: TaskId::new(row.get(columns::ID))?,
        title: TaskTitle::new(row.get::<_, &str>(columns::TITLE))
            .map_err(|refused| StoreError::Domain(refused.into()))?,
        completed: row.get(columns::COMPLETED),
    })
}

impl<C: Clock> TaskStore for PostgresTaskStore<C> {
    fn list(&self) -> StoreResult<Vec<TaskItem>> {
        let mut connection = persistence::borrow(&self.pool)?;
        connection
            .query(sql::LIST_TASKS, &[])
            .map_err(|failure| persistence::unavailable(&failure))?
            .iter()
            .map(read_task)
            .collect()
    }

    fn create(&self, title: &TaskTitle) -> StoreResult<TaskItem> {
        persistence::in_transaction(&self.pool, |transaction| {
            let row = transaction
                .query_opt(sql::INSERT_TASK, &[&title.value()])
                .map_err(|failure| persistence::unavailable(&failure))?
                .ok_or_else(|| StoreError::Unavailable(messages::ROW_VANISHED.to_owned()))?;
            read_task(&row)
        })
    }

    /// Sets completion, and announces it when a task becomes completed.
    ///
    /// The event fires on the transition only. Setting a completed task completed
    /// again announces nothing: the outbox already tolerates a repeat, but
    /// manufacturing one on every idempotent request would make the backlog grow
    /// with messages that say nothing new. Marking a task incomplete announces
    /// nothing either, because no event here describes that.
    fn set_completed(&self, id: TaskId, completed: bool) -> StoreResult<TaskItem> {
        persistence::in_transaction(&self.pool, |transaction| {
            let row = transaction
                .query_opt(sql::SET_TASK_COMPLETED, &[&id.value(), &completed])
                .map_err(|failure| persistence::unavailable(&failure))?
                .ok_or_else(|| StoreError::Domain(errors::not_found()))?;

            let task = read_task(&row)?;
            let was_completed: bool = row.get(columns::WAS_COMPLETED);

            if task.completed && !was_completed {
                let announcement = DomainEventFor::completion(&task, self.clock.now());
                outbox::write(transaction, &announcement)?;
            }

            Ok(task)
        })
    }

    fn delete(&self, id: TaskId) -> StoreResult<()> {
        persistence::in_transaction(&self.pool, |transaction| {
            let removed = transaction
                .execute(sql::DELETE_TASK, &[&id.value()])
                .map_err(|failure| persistence::unavailable(&failure))?;
            if removed == NOTHING_CHANGED {
                return Err(StoreError::Domain(errors::not_found()));
            }
            Ok(())
        })
    }

    fn delete_completed(&self) -> StoreResult<u64> {
        persistence::in_transaction(&self.pool, |transaction| {
            transaction
                .execute(sql::DELETE_COMPLETED_TASKS, &[])
                .map_err(|failure| persistence::unavailable(&failure))
        })
    }
}

/// Builds the event a task's completion announces.
///
/// A free function rather than a method on the domain type, because the
/// identifier for the event is generated here: the domain owns no source of
/// randomness, for the same reason it owns no clock.
struct DomainEventFor;

impl DomainEventFor {
    fn completion(task: &TaskItem, at: std::time::SystemTime) -> domain::events::DomainEvent {
        domain::events::DomainEvent::TaskCompleted {
            event_id: Uuid::new_v4(),
            occurred_at: at,
            task_id: task.id.value(),
            title: task.title.value().to_owned(),
        }
    }
}

/// Signups, in PostgreSQL.
///
/// The signup row and the outbox row commit together. The database assigns the
/// identifier and the event carries it, so the two statements run inside one
/// transaction: either both rows land or neither does. No consumer ever hears
/// about a signup that failed to store, and no stored signup goes unannounced.
pub struct PostgresSignupStore<C> {
    pool: ConnectionPool,
    clock: C,
}

impl<C: Clock> PostgresSignupStore<C> {
    /// Wires the store to a pool and a clock.
    pub const fn new(pool: ConnectionPool, clock: C) -> Self {
        Self { pool, clock }
    }
}

impl<C: Clock> SignupStore for PostgresSignupStore<C> {
    fn save(&self, signup: &Signup) -> StoreResult<SignupId> {
        persistence::in_transaction(&self.pool, |transaction| {
            let now = self.clock.now();
            let row = transaction
                .query_opt(
                    sql::INSERT_SIGNUP,
                    &[
                        &signup.full_name.value(),
                        &signup.email.value(),
                        &signup.plan.value(),
                        &signup.seats.value(),
                        &signup.notes.value(),
                        &now,
                    ],
                )
                .map_err(|failure| persistence::unavailable(&failure))?
                .ok_or_else(|| StoreError::Unavailable(messages::ROW_VANISHED.to_owned()))?;

            let id = SignupId::new(row.get(columns::ID))?;
            outbox::write(transaction, &signup.recorded(id, Uuid::new_v4(), now))?;
            Ok(id)
        })
    }
}

/// Stock rows, in PostgreSQL.
///
/// It reads every row ordered by name, and nothing else. Searching and ordering
/// by the requested column stay in the domain, so a second store cannot answer
/// the same request in a different order. The `ORDER BY` exists to make the input
/// to that sort stable, so rows that tie keep a predictable order.
pub struct PostgresInventoryStore {
    pool: ConnectionPool,
}

impl PostgresInventoryStore {
    /// Wires the store to a pool.
    #[must_use]
    pub const fn new(pool: ConnectionPool) -> Self {
        Self { pool }
    }
}

impl InventoryStore for PostgresInventoryStore {
    fn items(&self) -> StoreResult<Vec<InventoryItem>> {
        let mut connection = persistence::borrow(&self.pool)?;
        Ok(connection
            .query(sql::LIST_INVENTORY, &[])
            .map_err(|failure| persistence::unavailable(&failure))?
            .iter()
            .map(persistence::read_inventory)
            .collect())
    }
}
