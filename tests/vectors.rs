//! The gateway's own examples and vectors, copied byte for byte into `vectors/`: every example
//! decodes into its wire type, the 20 forwarder vectors recompute, and a tampered copy of each
//! kind of vector is refused. `scripts/sync-vectors.sh` refreshes the copy.

use std::collections::BTreeSet;

use kenari_pay_sdk::address::forwarder::{
    address, check, checksum_address, format_address, format_salt, initcode, keccak256,
    parse_address, parse_salt, ForwarderError, MAX_FEE_BPS,
};
use kenari_pay_sdk::address::{self, pins, Expect, Refusal, Trust};
use kenari_pay_sdk::webhook;
use kenari_pay_sdk::wire::{
    Address, AddressKind, CustomerDeposits, Deposit, DepositStatus, DepositsPage, ErrorBody,
    EventType, Family, Keys, Networks, PoolFile, SweepsPage, WebhookEvent,
};
use serde::{de::DeserializeOwned, Serialize};

const ADDRESS_EVM: &str = include_str!("../vectors/address_evm.json");
const ADDRESS_FORWARDER: &str = include_str!("../vectors/address_evm_forwarder.json");
const ADDRESSES: &str = include_str!("../vectors/addresses.json");
const ATTESTATION: &str = include_str!("../vectors/attestation_vector.json");
const DEPOSIT: &str = include_str!("../vectors/deposit_confirmed.json");
const DEPOSITS_PAGE: &str = include_str!("../vectors/deposits_page.json");
const ERROR: &str = include_str!("../vectors/error_pool_exhausted.json");
const KEYS: &str = include_str!("../vectors/keys_list.json");
const NETWORKS: &str = include_str!("../vectors/networks.json");
const POOL_FILE: &str = include_str!("../vectors/pool_file.json");
const SWEEPS_PAGE: &str = include_str!("../vectors/sweeps_page.json");
const WEBHOOK_BODY: &str = include_str!("../vectors/webhook_deposit_confirmed.json");
const WEBHOOK_VECTOR: &str = include_str!("../vectors/webhook_signature_vector.json");

/// Decode an example, and serialize the result back: the JSON must be the same value, so a field
/// the type dropped or renamed fails here and not at a merchant.
fn round_trip<T: DeserializeOwned + Serialize>(name: &str, text: &str) -> T {
    let parsed: T = serde_json::from_str(text).unwrap_or_else(|e| panic!("{name}: {e}"));
    let original: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(
        serde_json::to_value(&parsed).unwrap(),
        original,
        "{name} does not round-trip through the wire type"
    );
    parsed
}

