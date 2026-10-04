//! Running the readiness probe under a deadline.

use crate::ports::HealthProbe;
use domain::health::READY_TIMEOUT;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

/// The outcome of a readiness probe.
///
/// The failure arrives as an `Option` rather than a nullable field, so reading
/// it without checking does not compile.
#[derive(Debug)]
pub struct ReadinessCheck {
    /// Whether the dependency answered in time.
    pub ready: bool,
    /// Why it did not, when it did not.
    pub failure: Option<String>,
}

impl ReadinessCheck {
    /// The dependency answered.
    #[must_use]
    pub const fn available() -> Self {
        Self {
            ready: true,
            failure: None,
        }
    }

    /// It did not.
    #[must_use]
    pub const fn unavailable(failure: String) -> Self {
        Self {
            ready: false,
            failure: Some(failure),
        }
    }
}

/// Asks the probe, and gives up after the readiness timeout.
///
/// The service reports the failure rather than logging it, so the logger stays a
/// concern of the layer owning request context, and a test reads the outcome
/// without capturing log output.
///
/// The deadline is enforced by handing the probe to a thread and waiting on a
/// channel. The thread is deliberately not joined on timeout: a blocking probe
/// cannot be interrupted from outside in Rust any more than it can in the
/// siblings, so waiting for it would reintroduce the hang the timeout exists to
/// cut short. It finishes on its own and its answer is dropped.
pub struct HealthService<P> {
    probe: P,
    timeout: Duration,
}

impl<P: HealthProbe + Clone + Send + 'static> HealthService<P> {
    /// Wires the service to a probe, using the domain's readiness timeout.
    pub const fn new(probe: P) -> Self {
        Self {
            probe,
            timeout: READY_TIMEOUT,
        }
    }

    /// Wires the service to a probe with a timeout a test chooses.
    pub const fn with_timeout(probe: P, timeout: Duration) -> Self {
        Self { probe, timeout }
    }

    /// Whether the dependency answered within the timeout.
    #[must_use]
    pub fn check(&self) -> ReadinessCheck {
        let (answered, answer) = mpsc::channel();
        let probe = self.probe.clone();

        thread::spawn(move || {
            let outcome = probe.ping().map_err(|failure| failure.to_string());
            // A send that fails means the waiter already gave up, which is the
            // timeout path and not an error.
            drop(answered.send(outcome));
        });

        match answer.recv_timeout(self.timeout) {
            Ok(Ok(())) => ReadinessCheck::available(),
            Ok(Err(failure)) => ReadinessCheck::unavailable(failure),
            Err(waited) => ReadinessCheck::unavailable(waited.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    // A test asserts by panicking, so the lints that forbid a panic in a service
    // have to be lifted here.
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::HealthService;
    use crate::ports::{HealthProbe, StoreError, StoreResult};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    const DATABASE_DOWN: &str = "connection refused";
    const A_SHORT_DEADLINE: Duration = Duration::from_millis(50);
    const LONGER_THAN_THE_DEADLINE: Duration = Duration::from_millis(2_000);

    #[derive(Clone)]
    struct Answering;

    impl HealthProbe for Answering {
        fn ping(&self) -> StoreResult<()> {
            Ok(())
        }
    }

    #[derive(Clone)]
    struct Refusing;

    impl HealthProbe for Refusing {
        fn ping(&self) -> StoreResult<()> {
            Err(StoreError::Unavailable(DATABASE_DOWN.to_owned()))
        }
    }

    /// Takes longer than any deadline a test sets, and records that it ran.
    #[derive(Clone)]
    struct Hanging {
        finished: Arc<AtomicBool>,
    }

    impl HealthProbe for Hanging {
        fn ping(&self) -> StoreResult<()> {
            std::thread::sleep(LONGER_THAN_THE_DEADLINE);
            self.finished.store(true, Ordering::SeqCst);
            Ok(())
        }
    }

    #[test]
    fn reports_ready_when_the_dependency_answers() {
        let outcome = HealthService::new(Answering).check();

        assert!(outcome.ready);
        assert!(outcome.failure.is_none());
    }

    #[test]
    fn reports_the_reason_the_dependency_gave() {
        let outcome = HealthService::new(Refusing).check();

        assert!(!outcome.ready);
        assert_eq!(outcome.failure.unwrap(), DATABASE_DOWN);
    }

    #[test]
    fn gives_up_on_a_probe_that_outlasts_the_deadline_rather_than_waiting_for_it() {
        let finished = Arc::new(AtomicBool::new(false));
        let probe = Hanging {
            finished: Arc::clone(&finished),
        };

        let outcome = HealthService::with_timeout(probe, A_SHORT_DEADLINE).check();

        assert!(!outcome.ready);
        assert!(outcome.failure.is_some());
        assert!(
            !finished.load(Ordering::SeqCst),
            "check returned before the probe did, which is the whole point of a deadline"
        );
    }
}
