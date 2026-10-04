//! The API process.

use std::process::ExitCode;
use std::sync::Arc;
use web::constants::BLOCKING_THREADS;
use web::service::{self, Api};
use web::settings::Settings;

/// Builds the runtime, then runs the service on it.
///
/// The runtime is built here rather than taken from `#[tokio::main]`, because the
/// attribute offers no way to bound the blocking pool and the bound is the whole
/// point: store calls block, they run on that pool, and a pool whose default
/// ceiling is five hundred and twelve threads turns a database stall into five
/// hundred threads waiting on it. `BLOCKING_THREADS` states the ceiling, with one
/// thread reserved for the readiness probe so an overloaded instance can still
/// say so.
fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .json()
        .with_current_span(false)
        .init();

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .max_blocking_threads(BLOCKING_THREADS)
        .build()
    {
        Ok(runtime) => runtime,
        Err(failure) => {
            tracing::error!(%failure, "cannot start the runtime");
            return ExitCode::FAILURE;
        }
    };

    runtime.block_on(run())
}

/// Reads the environment, builds the service, starts the relay, and serves.
///
/// Every failure here ends the process with a message naming what was wrong. A
/// service that started without a token, or without a database, would answer
/// health checks and fail every real request.
async fn run() -> ExitCode {
    let settings = match Settings::from_environment() {
        Ok(settings) => settings,
        Err(missing) => {
            tracing::error!(%missing, "cannot start");
            return ExitCode::FAILURE;
        }
    };

    let api = match Api::new(&settings) {
        Ok(api) => Arc::new(api),
        Err(failure) => {
            tracing::error!(%failure, "cannot start");
            return ExitCode::FAILURE;
        }
    };

    if let Err(failure) = service::start_relay(&settings) {
        tracing::error!(%failure, "cannot start the outbox relay");
        return ExitCode::FAILURE;
    }

    match service::serve(api, &settings).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(failure) => {
            tracing::error!(%failure, "stopped serving");
            ExitCode::FAILURE
        }
    }
}
