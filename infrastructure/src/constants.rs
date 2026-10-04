//! Every literal this crate carries: the SQL it runs, the columns it reads, and
//! the messages it logs.
//!
//! SQL belongs here rather than inline for the same reason any other literal
//! does. A statement spelled twice drifts, and a column name typed into three
//! stores fails at runtime in whichever one nobody exercised.

/// How many messages one relay pass claims.
pub const OUTBOX_BATCH_SIZE: i64 = 50;

/// Column names the stores read by name rather than by position.
pub mod columns {
    /// A task or signup identifier.
    pub const ID: &str = "id";
    /// A task title.
    pub const TITLE: &str = "title";
    /// Whether a task is done.
    pub const COMPLETED: &str = "completed";
    /// What a task's completion flag was before an update.
    pub const WAS_COMPLETED: &str = "was_completed";
    /// A stock row's name.
    pub const NAME: &str = "name";
    /// How many are left.
    pub const QUANTITY: &str = "quantity";
    /// What the table says about that number.
    pub const STATUS: &str = "status";
    /// An event identifier.
    pub const EVENT_ID: &str = "event_id";
    /// An event's wire name.
    pub const TYPE: &str = "type";
    /// An event's JSON body.
    pub const PAYLOAD: &str = "payload";
    /// When an event happened.
    pub const OCCURRED_AT: &str = "occurred_at";
}

/// The statements this crate runs.
pub mod sql {
    /// Every task, in identifier order.
    pub const LIST_TASKS: &str = "SELECT id, title, completed FROM tasks ORDER BY id";

    /// Records a task and hands back the identifier in the same round trip.
    pub const INSERT_TASK: &str = "INSERT INTO tasks (title, completed) VALUES ($1, false) \
         RETURNING id, title, completed";

    /// Sets a task's completion and reports what it was beforehand.
    ///
    /// The prior value decides whether an event is due, and reading it in a
    /// separate statement would leave a window in which another transaction
    /// changed it. The `FOR UPDATE` in the common table expression takes the row
    /// lock before the update, so the before-and-after pair this returns
    /// describes one atomic step.
    pub const SET_TASK_COMPLETED: &str = "WITH previous AS ( \
           SELECT id, completed FROM tasks WHERE id = $1 FOR UPDATE \
         ) \
         UPDATE tasks SET completed = $2 FROM previous \
         WHERE tasks.id = previous.id \
         RETURNING tasks.id, tasks.title, tasks.completed, \
                   previous.completed AS was_completed";

    /// Removes one task.
    pub const DELETE_TASK: &str = "DELETE FROM tasks WHERE id = $1";

    /// Removes the completed ones.
    pub const DELETE_COMPLETED_TASKS: &str = "DELETE FROM tasks WHERE completed";

    /// Records a signup and hands back its identifier.
    pub const INSERT_SIGNUP: &str = "INSERT INTO signups (full_name, email, plan, seats, notes, created_at) \
         VALUES ($1, $2, $3, $4, $5, $6) RETURNING id";

    /// Every stock row, ordered so the domain's sort has a stable input.
    pub const LIST_INVENTORY: &str = "SELECT name, quantity, status FROM inventory ORDER BY name";

    /// Seeds one stock row, leaving an existing one alone.
    pub const SEED_INVENTORY: &str = "INSERT INTO inventory (name, quantity, status) VALUES ($1, $2, $3) \
         ON CONFLICT (name) DO NOTHING";

    /// Writes one outbox message.
    ///
    /// `$3::text::jsonb` rather than `$3::jsonb`, and the two steps are the whole
    /// point. PostgreSQL resolves a parameter's type from the cast it sits under,
    /// so `$3::jsonb` tells the driver to send a Rust `String` as `jsonb`, which
    /// it will not do: every write of an event failed with a parameter
    /// serialization error, so no event could reach the table at all. Naming
    /// `text` first says what the Rust side actually holds, which is the JSON as
    /// a string, and the second cast is the one the column asks for. The claim
    /// reads it back the same way round, with `payload::text`.
    pub const INSERT_OUTBOX: &str = "INSERT INTO outbox (event_id, type, payload, occurred_at) \
         VALUES ($1, $2, $3::text::jsonb, $4)";

    /// Claims a batch of deliverable messages.
    ///
    /// `FOR UPDATE SKIP LOCKED` is the whole point. A second replica running the
    /// same relay takes a different batch rather than the same rows; without it,
    /// two relays read identical rows and every consumer sees each message
    /// twice. At-least-once delivery tolerates a repeat, but that is no reason to
    /// manufacture one on every pass.
    ///
    /// `payload::text` is not decoration. The column is `jsonb`, the driver
    /// decodes `jsonb` into a JSON value rather than a string, and the row type
    /// reads it as a string: without the cast the first claim of the service's
    /// life panics inside the relay thread.
    ///
    /// `type = ANY($2)` passes the wire names this deployment can resolve. A row
    /// naming anything else is left for a deployment that knows it, and leaving
    /// it in the claim would put it at the head of every ordered batch, so a
    /// backlog of fifty unknown events would starve every valid event behind
    /// them indefinitely.
    ///
    /// `LIMIT` precedes `FOR UPDATE` because PostgreSQL requires that order.
    pub const CLAIM_OUTBOX: &str = "SELECT event_id, type, payload::text AS payload, occurred_at \
         FROM outbox \
         WHERE published_at IS NULL AND quarantined_at IS NULL AND type = ANY($2) \
         ORDER BY occurred_at LIMIT $1 \
         FOR UPDATE SKIP LOCKED";

    /// Marks one message delivered.
    pub const MARK_PUBLISHED: &str = "UPDATE outbox SET published_at = $1 WHERE event_id = $2";

    /// Takes one unreadable message out of the claim without acknowledging it.
    pub const QUARANTINE_MESSAGE: &str =
        "UPDATE outbox SET quarantined_at = $1 WHERE event_id = $2";

    /// Asks the database to answer something trivial.
    pub const PING: &str = "SELECT 1";

    /// Bounds how long a statement waits for a row lock.
    ///
    /// PostgreSQL waits forever by default, which turns contention into a request
    /// that never answers and never errors: no log line, no metric, nothing for
    /// an operator to act on. The task store takes a row lock to read a
    /// completion flag before changing it, so this is reachable whenever two
    /// requests complete the same task at once.
    pub const BOUND_LOCK_WAITS: &str = "SET lock_timeout = '2s'";
}

/// What this crate logs, and what it says when it cannot act.
pub mod messages {
    /// A connection could not be borrowed from the pool.
    pub const NO_CONNECTION: &str = "the database connection pool had nothing to give";
    /// A statement failed.
    pub const QUERY_FAILED: &str = "the database refused a statement";
    /// A statement that should have returned a row returned none.
    pub const ROW_VANISHED: &str = "the row the statement should have returned is absent";
    /// An event could not be written as JSON.
    pub const SERIALIZE_FAILED: &str = "the event could not be written as JSON";
    /// The schema script is missing from the binary.
    pub const SCHEMA_MISSING: &str = "the schema script is not compiled into this binary";
    /// A stored message names a type this deployment does not know.
    pub const UNKNOWN_TYPE: &str = "outbox message names a type this deployment cannot resolve";
    /// A stored message names a known type and carries a body that will not read.
    pub const UNREADABLE_PAYLOAD: &str =
        "outbox message quarantined: its body does not read as the type it names";
    /// A relay pass failed.
    pub const RELAY_FAILED: &str = "the outbox relay pass did not complete";
}
