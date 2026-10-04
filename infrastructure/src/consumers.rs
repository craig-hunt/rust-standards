//! What the service does when it hears one of its own events.

use application::ports::{EventConsumer, StoreResult};
use domain::events::DomainEvent;

/// Notes that a signup happened.
///
/// Logging stands in for whatever a real deployment would do here, and it is
/// deliberately idempotent. The outbox delivers at least once, so a consumer that
/// charged a card or sent a mail would need to recognize a repeat before acting.
#[derive(Debug, Clone, Copy, Default)]
pub struct SignupRecordedConsumer;

impl EventConsumer for SignupRecordedConsumer {
    fn accepts(&self, event: &DomainEvent) -> bool {
        matches!(event, DomainEvent::SignupRecorded { .. })
    }

    fn consume(&self, event: &DomainEvent) -> StoreResult<()> {
        if let DomainEvent::SignupRecorded {
            signup_id,
            plan,
            seats,
            ..
        } = event
        {
            tracing::info!(signup_id, %plan, seats, "signup recorded");
        }
        Ok(())
    }
}

/// Notes that a task was completed.
#[derive(Debug, Clone, Copy, Default)]
pub struct TaskCompletedConsumer;

impl EventConsumer for TaskCompletedConsumer {
    fn accepts(&self, event: &DomainEvent) -> bool {
        matches!(event, DomainEvent::TaskCompleted { .. })
    }

    fn consume(&self, event: &DomainEvent) -> StoreResult<()> {
        if let DomainEvent::TaskCompleted { task_id, title, .. } = event {
            tracing::info!(task_id, %title, "task completed");
        }
        Ok(())
    }
}
