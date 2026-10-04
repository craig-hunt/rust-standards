//! The persistence and the clock this layer needs somebody else to supply.
//!
//! A port lives beside its caller rather than beside its implementation, so
//! `infrastructure` depends on `application` and never the reverse. A store that
//! grows a method no feature calls has grown it for the wrong reason.
//!
//! Every method is synchronous and returns a `Result`. The siblings differ here
//! and the reasoning is recorded in the README: a Rust service that wanted async
//! ports would have to choose a runtime in this crate and put `async_trait` or a
//! lifetime-bound future in every signature, which is a framework decision
//! reaching into the layer that exists to be free of them. Blocking work moves
//! to a thread the runtime sets aside for it, at the edge, where the runtime is
//! already a fact.

use domain::errors::DomainError;
use domain::events::DomainEvent;
use domain::inventory::InventoryItem;
use domain::signups::{Signup, SignupId};
use domain::tasks::{TaskId, TaskItem, TaskTitle};
use std::time::SystemTime;

/// A failure a port reports.
///
/// Either the domain refused something, or the dependency did. Keeping the two
/// apart is what lets the edge answer 422 for the first and 500 for the second
/// without inspecting a message.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// The domain refused a value the store read or was asked to write.
    #[error(transparent)]
    Domain(#[from] DomainError),
    /// The dependency itself failed.
    #[error("{0}")]
    Unavailable(String),
}

/// What a port answers.
pub type StoreResult<T> = Result<T, StoreError>;

/// The persistence the task feature calls, and nothing more.
pub trait TaskStore {
    /// Every task, in a stable order.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Unavailable`] when the dependency fails, or
    /// [`StoreError::Domain`] when a value it read or was asked to write is one
    /// the rules refuse.
    fn list(&self) -> StoreResult<Vec<TaskItem>>;

    /// Records a new task, incomplete.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Unavailable`] when the dependency fails, or
    /// [`StoreError::Domain`] when a value it read or was asked to write is one
    /// the rules refuse.
    fn create(&self, title: &TaskTitle) -> StoreResult<TaskItem>;

    /// Sets completion, announcing the change when a task becomes completed.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Unavailable`] when the dependency fails, or
    /// [`StoreError::Domain`] when a value it read or was asked to write is one
    /// the rules refuse.
    ///
    /// Returns [`StoreError::Domain`] carrying a not-found failure when no task
    /// has that identifier.
    fn set_completed(&self, id: TaskId, completed: bool) -> StoreResult<TaskItem>;

    /// Removes one task.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Unavailable`] when the dependency fails, or
    /// [`StoreError::Domain`] when a value it read or was asked to write is one
    /// the rules refuse.
    ///
    /// Returns [`StoreError::Domain`] carrying a not-found failure when no task
    /// has that identifier.
    fn delete(&self, id: TaskId) -> StoreResult<()>;

    /// Removes the completed ones, answering how many went.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Unavailable`] when the dependency fails, or
    /// [`StoreError::Domain`] when a value it read or was asked to write is one
    /// the rules refuse.
    fn delete_completed(&self) -> StoreResult<u64>;
}

/// Records a validated signup and answers with the identifier it received.
pub trait SignupStore {
    /// Writes the signup and the event announcing it in one transaction.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Unavailable`] when the dependency fails, or
    /// [`StoreError::Domain`] when a value it read or was asked to write is one
    /// the rules refuse.
    fn save(&self, signup: &Signup) -> StoreResult<SignupId>;
}

/// Reads the stock rows.
///
/// Searching and ordering stay in the domain, so a second store implementation
/// cannot quietly answer with a different order.
pub trait InventoryStore {
    /// Every row, ordered so the domain's sort has a stable input.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Unavailable`] when the dependency fails, or
    /// [`StoreError::Domain`] when a value it read or was asked to write is one
    /// the rules refuse.
    fn items(&self) -> StoreResult<Vec<InventoryItem>>;
}

/// Confirms that the backing database answers.
///
/// It returns nothing on success and an error otherwise, so an implementation
/// cannot report trouble by answering false and leaving the reason behind.
pub trait HealthProbe {
    /// Completes a round trip, or says why it could not.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Unavailable`] when the database did not answer.
    fn ping(&self) -> StoreResult<()>;
}

/// Reacts to an event the service published.
///
/// A consumer must absorb repeats. The outbox delivers at least once: a relay
/// can publish a message and fail before recording that it did, and the next
/// pass sends it again. A consumer assuming exactly-once delivery will
/// double-charge, double-mail, or double-count the first time that happens.
pub trait EventConsumer {
    /// Whether this consumer wants the event.
    fn accepts(&self, event: &DomainEvent) -> bool;

    /// Acts on it.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Unavailable`] when the consumer could not act. A
    /// relay leaves the message pending rather than marking it delivered, so the
    /// next pass offers it again.
    fn consume(&self, event: &DomainEvent) -> StoreResult<()>;
}

/// Delivers an event to the consumers that want it.
pub trait EventDispatcher {
    /// Hands the event to every consumer that accepts it.
    ///
    /// # Errors
    ///
    /// Returns the first failure a consumer reported, leaving the rest unoffered
    /// so the message stays pending.
    fn dispatch(&self, event: &DomainEvent) -> StoreResult<()>;
}

/// The current time, as a dependency rather than a call to the platform.
///
/// An event carries the moment it happened. Reading that from the platform
/// directly leaves no way to assert the value a test expects.
pub trait Clock {
    /// Now.
    fn now(&self) -> SystemTime;
}

// A port implemented for a shared reference, so a test can hand a service a
// borrow of a fake it still wants to read afterwards. The alternative is wrapping
// every fake in an Arc or a Rc, which puts a reference count in a test to work
// around an ownership rule rather than to express anything.
//
// Written out per trait rather than derived, because Rust has no way to say
// "every trait in this module also applies to a reference to its implementor".

impl<T: TaskStore> TaskStore for &T {
    fn list(&self) -> StoreResult<Vec<TaskItem>> {
        (*self).list()
    }

    fn create(&self, title: &TaskTitle) -> StoreResult<TaskItem> {
        (*self).create(title)
    }

    fn set_completed(&self, id: TaskId, completed: bool) -> StoreResult<TaskItem> {
        (*self).set_completed(id, completed)
    }

    fn delete(&self, id: TaskId) -> StoreResult<()> {
        (*self).delete(id)
    }

    fn delete_completed(&self) -> StoreResult<u64> {
        (*self).delete_completed()
    }
}

impl<T: SignupStore> SignupStore for &T {
    fn save(&self, signup: &Signup) -> StoreResult<SignupId> {
        (*self).save(signup)
    }
}

impl<T: InventoryStore> InventoryStore for &T {
    fn items(&self) -> StoreResult<Vec<InventoryItem>> {
        (*self).items()
    }
}

impl<T: HealthProbe> HealthProbe for &T {
    fn ping(&self) -> StoreResult<()> {
        (*self).ping()
    }
}

impl<T: EventConsumer> EventConsumer for &T {
    fn accepts(&self, event: &DomainEvent) -> bool {
        (*self).accepts(event)
    }

    fn consume(&self, event: &DomainEvent) -> StoreResult<()> {
        (*self).consume(event)
    }
}

impl<T: Clock> Clock for &T {
    fn now(&self) -> SystemTime {
        (*self).now()
    }
}
