//! Forwarder addresses: the salt, the clone's init code and the CREATE2 address, as plain math.
//!
//! A deposit address of the forwarder model is a clone of one `Forwarder` implementation, made by
//! a `ForwarderFactory`, that can only send what it receives to the merchant's payout wallet. The
//! address commits to the factory, the implementation, the payout and the sealed fee, so a
//! merchant recomputes it from the `forwarder` block of `POST /v1/addresses` and refuses one that
//! does not match. This module is a copy of the gateway's `forwarder.rs` and both are held to the
//! same 20 vectors (`vectors/addresses.json`).
//!
//! ```text
//! salt     = keccak256("kenari-pay:forwarder:v1:" ++ merchant_id ++ ":" ++ customer_ref ++ ":" ++ n)
//! args     = payout (20 bytes) ++ fee_bps (2 bytes, big endian)
//! initcode = 0x61 ++ uint16(len(args) + 45) ++ 0x3d81600a3d39f3363d3d373d3d3d363d73
//!            ++ implementation (20 bytes) ++ 0x5af43d82803e903d91602b57fd5bf3 ++ args
//! address  = last 20 bytes of keccak256(0xff ++ factory ++ salt ++ keccak256(initcode))
//! ```

use sha3::{Digest, Keccak256};

/// An EVM address as 20 bytes.
pub type EvmAddress = [u8; 20];

/// The most that can be sealed into an address or set as a rate, 3 percent, as the factory's
/// `MAX_FEE_BPS`.
pub const MAX_FEE_BPS: u16 = 300;

/// The fee sealed into every new address unless the operator sets another one for the merchant,
/// 0.5 percent.
pub const DEFAULT_FEE_BPS: u16 = 50;

/// What the salt of every address starts with.
pub const SALT_PREFIX: &str = "kenari-pay:forwarder:v1:";

/// Why an address cannot be computed. The factory's `addressOf` reverts for the same two reasons,
/// so an address computed some other way could never be deployed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForwarderError {
    /// The fee is above [`MAX_FEE_BPS`].
    FeeTooHigh(u16),
    /// The payout is the zero address.
    ZeroPayout,
}

impl std::fmt::Display for ForwarderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ForwarderError::FeeTooHigh(bps) => {
                write!(f, "fee_bps {bps} is above the maximum of {MAX_FEE_BPS}")
            }
            ForwarderError::ZeroPayout => f.write_str("the payout is the zero address"),
        }
    }
}

impl std::error::Error for ForwarderError {}

/// Keccak-256, the hash of the EVM (not NIST SHA3-256).
pub fn keccak256(data: &[u8]) -> [u8; 32] {
    Keccak256::digest(data).into()
}

/// The salt of a customer's address:
/// `keccak256("kenari-pay:forwarder:v1:" ++ merchant_id ++ ":" ++ customer_ref ++ ":" ++ n)`, the
/// text in UTF-8 and `n` in decimal. `n` is 0 for a customer's first forwarder address and one
/// more for every reissue.
pub fn salt(merchant_id: &str, customer_ref: &str, n: u64) -> [u8; 32] {
    let mut text =
        String::with_capacity(SALT_PREFIX.len() + merchant_id.len() + customer_ref.len() + 22);
    text.push_str(SALT_PREFIX);
    text.push_str(merchant_id);
    text.push(':');
    text.push_str(customer_ref);
    text.push(':');
    text.push_str(&n.to_string());
    keccak256(text.as_bytes())
}

/// The refusals of the factory's `addressOf`: a fee above 300 basis points and a zero payout.
pub fn check(payout: &EvmAddress, fee_bps: u16) -> Result<(), ForwarderError> {
    if fee_bps > MAX_FEE_BPS {
        return Err(ForwarderError::FeeTooHigh(fee_bps));
    }
    if payout.iter().all(|b| *b == 0) {
        return Err(ForwarderError::ZeroPayout);
    }
    Ok(())
}

