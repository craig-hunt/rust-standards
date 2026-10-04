//! Liveness and readiness, and what each answers.

use std::time::Duration;

/// Every literal the health feature carries, named once.
pub mod constants {
    /// Reported when the thing asked about is working.
    pub const STATUS_OK: &str = "ok";
    /// Reported when a dependency is not answering.
    pub const STATUS_UNAVAILABLE: &str = "unavailable";
    /// Logged when a readiness probe fails.
    pub const MSG_NOT_READY: &str = "readiness check failed";
}

/// How long a readiness probe waits on the database before reporting the
/// instance unavailable.
///
/// A probe that waits as long as the database takes turns a slow dependency into
/// a hung probe, and the platform then keeps routing traffic to an instance that
/// cannot serve it.
pub const READY_TIMEOUT: Duration = Duration::from_secs(2);

/// What a health probe answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealthReport {
    /// Either [`constants::STATUS_OK`] or [`constants::STATUS_UNAVAILABLE`].
    pub status: &'static str,
}

impl HealthReport {
    /// The answer when the thing asked about is working.
    #[must_use]
    pub const fn ok() -> Self {
        Self {
            status: constants::STATUS_OK,
        }
    }

    /// The answer when a dependency is not.
    #[must_use]
    pub const fn unavailable() -> Self {
        Self {
            status: constants::STATUS_UNAVAILABLE,
        }
    }
}
