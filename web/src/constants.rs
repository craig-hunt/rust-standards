//! Every literal the HTTP surface carries, named once.

/// The paths this service answers.
pub mod paths {
    /// Everything behind the bearer token.
    pub const API_PREFIX: &str = "/api/";
    /// The task collection.
    pub const TASKS: &str = "/api/tasks";
    /// One task, with its identifier appended.
    pub const TASKS_PREFIX: &str = "/api/tasks/";
    /// The signup collection.
    pub const SIGNUPS: &str = "/api/signups";
    /// The stock collection.
    pub const INVENTORY: &str = "/api/inventory";
    /// Liveness.
    pub const HEALTH: &str = "/health";
    /// Readiness.
    pub const HEALTH_READY: &str = "/health/ready";
}

/// The query parameters this service reads.
pub mod query {
    /// Which tasks to show.
    pub const FILTER: &str = "filter";
    /// What a stock name must contain.
    pub const SEARCH: &str = "search";
    /// Which column orders the rows.
    pub const SORT: &str = "sort";
    /// Which way that order runs.
    pub const DIRECTION: &str = "direction";
}

/// The methods this service answers.
///
/// Named because a route match on a bare string is a route match that a typo
/// turns into a silent 405.
pub mod methods {
    /// Reads.
    pub const GET: &str = "GET";
    /// Creates.
    pub const POST: &str = "POST";
    /// Changes part of something.
    pub const PATCH: &str = "PATCH";
    /// Removes.
    pub const DELETE: &str = "DELETE";
}

/// The headers this service reads and writes.
pub mod headers {
    /// Correlates a request across log lines.
    pub const REQUEST_ID: &str = "x-request-id";
    /// Carries the bearer token.
    pub const AUTHORIZATION: &str = "authorization";
    /// Tells a client how to authenticate.
    pub const WWW_AUTHENTICATE: &str = "www-authenticate";
    /// The scheme this service accepts.
    pub const BEARER_CHALLENGE: &str = "Bearer";
    /// The prefix a token arrives behind.
    pub const BEARER_PREFIX: &str = "Bearer ";
    /// The methods a path answers, when it did not answer this one.
    pub const ALLOW: &str = "allow";
}

/// The media types this service writes.
pub mod media {
    /// A successful answer.
    pub const JSON: &str = "application/json";
    /// A failure, as RFC 9457 describes it.
    pub const PROBLEM_JSON: &str = "application/problem+json";
}

/// The titles RFC 9457 pairs with each status.
pub mod titles {
    /// 400.
    pub const BAD_REQUEST: &str = "Bad Request";
    /// 401.
    pub const UNAUTHORIZED: &str = "Unauthorized";
    /// 404.
    pub const NOT_FOUND: &str = "Not Found";
    /// 405.
    pub const METHOD_NOT_ALLOWED: &str = "Method Not Allowed";
    /// 422.
    pub const UNPROCESSABLE: &str = "Unprocessable Content";
    /// 500.
    pub const INTERNAL: &str = "Internal Server Error";
    /// 503.
    pub const UNAVAILABLE: &str = "Service Unavailable";
}

/// Codes this layer raises, as opposed to the ones the domain does.
pub mod codes {
    /// No token, or the wrong one.
    pub const UNAUTHORIZED: &str = "unauthorized";
    /// No route answers that path.
    pub const NOT_FOUND: &str = "not_found";
    /// That path does not answer that method.
    pub const METHOD_NOT_ALLOWED: &str = "method_not_allowed";
    /// Every store worker is busy and the queue wait ran out.
    pub const BUSY: &str = "busy";
}

/// The sentences this layer writes.
pub mod messages {
    /// Detail for [`codes::UNAUTHORIZED`].
    pub const UNAUTHORIZED: &str = "a valid bearer token is required";
    /// Detail for [`codes::NOT_FOUND`].
    pub const NO_SUCH_ROUTE: &str = "no route answers that path";
    /// Detail for [`codes::METHOD_NOT_ALLOWED`].
    pub const METHOD_NOT_ALLOWED: &str = "that path does not answer that method";
    /// Detail a readiness probe receives.
    pub const NOT_READY: &str = "readiness check failed";
    /// Detail for [`codes::BUSY`].
    pub const BUSY: &str = "the service is at capacity, so this request was not started";
    /// What the log says when a blocking store task did not come back.
    pub const STORE_WORKER_LOST: &str = "a store call did not return from the blocking pool";
}

/// The environment variables this service reads.
pub mod environment {
    /// Which port to listen on.
    pub const PORT: &str = "PORT";
    /// How to reach the database.
    pub const DATABASE_URL: &str = "DATABASE_URL";
    /// The token `/api` expects.
    pub const API_TOKEN: &str = "API_TOKEN";
}

/// The largest request body this service reads.
///
/// Reading to the end of a stream is an invitation to send one that never ends,
/// and the server would hold memory until it died rather than answering 400.
pub const MAX_BODY_BYTES: usize = 1_048_576;

/// The longest client-supplied request identifier this service echoes.
pub const MAX_REQUEST_ID_LENGTH: usize = 64;

/// How many connections the pool keeps.
pub const POOL_SIZE: u32 = 8;

/// How many store calls may occupy the blocking pool at once.
///
/// The same number as the connection pool, and a test asserts the two agree. A
/// ninth concurrent store call could not get a connection anyway, so admitting
/// it would only move the wait from a queue this service can see into the pool's
/// internal one, where nothing reports it.
pub const STORE_WORKERS: usize = 8;

/// How long a readiness probe may hold a blocking thread while it waits.
///
/// One, reserved. The probe's own work runs on a thread the application layer
/// owns; this is the thread that sits waiting for its answer. Reserving it is
/// what keeps a readiness check answerable while every store worker is busy,
/// which is the difference between a platform seeing an overloaded instance and
/// a platform seeing a dead one.
pub const PROBE_WORKERS: usize = 1;

/// How many blocking threads the runtime is allowed.
///
/// Stated rather than left at the default, because the default is five hundred
/// and twelve: a bound that large is a bound in name only, and the point of
/// moving blocking work off the runtime is to keep its cost countable.
pub const BLOCKING_THREADS: usize = STORE_WORKERS + PROBE_WORKERS;

/// How long a request waits for a store worker before the service refuses it.
///
/// A request that cannot start within a second during a spike is better refused
/// than queued: the caller's own timeout is usually shorter than the queue it
/// would join, so the work would complete for nobody while holding a connection
/// that another request could have used.
pub const STORE_QUEUE_WAIT: std::time::Duration = std::time::Duration::from_secs(1);

/// How often the relay drains the outbox.
pub const RELAY_INTERVAL_SECONDS: u64 = 5;
