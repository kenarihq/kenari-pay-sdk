//! The merchant API as requests and answers, with no IO.
//!
//! [`Api`] renders each call to an [`HttpRequest`], plain data that any HTTP client can send, and
//! [`Api::decode`] turns the status and body that come back into a wire type. The SDK never opens a
//! connection and never sets a timeout. The merchant's own HTTP client does both, so it keeps its
//! transport rules (timeouts, proxies, TLS, retries, redirects). A merchant that wants the calls
//! ready made turns on the `reqwest` feature and uses `Client` instead.
//!
//! ```
//! use kenari_pay_sdk::api::{Api, ApiError, Method};
//! use kenari_pay_sdk::wire::{DepositsPage, Family};
//!
//! let api = Api::new("https://pay-sandbox.kenari.id/", "kp_test_example").unwrap();
//!
//! // Render a call and send it with your own client.
//! let request = api.create_address("usr_1", Family::Evm);
//! assert_eq!(request.method, Method::Post);
//! assert_eq!(request.url, "https://pay-sandbox.kenari.id/v1/addresses");
//!
//! // Decode what came back. A non 2xx answer is an error that holds the gateway's error code.
//! let answer = br#"{"error":{"code":"unauthorized","message":"bad key"}}"#;
//! let error = Api::decode::<DepositsPage>(401, answer).unwrap_err();
//! assert_eq!(
//!     error,
//!     ApiError::Status { code: 401, error_code: Some("unauthorized".into()) }
//! );
//!
//! // The key never goes over plain http, except to a loopback address for a local fake.
//! assert_eq!(Api::new("http://pay.kenari.id", "kp_test_example").unwrap_err(), ApiError::BaseUrl);
//! assert!(Api::new("http://127.0.0.1:8080", "kp_test_example").is_ok());
//! ```

use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::wire::{ErrorBody, Family, PoolFile};

/// The HTTP method of a request. The API uses these two.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Method {
    /// `GET`
    Get,
    /// `POST`
    Post,
}

impl Method {
    /// The method name in upper case.
    pub fn as_str(self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Post => "POST",
        }
    }
}

/// A request to send, as plain data.
///
/// `headers` holds the `Authorization: Bearer` header and, for a request with a body, the
/// `Content-Type`. Send exactly these. The `Debug` form hides the bearer key.
#[derive(Clone, PartialEq, Eq)]
pub struct HttpRequest {
    /// The method.
    pub method: Method,
    /// The absolute URL, query string included.
    pub url: String,
    /// The headers to send, name and value.
    pub headers: Vec<(String, String)>,
    /// The body, JSON in UTF-8, for a request that has one.
    pub body: Option<Vec<u8>>,
}

impl std::fmt::Debug for HttpRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let headers: Vec<(&str, &str)> = self
            .headers
            .iter()
            .map(|(name, value)| {
                let shown = if name.eq_ignore_ascii_case("authorization") {
                    "<hidden>"
                } else {
                    value.as_str()
                };
                (name.as_str(), shown)
            })
            .collect();
        f.debug_struct("HttpRequest")
            .field("method", &self.method)
            .field("url", &self.url)
            .field("headers", &headers)
            .field("body_bytes", &self.body.as_ref().map(Vec::len))
            .finish()
    }
}

/// Why a request cannot be built, or why an answer is not the wire type that was asked for. The
/// `Display` text is fixed, so no text of the gateway and no value the merchant passed reaches a
/// merchant's customer through `to_string()`. The detail is in `Debug`, for a server log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiError {
    /// The gateway answered with a status that is not 2xx. The message of the body is never kept,
    /// only its `error.code` when that is a plain snake case word.
    Status {
        /// The HTTP status.
        code: u16,
        /// The gateway's `error.code`, such as `not_found` or `pool_exhausted`, when the body had one.
        error_code: Option<String>,
    },
    /// A 2xx body that is not the documented shape. The text is the parser's, for a log.
    Decode(String),
    /// A value given to a call cannot be one path segment: it is empty, `.` or `..`. A dot segment
    /// is collapsed by every URL parser, so `/v1/customers/../deposits` would reach
    /// `/v1/deposits`, the merchant wide feed. No request is built, and the value is not kept
    /// here.
    Argument,
    /// The base URL given to [`Api::new`] is not `https://`, and not `http://` to a loopback host
    /// either, so the bearer key would cross the network in clear text. The URL is not kept here.
    BaseUrl,
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiError::Status { code, .. } => write!(f, "gateway answered {code}"),
            ApiError::Decode(_) => f.write_str("gateway answer unreadable"),
            ApiError::Argument => f.write_str("request argument cannot be a path segment"),
            ApiError::BaseUrl => f.write_str("base URL must be https or a loopback address"),
        }
    }
}

