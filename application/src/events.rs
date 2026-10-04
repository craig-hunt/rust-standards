//! Delivering an event to the consumers that want it.

use crate::ports::{EventConsumer, EventDispatcher, StoreResult};
use domain::events::DomainEvent;

/// Hands an event to every consumer that accepts it.
///
/// This is the whole of what a mediator library would sell for this job.
/// Dispatch costs a loop and a trait rather than a dependency with its own
/// release cadence, its own macros, and its own license.
pub struct FanOut {
    consumers: Vec<Box<dyn EventConsumer + Send + Sync>>,
}

impl FanOut {
    /// Registers the consumers, in the order they will be offered events.
    #[must_use]
    pub fn new(consumers: Vec<Box<dyn EventConsumer + Send + Sync>>) -> Self {
        Self { consumers }
    }
}

impl EventDispatcher for FanOut {
    /// Offers the event to each consumer, stopping at the first failure.
    ///
    /// Stopping matters: the relay marks a message delivered only when dispatch
    /// returned successfully, so a consumer that failed leaves the message
    /// pending and the next pass offers it again. Carrying on past a failure
    /// would acknowledge a message one consumer never handled.
    fn dispatch(&self, event: &DomainEvent) -> StoreResult<()> {
        for consumer in &self.consumers {
            if consumer.accepts(event) {
                consumer.consume(event)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    // A test asserts by panicking, so the lints that forbid a panic in a service
    // have to be lifted here. Scoped to the test module, so production code
    // still cannot reach for any of them.
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::FanOut;
    use crate::ports::{EventConsumer, EventDispatcher, StoreError, StoreResult};
    use domain::events::DomainEvent;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::SystemTime;
    use uuid::Uuid;

    const SIGNUP_ID: i64 = 7;
    const TASK_ID: i64 = 4;
    const EMAIL: &str = "ada@example.com";
    const PLAN: &str = "Growth";
    const TITLE: &str = "Read the ADR";
    const SEATS: i32 = 3;
    const REFUSED: &str = "the consumer could not act";
    const ONE_DELIVERY: usize = 1;
    const NOTHING_DELIVERED: usize = 0;

    /// Counts what it accepted, through a handle the test keeps.
    ///
    /// The counter is shared rather than owned, because the dispatcher takes the
    /// consumer by value and a test still has to read what happened.
    struct Counting {
        wants_signups: bool,
        seen: Arc<AtomicUsize>,
    }

    impl Counting {
        fn wanting_signups(wants_signups: bool) -> (Self, Arc<AtomicUsize>) {
            let seen = Arc::new(AtomicUsize::new(NOTHING_DELIVERED));
            (
                Self {
                    wants_signups,
                    seen: Arc::clone(&seen),
                },
                seen,
            )
        }
    }

    impl EventConsumer for Counting {
        fn accepts(&self, event: &DomainEvent) -> bool {
            matches!(event, DomainEvent::SignupRecorded { .. }) == self.wants_signups
        }

        fn consume(&self, _event: &DomainEvent) -> StoreResult<()> {
            self.seen.fetch_add(ONE_DELIVERY, Ordering::Relaxed);
            Ok(())
        }
    }

    /// Refuses everything it is offered.
    struct Refusing;

    impl EventConsumer for Refusing {
        fn accepts(&self, _event: &DomainEvent) -> bool {
            true
        }

        fn consume(&self, _event: &DomainEvent) -> StoreResult<()> {
            Err(StoreError::Unavailable(REFUSED.to_owned()))
        }
    }

    /// Records the order consumers were offered the event.
    struct Recording {
        name: &'static str,
        order: Arc<Mutex<Vec<&'static str>>>,
    }

    impl EventConsumer for Recording {
        fn accepts(&self, _event: &DomainEvent) -> bool {
            true
        }

        fn consume(&self, _event: &DomainEvent) -> StoreResult<()> {
            if let Ok(mut seen) = self.order.lock() {
                seen.push(self.name);
            }
            Ok(())
        }
    }

    fn signup_recorded() -> DomainEvent {
        DomainEvent::SignupRecorded {
            event_id: Uuid::new_v4(),
            occurred_at: SystemTime::now(),
            signup_id: SIGNUP_ID,
            email: EMAIL.to_owned(),
            plan: PLAN.to_owned(),
            seats: SEATS,
        }
    }

    fn task_completed() -> DomainEvent {
        DomainEvent::TaskCompleted {
            event_id: Uuid::new_v4(),
            occurred_at: SystemTime::now(),
            task_id: TASK_ID,
            title: TITLE.to_owned(),
        }
    }

    #[test]
    fn an_event_reaches_the_consumer_that_wants_it_and_no_other() {
        let (signups, signups_seen) = Counting::wanting_signups(true);
        let (tasks, tasks_seen) = Counting::wanting_signups(false);
        let dispatcher = FanOut::new(vec![Box::new(signups), Box::new(tasks)]);

        dispatcher.dispatch(&signup_recorded()).unwrap();

        assert_eq!(signups_seen.load(Ordering::Relaxed), ONE_DELIVERY);
        assert_eq!(tasks_seen.load(Ordering::Relaxed), NOTHING_DELIVERED);
    }

    #[test]
    fn the_other_event_reaches_the_other_consumer() {
        let (signups, signups_seen) = Counting::wanting_signups(true);
        let (tasks, tasks_seen) = Counting::wanting_signups(false);
        let dispatcher = FanOut::new(vec![Box::new(signups), Box::new(tasks)]);

        dispatcher.dispatch(&task_completed()).unwrap();

        assert_eq!(tasks_seen.load(Ordering::Relaxed), ONE_DELIVERY);
        assert_eq!(signups_seen.load(Ordering::Relaxed), NOTHING_DELIVERED);
    }

    #[test]
    fn every_consumer_that_accepts_receives_it_not_merely_the_first() {
        let (first, first_seen) = Counting::wanting_signups(true);
        let (second, second_seen) = Counting::wanting_signups(true);
        let dispatcher = FanOut::new(vec![Box::new(first), Box::new(second)]);

        dispatcher.dispatch(&signup_recorded()).unwrap();

        assert_eq!(first_seen.load(Ordering::Relaxed), ONE_DELIVERY);
        assert_eq!(second_seen.load(Ordering::Relaxed), ONE_DELIVERY);
    }

    #[test]
    fn nothing_listening_is_not_a_failure() {
        FanOut::new(Vec::new())
            .dispatch(&signup_recorded())
            .unwrap();
    }

    #[test]
    fn a_consumer_that_fails_stops_the_dispatch_so_the_message_stays_pending() {
        let (after, after_seen) = Counting::wanting_signups(true);
        let dispatcher = FanOut::new(vec![Box::new(Refusing), Box::new(after)]);

        let outcome = dispatcher.dispatch(&signup_recorded());

        assert!(outcome.is_err(), "the relay must not mark this delivered");
        assert_eq!(
            after_seen.load(Ordering::Relaxed),
            NOTHING_DELIVERED,
            "carrying on would acknowledge a message one consumer never handled"
        );
    }

    #[test]
    fn consumers_are_offered_the_event_in_the_order_they_were_registered() {
        let order = Arc::new(Mutex::new(Vec::new()));
        let dispatcher = FanOut::new(vec![
            Box::new(Recording {
                name: EMAIL,
                order: Arc::clone(&order),
            }),
            Box::new(Recording {
                name: TITLE,
                order: Arc::clone(&order),
            }),
        ]);

        dispatcher.dispatch(&signup_recorded()).unwrap();

        assert_eq!(*order.lock().unwrap(), vec![EMAIL, TITLE]);
    }
}
