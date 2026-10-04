//! Recording a signup that already passed validation.

use crate::ports::{SignupStore, StoreResult};
use domain::signups::{Signup, SignupConfirmation};

/// Validation stays in the domain and runs before this call, so the service
/// cannot receive a signup with a missing email or a plan nobody offers. The
/// type system says so: a [`Signup`] cannot be built out of an invalid request.
pub struct SignupService<S> {
    store: S,
}

impl<S: SignupStore> SignupService<S> {
    /// Wires the service to a store.
    pub const fn new(store: S) -> Self {
        Self { store }
    }

    /// Records the signup and reads it back.
    ///
    /// # Errors
    ///
    /// Returns whatever the store reported.
    pub fn create(&self, signup: &Signup) -> StoreResult<SignupConfirmation> {
        let id = self.store.save(signup)?;
        Ok(SignupConfirmation {
            id,
            summary: signup.summary(),
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::SignupService;
    use crate::ports::{SignupStore, StoreResult};
    use domain::signups::{Signup, SignupId, SignupRequest, constants, validate};
    use std::cell::RefCell;

    const NAME: &str = "Ada Lovelace";
    const EMAIL: &str = "ada@example.com";
    const NOTE: &str = "Platform team first.";
    const SEATS: i32 = 3;
    const ASSIGNED_ID: i64 = 12;
    const EXPECTED_SUMMARY: &str = "Ada Lovelace on the Growth plan, 3 seat(s).";
    const ONE_SAVE: usize = 1;

    /// Records what it was asked to save and answers with a fixed identifier.
    struct Capturing {
        saved: RefCell<Vec<Signup>>,
    }

    impl Capturing {
        fn new() -> Self {
            Self {
                saved: RefCell::new(Vec::new()),
            }
        }
    }

    impl SignupStore for Capturing {
        fn save(&self, signup: &Signup) -> StoreResult<SignupId> {
            self.saved.borrow_mut().push(signup.clone());
            Ok(SignupId::new(ASSIGNED_ID)?)
        }
    }

    fn signup() -> Signup {
        validate(&SignupRequest {
            full_name: NAME.to_owned(),
            email: EMAIL.to_owned(),
            plan: constants::PLAN_GROWTH.to_owned(),
            seats: Some(SEATS),
            notes: NOTE.to_owned(),
            accept_terms: true,
        })
        .unwrap()
    }

    #[test]
    fn the_confirmation_carries_the_identifier_the_store_assigned() {
        let store = Capturing::new();

        let confirmation = SignupService::new(&store).create(&signup()).unwrap();

        assert_eq!(confirmation.id.value(), ASSIGNED_ID);
        assert_eq!(confirmation.summary, EXPECTED_SUMMARY);
        assert_eq!(store.saved.borrow().len(), ONE_SAVE);
    }

    #[test]
    fn the_signup_reaching_the_store_is_the_one_that_was_validated() {
        let store = Capturing::new();

        SignupService::new(&store).create(&signup()).unwrap();

        assert_eq!(store.saved.borrow()[0], signup());
    }
}