impl std::error::Error for ApiError {}

/// The merchant API of one gateway, as requests to send and answers to decode.
///
/// It holds the base URL and the merchant's API key and nothing else, so it is cheap to clone and
/// to share. The `Debug` form hides the key.
#[derive(Clone)]
pub struct Api {
    base: String,
    api_key: String,
}

impl std::fmt::Debug for Api {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Api")
            .field("base", &self.base)
            .field("api_key", &"<hidden>")
            .finish()
    }
}

/// One path segment from a merchant's value. Every byte outside the RFC 3986 unreserved set
/// becomes `%XX`, so a deposit key (`chain:tx_hash:event_index`) or a `customer_ref` stays one
/// segment whatever it holds, and the `%` of a value that already looks encoded is encoded too,
/// so `%2e%2e` stays text and never becomes a dot segment.
///
/// An empty value, `.` and `..` are refused with [`ApiError::Argument`], because they are the
/// only values that are a dot segment after encoding, and a URL parser collapses a dot segment
/// out of the path.
fn encode_path_segment(raw: &str) -> Result<String, ApiError> {
    if raw.is_empty() || raw == "." || raw == ".." {
        return Err(ApiError::Argument);
    }
    let mut out = String::with_capacity(raw.len() + 8);
    for b in raw.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    Ok(out)
}

/// A body that is plain wire data. It cannot fail to serialize, because it holds only text,
/// numbers, booleans and maps with text keys.
fn json_body<T: Serialize>(value: &T) -> Vec<u8> {
    serde_json::to_vec(value).expect("plain wire data always serializes")
}

/// An `error.code` worth keeping: lower case letters, digits and underscores, 1 to 64 of them.
fn plain_code(code: &str) -> bool {
    !code.is_empty()
        && code.len() <= 64
        && code
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// Whether `authority` (the part of a URL between `://` and the first `/`, `?` or `#`) is exactly
/// a loopback host with an optional port: `localhost`, `127.0.0.1` or `[::1]`.
///
/// Nothing else counts, so a host that merely starts like a loopback one (`127.0.0.1.evil.test`,
/// `localhost.evil.test`) is not one.
fn is_loopback_authority(authority: &str) -> bool {
    let host_end = if authority.starts_with('[') {
        authority.find(']').map(|i| i + 1)
    } else {
        Some(authority.find(':').unwrap_or(authority.len()))
    };
    let Some(host_end) = host_end else {
        return false;
    };
    let (host, after) = authority.split_at(host_end);
    let port_ok = after.is_empty()
        || after.strip_prefix(':').is_some_and(|port| {
            !port.is_empty()
                && port.bytes().all(|b| b.is_ascii_digit())
                && port.parse::<u16>().is_ok()
        });
    port_ok && (host.eq_ignore_ascii_case("localhost") || host == "127.0.0.1" || host == "[::1]")
}

/// The base URL as `Api` keeps it: trimmed, no trailing slash, and `https://`, or `http://` to a
/// loopback host. The bearer key goes to this host on every call.
fn checked_base(base_url: &str) -> Result<String, ApiError> {
    let base = base_url.trim().trim_end_matches('/');
    let (scheme, rest) = base.split_once("://").ok_or(ApiError::BaseUrl)?;
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..end];
    // Refuse what could make a URL parser read another host than the one that was checked here:
    // user info (`127.0.0.1@evil.test`), a backslash (a slash to the WHATWG parser), and white
    // space or control characters (which it strips).
    if authority.is_empty()
        || authority
            .chars()
            .any(|c| matches!(c, '@' | '\\') || c.is_whitespace() || c.is_control())
    {
        return Err(ApiError::BaseUrl);
    }
    let allowed = scheme.eq_ignore_ascii_case("https")
        || (scheme.eq_ignore_ascii_case("http") && is_loopback_authority(authority));
    if allowed {
        Ok(base.to_string())
    } else {
        Err(ApiError::BaseUrl)
    }
}

