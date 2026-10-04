//! Something the domain recorded as having happened.

use std::time::SystemTime;
use uuid::Uuid;

/// An event the service published.
///
/// Every payload carries primitives rather than domain types. An event outlives
/// the process that raised it and may reach a consumer sharing no code with this
/// service, so its wire shape stays independent of the types in here. A typed
/// identifier protects calls inside this process; it would only couple a reader
/// outside it.
///
/// One enum rather than a trait object, so a relay matches exhaustively and a
/// new variant stops the build until it has a wire name and a consumer.
///
/// `occurred_at` is a [`SystemTime`] because that is what the standard library
/// offers. Rendering it as text is the edge's job, which is where a calendar
/// lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainEvent {
    /// A validated signup reached the database.
    SignupRecorded {
        /// Identifies this event, so a consumer can recognize a repeat.
        event_id: Uuid,
        /// When the signup was recorded.
        occurred_at: SystemTime,
        /// The identifier the database assigned.
        signup_id: i64,
        /// The address the signup gave.
        email: String,
        /// The plan it chose.
        plan: String,
        /// How many seats it took.
        seats: i32,
    },
    /// A task was marked completed.
    TaskCompleted {
        /// Identifies this event, so a consumer can recognize a repeat.
        event_id: Uuid,
        /// When the task was completed.
        occurred_at: SystemTime,
        /// Which task.
        task_id: i64,
        /// Its title at the moment it was completed.
        title: String,
    },
}

impl DomainEvent {
    /// The identifier a consumer deduplicates on.
    #[must_use]
    pub const fn event_id(&self) -> Uuid {
        match self {
            Self::SignupRecorded { event_id, .. } | Self::TaskCompleted { event_id, .. } => {
                *event_id
            }
        }
    }

    /// When it happened.
    #[must_use]
    pub const fn occurred_at(&self) -> SystemTime {
        match self {
            Self::SignupRecorded { occurred_at, .. } | Self::TaskCompleted { occurred_at, .. } => {
                *occurred_at
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::DomainEvent;
    use std::time::SystemTime;
    use uuid::Uuid;

    const SIGNUP_ID: i64 = 7;
    const TASK_ID: i64 = 4;
    const EMAIL: &str = "ada@example.com";
    const PLAN: &str = "Growth";
    const SEATS: i32 = 3;
    const TITLE: &str = "Read the ADR";

    fn signup_recorded(event_id: Uuid, occurred_at: SystemTime) -> DomainEvent {
        DomainEvent::SignupRecorded {
            event_id,
            occurred_at,
            signup_id: SIGNUP_ID,
            email: EMAIL.to_owned(),
            plan: PLAN.to_owned(),
            seats: SEATS,
        }
    }

    fn task_completed(event_id: Uuid, occurred_at: SystemTime) -> DomainEvent {
        DomainEvent::TaskCompleted {
            event_id,
            occurred_at,
            task_id: TASK_ID,
            title: TITLE.to_owned(),
        }
    }

    #[test]
    fn every_event_reports_the_identifier_a_consumer_deduplicates_on() {
        let event_id = Uuid::new_v4();
        let occurred_at = SystemTime::now();

        assert_eq!(signup_recorded(event_id, occurred_at).event_id(), event_id);
        assert_eq!(task_completed(event_id, occurred_at).event_id(), event_id);
    }

    #[test]
    fn every_event_reports_when_it_happened() {
        let event_id = Uuid::new_v4();
        let occurred_at = SystemTime::now();

        assert_eq!(
            signup_recorded(event_id, occurred_at).occurred_at(),
            occurred_at
        );
        assert_eq!(
            task_completed(event_id, occurred_at).occurred_at(),
            occurred_at
        );
    }

    #[test]
    fn two_events_with_different_identifiers_are_different_events() {
        let occurred_at = SystemTime::now();

        assert_ne!(
            signup_recorded(Uuid::new_v4(), occurred_at),
            signup_recorded(Uuid::new_v4(), occurred_at)
        );
    }
}
