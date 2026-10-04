//! Picking the handler for a request, and guarding what needs guarding.

use crate::constants::paths;
use crate::problem::Failure;
use crate::requests;
use hyper::Request;

/// Which part of the surface a path belongs to.
///
/// An enum rather than a chain of string comparisons spread through the service,
/// so adding a surface is a variant the match has to handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    /// Liveness or readiness. Open, because a probe carries no token.
    Health,
    /// The task collection or one task.
    Tasks,
    /// The signup collection.
    Signups,
    /// The stock collection.
    Inventory,
}

impl Surface {
    /// Which surface answers a path, if any does.
    #[must_use]
    pub fn of(path: &str) -> Option<Self> {
        match path {
            paths::HEALTH | paths::HEALTH_READY => Some(Self::Health),
            paths::TASKS => Some(Self::Tasks),
            paths::SIGNUPS => Some(Self::Signups),
            paths::INVENTORY => Some(Self::Inventory),
            under if under.starts_with(paths::TASKS_PREFIX) => Some(Self::Tasks),
            _ => None,
        }
    }

    /// Whether reaching this surface requires a token.
    ///
    /// Asked of the surface rather than checked inside each handler. A
    /// per-handler check is one a new handler can forget, and the failure mode of
    /// forgetting is an open endpoint that nothing in the build notices.
    #[must_use]
    pub const fn requires_a_token(self) -> bool {
        match self {
            Self::Health => false,
            Self::Tasks | Self::Signups | Self::Inventory => true,
        }
    }
}

/// Refuses a request that should carry a token and does not.
///
/// The comparison is constant-time over the bytes that are present. A plain
/// equality check leaks, through timing, how much of a guess was right, which
/// turns a brute-force search for the token from infeasible into linear in its
/// length. The length itself is still observable, and that is accepted: a
/// token's length is not the secret.
///
/// # Errors
///
/// Returns [`Failure::Unauthorized`] when no token arrived or the wrong one did.
pub fn authorize<B>(request: &Request<B>, expected: &str) -> Result<(), Failure> {
    let Some(presented) = requests::bearer_token(request) else {
        return Err(Failure::Unauthorized);
    };
    if constant_time_eq(presented.as_bytes(), expected.as_bytes()) {
        Ok(())
    } else {
        Err(Failure::Unauthorized)
    }
}

/// Compares two byte strings without stopping at the first difference.
///
/// Written out rather than taken from a crate, because it is six lines and a
/// reader of a standards repository should be able to see what it does.
fn constant_time_eq(presented: &[u8], expected: &[u8]) -> bool {
    if presented.len() != expected.len() {
        return false;
    }
    let mut difference = 0_u8;
    for (left, right) in presented.iter().zip(expected.iter()) {
        difference |= left ^ right;
    }
    difference == 0
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{Surface, authorize, constant_time_eq};
    use crate::constants::{headers, paths};
    use hyper::Request;

    const TOKEN: &str = "a-token";
    const WRONG_TOKEN: &str = "not-the-token";
    const TRUNCATED: &str = "a-toke";
    const EXTENDED: &str = "a-tokenn";
    const ONE_TASK: &str = "/api/tasks/4";
    const DEEPER: &str = "/api/tasks/4/notes";
    const ELSEWHERE: &str = "/api/nowhere";
    const ROOT: &str = "/";

    fn request(path: &str) -> Request<()> {
        Request::builder().uri(path).body(()).unwrap()
    }

    fn with_authorization(value: &str) -> Request<()> {
        Request::builder()
            .uri(paths::TASKS)
            .header(headers::AUTHORIZATION, value)
            .body(())
            .unwrap()
    }

    #[test]
    fn each_path_belongs_to_the_surface_that_answers_it() {
        assert_eq!(Surface::of(paths::HEALTH), Some(Surface::Health));
        assert_eq!(Surface::of(paths::HEALTH_READY), Some(Surface::Health));
        assert_eq!(Surface::of(paths::TASKS), Some(Surface::Tasks));
        assert_eq!(Surface::of(ONE_TASK), Some(Surface::Tasks));
        assert_eq!(Surface::of(paths::SIGNUPS), Some(Surface::Signups));
        assert_eq!(Surface::of(paths::INVENTORY), Some(Surface::Inventory));
    }

    #[test]
    fn a_path_nobody_serves_belongs_to_no_surface() {
        assert_eq!(Surface::of(ELSEWHERE), None);
        assert_eq!(Surface::of(ROOT), None);
    }

    #[test]
    fn a_subtree_below_one_task_still_belongs_to_tasks_and_the_handler_refuses_it() {
        assert_eq!(
            Surface::of(DEEPER),
            Some(Surface::Tasks),
            "the surface routes; the handler decides the identifier is unreadable"
        );
    }

    #[test]
    fn the_probes_are_open_and_everything_else_is_not() {
        assert!(!Surface::Health.requires_a_token());
        assert!(Surface::Tasks.requires_a_token());
        assert!(Surface::Signups.requires_a_token());
        assert!(Surface::Inventory.requires_a_token());
    }

    #[test]
    fn the_correct_token_is_admitted() {
        let presented = format!("{}{TOKEN}", headers::BEARER_PREFIX);

        assert!(authorize(&with_authorization(&presented), TOKEN).is_ok());
    }

    #[test]
    fn every_wrong_presentation_is_refused() {
        for wrong in [WRONG_TOKEN, TRUNCATED, EXTENDED, ""] {
            let presented = format!("{}{wrong}", headers::BEARER_PREFIX);
            assert!(
                authorize(&with_authorization(&presented), TOKEN).is_err(),
                "{wrong} must not be admitted"
            );
        }
    }

    #[test]
    fn a_token_without_the_scheme_is_refused() {
        assert!(authorize(&with_authorization(TOKEN), TOKEN).is_err());
        assert!(authorize(&request(paths::TASKS), TOKEN).is_err());
    }

    #[test]
    fn the_comparison_answers_correctly_whatever_the_lengths() {
        assert!(constant_time_eq(TOKEN.as_bytes(), TOKEN.as_bytes()));
        assert!(!constant_time_eq(TOKEN.as_bytes(), TRUNCATED.as_bytes()));
        assert!(!constant_time_eq(TOKEN.as_bytes(), EXTENDED.as_bytes()));
        assert!(!constant_time_eq(TOKEN.as_bytes(), WRONG_TOKEN.as_bytes()));
        assert!(constant_time_eq(b"", b""));
    }
}