impl Api {
    /// An API for `base_url` (`https://pay.kenari.id` live or `https://pay-sandbox.kenari.id`
    /// sandbox) and the merchant's key (`kp_live_...` or `kp_test_...`). White space around both
    /// and a trailing slash on the URL are dropped.
    ///
    /// The key is sent on every call, so the URL must be `https://`. The one exception is
    /// `http://` to a loopback host (`localhost`, `127.0.0.1` or `[::1]`, with or without a
    /// port), for a test or a local fake gateway.
    ///
    /// # Errors
    ///
    /// [`ApiError::BaseUrl`] for any other URL. Treat it as a reason not to start.
    pub fn new(base_url: &str, api_key: &str) -> Result<Api, ApiError> {
        Ok(Api {
            base: checked_base(base_url)?,
            api_key: api_key.trim().to_string(),
        })
    }

    fn headers(&self, with_body: bool) -> Vec<(String, String)> {
        let mut headers = vec![(
            "Authorization".to_string(),
            format!("Bearer {}", self.api_key),
        )];
        if with_body {
            headers.push(("Content-Type".to_string(), "application/json".to_string()));
        }
        headers
    }

    fn get(&self, path: &str) -> HttpRequest {
        HttpRequest {
            method: Method::Get,
            url: format!("{}{path}", self.base),
            headers: self.headers(false),
            body: None,
        }
    }

    fn post(&self, path: &str, body: Option<Vec<u8>>) -> HttpRequest {
        HttpRequest {
            method: Method::Post,
            url: format!("{}{path}", self.base),
            headers: self.headers(body.is_some()),
            body,
        }
    }

    /// `GET /v1/networks`: the enabled networks. Decode with [`Networks`](crate::wire::Networks).
    pub fn networks(&self) -> HttpRequest {
        self.get("/v1/networks")
    }

    /// `POST /v1/addresses`: the customer's permanent address of `family`, allocated on the first
    /// call and the same one on every later call. Decode with [`Address`](crate::wire::Address)
    /// and run [`crate::address::verify`] before showing or storing it.
    pub fn create_address(&self, customer_ref: &str, family: Family) -> HttpRequest {
        let body = serde_json::json!({
            "customer_ref": customer_ref,
            "family": family,
        });
        self.post("/v1/addresses", Some(json_body(&body)))
    }

    /// `POST /v1/addresses/{id}/reissue`: move the customer to a fresh address of the model that
    /// applies now. Decode with [`Address`](crate::wire::Address). The old address stays watched.
    ///
    /// # Errors
    ///
    /// [`ApiError::Argument`] when `address_id` is empty, `.` or `..`.
    pub fn reissue(&self, address_id: &str) -> Result<HttpRequest, ApiError> {
        let path = format!("/v1/addresses/{}/reissue", encode_path_segment(address_id)?);
        Ok(self.post(&path, None))
    }

    /// `GET /v1/deposits/{id_or_key}`: one deposit by `dep_...` id or by its key, the re-fetch to
    /// do before acting on any webhook. Decode with [`Api::decode_optional`] for a
    /// [`Deposit`](crate::wire::Deposit), because a 404 means there is no such deposit.
    ///
    /// # Errors
    ///
    /// [`ApiError::Argument`] when `id_or_key` is empty, `.` or `..`.
    pub fn deposit(&self, id_or_key: &str) -> Result<HttpRequest, ApiError> {
        Ok(self.get(&format!("/v1/deposits/{}", encode_path_segment(id_or_key)?)))
    }

