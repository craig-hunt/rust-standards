//! Reading a request: its query, its body, and its identifier.

use crate::constants::{MAX_BODY_BYTES, MAX_REQUEST_ID_LENGTH, headers};
use crate::problem::Failure;
use http_body_util::{BodyExt, Limited};
use hyper::body::{Body, Buf};
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
    /// The most bytes one Unicode scalar value occupies in UTF-8.
    const WIDEST_CHARACTER: usize = 4;

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
    /// Written out rather than taken from a crate. It is twenty lines, the
    /// alternative is a dependency at the edge for one function, and a reader can
    /// see exactly what is accepted.
    ///
    /// It decodes into bytes and converts once at the end. An escape names a
    /// byte, not a character: an earlier version pushed each decoded byte
    /// straight into the string as a `char`, which turned every byte above 127
    /// into the Latin-1 character of the same number, so `%C3%A9` arrived as two
    /// characters instead of the one the client sent and no accented search term
    /// could match. Collecting the bytes first lets the UTF-8 sequence be read as
    /// the sequence it is.
    fn decode(encoded: &str) -> String {
        let mut decoded: Vec<u8> = Vec::with_capacity(encoded.len());
        let mut characters = encoded.chars();
        while let Some(character) = characters.next() {
            match character {
                Self::PLUS => decoded.push(Self::SPACE as u8),
                Self::ESCAPE => {
                    let hex: String = characters.by_ref().take(Self::ESCAPE_WIDTH).collect();
                    if let Ok(byte) = u8::from_str_radix(&hex, Self::HEX_RADIX) {
                        decoded.push(byte);
                    } else {
                        // A malformed escape is kept verbatim rather than
                        // dropped, so a search term reaches the domain as the
                        // client typed it and the domain decides.
                        decoded.push(Self::ESCAPE as u8);
                        decoded.extend_from_slice(hex.as_bytes());
                    }
                }
                other => {
                    let mut room = [0_u8; Self::WIDEST_CHARACTER];
                    decoded.extend_from_slice(other.encode_utf8(&mut room).as_bytes());
                }
            }
        }

        // A sequence that is not UTF-8 becomes the replacement character rather
        // than refusing the request, which is the same stance the malformed
        // escape above takes: the edge hands the domain what arrived and the
        // domain decides whether it matches anything.
        String::from_utf8_lossy(&decoded).into_owned()
    }
}

/// Reads a request body as JSON.
///
/// The cap is applied to the stream rather than to what the stream produced.
/// Checking the length after collecting was the first version, and it only ever
/// refused a body the server had already buffered: a chunked request, or one
/// whose `Content-Length` lies, could spend as much memory as it liked before
/// the check ran. `Limited` stops reading at the cap and errors, so the refusal
/// costs one cap's worth of memory whatever the client claimed.
///
/// The declared length is still checked first, because refusing an oversized
/// upload before reading a byte of it is cheaper for both ends.
///
/// Generic over the body type, so a test can hand this a body it built. Pinning
/// the parameter to hyper's `Incoming` left the whole of this function, and
/// every route that reads a body, reachable only by a live socket, which is how
/// a service ends up with no tests over the shapes it accepts.
///
/// Unknown members fail rather than being ignored: a client misspelling
/// `completed` would otherwise get a 200 and no change, with no way to discover
/// the typo.
///
/// # Errors
///
/// Returns [`Failure::InvalidBody`] for a body that is oversized, not JSON, not
/// the shape the route expects, or carrying a member the route does not know.
pub async fn read_json<T, B>(request: Request<B>) -> Result<T, Failure>
where
    T: DeserializeOwned,
    B: Body,
    B::Data: Buf,
    B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    if oversized(&request) {
        return Err(Failure::InvalidBody);
    }

    let collected = Limited::new(request.into_body(), MAX_BODY_BYTES)
        .collect()
        .await
        .map_err(|_| Failure::InvalidBody)?
        .to_bytes();

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

/// The punctuation a correlation identifier may carry beside letters and digits.
///
/// Named rather than matched inline, because the set is the rule: these three
/// are what the common identifier formats use, and anything else reaches a log
/// line and a response header where it has no business.
const SAFE_PUNCTUATION: [u8; 3] = *b"-_.";

fn usable(supplied: &str) -> bool {
    !supplied.is_empty()
        && supplied.len() <= MAX_REQUEST_ID_LENGTH
        && supplied
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || SAFE_PUNCTUATION.contains(&byte))
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

#[cfg(test)]
mod tests {
    // A test asserts by panicking, so the lints that forbid a panic in a service
    // have to be lifted here.
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{Query, read_json, request_id};
    use crate::constants::{MAX_BODY_BYTES, MAX_REQUEST_ID_LENGTH, headers, paths, query};
    use crate::problem::Failure;
    use http_body_util::Full;
    use hyper::body::Bytes;
    use hyper::{Request, header};
    use serde::Deserialize;

    const ACCENTED_TERM: &str = "café";
    const ACCENTED_ENCODED: &str = "/api/inventory?search=caf%C3%A9";
    const SPACED_TERM: &str = "access badge";
    const SPACED_ENCODED: &str = "/api/inventory?search=access+badge";
    const AMPERSAND_TERM: &str = "nuts & bolts";
    const AMPERSAND_ENCODED: &str = "/api/inventory?search=nuts+%26+bolts";
    const MALFORMED_ESCAPE: &str = "/api/inventory?search=%zz";
    const KEPT_VERBATIM: &str = "%zz";
    const ONE_PARAMETER: usize = 1;

