//! Reading a request: its query, its body, and its identifier.

use crate::constants::{MAX_BODY_BYTES, MAX_REQUEST_ID_LENGTH, headers};
use crate::problem::Failure;
use http_body_util::BodyExt;
use hyper::body::Incoming;
use hyper::{Request, header};
use serde::de::DeserializeOwned;
use std::collections::BTreeMap;
use uuid::Uuid;

/// The query string, read once into a map.
///
/// It decodes percent-escapes, splitting before decoding. Decoding the whole
/// string first and splitting after would let an encoded separator inside a
/// value split into extra parameters, so a search term containing an ampersand
/// would arrive as two.
#[derive(Debug, Default)]
pub struct Query {
    parameters: BTreeMap<String, String>,
}

impl Query {
    const PAIR_SEPARATOR: char = '&';
    const VALUE_SEPARATOR: char = '=';
    const ESCAPE: char = '%';
    const PLUS: char = '+';
    const SPACE: char = ' ';
    const HEX_RADIX: u32 = 16;
    const ESCAPE_WIDTH: usize = 2;

    /// Reads the query string of a request.
    #[must_use]
    pub fn of<B>(request: &Request<B>) -> Self {
        let mut parameters = BTreeMap::new();
        if let Some(raw) = request.uri().query() {
            for pair in raw.split(Self::PAIR_SEPARATOR) {
                let (name, value) = pair.split_once(Self::VALUE_SEPARATOR).unwrap_or((pair, ""));
                parameters.insert(Self::decode(name), Self::decode(value));
            }
        }
        Self { parameters }
    }

    /// A parameter's value, absent when the client sent none.
    ///
    /// Absent is what the domain's parsers read as "use the default", so this
    /// returns `None` rather than an empty string for a parameter nobody sent.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&str> {
        self.parameters.get(name).map(String::as_str)
    }

    /// Decodes percent-escapes and the plus-for-space convention.
    ///
    /// Written out rather than taken from a crate. It is fifteen lines, the
    /// alternative is a dependency at the edge for one function, and a reader can
    /// see exactly what is accepted.
    fn decode(encoded: &str) -> String {
        let mut decoded = String::with_capacity(encoded.len());
        let mut characters = encoded.chars();
        while let Some(character) = characters.next() {
            match character {
                Self::PLUS => decoded.push(Self::SPACE),
                Self::ESCAPE => {
                    let hex: String = characters.by_ref().take(Self::ESCAPE_WIDTH).collect();
                    if let Ok(byte) = u8::from_str_radix(&hex, Self::HEX_RADIX) {
                        decoded.push(char::from(byte));
                    } else {
                        // A malformed escape is kept verbatim rather than
                        // dropped, so a search term reaches the domain as the
                        // client typed it and the domain decides.
                        decoded.push(Self::ESCAPE);
                        decoded.push_str(&hex);
                    }
                }
                other => decoded.push(other),
            }
        }
        decoded
    }
}

/// Reads a request body as JSON.
///
/// The read is capped. Unknown members fail rather than being ignored: a client
/// misspelling `completed` would otherwise get a 200 and no change, with no way
/// to discover the typo.
///
/// # Errors
///
/// Returns [`Failure::InvalidBody`] for a body that is oversized, not JSON, not
/// the shape the route expects, or carrying a member the route does not know.
pub async fn read_json<T: DeserializeOwned>(request: Request<Incoming>) -> Result<T, Failure> {
    if oversized(&request) {
        return Err(Failure::InvalidBody);
    }

    let collected = request
        .into_body()
        .collect()
        .await
        .map_err(|_| Failure::InvalidBody)?
        .to_bytes();
    if collected.len() > MAX_BODY_BYTES {
        return Err(Failure::InvalidBody);
    }

    let mut deserializer = serde_json::Deserializer::from_slice(&collected);
    let value = T::deserialize(&mut deserializer).map_err(|_| Failure::InvalidBody)?;
    // Trailing content fails too. serde reads the first value and stops, so a
    // body of two objects would bind the first and discard the second without a
    // word, and the route's contract says one object.
    deserializer.end().map_err(|_| Failure::InvalidBody)?;
    Ok(value)
}

/// Whether the declared length already exceeds the cap.
///
/// Checked before reading, so an oversized body is refused without being
/// buffered first.
fn oversized<B>(request: &Request<B>) -> bool {
    request
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|declared| declared.to_str().ok())
        .and_then(|declared| declared.parse::<usize>().ok())
        .is_some_and(|declared| declared > MAX_BODY_BYTES)
}

/// The identifier this request is logged and answered under.
///
/// A client-supplied identifier is echoed only when it is short and made of
/// characters safe in a header and a log line. An unfiltered value travels into
/// both: a newline would let a caller forge log entries, and an unbounded one
/// would let a caller choose how much of the log it occupies. Anything failing
/// the check is replaced rather than rejected, because a bad header is no reason
/// to refuse the request.
#[must_use]
pub fn request_id<B>(request: &Request<B>) -> String {
    request
        .headers()
        .get(headers::REQUEST_ID)
        .and_then(|supplied| supplied.to_str().ok())
        .filter(|supplied| usable(supplied))
        .map_or_else(|| Uuid::new_v4().to_string(), ToOwned::to_owned)
}

fn usable(supplied: &str) -> bool {
    !supplied.is_empty()
        && supplied.len() <= MAX_REQUEST_ID_LENGTH
        && supplied
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

/// The bearer token a request presented, if it presented one correctly.
#[must_use]
pub fn bearer_token<B>(request: &Request<B>) -> Option<&str> {
    request
        .headers()
        .get(headers::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix(headers::BEARER_PREFIX))
}