    /// `GET /v1/deposits?after_seq=&limit=`: the change feed, ascending by `seq`. `limit` is 1 to
    /// 500 and the gateway's default is 100. Decode with [`DepositsPage`](crate::wire::DepositsPage).
    /// See [`crate::feed`] for the cursor rule.
    pub fn deposits(&self, after_seq: i64, limit: u32) -> HttpRequest {
        self.get(&format!("/v1/deposits?after_seq={after_seq}&limit={limit}"))
    }

    /// `GET /v1/customers/{customer_ref}/deposits?limit=`: the customer's newest deposits, newest
    /// first. `limit` is 1 to 50 and the gateway's default is 10. Decode with
    /// [`CustomerDeposits`](crate::wire::CustomerDeposits).
    ///
    /// # Errors
    ///
    /// [`ApiError::Argument`] when `customer_ref` is empty, `.` or `..`. Without that refusal
    /// `..` would render `/v1/customers/../deposits`, which a URL parser collapses to
    /// `/v1/deposits`, the feed of every customer.
    pub fn customer_deposits(
        &self,
        customer_ref: &str,
        limit: u32,
    ) -> Result<HttpRequest, ApiError> {
        Ok(self.get(&format!(
            "/v1/customers/{}/deposits?limit={limit}",
            encode_path_segment(customer_ref)?
        )))
    }

    /// `GET /v1/sweeps?after_seq=&limit=`: what the sweeper moved into the payout wallet, ascending
    /// by `seq`. `limit` is 1 to 500. Decode with [`SweepsPage`](crate::wire::SweepsPage).
    pub fn sweeps(&self, after_seq: i64, limit: u32) -> HttpRequest {
        self.get(&format!("/v1/sweeps?after_seq={after_seq}&limit={limit}"))
    }

    /// `GET /v1/keys`, for a merchant on pool addresses: its key epochs. Decode with
    /// [`Keys`](crate::wire::Keys).
    pub fn keys(&self) -> HttpRequest {
        self.get("/v1/keys")
    }

    /// `POST /v1/keys`, for a merchant on pool addresses: register the next key epoch with its
    /// attestation public key (64 hex characters). Decode with [`Key`](crate::wire::Key).
    pub fn register_key(&self, key_epoch: u32, attest_pubkey_hex: &str) -> HttpRequest {
        let body = serde_json::json!({
            "key_epoch": key_epoch,
            "attest_pubkey": attest_pubkey_hex,
        });
        self.post("/v1/keys", Some(json_body(&body)))
    }

    /// `POST /v1/pool`, for a merchant on pool addresses: import a pool file. All or nothing. Decode
    /// with [`PoolImported`](crate::wire::PoolImported).
    pub fn import_pool(&self, file: &PoolFile) -> HttpRequest {
        self.post("/v1/pool", Some(json_body(file)))
    }

    /// Decode the answer to a request: the wire type for a 2xx status, an [`ApiError`] otherwise.
    ///
    /// A non 2xx body is read only for its `error.code`, and the code is kept only when it is a
    /// plain snake case word of at most 64 characters. A body that is not the gateway's error shape
    /// (an HTML page from a proxy, for one) gives `error_code: None`.
    pub fn decode<T: DeserializeOwned>(status: u16, body: &[u8]) -> Result<T, ApiError> {
        if !(200..300).contains(&status) {
            let error_code = serde_json::from_slice::<ErrorBody>(body)
                .ok()
                .map(|parsed| parsed.error.code)
                .filter(|code| plain_code(code));
            return Err(ApiError::Status {
                code: status,
                error_code,
            });
        }
        serde_json::from_slice(body).map_err(|e| ApiError::Decode(e.to_string()))
    }

