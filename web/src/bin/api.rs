//! The API process.

use std::sync::Arc;
use web::service::{self, Api};
use web::settings::Settings;

/// Reads the environment, builds the service, starts the relay, and serves.
///
/// Every failure here ends the process with a message naming what was wrong. A
/// service that started without a token, or without a database, would answer
/// health checks and fail every real request.
#[tokio::main]
async fn main() -> std::process::ExitCode {
    tracing_subscriber::fmt()
        .json()
        .with_current_span(false)
        .init();

    let settings = match Settings::from_environment() {
        Ok(settings) => settings,
        Err(missing) => {
            tracing::error!(%missing, "cannot start");
            return std::process::ExitCode::FAILURE;
        }
    };

    let api = match Api::new(&settings) {
        Ok(api) => Arc::new(api),
        Err(failure) => {
            tracing::error!(%failure, "cannot start");
            return std::process::ExitCode::FAILURE;
        }
    };

    if let Err(failure) = service::start_relay(&settings) {
        tracing::error!(%failure, "cannot start the outbox relay");
        return std::process::ExitCode::FAILURE;
    }

    match service::serve(api, &settings).await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(failure) => {
            tracing::error!(%failure, "stopped serving");
            std::process::ExitCode::FAILURE
        }
    }
}
