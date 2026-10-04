//! Writing a response: the body, the media type, and the length.

use crate::constants::{headers, media};
use crate::problem::{Failure, Problem};
use http_body_util::Full;
use hyper::body::Bytes;
use hyper::{Response, StatusCode, header};
use serde::Serialize;

/// The body type every response in this service uses.
pub type Body = Full<Bytes>;

/// What a handler answers.
pub type Answer = Result<Response<Body>, Failure>;

/// A JSON response, with its media type and its exact length.
///
/// # Errors
///
/// Never, in practice: every type this service serializes is a struct of
/// primitives it owns. A serialization failure is reported as an internal error
/// rather than panicking, so one unexpected shape cannot take the process down.
pub fn json<T: Serialize>(status: StatusCode, body: &T) -> Answer {
    let rendered = serde_json::to_vec(body).map_err(|failure| {
        Failure::Store(application::ports::StoreError::Unavailable(
            failure.to_string(),
        ))
    })?;
    build(status, media::JSON, rendered)
}

/// A response with no body at all, rather than a body of length zero.
///
/// # Errors
///
/// Never. The signature matches [`json`] so a handler can return either.
pub fn empty(status: StatusCode) -> Answer {
    build(status, media::JSON, Vec::new())
}

/// A failure, in the problem shape.
#[must_use]
pub fn problem(failure: &Failure) -> Response<Body> {
    let rendered = failure.to_problem();
    let status = rendered.status_code();
    let allowed = match failure {
        Failure::MethodNotAllowed { allowed } => Some(allowed.clone()),
        _ => None,
    };

    let body = serde_json::to_vec(&rendered).unwrap_or_else(|_| fallback(&rendered));
    let mut response = Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, Failure::media_type());

    if matches!(failure, Failure::Unauthorized) {
        response = response.header(headers::WWW_AUTHENTICATE, headers::BEARER_CHALLENGE);
    }
    if let Some(methods) = allowed {
        response = response.header(headers::ALLOW, methods);
    }

    response
        .body(Full::new(Bytes::from(body)))
        .unwrap_or_else(|_| {
            let mut bare = Response::new(Full::new(Bytes::new()));
            *bare.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
            bare
        })
}

/// The body to send when even the problem could not be serialized.
///
/// A map of owned strings cannot fail to serialize, so this is unreachable. It
/// exists because the alternative is unwrapping, and a panic at the edge takes
/// down a connection that could still have been answered.
fn fallback(rendered: &Problem) -> Vec<u8> {
    rendered.detail.clone().into_bytes()
}

fn build(status: StatusCode, content_type: &str, body: Vec<u8>) -> Answer {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, content_type)
        .body(Full::new(Bytes::from(body)))
        .map_err(|failure| {
            Failure::Store(application::ports::StoreError::Unavailable(
                failure.to_string(),
            ))
        })
}