    /// Like [`Api::decode`], with a 404 as `Ok(None)`: for [`Api::deposit`], where the gateway
    /// saying there is no such deposit is an answer and not a failure.
    ///
    /// Any 404 counts, including one from a proxy in front of the gateway. A merchant that is not
    /// sure its base URL is right checks it with [`Api::networks`] at start up.
    pub fn decode_optional<T: DeserializeOwned>(
        status: u16,
        body: &[u8],
    ) -> Result<Option<T>, ApiError> {
        if status == 404 {
            return Ok(None);
        }
        Api::decode(status, body).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_segment_encoding_keeps_one_segment() {
        let cells: &[(&str, &str, &str)] = &[
            ("evm key", "base:0xabc:12", "base%3A0xabc%3A12"),
            ("unreserved kept", "AZaz09-._~", "AZaz09-._~"),
            ("slash", "a/b", "a%2Fb"),
            ("slash alone", "/", "%2F"),
            ("dot dot slash", "../keys", "..%2Fkeys"),
            ("space and percent", "a b%", "a%20b%25"),
            ("query and fragment", "a?b#c", "a%3Fb%23c"),
            ("backslash", "a\\b", "a%5Cb"),
            ("non ascii", "\u{e9}", "%C3%A9"),
            ("newline", "a\nb", "a%0Ab"),
            ("plus and at", "a+b@c", "a%2Bb%40c"),
            // Three dots and a dot inside a word are ordinary segments.
            ("three dots", "...", "..."),
            ("dot in a word", "a.b", "a.b"),
            ("dot space", ". ", ".%20"),
            // A value that already looks percent encoded keeps its `%` as text, so it can never
            // decode to a dot segment in a URL parser.
            ("encoded dots", "%2e%2e", "%252e%252e"),
            ("encoded dots upper", "%2E%2E", "%252E%252E"),
            ("one encoded dot", "%2e", "%252e"),
            ("dot then encoded dot", ".%2e", ".%252e"),
            ("encoded dot then dot", "%2e.", "%252e."),
        ];
        for (name, raw, want) in cells {
            assert_eq!(
                encode_path_segment(raw).as_deref(),
                Ok(*want),
                "encode cell `{name}`"
            );
        }
    }

    #[test]
    fn an_empty_or_dot_segment_is_refused() {
        for (name, raw) in [("empty", ""), ("dot", "."), ("dot dot", "..")] {
            assert_eq!(
                encode_path_segment(raw),
                Err(ApiError::Argument),
                "refusal cell `{name}`"
            );
        }
    }

    /// What a URL parser does to a rendered path: drop `.` segments and let `..` remove the
    /// segment before it, the WHATWG rule that reqwest's `url`, curl and browsers all follow, with
    /// `%2e` counted as a dot. A rendered path must come out with every segment still there.
    fn collapse(path: &str) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for segment in path.split('/').skip(1) {
            match segment.to_ascii_lowercase().as_str() {
                "." | "%2e" => {}
                ".." | ".%2e" | "%2e." | "%2e%2e" => {
                    out.pop();
                }
                _ => out.push(segment.to_string()),
            }
        }
        out
    }

    #[test]
    fn no_value_makes_a_rendered_path_collapse() {
        let api = Api::new("https://pay.kenari.id", "kp_test_x").unwrap();
        let values = [
            "", ".", "..", "...", "%2e", "%2e%2e", ".%2e", "%2e.", "%2E%2E", "a/..", "../..", "/",
            "a/b", "..\\", "./", "\t.", ".\n.",
        ];
        for value in values {
            let rendered = [
                api.customer_deposits(value, 10),
                api.deposit(value),
                api.reissue(value),
            ];
            for request in rendered {
                let Ok(request) = request else {
                    assert!(matches!(value, "" | "." | ".."), "{value:?} was refused");
                    continue;
                };
                let path = request
                    .url
                    .strip_prefix("https://pay.kenari.id")
                    .unwrap()
                    .split('?')
                    .next()
                    .unwrap()
                    .to_string();
                let want = path.split('/').skip(1).count();
                assert_eq!(collapse(&path).len(), want, "{value:?} rendered {path}");
            }
        }
    }

    #[test]
    fn only_plain_codes_are_kept() {
        for good in ["not_found", "pool_exhausted", "a", "internal", "x9_y"] {
            assert!(plain_code(good), "{good}");
        }
        for bad in [
            "",
            "Not_Found",
            "not found",
            "not-found",
            "é",
            "a\nb",
            "<script>",
            &"a".repeat(65),
        ] {
            assert!(!plain_code(bad), "{bad:?}");
        }
        assert!(plain_code(&"a".repeat(64)));
    }
}
