//! Moving blocking store work off the runtime's workers, onto a bounded pool.
//!
//! The ports block, deliberately and for the reason the README records. That
//! decision has a cost the first version of this service did not pay: a blocking
//! call made directly inside a handler occupies one of hyper's runtime workers
//! for as long as the database takes, and a runtime worker is shared by every
//! connection assigned to it. One slow query therefore stops unrelated requests
//! from being polled at all, including the readiness probe that would have told
//! the platform what was happening.
//!
//! So store work runs here instead, on tokio's blocking pool, behind a count of
//! permits that matches the connection pool and a wait that gives up rather than
//! growing. Both bounds are stated in `constants`, with the reasoning.

use crate::constants::{STORE_QUEUE_WAIT, STORE_WORKERS, messages};
use crate::problem::Failure;
use application::ports::{StoreError, StoreResult};
use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// A bounded way to run blocking work.
///
/// Built once and shared, because the bound is the point: one of these per
/// request would bound nothing.
#[derive(Debug, Clone)]
pub struct Offload {
    permits: Arc<Semaphore>,
}

impl Default for Offload {
    fn default() -> Self {
        Self::new()
    }
}

impl Offload {
    /// A pool admitting [`STORE_WORKERS`] calls at once.
    #[must_use]
    pub fn new() -> Self {
        Self {
            permits: Arc::new(Semaphore::new(STORE_WORKERS)),
        }
    }

    /// Runs one blocking call on the pool and answers what it answered.
    ///
    /// # Errors
    ///
    /// Returns [`Failure::Busy`] when no worker became free within
    /// [`STORE_QUEUE_WAIT`], and whatever the store refused otherwise.
    pub async fn run<T, F>(&self, work: F) -> Result<T, Failure>
    where
        F: FnOnce() -> StoreResult<T> + Send + 'static,
        T: Send + 'static,
    {
        let permit = self.permit().await?;

        // The permit moves into the blocking task and is released when the work
        // finishes, so a call that outlives its waiter still frees its worker.
        let finished = tokio::task::spawn_blocking(move || {
            let outcome = work();
            drop(permit);
            outcome
        })
        .await;

        match finished {
            Ok(outcome) => outcome.map_err(Failure::from),
            // The blocking task panicked or the runtime is shutting down.
            // Reported as an internal failure rather than as congestion, because
            // retrying will not help and the log line is what an operator needs.
            Err(_) => Err(Failure::Store(StoreError::Unavailable(
                messages::STORE_WORKER_LOST.to_owned(),
            ))),
        }
    }

    async fn permit(&self) -> Result<OwnedSemaphorePermit, Failure> {
        let waiting = Arc::clone(&self.permits).acquire_owned();

        match tokio::time::timeout(STORE_QUEUE_WAIT, waiting).await {
            Ok(Ok(permit)) => Ok(permit),
            // The semaphore is closed, which happens on the way down.
            Ok(Err(_)) => Err(Failure::Store(StoreError::Unavailable(
                messages::STORE_WORKER_LOST.to_owned(),
            ))),
            Err(_) => Err(Failure::Busy),
        }
    }
}

#[cfg(test)]
mod tests {
    // A test asserts by panicking, so the lints that forbid a panic in a service
    // have to be lifted here.
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::Offload;
    use crate::constants::{POOL_SIZE, STORE_QUEUE_WAIT, STORE_WORKERS};
    use crate::problem::Failure;
    use application::ports::StoreError;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    const REFUSED: &str = "the store said no";
    const ANSWER: usize = 42;
    const TIMES_THE_QUEUE_WAIT: u32 = 3;
    const LONGER_THAN_THE_QUEUE_WAIT: Duration =
        STORE_QUEUE_WAIT.saturating_mul(TIMES_THE_QUEUE_WAIT);

    #[test]
    fn the_worker_count_matches_the_connection_pool() {
        assert_eq!(
            u32::try_from(STORE_WORKERS).unwrap(),
            POOL_SIZE,
            "a store worker with no connection to borrow would wait where nothing reports it"
        );
    }

    #[tokio::test]
    async fn the_answer_comes_back_from_the_pool() {
        let answered = Offload::new().run(|| Ok(ANSWER)).await.unwrap();

        assert_eq!(answered, ANSWER);
    }

    #[tokio::test]
    async fn a_refusal_comes_back_as_the_store_wrote_it() {
        let refused = Offload::new()
            .run(|| Err::<(), _>(StoreError::Unavailable(REFUSED.to_owned())))
            .await
            .unwrap_err();

        assert!(matches!(
            refused,
            Failure::Store(StoreError::Unavailable(reason)) if reason == REFUSED
        ));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_request_arriving_at_a_full_pool_is_refused_rather_than_queued() {
        let offload = Offload::new();
        let running = Arc::new(AtomicUsize::new(0));

        // Occupy every worker with work that outlasts the queue wait.
        let occupied: Vec<_> = (0..STORE_WORKERS)
            .map(|_| {
                let pool = offload.clone();
                let counted = Arc::clone(&running);
                tokio::spawn(async move {
                    pool.run(move || {
                        counted.fetch_add(1, Ordering::SeqCst);
                        std::thread::sleep(LONGER_THAN_THE_QUEUE_WAIT);
                        Ok(())
                    })
                    .await
                })
            })
            .collect();

        while running.load(Ordering::SeqCst) < STORE_WORKERS {
            tokio::task::yield_now().await;
        }

        let started_waiting = Instant::now();
        let refused = offload.run(|| Ok(())).await.unwrap_err();

        assert!(
            matches!(refused, Failure::Busy),
            "a full pool answers 503 rather than growing a queue nobody bounded"
        );
        assert!(
            started_waiting.elapsed() < LONGER_THAN_THE_QUEUE_WAIT,
            "the refusal arrives on the queue wait, not when the pool drains"
        );

        for occupied in occupied {
            occupied.await.unwrap().unwrap();
        }
    }
}
