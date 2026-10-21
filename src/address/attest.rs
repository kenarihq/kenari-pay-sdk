//! The attestation of a pool address: an ed25519 signature by the merchant's own attestation key
//! over `kenari-pay:attest:v2:{key_epoch}:{family}:{pool_index}:{address}`.
//!
//! It proves an address is an entry of the merchant's pool. It does not prove whose it is, which
//! is why a merchant also keeps one address per customer and never reuses an address.

use ed25519_dalek::{Signature, VerifyingKey};

use super::{canonical_address, TrustError};
use crate::wire::Family;

/// The exact bytes an epoch's attestation key signs for one pool entry. `address` must already be
/// canonical. The epoch is part of the message, so an entry signed for one epoch can never pass as
/// an entry of another.
pub fn attestation_message(
    key_epoch: u32,
    family: Family,
    pool_index: u32,
    address: &str,
) -> String {
    format!(
        "kenari-pay:attest:v2:{}:{}:{}:{}",
        key_epoch,
        family.as_str(),
        pool_index,
        address
    )
}

/// Verify a hex ed25519 attestation against a hex public key.
///
/// The address is put into canonical form first, and one that is not a well formed address of
/// `family` is refused. Any malformed input is `false`, never a panic. The check is
/// `verify_strict`, which also refuses a small order public key and a non canonical signature.
pub fn verify_attestation(
    attest_pubkey_hex: &str,
    key_epoch: u32,
    family: Family,
    pool_index: u32,
    address: &str,
    signature_hex: &str,
) -> bool {
    let Some(address) = canonical_address(family, address) else {
        return false;
    };
    let Ok(key) = parse_key(attest_pubkey_hex) else {
        return false;
    };
    verify_with(&key, key_epoch, family, pool_index, &address, signature_hex)
}

/// A public key from its hex text. A small order key is refused here and again by every
/// verification, so a merchant learns of it at start up.
pub(crate) fn parse_key(hex_text: &str) -> Result<VerifyingKey, TrustError> {
    let bytes = hex::decode(hex_text.trim()).map_err(|_| TrustError::Key)?;
    let bytes = <[u8; 32]>::try_from(bytes.as_slice()).map_err(|_| TrustError::Key)?;
    let key = VerifyingKey::from_bytes(&bytes).map_err(|_| TrustError::Key)?;
    if key.is_weak() {
        return Err(TrustError::WeakKey);
    }
    Ok(key)
}

/// `verify_strict` of one entry. `address` must be canonical.
pub(crate) fn verify_with(
    key: &VerifyingKey,
    key_epoch: u32,
    family: Family,
    pool_index: u32,
    address: &str,
    signature_hex: &str,
) -> bool {
    let Ok(sig_bytes) = hex::decode(signature_hex.trim()) else {
        return false;
    };
    let Ok(sig_bytes) = <[u8; 64]>::try_from(sig_bytes.as_slice()) else {
        return false;
    };
    let signature = Signature::from_bytes(&sig_bytes);
    let message = attestation_message(key_epoch, family, pool_index, address);
    key.verify_strict(message.as_bytes(), &signature).is_ok()
}
