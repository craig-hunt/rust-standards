//! The migrator process: applies the schema and exits.
//!
//! A separate process on purpose. If the API applied the schema at startup, two
//! replicas rolling out together would run the same DDL at the same moment, and
//! PostgreSQL answers concurrent DDL on one table with a lock wait or a deadlock
//! rather than with a tidy no-op. Running migration to completion before any
//! replica starts removes the race instead of hoping to win it.
//!
//! It reads only the database settings, so it cannot be blocked by a missing API
//! token it would never present.

use web::constants::POOL_SIZE;
use web::settings::DatabaseSettings;

fn main() -> std::process::ExitCode {
    tracing_subscriber::fmt()
        .json()
        .with_current_span(false)
        .init();

    let settings = match DatabaseSettings::from_environment() {
        Ok(settings) => settings,
        Err(missing) => {
            tracing::error!(%missing, "cannot migrate");
            return std::process::ExitCode::FAILURE;
        }
    };

    let applied = infrastructure::persistence::open(&settings.url, POOL_SIZE)
        .and_then(|pool| infrastructure::persistence::apply_schema(&pool));

    match applied {
        Ok(()) => {
            tracing::info!("schema applied");
            std::process::ExitCode::SUCCESS
        }
        Err(failure) => {
            tracing::error!(%failure, "cannot migrate");
            std::process::ExitCode::FAILURE
        }
    }
}
