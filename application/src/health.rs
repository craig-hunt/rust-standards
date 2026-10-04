//! Running the readiness probe under a deadline, on a pool that cannot grow.

use crate::ports::HealthProbe;
use domain::health::READY_TIMEOUT;
use std::sync::mpsc::{self, Receiver, Sender, SyncSender, TrySendError};
use std::thread;
use std::time::Duration;

/// What this service says when it could not even ask.
mod messages {
    /// A probe is running and another already waits behind it.
    pub(super) const PROBE_QUEUE_FULL: &str = "a readiness probe is already waiting its turn";
    /// The worker is gone, which means the process is on its way down.
    pub(super) const PROBE_STOPPED: &str = "the readiness probe worker has stopped";
}

/// How many probes may wait for the worker before the service refuses to queue
/// another.
///
/// One. A probe that has not answered is a dependency that is not answering, and
/// the answer to the next caller does not improve by making it wait longer. The
/// bound is what makes the refusal immediate instead of unbounded: a queue
/// nobody caps turns a database hang into a pile of waiters, each holding a
/// connection open, which is the shape of the incident the deadline exists to
/// prevent.
const QUEUED_PROBES: usize = 1;

/// The name the worker thread carries in a backtrace and a thread dump.
const WORKER: &str = "readiness-probe";

/// What one probe answered.
type ProbeOutcome = Result<(), String>;

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
/// The deadline is enforced by handing the probe to one worker thread and waiting
/// on a channel. The worker is started once, at construction, and never grows: an
/// earlier version spawned a thread per request, which meant an unauthenticated
/// probe path could create a thread per call, and a hanging database turned
/// repeated probes into thread exhaustion. The queue in front of the worker is
/// bounded, so a caller arriving while a probe hangs is told so immediately
/// rather than waiting behind an unbounded line.
///
/// The worker is deliberately not interrupted on timeout: a blocking probe cannot
/// be cancelled from outside in Rust any more than it can in the siblings, so
/// waiting for it would reintroduce the hang the timeout exists to cut short. It
/// finishes on its own, its answer is dropped, and it takes the next request.
pub struct HealthService {
    asking: SyncSender<Sender<ProbeOutcome>>,
    timeout: Duration,
}

impl HealthService {
    /// Wires the service to a probe, using the domain's readiness timeout.
    pub fn new<P: HealthProbe + Send + 'static>(probe: P) -> Self {
        Self::with_timeout(probe, READY_TIMEOUT)
    }

    /// Wires the service to a probe with a timeout a test chooses.
    pub fn with_timeout<P: HealthProbe + Send + 'static>(probe: P, timeout: Duration) -> Self {
        let (asking, asked) = mpsc::sync_channel(QUEUED_PROBES);
        // Named rather than anonymous, so a thread dump taken during an incident
        // says which thread is sitting in the database driver.
        let worker = thread::Builder::new().name(WORKER.to_owned());
        drop(worker.spawn(move || answer_probes(&probe, &asked)));

        Self { asking, timeout }
    }

    /// Whether the dependency answered within the timeout.
    #[must_use]
    pub fn check(&self) -> ReadinessCheck {
        let (answered, answer) = mpsc::channel();

        match self.asking.try_send(answered) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                return ReadinessCheck::unavailable(messages::PROBE_QUEUE_FULL.to_owned());
            }
            Err(TrySendError::Disconnected(_)) => {
                return ReadinessCheck::unavailable(messages::PROBE_STOPPED.to_owned());
            }
        }

        match answer.recv_timeout(self.timeout) {
            Ok(Ok(())) => ReadinessCheck::available(),
            Ok(Err(failure)) => ReadinessCheck::unavailable(failure),
            Err(waited) => ReadinessCheck::unavailable(waited.to_string()),
        }
    }
}

/// Answers probe requests until the service that owns the channel is dropped.
///
/// A send that fails means the waiter already gave up, which is the timeout path
/// and not an error.
fn answer_probes<P: HealthProbe>(probe: &P, asked: &Receiver<Sender<ProbeOutcome>>) {
    while let Ok(reply) = asked.recv() {
        let outcome = probe.ping().map_err(|failure| failure.to_string());
        drop(reply.send(outcome));
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
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::time::Duration;

    const DATABASE_DOWN: &str = "connection refused";
    const A_SHORT_DEADLINE: Duration = Duration::from_millis(50);
    const LONGER_THAN_THE_DEADLINE: Duration = Duration::from_millis(2_000);
    const CALLS_WHILE_THE_FIRST_HANGS: usize = 3;
    const PROBES_THE_WORKER_STARTED: usize = 1;

    struct Answering;

    impl HealthProbe for Answering {
        fn ping(&self) -> StoreResult<()> {
            Ok(())
        }
    }

    struct Refusing;

    impl HealthProbe for Refusing {
        fn ping(&self) -> StoreResult<()> {
            Err(StoreError::Unavailable(DATABASE_DOWN.to_owned()))
        }
    }

    /// Takes longer than any deadline a test sets, and counts its calls.
    struct Hanging {
        started: Arc<AtomicUsize>,
        finished: Arc<AtomicBool>,
    }

    impl HealthProbe for Hanging {
        fn ping(&self) -> StoreResult<()> {
            self.started.fetch_add(1, Ordering::SeqCst);
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
        let started = Arc::new(AtomicUsize::new(0));
        let finished = Arc::new(AtomicBool::new(false));
        let probe = Hanging {
            started: Arc::clone(&started),
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

    #[test]
    fn repeated_calls_during_a_hang_start_no_further_probes() {
        let started = Arc::new(AtomicUsize::new(0));
        let probe = Hanging {
            started: Arc::clone(&started),
            finished: Arc::new(AtomicBool::new(false)),
        };
        let service = HealthService::with_timeout(probe, A_SHORT_DEADLINE);

        for _ in 0..CALLS_WHILE_THE_FIRST_HANGS {
            assert!(!service.check().ready);
        }

        assert_eq!(
            started.load(Ordering::SeqCst),
            PROBES_THE_WORKER_STARTED,
            "one worker runs the probes, so a hang costs one thread however many callers arrive"
        );
    }
}
