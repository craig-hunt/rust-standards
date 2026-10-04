//! One failure, in the shape RFC 9457 defines.

use crate::constants::{codes, media, titles};
use application::ports::StoreError;
use domain::errors::{DomainError, FieldError, FieldProblems, codes as domain_codes, messages};
use hyper::StatusCode;
use serde::Serialize;

/// The `type` member, when this service has no document to point at.
const TYPE_BLANK: &str = "about:blank";

/// A failure a client can act on.
///
/// `code` and `fields` are the extension members. The Go sibling answers with a
/// bare error object; this adds the envelope and media type the standard defines
/// while keeping the machine-readable parts a client already matches on. That
/// divergence is deliberate and recorded in the README.
///
/// `fields` is omitted when empty rather than written as `{}`, so a client can
/// treat its presence as meaning there are per-field problems to render.
#[derive(Debug, Serialize)]
pub struct Problem {
    /// Always [`TYPE_BLANK`] here.
    pub r#type: &'static str,
    /// The sentence RFC 9457 pairs with the status.
    pub title: &'static str,
    /// The status itself, repeated in the body as the standard requires.
    pub status: u16,
    /// What went wrong.
    pub detail: String,
    /// The stable code a client matches on.
    pub code: String,
    /// Problems keyed by field, absent when none apply.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fields: Option<FieldProblems>,
}

impl Problem {
    /// Builds a problem, treating an empty field map as absent.
    ///
    /// Both ways in normalize, so the contract this type documents holds however
    /// it was built.
    #[must_use]
    pub fn new(
        status: StatusCode,
        title: &'static str,
        code: &str,
        detail: &str,
        fields: FieldProblems,
    ) -> Self {
        Self {
            r#type: TYPE_BLANK,
            title,
            status: status.as_u16(),
            detail: detail.to_owned(),
            code: code.to_owned(),
            fields: if fields.is_empty() {
                None
            } else {
                Some(fields)
            },
        }
    }

    /// A failure carrying no per-field problems.
    #[must_use]
    pub fn of(status: StatusCode, title: &'static str, code: &str, detail: &str) -> Self {
        Self::new(status, title, code, detail, FieldProblems::new())
    }

    /// The status this problem answers with.
    #[must_use]
    pub const fn status_code(&self) -> StatusCode {
        match StatusCode::from_u16(self.status) {
            Ok(status) => status,
            // Unreachable: every status in this type came from a StatusCode.
            // Answering 500 rather than panicking keeps a request alive that a
            // future edit could otherwise kill.
            Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

/// Everything the edge can be asked to answer.
///
/// A single enum so one function maps every failure to a status, and a new
/// variant stops the build there until somebody decides what status it deserves.
/// That is the Rust form of the sealed hierarchy the Java sibling needs a
/// `permits` clause for.
#[derive(Debug, thiserror::Error)]
pub enum Failure {
    /// The domain refused something, or a store did.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// The body was absent, oversized, or not the JSON object the route expects.
    #[error("{}", messages::INVALID_BODY)]
    InvalidBody,
    /// No token, or the wrong one.
    #[error("{}", crate::constants::messages::UNAUTHORIZED)]
    Unauthorized,
    /// No route answers that path.
    #[error("{}", crate::constants::messages::NO_SUCH_ROUTE)]
    NoSuchRoute,
    /// That path does not answer that method.
    #[error("{}", crate::constants::messages::METHOD_NOT_ALLOWED)]
    MethodNotAllowed {
        /// The methods it does answer.
        allowed: String,
    },
    /// A dependency is not answering.
    #[error("{}", crate::constants::messages::NOT_READY)]
    NotReady,
}

impl Failure {
    /// This failure as the body a client receives.
    ///
    /// An unexpected store failure answers with a generic detail. The detail a
    /// client receives says nothing about the internals, because a message naming
    /// a table or a driver tells an attacker more than it tells the caller; the
    /// real reason reaches the log instead.
    #[must_use]
    pub fn to_problem(&self) -> Problem {
        match self {
            Self::Store(StoreError::Domain(DomainError::Invalid {
                code,
                detail,
                fields,
            })) => Problem::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                titles::UNPROCESSABLE,
                code,
                detail,
                fields.clone(),
            ),
            Self::Store(StoreError::Domain(DomainError::NotFound { code, detail })) => {
                Problem::of(StatusCode::NOT_FOUND, titles::NOT_FOUND, code, detail)
            }
            Self::Store(StoreError::Unavailable(_)) => Problem::of(
                StatusCode::INTERNAL_SERVER_ERROR,
                titles::INTERNAL,
                domain_codes::INTERNAL,
                messages::INTERNAL,
            ),
            Self::InvalidBody => Problem::of(
                StatusCode::BAD_REQUEST,
                titles::BAD_REQUEST,
                domain_codes::INVALID_BODY,
                messages::INVALID_BODY,
            ),
            Self::Unauthorized => Problem::of(
                StatusCode::UNAUTHORIZED,
                titles::UNAUTHORIZED,
                codes::UNAUTHORIZED,
                crate::constants::messages::UNAUTHORIZED,
            ),
            Self::NoSuchRoute => Problem::of(
                StatusCode::NOT_FOUND,
                titles::NOT_FOUND,
                codes::NOT_FOUND,
                crate::constants::messages::NO_SUCH_ROUTE,
            ),
            Self::MethodNotAllowed { .. } => Problem::of(
                StatusCode::METHOD_NOT_ALLOWED,
                titles::METHOD_NOT_ALLOWED,
                codes::METHOD_NOT_ALLOWED,
                crate::constants::messages::METHOD_NOT_ALLOWED,
            ),
            Self::NotReady => Problem::of(
                StatusCode::SERVICE_UNAVAILABLE,
                titles::UNAVAILABLE,
                domain::health::constants::STATUS_UNAVAILABLE,
                crate::constants::messages::NOT_READY,
            ),
        }
    }

    /// The media type a problem is written as.
    #[must_use]
    pub const fn media_type() -> &'static str {
        media::PROBLEM_JSON
    }
}

impl From<DomainError> for Failure {
    fn from(refused: DomainError) -> Self {
        Self::Store(StoreError::Domain(refused))
    }
}

impl From<FieldError> for Failure {
    fn from(refused: FieldError) -> Self {
        Self::Store(StoreError::Domain(refused.into()))
    }
}
