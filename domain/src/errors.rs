//! The failures the domain reports, and the codes a client matches on.
//!
//! One enum per boundary, with `thiserror` deriving the message. The Go sibling
//! returns errors and matches them by identity with `errors.Is`; Rust matches
//! them by pattern, which is the same guarantee enforced by the compiler rather
//! than by a convention about sentinel values.
//!
//! Nothing here boxes an error. A boxed error is a failure a caller can only
//! print, and the edge needs to pick a status from it. `anyhow` appears exactly
//! once, at the process boundary in the `web` crate, where printing really is
//! all that is left to do.

use std::collections::BTreeMap;

/// The envelope codes and messages every feature shares.
///
/// Per-feature codes live beside the feature that raises them. These are the
/// ones a client sees for any validation failure, whatever produced it.
pub mod codes {
    /// A body that was not one JSON object with known members.
    pub const INVALID_BODY: &str = "invalid_body";
    /// One or more fields failed validation.
    pub const VALIDATION: &str = "validation_failed";
    /// The service could not complete the request.
    pub const INTERNAL: &str = "internal_error";
}

/// The sentences that accompany [`codes`].
pub mod messages {
    /// Detail for [`super::codes::INVALID_BODY`].
    pub const INVALID_BODY: &str = "request body must hold one JSON object with known fields";
    /// Detail for [`super::codes::VALIDATION`].
    pub const VALIDATION: &str = "one or more fields need attention";
    /// Detail for [`super::codes::INTERNAL`].
    pub const INTERNAL: &str = "the server could not complete the request";
}

/// Problems keyed by the field each one belongs to.
///
/// A `BTreeMap` rather than a `HashMap`, so the order a client receives is the
/// order a reader would expect and two runs produce the same bytes. A response
/// whose member order changes between requests cannot be compared in a test
/// without sorting it first, and that sorting is a step somebody forgets.
pub type FieldProblems = BTreeMap<String, String>;

/// Why the domain refused a value.
///
/// Carries the field it concerns, so the validator can merge several of these
/// into one answer without any sentence being written twice.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{problem}")]
pub struct FieldError {
    /// The member name a client sent.
    pub field: &'static str,
    /// The sentence a form shows next to that input.
    pub problem: &'static str,
}

impl FieldError {
    /// Names one field and the problem with it.
    #[must_use]
    pub const fn new(field: &'static str, problem: &'static str) -> Self {
        Self { field, problem }
    }

    /// This failure as the single-entry map the validator merges.
    #[must_use]
    pub fn into_problems(self) -> FieldProblems {
        let mut problems = FieldProblems::new();
        problems.insert(self.field.to_owned(), self.problem.to_owned());
        problems
    }
}

/// A failure the edge turns into a status and a body.
///
/// Two variants because a status depends on exactly two questions: did the
/// caller send something the rules reject, or did it name something that does
/// not exist. An enum rather than a trait object, so the mapper at the edge
/// matches exhaustively and a third variant stops the build until somebody
/// decides what status it deserves.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DomainError {
    /// The request carried a value the rules reject.
    #[error("{detail}")]
    Invalid {
        /// The stable code a client matches on.
        code: &'static str,
        /// The sentence describing the failure.
        detail: &'static str,
        /// Per-field problems, empty when none apply.
        fields: FieldProblems,
    },
    /// The request named a resource that does not exist.
    #[error("{detail}")]
    NotFound {
        /// The stable code a client matches on.
        code: &'static str,
        /// The sentence describing the failure.
        detail: &'static str,
    },
}

impl DomainError {
    /// A validation failure carrying no per-field problems.
    #[must_use]
    pub fn invalid(code: &'static str, detail: &'static str) -> Self {
        Self::Invalid {
            code,
            detail,
            fields: FieldProblems::new(),
        }
    }