#[test]
fn the_vectors_directory_is_the_contract_files_and_the_forwarder_vectors() {
    let dir = format!("{}/vectors", env!("CARGO_MANIFEST_DIR"));
    let on_disk: BTreeSet<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    let expected: BTreeSet<String> = [
        "address_evm.json",
        "address_evm_forwarder.json",
        "addresses.json",
        "attestation_vector.json",
        "deposit_confirmed.json",
        "deposits_page.json",
        "error_pool_exhausted.json",
        "keys_list.json",
        "networks.json",
        "pool_file.json",
        "sweeps_page.json",
        "webhook_deposit_confirmed.json",
        "webhook_signature_vector.json",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    assert_eq!(
        on_disk, expected,
        "a file was added or removed, so add a test for it and list it here"
    );
    for name in &on_disk {
        let text = std::fs::read_to_string(format!("{dir}/{name}")).unwrap();
        serde_json::from_str::<serde_json::Value>(&text)
            .unwrap_or_else(|e| panic!("{name} is not JSON: {e}"));
    }
}

// ---- the examples decode ----------------------------------------------------------------------

#[test]
fn every_example_decodes_into_its_wire_type_and_round_trips() {
    let networks: Networks = round_trip("networks.json", NETWORKS);
    assert!(networks.livemode);
    assert!(networks
        .networks
        .iter()
        .any(|n| n.chain == "base" && n.family == Family::Evm));
    assert!(networks
        .networks
        .iter()
        .any(|n| n.chain == "solana" && n.family == Family::Solana));

    let pool: Address = round_trip("address_evm.json", ADDRESS_EVM);
    assert_eq!(pool.kind, Some(AddressKind::Pool));
    assert!(pool.forwarder.is_none());
    assert!(pool.key_epoch.is_some() && pool.pool_index.is_some() && pool.attestation.is_some());

    let forwarder: Address = round_trip("address_evm_forwarder.json", ADDRESS_FORWARDER);
    assert_eq!(forwarder.kind, Some(AddressKind::Forwarder));
    assert!(forwarder.forwarder.is_some());
    assert_eq!(
        (
            forwarder.key_epoch,
            forwarder.pool_index,
            forwarder.attestation
        ),
        (None, None, None)
    );
    assert!(!forwarder.livemode);

    let deposit: Deposit = round_trip("deposit_confirmed.json", DEPOSIT);
    assert_eq!(deposit.status, DepositStatus::Confirmed);
    assert_eq!(deposit.amount_units(), Some(25_000_000));
    assert_eq!(
        deposit.key,
        format!(
            "{}:{}:{}",
            deposit.chain, deposit.tx_hash, deposit.event_index
        )
    );
    assert!(deposit.livemode && !deposit.simulated);

    let page: DepositsPage = round_trip("deposits_page.json", DEPOSITS_PAGE);
    assert_eq!(page.deposits.len(), 2);
    // The second one carries every nullable field as null.
    let detected = &page.deposits[1];
    assert_eq!(detected.status, DepositStatus::Detected);
    assert_eq!(detected.family, Family::Solana);
    assert!(detected.from.is_none() && detected.block_hash.is_none());
    assert!(detected.confirmed_at.is_none());
    assert!(detected.below_minimum);
    assert_eq!(detected.amount_units(), Some(500_000));
    for d in &page.deposits {
        assert_eq!(
            d.key,
            format!("{}:{}:{}", d.chain, d.tx_hash, d.event_index)
        );
    }
    // The customer's deposits are the same objects without the cursor.
    let customer = serde_json::json!({ "deposits": page.deposits }).to_string();
    let customer: CustomerDeposits = round_trip("customer deposits", &customer);
    assert_eq!(customer.deposits.len(), 2);

    let sweeps: SweepsPage = round_trip("sweeps_page.json", SWEEPS_PAGE);
    assert_eq!(sweeps.sweeps.len(), 3);
    assert_eq!(sweeps.next_seq, 43);
    assert_eq!(sweeps.sweeps[0].amount_units(), Some(24_875_000));
    assert_eq!(sweeps.sweeps[0].fee_units(), Some(125_000));
    // The native currency item has no asset and no contract.
    assert!(sweeps.sweeps[2].asset.is_none() && sweeps.sweeps[2].contract.is_none());
    assert_eq!(sweeps.sweeps[2].decimals, 18);
    assert_eq!(sweeps.sweeps[2].amount_units(), Some(9_950_000_000_000_000));

    let keys: Keys = round_trip("keys_list.json", KEYS);
    assert_eq!(keys.keys.len(), 2);
    assert!(keys.keys[0].pool.contains_key("solana"));
    assert!(!keys.keys[1].pool.contains_key("solana"));

    let file: PoolFile = round_trip("pool_file.json", POOL_FILE);
    assert_eq!(file.version, 2);
    assert_eq!(file.entries.len(), 2);

    let error: ErrorBody = round_trip("error_pool_exhausted.json", ERROR);
    assert_eq!(error.error.code, "pool_exhausted");

    let event: WebhookEvent = round_trip("webhook_deposit_confirmed.json", WEBHOOK_BODY);
    assert_eq!(event.event_type, EventType::DepositConfirmed);
    let refs = event.deposit.as_ref().unwrap();
    assert_eq!(refs.key, deposit.key);
    assert_eq!(refs.deposit_id, deposit.id);
    assert_eq!(refs.seq, deposit.seq);
}

#[test]
fn the_pool_file_entries_verify_under_the_vector_key() {
    let file: PoolFile = serde_json::from_str(POOL_FILE).unwrap();
    let vector: serde_json::Value = serde_json::from_str(ATTESTATION).unwrap();
    assert_eq!(
        file.attest_pubkey,
        vector["attest_pubkey_hex"].as_str().unwrap()
    );
    for entry in &file.entries {
        assert!(
            address::verify_attestation(
                &file.attest_pubkey,
                file.key_epoch,
                file.family,
                entry.pool_index,
                &entry.address,
                &entry.attestation
            ),
            "pool_file.json entry {} must verify",
            entry.pool_index
        );
    }
}

// ---- the 20 forwarder vectors ----------------------------------------------------------------

#[derive(Debug, serde::Deserialize)]
struct Vector {
    factory: String,
    implementation: String,
    payout: String,
    fee_bps: u16,
    salt: String,
    address: String,
}

fn vectors() -> Vec<Vector> {
    serde_json::from_str(ADDRESSES).expect("the vectors file parses")
}

#[test]
fn there_are_twenty_vectors_over_two_factories_and_the_fee_range() {
    let v = vectors();
    assert_eq!(v.len(), 20);
    let factories: BTreeSet<&str> = v.iter().map(|x| x.factory.as_str()).collect();
    assert_eq!(factories.len(), 2);
    assert!(v.iter().any(|x| x.fee_bps == 0));
    assert!(v.iter().any(|x| x.fee_bps == MAX_FEE_BPS));
}

#[test]
fn every_forwarder_vector_recomputes() {
    for (n, v) in vectors().iter().enumerate() {
        let factory = parse_address(&v.factory).unwrap();
        let implementation = parse_address(&v.implementation).unwrap();
        let payout = parse_address(&v.payout).unwrap();
        let salt = parse_salt(&v.salt).unwrap_or_else(|| panic!("vector {n}: bad salt"));
        let got = address(&factory, &implementation, &payout, v.fee_bps, &salt)
            .unwrap_or_else(|e| panic!("vector {n}: {e}"));
        // The file spells addresses with the EIP-55 checksum, so the text compares as it is.
        assert_eq!(checksum_address(&got), v.address, "vector {n}");
        assert_eq!(
            format_address(&got),
            v.address.to_ascii_lowercase(),
            "vector {n}"
        );
        assert_eq!(parse_address(&v.address).unwrap(), got, "vector {n}");
        assert_eq!(checksum_address(&factory), v.factory, "vector {n}");
        assert_eq!(
            checksum_address(&implementation),
            v.implementation,
            "vector {n}"
        );
        assert_eq!(checksum_address(&payout), v.payout, "vector {n}");
        assert_eq!(
            format_salt(&salt),
            v.salt.to_ascii_lowercase(),
            "vector {n}"
        );
    }
}

#[test]
fn a_changed_input_changes_the_address_of_every_vector() {
    for (n, v) in vectors().iter().enumerate() {
        let factory = parse_address(&v.factory).unwrap();
        let implementation = parse_address(&v.implementation).unwrap();
        let payout = parse_address(&v.payout).unwrap();
        let salt = parse_salt(&v.salt).unwrap();
        let expected = parse_address(&v.address).unwrap();

        let mut other = salt;
        other[31] ^= 1;
        assert_ne!(
            address(&factory, &implementation, &payout, v.fee_bps, &other).unwrap(),
            expected,
            "salt {n}"
        );
        let mut other = payout;
        other[19] ^= 1;
        if other.iter().any(|b| *b != 0) {
            assert_ne!(
                address(&factory, &implementation, &other, v.fee_bps, &salt).unwrap(),
                expected,
                "payout {n}"
            );
        }
        let mut other = factory;
        other[0] ^= 1;
        assert_ne!(
            address(&other, &implementation, &payout, v.fee_bps, &salt).unwrap(),
            expected,
            "factory {n}"
        );
        let mut other = implementation;
        other[0] ^= 1;
        assert_ne!(
            address(&factory, &other, &payout, v.fee_bps, &salt).unwrap(),
            expected,
            "implementation {n}"
        );
        let fee = if v.fee_bps == 0 { 1 } else { v.fee_bps - 1 };
        assert_ne!(
            address(&factory, &implementation, &payout, fee, &salt).unwrap(),
            expected,
            "fee {n}"
        );
    }
}

#[test]
fn the_address_is_the_create2_of_the_initcode_hash() {
    // Written out by hand from the formula, so the function and the formula cannot drift apart.
    let v = &vectors()[2];
    let factory = parse_address(&v.factory).unwrap();
    let implementation = parse_address(&v.implementation).unwrap();
    let payout = parse_address(&v.payout).unwrap();
    let salt = parse_salt(&v.salt).unwrap();
    let code = initcode(&implementation, &payout, v.fee_bps);
    let mut args = payout.to_vec();
    args.extend_from_slice(&v.fee_bps.to_be_bytes());
    assert!(code.ends_with(&args));
    assert_eq!(&code[..3], &[0x61, 0x00, 0x43]);
    let mut buf = vec![0xffu8];
    buf.extend_from_slice(&factory);
    buf.extend_from_slice(&salt);
    buf.extend_from_slice(&keccak256(&code));
    let hash = keccak256(&buf);
    assert_eq!(&hash[12..], &parse_address(&v.address).unwrap());
}

#[test]
fn the_refusals_match_the_factory() {
    let v = &vectors()[0];
    let factory = parse_address(&v.factory).unwrap();
    let implementation = parse_address(&v.implementation).unwrap();
    let payout = parse_address(&v.payout).unwrap();
    let salt = parse_salt(&v.salt).unwrap();
    assert_eq!(
        address(&factory, &implementation, &payout, MAX_FEE_BPS + 1, &salt),
        Err(ForwarderError::FeeTooHigh(MAX_FEE_BPS + 1))
    );
    assert_eq!(
        address(&factory, &implementation, &[0u8; 20], 50, &salt),
        Err(ForwarderError::ZeroPayout)
    );
    assert_eq!(check(&payout, MAX_FEE_BPS), Ok(()));
    assert!(check(&payout, MAX_FEE_BPS + 1).is_err());
    assert!(check(&[0u8; 20], 0).is_err());
}

#[test]
fn the_forwarder_example_recomputes_from_its_own_block() {
    let example: Address = serde_json::from_str(ADDRESS_FORWARDER).unwrap();
    let block = example.forwarder.unwrap();
    let computed = address(
        &parse_address(&block.factory).unwrap(),
        &parse_address(&block.implementation).unwrap(),
        &parse_address(&block.payout).unwrap(),
        block.fee_bps,
        &parse_salt(&block.salt).unwrap(),
    )
    .unwrap();
    assert_eq!(format_address(&computed), example.address);
}

// ---- a tampered copy of each vector is refused -------------------------------------------------

#[test]
fn a_tampered_copy_of_each_vector_is_refused() {
    // Forwarder vectors: the same inputs with the address of the vector changed.
    for (n, v) in vectors().iter().enumerate() {
        let factory = parse_address(&v.factory).unwrap();
        let implementation = parse_address(&v.implementation).unwrap();
        let payout = parse_address(&v.payout).unwrap();
        let salt = parse_salt(&v.salt).unwrap();
        let computed = address(&factory, &implementation, &payout, v.fee_bps, &salt).unwrap();
        let mut tampered = parse_address(&v.address).unwrap();
        tampered[10] ^= 0x01;
        assert_ne!(computed, tampered, "vector {n} tampered");
    }

    // The forwarder example file: a changed salt and a changed address are refused by the check.
    let trust = Trust::new()
        .chains(["base-sepolia"])
        .pin("base-sepolia", pins::BASE_SEPOLIA)
        .payouts(Family::Evm, ["0xeb1aaeefe4370f06e25bc4bf41e6dc3807fa6dc1"])
        .unwrap();
    let example: Address = serde_json::from_str(ADDRESS_FORWARDER).unwrap();
    let expect = Expect::new(&example.customer_ref, example.family, example.livemode);
    assert!(address::verify(&example, &expect, &trust).is_ok());
    let mut bad_salt = example.clone();
    let salt = bad_salt.forwarder.as_ref().unwrap().salt.clone();
    bad_salt.forwarder.as_mut().unwrap().salt = format!("{}1", &salt[..salt.len() - 1]);
    assert_eq!(
        address::verify(&bad_salt, &expect, &trust),
        Err(Refusal::AddressMismatch)
    );
    let mut bad_address = example.clone();
    bad_address.address = format!("{}1", &example.address[..example.address.len() - 1]);
    assert_eq!(
        address::verify(&bad_address, &expect, &trust),
        Err(Refusal::AddressMismatch)
    );

    // The pool example file: a changed attestation and a changed address are refused.
    let vector: serde_json::Value = serde_json::from_str(ATTESTATION).unwrap();
    let pool_trust = Trust::new()
        .attest_key(1, vector["attest_pubkey_hex"].as_str().unwrap())
        .unwrap();
    let pool: Address = serde_json::from_str(ADDRESS_EVM).unwrap();
    let expect = Expect::new(&pool.customer_ref, pool.family, pool.livemode);
    assert!(address::verify(&pool, &expect, &pool_trust).is_ok());
    let mut bad_sig = pool.clone();
    let sig = bad_sig.attestation.clone().unwrap();
    bad_sig.attestation = Some(format!("{}0", &sig[..sig.len() - 1]));
    if bad_sig.attestation == pool.attestation {
        bad_sig.attestation = Some(format!("{}1", &sig[..sig.len() - 1]));
    }
    assert_eq!(
        address::verify(&bad_sig, &expect, &pool_trust),
        Err(Refusal::Attestation)
    );
    let mut bad_address = pool.clone();
    bad_address.address = "0x6fac4d18c912343bf86fa7049364dd4e424ab9c0".into();
    assert_eq!(
        address::verify(&bad_address, &expect, &pool_trust),
        Err(Refusal::Attestation)
    );

    // The webhook vector: each of its four parts changed on its own.
    let v: serde_json::Value = serde_json::from_str(WEBHOOK_VECTOR).unwrap();
    let secret = v["secret"].as_str().unwrap();
    let ts = v["timestamp"].as_i64().unwrap();
    let body = v["raw_body"].as_str().unwrap();
    let header = v["header"].as_str().unwrap();
    assert!(webhook::verify(header, body.as_bytes(), secret, ts).is_ok());
    let tampered_body = body.replace(
        "dep_01JAXF2Q8N3M6P9R1T4V7X0Z2B",
        "dep_01JAXF2Q8N3M6P9R1T4V7X0Z2C",
    );
    assert_ne!(tampered_body, body);
    assert_eq!(
        webhook::verify(header, tampered_body.as_bytes(), secret, ts),
        Err(webhook::WebhookError::Mismatch)
    );
    assert_eq!(
        webhook::verify(
            header,
            body.as_bytes(),
            "whsec_test_9f8e7d6c5b4a39281706f5e4d3c2b1a1",
            ts
        ),
        Err(webhook::WebhookError::Mismatch)
    );
    assert_eq!(
        webhook::verify(
            &header.replace("v1=a1", "v1=a2"),
            body.as_bytes(),
            secret,
            ts
        ),
        Err(webhook::WebhookError::Mismatch)
    );
    assert_eq!(
        webhook::verify(
            &header.replace("t=1760700000", "t=1760700001"),
            body.as_bytes(),
            secret,
            ts
        ),
        Err(webhook::WebhookError::Mismatch)
    );
}
