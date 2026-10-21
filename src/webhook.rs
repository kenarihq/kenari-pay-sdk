//! The check of a webhook the gateway sent.
//!
//! The gateway `POST`s to the merchant's webhook URL with a `Kenari-Pay-Signature` header,
//! `t=<unix seconds>,v1=<hex>`, where `v1` is the HMAC-SHA256, keyed by the UTF-8 bytes of the
//! merchant's webhook secret (`whsec_...`), of `"<t>.<raw body>"`. [`verify`] recomputes it over
//! the exact raw body, compares in constant time, and refuses a `t` more than
//! [`TOLERANCE_SECS`] from the merchant's own clock in either direction. An empty secret refuses
//! every webhook.
//!
//! **A verified webhook proves the request came from the gateway. It does not prove the body is
//! true.** The body carries ids only, and a merchant acts on a re-fetch of the deposit
//! (`GET /v1/deposits/{key}`), never on the body. Deliveries can arrive late, twice and out of
//! order, so a handler must be idempotent and the pull feed stays the backstop.
//!
//! ```
//! use kenari_pay_sdk::webhook;
//! use kenari_pay_sdk::wire::EventType;
//!
//! // The values of `vectors/webhook_signature_vector.json`.
//! let secret = "whsec_test_9f8e7d6c5b4a39281706f5e4d3c2b1a0";
//! let header = "t=1760700000,v1=a1074a2b7d9722b34cbd3e1c63d094dc3c1b78fbba8da21beb8b07f1904a9735";
//! let body = br#"{"id":"evt_01JAXF3K9V0QH2M7Z5R8T4W6YB","type":"deposit.confirmed","created_at":1760700000,"data":{"deposit_id":"dep_01JAXF2Q8N3M6P9R1T4V7X0Z2B","key":"base:0x5c504ed432cb51138bcf09aa5e8a410dd4a1e204ef84bfed1be16dfba1b22060:12","seq":42}}"#;
//! let now = 1760700010; // your own clock, in unix seconds
//!
//! let event = webhook::verify(header, body, secret, now).unwrap();
//! assert_eq!(event.event_type, EventType::DepositConfirmed);
//! // Now re-fetch `event.data.key` and act on that.
//! assert_eq!(event.data.seq, 42);
//! ```

use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

use crate::wire::WebhookEvent;

/// How far the `t` of a signature may sit from the merchant's clock, in seconds, either way.
pub const TOLERANCE_SECS: u64 = 300;

/// Why a webhook is refused. Each variant has a fixed text and the body is never part of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebhookError {
    /// The header is not `t=<int>,v1=<hex>`. An unknown extra key is ignored.
    Malformed,
    /// `t` is more than [`TOLERANCE_SECS`] from `now`.
    Stale,
    /// The signature does not match, or the secret is empty.
    Mismatch,
    /// The signature is right and the body is not a webhook event.
    Body,
}

impl std::fmt::Display for WebhookError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            WebhookError::Malformed => "malformed signature header",
            WebhookError::Stale => "timestamp outside tolerance",
            WebhookError::Mismatch => "signature mismatch",
            WebhookError::Body => "webhook body unreadable",
        })
    }
}

impl std::error::Error for WebhookError {}

/// Check a webhook and read its event.
///
/// `signature_header` is the value of `Kenari-Pay-Signature`, `raw_body` the exact bytes of the
/// request body before any parsing, `secret` the merchant's webhook secret and `now` the
/// merchant's own clock in unix seconds. The signature is checked first and the body is read only
/// when it holds, so nothing in an unsigned body is parsed. An event type this SDK does not know
/// is [`EventType::Unknown`](crate::wire::EventType::Unknown) and still a success, so answer 2xx
/// and ignore it.
pub fn verify(
    signature_header: &str,
    raw_body: &[u8],
    secret: &str,
    now: i64,
) -> Result<WebhookEvent, WebhookError> {
    verify_signature(signature_header, raw_body, secret, now)?;
    serde_json::from_slice(raw_body).map_err(|_| WebhookError::Body)
}

/// Check the signature alone, for a merchant that reads the body itself.
///
/// The same check as [`verify`], in this order: the header shape, the tolerance (computed with
/// `abs_diff`, so a hostile `t` such as `i64::MIN` cannot overflow), the secret (empty fails
/// closed) and the constant time comparison of the MAC. When the header carries a key twice, the
/// last one counts.
pub fn verify_signature(
    signature_header: &str,
    raw_body: &[u8],
    secret: &str,
    now: i64,
) -> Result<(), WebhookError> {
    let mut timestamp: Option<i64> = None;
    let mut v1: Option<Vec<u8>> = None;
    for part in signature_header.split(',') {
        let (key, value) = part.trim().split_once('=').ok_or(WebhookError::Malformed)?;
        match key {
            "t" => timestamp = Some(value.parse().map_err(|_| WebhookError::Malformed)?),
            "v1" => v1 = Some(hex::decode(value).map_err(|_| WebhookError::Malformed)?),
            _ => {}
        }
    }
    let (timestamp, v1) = (
        timestamp.ok_or(WebhookError::Malformed)?,
        v1.ok_or(WebhookError::Malformed)?,
    );
    if now.abs_diff(timestamp) > TOLERANCE_SECS {
        return Err(WebhookError::Stale);
    }
    // An empty secret is never a valid key, so a deployment with a blank secret cannot be
    // satisfied by an attacker who signs with the empty key.
    if secret.is_empty() {
        return Err(WebhookError::Mismatch);
    }
    let tag = mac(secret, timestamp, raw_body)
        .ok_or(WebhookError::Mismatch)?
        .finalize()
        .into_bytes();
    if bool::from(tag.as_slice().ct_eq(&v1)) {
        Ok(())
    } else {
        Err(WebhookError::Mismatch)
    }
}

/// The header value the gateway sends for a body: `t=<timestamp>,v1=<hex>`.
///
/// A merchant needs it only to test its own handler, for example to post a signed body to
/// itself. A header signed with an empty secret is refused by [`verify`] whatever secret it is
/// checked against.
pub fn sign(secret: &str, timestamp: i64, raw_body: &[u8]) -> String {
    // The empty tag when no MAC can be built (it never happens for HMAC) is a header that
    // `verify` refuses, so the fallback cannot sign anything.
    let tag = mac(secret, timestamp, raw_body)
        .map(|mac| hex::encode(mac.finalize().into_bytes()))
        .unwrap_or_default();
    format!("t={timestamp},v1={tag}")
}

fn mac(secret: &str, timestamp: i64, raw_body: &[u8]) -> Option<Hmac<Sha256>> {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).ok()?;
    mac.update(timestamp.to_string().as_bytes());
    mac.update(b".");
    mac.update(raw_body);
    Some(mac)
}
