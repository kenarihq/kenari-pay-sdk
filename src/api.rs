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
//! let api = Api::new("https://pay-sandbox.kenari.id/", "kp_test_example");
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

/// Why an answer is not the wire type that was asked for. The `Display` text is fixed, so no text
/// of the gateway reaches a merchant's customer through `to_string()`. The detail is in `Debug`,
/// for a server log.
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
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiError::Status { code, .. } => write!(f, "gateway answered {code}"),
            ApiError::Decode(_) => f.write_str("gateway answer unreadable"),
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

/// Every byte outside the RFC 3986 unreserved set becomes `%XX`, so a deposit key
/// (`chain:tx_hash:event_index`) or a `customer_ref` stays one path segment whatever it holds.
fn encode_path_segment(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 8);
    for b in raw.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
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

impl Api {
    /// An API for `base_url` (`https://pay.kenari.id` live or `https://pay-sandbox.kenari.id`
    /// sandbox) and the merchant's key (`kp_live_...` or `kp_test_...`). White space around both
    /// and a trailing slash on the URL are dropped.
    pub fn new(base_url: &str, api_key: &str) -> Api {
        Api {
            base: base_url.trim().trim_end_matches('/').to_string(),
            api_key: api_key.trim().to_string(),
        }
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
    pub fn reissue(&self, address_id: &str) -> HttpRequest {
        let path = format!("/v1/addresses/{}/reissue", encode_path_segment(address_id));
        self.post(&path, None)
    }

    /// `GET /v1/deposits/{id_or_key}`: one deposit by `dep_...` id or by its key, the re-fetch to
    /// do before acting on any webhook. Decode with [`Api::decode_optional`] for a
    /// [`Deposit`](crate::wire::Deposit), because a 404 means there is no such deposit.
    pub fn deposit(&self, id_or_key: &str) -> HttpRequest {
        self.get(&format!("/v1/deposits/{}", encode_path_segment(id_or_key)))
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
    pub fn customer_deposits(&self, customer_ref: &str, limit: u32) -> HttpRequest {
        self.get(&format!(
            "/v1/customers/{}/deposits?limit={limit}",
            encode_path_segment(customer_ref)
        ))
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
            ("space and percent", "a b%", "a%20b%25"),
            ("query and fragment", "a?b#c", "a%3Fb%23c"),
            ("dot dot", "..", ".."),
            ("non ascii", "é", "%C3%A9"),
            ("empty", "", ""),
            ("newline", "a\nb", "a%0Ab"),
            ("plus and at", "a+b@c", "a%2Bb%40c"),
        ];
        for (name, raw, want) in cells {
            assert_eq!(encode_path_segment(raw), *want, "encode cell `{name}`");
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
