//! The attestation check and the canonical address form: the cells of kenari's own `sig.rs`
//! suite, ported over so the SDK keeps every guard kenari's copy has.

use kenari_pay_sdk::address::{attestation_message, canonical_address, verify_attestation};
use kenari_pay_sdk::wire::Family;

const ATTESTATION_VECTOR: &str = include_str!("../vectors/attestation_vector.json");
const KEYS_LIST: &str = include_str!("../vectors/keys_list.json");

fn family(name: &str) -> Family {
    match name {
        "evm" => Family::Evm,
        "solana" => Family::Solana,
        other => panic!("unknown family {other}"),
    }
}

fn other_family(f: Family) -> Family {
    if f == Family::Evm {
        Family::Solana
    } else {
        Family::Evm
    }
}

#[test]
fn attestation_vector_verifies_and_every_tamper_is_refused() {
    let v: serde_json::Value = serde_json::from_str(ATTESTATION_VECTOR).unwrap();
    let pk = v["attest_pubkey_hex"].as_str().unwrap();
    let entries = v["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 2, "one evm and one solana entry");
    for e in entries {
        let f = family(e["family"].as_str().unwrap());
        let epoch = e["key_epoch"].as_u64().unwrap() as u32;
        let index = e["pool_index"].as_u64().unwrap() as u32;
        let address = e["address"].as_str().unwrap();
        let sig = e["signature_hex"].as_str().unwrap();
        assert_eq!(
            attestation_message(epoch, f, index, address),
            e["message"].as_str().unwrap()
        );
        assert!(
            verify_attestation(pk, epoch, f, index, address, sig),
            "{f:?} vector must verify"
        );
        assert!(
            !verify_attestation(pk, epoch + 1, f, index, address, sig),
            "{f:?} other epoch"
        );
        assert!(
            !verify_attestation(pk, 0, f, index, address, sig),
            "{f:?} epoch zero"
        );
        assert!(
            !verify_attestation(pk, epoch, f, index + 1, address, sig),
            "{f:?} other index"
        );
        assert!(
            !verify_attestation(pk, epoch, other_family(f), index, address, sig),
            "{f:?} other family"
        );
        assert!(
            !verify_attestation(pk, epoch, Family::Unknown, index, address, sig),
            "{f:?} unknown family"
        );
        let mut flipped = sig.to_string();
        flipped.replace_range(0..1, if sig.starts_with('0') { "1" } else { "0" });
        assert!(
            !verify_attestation(pk, epoch, f, index, address, &flipped),
            "{f:?} flipped signature"
        );
        assert!(
            !verify_attestation(pk, epoch, f, index, address, &sig[..sig.len() - 2]),
            "{f:?} short signature"
        );
        assert!(
            !verify_attestation(pk, epoch, f, index, address, &format!("{sig}00")),
            "{f:?} long signature"
        );
        assert!(
            !verify_attestation(pk, epoch, f, index, address, "zz"),
            "{f:?} non-hex signature"
        );
        assert!(
            !verify_attestation(pk, epoch, f, index, address, ""),
            "{f:?} empty signature"
        );
        assert!(
            !verify_attestation("00", epoch, f, index, address, sig),
            "{f:?} short pubkey"
        );
        assert!(
            !verify_attestation("", epoch, f, index, address, sig),
            "{f:?} empty pubkey"
        );
        assert!(
            !verify_attestation(pk, epoch, f, index, "", sig),
            "{f:?} empty address"
        );
    }
    // The EVM entry is attested over the lowercase address. Mixed case canonicalizes to it, and an
    // address that is not 40 hex characters never verifies.
    let evm = &entries[0];
    let addr = evm["address"].as_str().unwrap();
    let sig = evm["signature_hex"].as_str().unwrap();
    let epoch = evm["key_epoch"].as_u64().unwrap() as u32;
    let upper = format!("0x{}", addr[2..].to_ascii_uppercase());
    assert!(
        verify_attestation(pk, epoch, Family::Evm, 0, &upper, sig),
        "case is not part of the identity"
    );
    assert!(!verify_attestation(
        pk,
        epoch,
        Family::Evm,
        0,
        &addr[..addr.len() - 1],
        sig
    ));
    // A small order public key is refused by verify_strict.
    let other = "0000000000000000000000000000000000000000000000000000000000000000";
    assert!(
        !verify_attestation(other, epoch, Family::Evm, 0, addr, sig),
        "small-order key refused"
    );
    // Epoch 2's key (keys_list.json) does not verify an epoch 1 signature either.
    let keys: serde_json::Value = serde_json::from_str(KEYS_LIST).unwrap();
    let epoch2 = keys["keys"][1]["attest_pubkey"].as_str().unwrap();
    assert!(
        !verify_attestation(epoch2, epoch, Family::Evm, 0, addr, sig),
        "another epoch's key"
    );
    // White space around the key and the signature, as kept in a config file, is ignored.
    assert!(verify_attestation(
        &format!(" {pk}\n"),
        epoch,
        Family::Evm,
        0,
        addr,
        &format!("\t{sig} ")
    ));
}

#[test]
fn canonical_address_cells() {
    let evm = "0x9858EfFD232B4033E47d90003D41EC34EcaEda94";
    let lower = "0x9858effd232b4033e47d90003d41ec34ecaeda94";
    let sol = "HAgk14JpMQLgt6rVgv7cBQFJWFto5Dqxi472uT3DKpqk";
    let cells: Vec<(&str, Family, String, Option<&str>)> = vec![
        (
            "evm mixed case lowercases",
            Family::Evm,
            evm.into(),
            Some(lower),
        ),
        (
            "evm already canonical",
            Family::Evm,
            lower.into(),
            Some(lower),
        ),
        ("evm trims", Family::Evm, format!(" {lower}\n"), Some(lower)),
        (
            "evm 0X prefix",
            Family::Evm,
            format!("0X{}", &lower[2..]),
            Some(lower),
        ),
        ("evm no prefix", Family::Evm, lower[2..].into(), None),
        ("evm short", Family::Evm, lower[..41].into(), None),
        ("evm long", Family::Evm, format!("{lower}0"), None),
        (
            "evm non hex",
            Family::Evm,
            "0xzz58effd232b4033e47d90003d41ec34ecaeda94".into(),
            None,
        ),
        ("solana unchanged", Family::Solana, sol.into(), Some(sol)),
        (
            "solana trims",
            Family::Solana,
            format!("\n{sol} "),
            Some(sol),
        ),
        ("solana short", Family::Solana, sol[..31].into(), None),
        ("solana long", Family::Solana, format!("{sol}{sol}"), None),
        (
            "solana bad char 0",
            Family::Solana,
            "0Agk14JpMQLgt6rVgv7cBQFJWFto5Dqxi472uT3DKpqk".into(),
            None,
        ),
        (
            "solana bad char l",
            Family::Solana,
            "lAgk14JpMQLgt6rVgv7cBQFJWFto5Dqxi472uT3DKpqk".into(),
            None,
        ),
        ("evm address as solana", Family::Solana, lower.into(), None),
        ("solana address as evm", Family::Evm, sol.into(), None),
        ("unknown family", Family::Unknown, lower.into(), None),
    ];
    for (name, f, raw, want) in cells {
        assert_eq!(
            canonical_address(f, &raw).as_deref(),
            want,
            "canonical cell `{name}`"
        );
    }
}

#[test]
fn a_signature_with_a_small_order_r_is_refused_though_it_verifies_non_strictly() {
    // The test key of the vector is bytes 0..31, so a signature can be made by hand. Take R as the
    // identity point, a point of order 1, and S = k * a with k = H(R, A, M). Then S*B = R + k*A holds,
    // so `verify` accepts it, and only `verify_strict` refuses a small order R.
    use curve25519_dalek::scalar::{clamp_integer, Scalar};
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};
    use sha2::{Digest, Sha512};

    let v: serde_json::Value = serde_json::from_str(ATTESTATION_VECTOR).unwrap();
    let pk_hex = v["attest_pubkey_hex"].as_str().unwrap();
    let a_bytes: [u8; 32] = hex::decode(pk_hex).unwrap().try_into().unwrap();
    let address = v["entries"][0]["address"].as_str().unwrap();
    let message = attestation_message(1, Family::Evm, 0, address);

    let seed: Vec<u8> = (0u8..32).collect();
    let h = Sha512::digest(&seed);
    let a = Scalar::from_bytes_mod_order(clamp_integer(h[..32].try_into().unwrap()));
    let mut r = [0u8; 32];
    r[0] = 1; // y = 1 is the identity point
    let k = Scalar::from_hash(
        Sha512::new()
            .chain_update(r)
            .chain_update(a_bytes)
            .chain_update(message.as_bytes()),
    );
    let s = k * a;
    let mut sig = [0u8; 64];
    sig[..32].copy_from_slice(&r);
    sig[32..].copy_from_slice(s.as_bytes());

    // `verify` accepting it shows the signature is well made, so only the strictness refuses it.
    let key = VerifyingKey::from_bytes(&a_bytes).unwrap();
    let forged = Signature::from_bytes(&sig);
    assert!(
        key.verify(message.as_bytes(), &forged).is_ok(),
        "non strict accepts it"
    );
    assert!(
        key.verify_strict(message.as_bytes(), &forged).is_err(),
        "strict refuses a small order R"
    );
    assert!(
        !verify_attestation(pk_hex, 1, Family::Evm, 0, address, &hex::encode(sig)),
        "the SDK refuses it"
    );
}