    const A_TITLE: &str = "Read the ADR";
    const A_BODY: &str = r#"{"title":"Read the ADR"}"#;
    const TWO_BODIES: &str = r#"{"title":"Read the ADR"}{"title":"and again"}"#;
    const AN_UNKNOWN_MEMBER: &str = r#"{"title":"Read the ADR","colour":"red"}"#;
    const PADDING: u8 = b'x';
    const ONE_BYTE_TOO_MANY: usize = MAX_BODY_BYTES + 1;
    const A_LIE: &str = "3";

    const A_USABLE_IDENTIFIER: &str = "req-1.2_3";
    const AN_IDENTIFIER_CARRYING_SPACES: &str = "one INFO something that never happened";
    const GENERATED_IDENTIFIER_LENGTH: usize = 36;

    #[derive(Debug, Deserialize)]
    #[serde(deny_unknown_fields, rename_all = "camelCase")]
    struct Sent {
        title: String,
    }

    fn asking(uri: &str) -> Request<()> {
        Request::builder().uri(uri).body(()).unwrap()
    }

    fn sending(body: &str) -> Request<Full<Bytes>> {
        Request::builder()
            .uri(paths::TASKS)
            .body(Full::new(Bytes::from(body.to_owned())))
            .unwrap()
    }

    #[test]
    fn a_percent_escape_names_a_byte_and_not_a_character() {
        let asked = Query::of(&asking(ACCENTED_ENCODED));

        assert_eq!(
            asked.get(query::SEARCH),
            Some(ACCENTED_TERM),
            "two escapes are one UTF-8 character, and decoding each as a character sends two"
        );
    }

    #[test]
    fn a_plus_reads_as_a_space() {
        assert_eq!(
            Query::of(&asking(SPACED_ENCODED)).get(query::SEARCH),
            Some(SPACED_TERM)
        );
    }

    #[test]
    fn an_encoded_separator_stays_inside_the_value_it_belongs_to() {
        let asked = Query::of(&asking(AMPERSAND_ENCODED));

        assert_eq!(asked.get(query::SEARCH), Some(AMPERSAND_TERM));
        assert_eq!(
            asked.parameters.len(),
            ONE_PARAMETER,
            "splitting after decoding would read this as two parameters"
        );
    }

    #[test]
    fn a_malformed_escape_reaches_the_domain_as_the_client_typed_it() {
        assert_eq!(
            Query::of(&asking(MALFORMED_ESCAPE)).get(query::SEARCH),
            Some(KEPT_VERBATIM)
        );
    }

    #[tokio::test]
    async fn a_body_the_route_expects_is_read() {
        let sent: Sent = read_json(sending(A_BODY)).await.unwrap();

        assert_eq!(sent.title, A_TITLE);
    }

    #[tokio::test]
    async fn a_second_object_after_the_first_is_refused() {
        let refused = read_json::<Sent, _>(sending(TWO_BODIES)).await.unwrap_err();

        assert!(matches!(refused, Failure::InvalidBody));
    }

    #[tokio::test]
    async fn a_member_the_route_does_not_know_is_refused() {
        let refused = read_json::<Sent, _>(sending(AN_UNKNOWN_MEMBER))
            .await
            .unwrap_err();

        assert!(matches!(refused, Failure::InvalidBody));
    }

    #[tokio::test]
    async fn a_body_over_the_cap_is_refused_even_when_nothing_declared_its_length() {
        let oversized = Request::builder()
            .uri(paths::TASKS)
            .body(Full::new(Bytes::from(vec![PADDING; ONE_BYTE_TOO_MANY])))
            .unwrap();

        let refused = read_json::<Sent, _>(oversized).await.unwrap_err();

        assert!(
            matches!(refused, Failure::InvalidBody),
            "the cap is on the stream, so a client that declares nothing is still held to it"
        );
    }

    #[tokio::test]
    async fn a_declared_length_over_the_cap_is_refused_before_the_body_is_read() {
        let lying = Request::builder()
            .uri(paths::TASKS)
            .header(header::CONTENT_LENGTH, (MAX_BODY_BYTES + 1).to_string())
            .body(Full::new(Bytes::from(A_BODY.to_owned())))
            .unwrap();

        assert!(matches!(
            read_json::<Sent, _>(lying).await.unwrap_err(),
            Failure::InvalidBody
        ));
    }

    #[tokio::test]
    async fn a_declared_length_that_lies_the_other_way_is_still_capped_by_the_stream() {
        let understated = Request::builder()
            .uri(paths::TASKS)
            .header(header::CONTENT_LENGTH, A_LIE)
            .body(Full::new(Bytes::from(vec![PADDING; ONE_BYTE_TOO_MANY])))
            .unwrap();

        assert!(matches!(
            read_json::<Sent, _>(understated).await.unwrap_err(),
            Failure::InvalidBody
        ));
    }

    #[test]
    fn a_usable_identifier_is_echoed() {
        let request = Request::builder()
            .uri(paths::TASKS)
            .header(headers::REQUEST_ID, A_USABLE_IDENTIFIER)
            .body(())
            .unwrap();

        assert_eq!(request_id(&request), A_USABLE_IDENTIFIER);
    }

    #[test]
    fn an_identifier_a_log_line_cannot_carry_safely_is_replaced() {
        let spaced = Request::builder()
            .uri(paths::TASKS)
            .header(headers::REQUEST_ID, AN_IDENTIFIER_CARRYING_SPACES)
            .body(())
            .unwrap();
        let overlong = Request::builder()
            .uri(paths::TASKS)
            .header(
                headers::REQUEST_ID,
                A_USABLE_IDENTIFIER.repeat(MAX_REQUEST_ID_LENGTH),
            )
            .body(())
            .unwrap();

        for request in [spaced, overlong] {
            let answered = request_id(&request);

            assert_eq!(
                answered.len(),
                GENERATED_IDENTIFIER_LENGTH,
                "a value this service will not echo is replaced by one it generated"
            );
        }
    }
}