/// The creation code of the clone: the proxy bytecode around `implementation`, then the
/// immutable arguments `payout ++ fee_bps`. It is what the factory CREATE2s, so its hash is in the
/// address.
pub fn initcode(implementation: &EvmAddress, payout: &EvmAddress, fee_bps: u16) -> Vec<u8> {
    const HEAD: [u8; 17] = [
        0x3d, 0x81, 0x60, 0x0a, 0x3d, 0x39, 0xf3, 0x36, 0x3d, 0x3d, 0x37, 0x3d, 0x3d, 0x3d, 0x36,
        0x3d, 0x73,
    ];
    const TAIL: [u8; 15] = [
        0x5a, 0xf4, 0x3d, 0x82, 0x80, 0x3e, 0x90, 0x3d, 0x91, 0x60, 0x2b, 0x57, 0xfd, 0x5b, 0xf3,
    ];
    let mut args = Vec::with_capacity(22);
    args.extend_from_slice(payout);
    args.extend_from_slice(&fee_bps.to_be_bytes());
    // 45 is the length of the proxy code that is not the arguments.
    let size = (args.len() + 45) as u16;
    let mut code = Vec::with_capacity(3 + HEAD.len() + 20 + TAIL.len() + args.len());
    code.push(0x61);
    code.extend_from_slice(&size.to_be_bytes());
    code.extend_from_slice(&HEAD);
    code.extend_from_slice(implementation);
    code.extend_from_slice(&TAIL);
    code.extend_from_slice(&args);
    code
}

/// The CREATE2 address of the forwarder, deployed or not. `Err` for a fee above 300 basis points
/// and for a zero payout, as the factory's `addressOf`.
pub fn address(
    factory: &EvmAddress,
    implementation: &EvmAddress,
    payout: &EvmAddress,
    fee_bps: u16,
    salt: &[u8; 32],
) -> Result<EvmAddress, ForwarderError> {
    check(payout, fee_bps)?;
    let code_hash = keccak256(&initcode(implementation, payout, fee_bps));
    let mut buf = Vec::with_capacity(1 + 20 + 32 + 32);
    buf.push(0xff);
    buf.extend_from_slice(factory);
    buf.extend_from_slice(salt);
    buf.extend_from_slice(&code_hash);
    let hash = keccak256(&buf);
    let mut out = [0u8; 20];
    out.copy_from_slice(&hash[12..]);
    Ok(out)
}

// ---- address text ---------------------------------------------------------------------------

/// Why a text is not an EVM address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressTextError {
    /// Not `0x` and 40 hex characters.
    Malformed,
    /// A mixed case spelling whose case is not the EIP-55 checksum.
    BadChecksum,
}

impl std::fmt::Display for AddressTextError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AddressTextError::Malformed => {
                f.write_str("not an EVM address, it is 0x and 40 hex characters")
            }
            AddressTextError::BadChecksum => {
                f.write_str("the mixed case spelling does not match the EIP-55 checksum")
            }
        }
    }
}

impl std::error::Error for AddressTextError {}

/// Parse `0x` and 40 hex characters. A text that mixes upper and lower case must carry a valid
/// EIP-55 checksum, so a typo in a wallet address is caught. All lower case and all upper case
/// spellings carry no checksum and are accepted. Surrounding white space is ignored.
pub fn parse_address(text: &str) -> Result<EvmAddress, AddressTextError> {
    let text = text.trim();
    let digits = text
        .strip_prefix("0x")
        .or_else(|| text.strip_prefix("0X"))
        .ok_or(AddressTextError::Malformed)?;
    if digits.len() != 40 || !digits.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(AddressTextError::Malformed);
    }
    let mut out = [0u8; 20];
    hex::decode_to_slice(digits, &mut out).map_err(|_| AddressTextError::Malformed)?;
    let mixed = digits.chars().any(|c| c.is_ascii_uppercase())
        && digits.chars().any(|c| c.is_ascii_lowercase());
    if mixed && checksum_address(&out)[2..] != *digits {
        return Err(AddressTextError::BadChecksum);
    }
    Ok(out)
}

/// The canonical form on the wire: `0x` and lower case hex.
pub fn format_address(a: &EvmAddress) -> String {
    format!("0x{}", hex::encode(a))
}

