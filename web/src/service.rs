//! The whole wiring, in one readable place.
//!
//! No dependency-injection container. Every adapter is constructed once, in
//! order, where a reader can see which implementation satisfies which port. A
//! container would move those decisions into attributes and a scan, and the
//! first question anybody asks of this repository is exactly the one a container
//! hides.

use crate::constants::{POOL_SIZE, RELAY_INTERVAL_SECONDS, headers, paths};
use crate::offload::Offload;
use crate::problem::Failure;
use crate::responses::{self, Body};
use crate::routing::{self, Surface};
use crate::settings::Settings;
use crate::{endpoints, requests};
use application::health::HealthService;
use application::inventory::InventoryService;
use application::ports::StoreResult;
use application::signups::SignupService;
use application::tasks::TaskService;
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use infrastructure::clock::SystemClock;
use infrastructure::consumers::{SignupRecordedConsumer, TaskCompletedConsumer};
use infrastructure::health::PostgresHealthProbe;
use infrastructure::outbox::Publisher;
use infrastructure::persistence::{self, ConnectionPool};
use infrastructure::stores::{PostgresInventoryStore, PostgresSignupStore, PostgresTaskStore};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;

/// Everything a request needs, built once.
///
/// Held behind an `Arc` because hyper hands each connection to its own task and
/// every task needs the same stores. Cloning the handle is a reference count;
/// cloning the stores would open a second pool.
pub struct Api {
    tasks: Arc<TaskService<PostgresTaskStore<SystemClock>>>,
    signups: Arc<SignupService<PostgresSignupStore<SystemClock>>>,
    inventory: Arc<InventoryService<PostgresInventoryStore>>,
    readiness: Arc<HealthService>,
    /// The bounded pool every store call runs on.
    ///
    /// One per service, because a pool built per request bounds nothing. Each
    /// service is behind its own `Arc` as well: the blocking pool needs an owned
    /// handle to move into a task, and a reference borrowed from this struct
    /// cannot outlive the call that borrowed it.
    offload: Offload,
    token: String,
}

impl Api {
    /// Builds every adapter and hands it to the service that needs it.
    ///
    /// # Errors
    ///
    /// Returns [`application::ports::StoreError`] when the pool cannot be opened.
    pub fn new(settings: &Settings) -> StoreResult<Self> {
        let pool = persistence::open(&settings.database.url, POOL_SIZE)?;
        let clock = SystemClock;

        Ok(Self {
            tasks: Arc::new(TaskService::new(PostgresTaskStore::new(
                pool.clone(),
                clock,
            ))),
            signups: Arc::new(SignupService::new(PostgresSignupStore::new(
                pool.clone(),
                clock,
            ))),
            inventory: Arc::new(InventoryService::new(PostgresInventoryStore::new(
                pool.clone(),
            ))),
            readiness: Arc::new(HealthService::new(PostgresHealthProbe::new(pool))),
            offload: Offload::new(),
            token: settings.api_token.clone(),
        })
    }

    /// Answers one request.
    ///
    /// Authorization is asked of the surface before any handler runs, so a route
    /// added later cannot be published without a token.
    async fn answer(&self, request: Request<Incoming>) -> Result<Response<Body>, Failure> {
        let Some(surface) = Surface::of(request.uri().path()) else {
            return Err(Failure::NoSuchRoute);
        };
        if surface.requires_a_token() {
            routing::authorize(&request, &self.token)?;
        }

        match surface {
            Surface::Health => endpoints::health(&self.readiness, &request).await,
            Surface::Tasks => endpoints::tasks(&self.tasks, &self.offload, request).await,
            Surface::Signups => endpoints::signups(&self.signups, &self.offload, request).await,
            Surface::Inventory => {
                endpoints::inventory(&self.inventory, &self.offload, &request).await
            }
        }
    }

    /// Answers one request, turning any failure into the problem shape.
    ///
    /// The one place a failure becomes a response. A per-handler catch is a
    /// per-handler chance to answer 200 for a failure.
    async fn handle(&self, request: Request<Incoming>) -> Response<Body> {
        let request_id = requests::request_id(&request);
        let method = request.method().clone();
        let path = request.uri().path().to_owned();

        let mut response = match self.answer(request).await {
            Ok(answered) => answered,
            Err(failure) => {
                // Logged here, where the reason is still available, and never
                // sent: the body carries a generic detail.
                tracing::warn!(%request_id, %method, %path, reason = %failure, "request refused");
                responses::problem(&failure)
            }
        };

        if let Ok(value) = request_id.parse() {
            response.headers_mut().insert(headers::REQUEST_ID, value);
        }
        response
    }
}

/// Starts the relay and serves until the process is asked to stop.
///
/// # Errors
///
/// Returns an error when the port cannot be bound.
pub async fn serve(api: Arc<Api>, settings: &Settings) -> std::io::Result<()> {
    let address = SocketAddr::from(([0, 0, 0, 0], settings.port));
    let listener = TcpListener::bind(address).await?;
    tracing::info!(port = settings.port, "listening");

    loop {
        let (stream, _) = listener.accept().await?;
        let served = Arc::clone(&api);

        tokio::spawn(async move {
            let connection = hyper::server::conn::http1::Builder::new().serve_connection(
                TokioIo::new(stream),
                service_fn(move |request| {
                    let answering = Arc::clone(&served);
                    async move { Ok::<_, std::convert::Infallible>(answering.handle(request).await) }
                }),
            );
            if let Err(failure) = connection.await {
                tracing::debug!(%failure, "connection ended");
            }
        });
    }
}

/// Drains the outbox on a timer, on a thread of its own.
///
/// The publisher blocks, because the ports do, so it runs on a thread rather
/// than a task. A blocking call on a runtime worker starves every other request
/// sharing that worker, which is the one mistake this arrangement has to avoid.
///
/// It catches everything a pass can raise. An error escaping the loop would stop
/// the relay silently, and the service would keep serving requests while its
/// backlog grew with nobody draining it.
///
/// # Errors
///
/// Returns [`application::ports::StoreError`] when the pool cannot be opened.
pub fn start_relay(settings: &Settings) -> StoreResult<()> {
    let pool: ConnectionPool = persistence::open(&settings.database.url, POOL_SIZE)?;
    let dispatcher = application::events::FanOut::new(vec![
        Box::new(SignupRecordedConsumer),
        Box::new(TaskCompletedConsumer),
    ]);
    let draining = Publisher::new(pool, dispatcher, SystemClock);
    let pause = Duration::from_secs(RELAY_INTERVAL_SECONDS);

    std::thread::spawn(move || {
        loop {
            std::thread::sleep(pause);
            match draining.publish_pending() {
                Ok(NOTHING_PENDING) => {}
                Ok(published) => tracing::info!(published, "outbox drained"),
                Err(failure) => tracing::error!(
                    %failure,
                    "{}",
                    infrastructure::constants::messages::RELAY_FAILED
                ),
            }
        }
    });

    Ok(())
}

/// A drained pass that found nothing to send.
const NOTHING_PENDING: usize = 0;

/// The root path answers with the same problem shape as any other unknown path.
///
/// Named so the service and its tests agree on what a reader should expect, since
/// nothing routes it.
pub const UNROUTED_EXAMPLE: &str = paths::API_PREFIX;