    /// A validation failure carrying the problems a form renders.
    #[must_use]
    pub const fn with_fields(fields: FieldProblems) -> Self {
        Self::Invalid {
            code: codes::VALIDATION,
            detail: messages::VALIDATION,
            fields,
        }
    }

    /// A resource that does not exist.
    #[must_use]
    pub const fn not_found(code: &'static str, detail: &'static str) -> Self {
        Self::NotFound { code, detail }
    }

    /// The stable code, whichever variant this is.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Invalid { code, .. } | Self::NotFound { code, .. } => code,
        }
    }

    /// The per-field problems, empty for a failure that names no field.
    #[must_use]
    pub fn fields(&self) -> FieldProblems {
        match self {
            Self::Invalid { fields, .. } => fields.clone(),
            Self::NotFound { .. } => FieldProblems::new(),
        }
    }
}

impl From<FieldError> for DomainError {
    fn from(failure: FieldError) -> Self {
        Self::with_fields(failure.into_problems())
    }
}

#[cfg(test)]
mod tests {
    // A test asserts by panicking, so the lints that forbid a panic in a service
    // have to be lifted here. Scoped to the test module, so production code still
    // cannot reach for any of them.
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{DomainError, FieldError, codes, messages};

    const FIELD: &str = "title";
    const PROBLEM: &str = "Enter a title.";
    const CODE: &str = "a_code";
    const DETAIL: &str = "a detail";

    /// Inserted out of alphabetical order, so the assertion proves the map
    /// reorders rather than merely happening to be sorted already.
    const LATE_IN_THE_ALPHABET: &str = "zulu";
    const EARLY_IN_THE_ALPHABET: &str = "alpha";
    const ONE_PROBLEM: usize = 1;
    const NO_PROBLEMS: usize = 0;

    #[test]
    fn a_field_failure_becomes_a_single_entry_map() {
        let problems = FieldError::new(FIELD, PROBLEM).into_problems();

        assert_eq!(problems.len(), ONE_PROBLEM);
        assert_eq!(problems.get(FIELD).unwrap(), PROBLEM);
    }

    #[test]
    fn a_field_failure_reads_as_its_problem() {
        assert_eq!(FieldError::new(FIELD, PROBLEM).to_string(), PROBLEM);
    }

    #[test]
    fn a_field_failure_converts_to_the_shared_validation_envelope() {
        let failure: DomainError = FieldError::new(FIELD, PROBLEM).into();

        assert_eq!(failure.code(), codes::VALIDATION);
        assert_eq!(failure.to_string(), messages::VALIDATION);
        assert_eq!(failure.fields().get(FIELD).unwrap(), PROBLEM);
    }

    #[test]
    fn a_validation_failure_without_fields_carries_none() {
        let failure = DomainError::invalid(CODE, DETAIL);

        assert_eq!(failure.code(), CODE);
        assert_eq!(failure.to_string(), DETAIL);
        assert_eq!(failure.fields().len(), NO_PROBLEMS);
    }

    #[test]
    fn a_not_found_failure_carries_no_fields_and_never_will() {
        let failure = DomainError::not_found(CODE, DETAIL);

        assert_eq!(failure.code(), CODE);
        assert_eq!(failure.to_string(), DETAIL);
        assert_eq!(
            failure.fields().len(),
            0,
            "a missing resource has no input to render a message beside"
        );
    }

    #[test]
    fn problems_are_ordered_so_two_runs_produce_the_same_bytes() {
        let mut problems = super::FieldProblems::new();
        problems.insert(LATE_IN_THE_ALPHABET.to_owned(), PROBLEM.to_owned());
        problems.insert(EARLY_IN_THE_ALPHABET.to_owned(), PROBLEM.to_owned());

        let reported = DomainError::with_fields(problems).fields();
        let keys: Vec<&str> = reported.keys().map(String::as_str).collect();

        assert_eq!(
            keys,
            vec!["alpha", "zulu"],
            "insertion order does not reach the wire"
        );
    }
}