/// The EIP-55 mixed case form, for people.
pub fn checksum_address(a: &EvmAddress) -> String {
    let lower = hex::encode(a);
    let hash = keccak256(lower.as_bytes());
    let mut out = String::from("0x");
    for (i, c) in lower.chars().enumerate() {
        let nibble = if i % 2 == 0 {
            hash[i / 2] >> 4
        } else {
            hash[i / 2] & 0x0f
        };
        if c.is_ascii_alphabetic() && nibble >= 8 {
            out.push(c.to_ascii_uppercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// A salt as the wire shows it: `0x` and 64 lower case hex characters.
pub fn format_salt(s: &[u8; 32]) -> String {
    format!("0x{}", hex::encode(s))
}

/// Parse a salt in the wire form. Upper case digits are accepted.
pub fn parse_salt(text: &str) -> Option<[u8; 32]> {
    let digits = text.trim().strip_prefix("0x")?;
    if digits.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    hex::decode_to_slice(digits, &mut out).ok()?;
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(n: u8) -> EvmAddress {
        let mut a = [0u8; 20];
        a[19] = n;
        a
    }

    #[test]
    fn the_salt_hashes_the_documented_text() {
        let expected = keccak256(b"kenari-pay:forwarder:v1:mch_A:usr_1:0");
        assert_eq!(salt("mch_A", "usr_1", 0), expected);
        // `n` is decimal text, not a byte.
        assert_eq!(
            salt("mch_A", "usr_1", 12),
            keccak256(b"kenari-pay:forwarder:v1:mch_A:usr_1:12")
        );
        assert_eq!(
            salt("mch_A", "usr_1", u64::MAX),
            keccak256(b"kenari-pay:forwarder:v1:mch_A:usr_1:18446744073709551615")
        );
    }

    #[test]
    fn the_salt_differs_in_each_input() {
        let base = salt("mch_A", "usr_1", 0);
        assert_ne!(base, salt("mch_B", "usr_1", 0));
        assert_ne!(base, salt("mch_A", "usr_2", 0));
        assert_ne!(base, salt("mch_A", "usr_1", 1));
        // A merchant id is `mch_` and a ULID, so it holds no colon and the text splits at its first
        // one, and `n` is digits only so it splits at the last. A customer ref with colons in it is
        // therefore never mistaken for another customer's or for a reissue.
        assert_ne!(salt("mch_A", "usr", 10), salt("mch_A", "usr:1", 0));
        assert_ne!(salt("mch_A", "usr:1", 0), salt("mch_A", "usr", 1));
        // Unicode customer refs hash their UTF-8 bytes.
        assert_ne!(salt("mch_A", "ü", 0), salt("mch_A", "u", 0));
        assert_eq!(
            salt("mch_A", "ü", 0),
            keccak256("kenari-pay:forwarder:v1:mch_A:ü:0".as_bytes())
        );
    }

    #[test]
    fn keccak_is_the_evm_hash_not_sha3() {
        // The well known keccak-256 of the empty input. NIST SHA3-256 of it starts a7ffc6f8.
        assert_eq!(
            hex::encode(keccak256(b"")),
            "c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470"
        );
    }

    #[test]
    fn the_check_refuses_a_high_fee_and_a_zero_payout() {
        assert_eq!(check(&addr(1), 0), Ok(()));
        assert_eq!(check(&addr(1), 300), Ok(()));
        assert_eq!(check(&addr(1), 301), Err(ForwarderError::FeeTooHigh(301)));
        assert_eq!(
            check(&addr(1), u16::MAX),
            Err(ForwarderError::FeeTooHigh(u16::MAX))
        );
        assert_eq!(check(&[0u8; 20], 50), Err(ForwarderError::ZeroPayout));
        // Both wrong: the call is refused whichever reason is named.
        assert!(check(&[0u8; 20], 301).is_err());
    }

    #[test]
    fn the_address_refuses_what_the_check_refuses() {
        let (f, i, s) = (addr(1), addr(2), [7u8; 32]);
        assert!(address(&f, &i, &addr(3), 50, &s).is_ok());
        assert_eq!(
            address(&f, &i, &[0u8; 20], 50, &s),
            Err(ForwarderError::ZeroPayout)
        );
        assert_eq!(
            address(&f, &i, &addr(3), 301, &s),
            Err(ForwarderError::FeeTooHigh(301))
        );
    }

    #[test]
    fn the_address_commits_to_every_input() {
        let (f, i, p, s) = (addr(1), addr(2), addr(3), [7u8; 32]);
        let base = address(&f, &i, &p, 50, &s).unwrap();
        assert_ne!(base, address(&addr(9), &i, &p, 50, &s).unwrap());
        assert_ne!(base, address(&f, &addr(9), &p, 50, &s).unwrap());
        assert_ne!(base, address(&f, &i, &addr(9), 50, &s).unwrap());
        assert_ne!(base, address(&f, &i, &p, 49, &s).unwrap());
        assert_ne!(base, address(&f, &i, &p, 50, &[8u8; 32]).unwrap());
    }

    #[test]
    fn the_initcode_has_the_documented_layout() {
        let (i, p) = (addr(2), addr(3));
        let code = initcode(&i, &p, 0x012c);
        // 0x61, the size 67 as two bytes, the proxy head, the implementation, the tail, the args.
        assert_eq!(code.len(), 3 + 17 + 20 + 15 + 22);
        assert_eq!(&code[..3], &[0x61, 0x00, 0x43]);
        assert_eq!(
            &code[3..20],
            &hex::decode("3d81600a3d39f3363d3d373d3d3d363d73").unwrap()[..]
        );
        assert_eq!(&code[20..40], &i);
        assert_eq!(
            &code[40..55],
            &hex::decode("5af43d82803e903d91602b57fd5bf3").unwrap()[..]
        );
        assert_eq!(&code[55..75], &p);
        assert_eq!(&code[75..], &[0x01, 0x2c]);
    }

    #[test]
    fn address_text_parses_and_formats() {
        let lower = "0x9858effd232b4033e47d90003d41ec34ecaeda94";
        let a = parse_address(lower).unwrap();
        assert_eq!(format_address(&a), lower);
        // The upper case spelling and the 0X prefix carry no checksum and are accepted.
        assert_eq!(
            parse_address("0X9858EFFD232B4033E47D90003D41EC34ECAEDA94").unwrap(),
            a
        );
        assert_eq!(parse_address(&format!("  {lower}\n")).unwrap(), a);
        for bad in [
            "",
            "0x",
            "9858effd232b4033e47d90003d41ec34ecaeda94",
            "0x9858effd232b4033e47d90003d41ec34ecaeda9",
            "0x9858effd232b4033e47d90003d41ec34ecaeda94a",
            "0x9858effd232b4033e47d90003d41ec34ecaeda9g",
        ] {
            assert_eq!(
                parse_address(bad),
                Err(AddressTextError::Malformed),
                "{bad}"
            );
        }
    }

    #[test]
    fn mixed_case_must_match_eip55() {
        // The EIP-55 test vectors.
        for text in [
            "0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed",
            "0xfB6916095ca1df60bB79Ce92cE3Ea74c37c5d359",
            "0xdbF03B407c01E7cD3CBea99509d93f8DDDC8C6FB",
            "0xD1220A0cf47c7B9Be7A2E6BA89F429762e7b9aDb",
        ] {
            let a = parse_address(text).unwrap();
            assert_eq!(checksum_address(&a), text);
            assert_eq!(parse_address(&text.to_ascii_lowercase()).unwrap(), a);
        }
        // One letter flipped in case.
        assert_eq!(
            parse_address("0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAEd"),
            Err(AddressTextError::BadChecksum)
        );
        assert_eq!(
            parse_address("0x5AaEB6053F3E94C9b9A09f33669435E7Ef1BeAed"),
            Err(AddressTextError::BadChecksum)
        );
    }

    #[test]
    fn salts_round_trip_through_text() {
        let s = salt("mch_A", "usr_1", 3);
        let text = format_salt(&s);
        assert_eq!(text.len(), 66);
        assert_eq!(parse_salt(&text), Some(s));
        assert_eq!(
            parse_salt(&text.to_ascii_uppercase().replace("0X", "0x")),
            Some(s)
        );
        assert_eq!(parse_salt("0x12"), None);
        assert_eq!(parse_salt(&text[2..]), None);
        assert_eq!(parse_salt(&format!("0x{}", "g".repeat(64))), None);
    }

    #[test]
    fn the_error_texts_are_fixed_and_carry_no_semicolon() {
        for text in [
            ForwarderError::FeeTooHigh(301).to_string(),
            ForwarderError::ZeroPayout.to_string(),
            AddressTextError::Malformed.to_string(),
            AddressTextError::BadChecksum.to_string(),
        ] {
            assert!(!text.contains(';'), "{text}");
        }
    }
}
