//! Asking the database to answer something trivial.

use crate::constants::sql;
use crate::persistence::{self, ConnectionPool};
use application::ports::{HealthProbe, StoreResult};

/// Runs a statement rather than merely borrowing a connection.
///
/// A pooled connection can be handed over after the server on the other end has
/// gone away. Only a round trip proves the database is still there.
#[derive(Clone)]
pub struct PostgresHealthProbe {
    pool: ConnectionPool,
}

impl PostgresHealthProbe {
    /// Wires the probe to a pool.
    #[must_use]
    pub const fn new(pool: ConnectionPool) -> Self {
        Self { pool }
    }
}

impl HealthProbe for PostgresHealthProbe {
    fn ping(&self) -> StoreResult<()> {
        let mut connection = persistence::borrow(&self.pool)?;
        connection
            .query_one(sql::PING, &[])
            .map_err(|failure| persistence::unavailable(&failure))?;
        Ok(())
    }
}
