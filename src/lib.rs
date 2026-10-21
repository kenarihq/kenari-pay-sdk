//! Merchant SDK for the kenari-pay deposit gateway.
#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod address;
pub mod api;
#[cfg(feature = "reqwest")]
pub mod client;
pub mod feed;
pub mod webhook;
pub mod wire;
