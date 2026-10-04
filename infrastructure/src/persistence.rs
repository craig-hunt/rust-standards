//! Opening the pool, running work on it, and applying the schema.

use crate::constants::{columns, messages, sql};
use application::ports::{StoreError, StoreResult};
use domain::inventory::seed;
use postgres::{Client, NoTls, Transaction};
use r2d2::{Pool, PooledConnection};
use r2d2_postgres::PostgresConnectionManager;

/// The pool type this crate hands around.
pub type ConnectionPool = Pool<PostgresConnectionManager<NoTls>>;

/// One borrowed connection.
pub type Connection = PooledConnection<PostgresConnectionManager<NoTls>>;

/// Opens the pool, in the one place that knows how.
///
/// Both entry points need a pool, and two copies of this would eventually
/// disagree about a setting that matters.
///
/// Every connection runs [`sql::BOUND_LOCK_WAITS`] as it is handed out, so no
/// statement can wait on a lock forever. That is configuration nothing else
/// would notice was missing, which is why a test asserts it applies.
///
/// # Errors
///
/// Returns [`StoreError::Unavailable`] when the connection string is not one
/// PostgreSQL accepts, or when the first connection cannot be established.
pub fn open(connection_string: &str, pool_size: u32) -> StoreResult<ConnectionPool> {
    let configuration = connection_string
        .parse()
        .map_err(|failure: postgres::Error| StoreError::Unavailable(failure.to_string()))?;
    let manager = PostgresConnectionManager::new(configuration, NoTls);

    Pool::builder()
        .max_size(pool_size)
        .connection_customizer(Box::new(BoundLockWaits))
        .build(manager)
        .map_err(|failure| StoreError::Unavailable(failure.to_string()))
}

/// Applies [`sql::BOUND_LOCK_WAITS`] to each new connection.
#[derive(Debug)]
struct BoundLockWaits;

impl r2d2::CustomizeConnection<Client, postgres::Error> for BoundLockWaits {
    fn on_acquire(&self, connection: &mut Client) -> Result<(), postgres::Error> {
        connection.batch_execute(sql::BOUND_LOCK_WAITS)
    }
}

/// Borrows a connection, turning a pool failure into a port failure.
///
/// # Errors
///
/// Returns [`StoreError::Unavailable`] when the pool had nothing to give.
pub fn borrow(pool: &ConnectionPool) -> StoreResult<Connection> {
    pool.get().map_err(|failure| {
        StoreError::Unavailable(format!("{}: {failure}", messages::NO_CONNECTION))
    })
}

/// Runs work inside one transaction, committing when it answers and rolling back
/// when it does not.
///
/// Every store that writes goes through this. A store opening its own
/// transaction and forgetting to commit would leave work uncommitted in a way no
/// test notices until two statements had to agree.
///
/// # Errors
///
/// Returns whatever the work reported, or [`StoreError::Unavailable`] when the
/// transaction could not be opened or committed.
pub fn in_transaction<T, F>(pool: &ConnectionPool, work: F) -> StoreResult<T>
where
    F: FnOnce(&mut Transaction<'_>) -> StoreResult<T>,
{
    let mut connection = borrow(pool)?;
    let mut transaction = connection
        .transaction()
        .map_err(|failure| unavailable(&failure))?;
    let answer = work(&mut transaction)?;
    transaction
        .commit()
        .map_err(|failure| unavailable(&failure))?;
    Ok(answer)
}

/// A database failure, as the ports describe failures.
///
/// The message is the driver's, which names the constraint or the relation. That
/// reaches a log, never a response body: the edge answers a generic detail,
/// because a message naming a table tells an attacker more than it tells the
/// caller.
#[must_use]
pub fn unavailable(failure: &postgres::Error) -> StoreError {
    StoreError::Unavailable(format!("{}: {failure}", messages::QUERY_FAILED))
}

/// Applies the schema and the seed rows.
///
/// Both steps repeat harmlessly, so starting twice against the same database is
/// not an error. A reference implementation demanding a clean database would
/// teach a workaround before it taught anything else.
///
/// The seed rows come from the domain rather than from the SQL script. Writing
/// the six names in both places would leave nothing to keep them equal, and the
/// first time one drifted the tests would still pass against whichever copy they
/// happened to read.
///
/// # Errors
///
/// Returns [`StoreError::Unavailable`] when a statement fails.
pub fn apply_schema(pool: &ConnectionPool) -> StoreResult<()> {
    const SCRIPT: &str = include_str!("schema.sql");

    in_transaction(pool, |transaction| {
        transaction
            .batch_execute(SCRIPT)
            .map_err(|failure| unavailable(&failure))?;
        let statement = transaction
            .prepare(sql::SEED_INVENTORY)
            .map_err(|failure| unavailable(&failure))?;
        for row in seed() {
            transaction
                .execute(&statement, &[&row.name, &row.quantity, &row.status.value()])
                .map_err(|failure| unavailable(&failure))?;
        }
        Ok(())
    })
}

/// Reads a stock row the way every caller of [`sql::LIST_INVENTORY`] must.
pub(crate) fn read_inventory(row: &postgres::Row) -> domain::inventory::InventoryItem {
    domain::inventory::InventoryItem {
        name: row.get(columns::NAME),
        quantity: row.get(columns::QUANTITY),
        status: domain::inventory::Status::new(row.get(columns::STATUS)),
    }
}
