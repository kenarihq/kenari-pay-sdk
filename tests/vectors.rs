//! The gateway's own examples and vectors, copied byte for byte into `vectors/`: every example
//! decodes into its wire type, the 20 forwarder vectors recompute, and a tampered copy of each
//! kind of vector is refused. `scripts/sync-vectors.sh` refreshes the copy.

use std::collections::BTreeSet;

use kenari_pay_sdk::address::forwarder::{
    address, check, checksum_address, format_address, format_salt, initcode, keccak256,
    parse_address, parse_salt, ForwarderError, MAX_FEE_BPS,
};
use kenari_pay_sdk::address::{self, pins, Expect, Refusal, Trust};
use kenari_pay_sdk::api::{Api, ApiError};
use kenari_pay_sdk::webhook;
use kenari_pay_sdk::wire::{
    Address, AddressKind, CustomerDeposits, Deposit, DepositStatus, DepositsPage, ErrorBody,
    EventType, Family, Keys, MatchedBy, Networks, PoolFile, QrisInvoice, QrisInvoiceStatus,
    QrisInvoicesPage, QrisProvider, SweepsPage, UniqueCodeMode, WebhookEvent,
};
use serde::{de::DeserializeOwned, Serialize};

const ADDRESS_EVM: &str = include_str!("../vectors/address_evm.json");
const ADDRESS_FORWARDER: &str = include_str!("../vectors/address_evm_forwarder.json");
const ADDRESSES: &str = include_str!("../vectors/addresses.json");
const ATTESTATION: &str = include_str!("../vectors/attestation_vector.json");
const DEPOSIT: &str = include_str!("../vectors/deposit_confirmed.json");
const DEPOSITS_PAGE: &str = include_str!("../vectors/deposits_page.json");
const ERROR: &str = include_str!("../vectors/error_pool_exhausted.json");
const ERROR_QRIS: &str = include_str!("../vectors/error_qris_unavailable.json");
const KEYS: &str = include_str!("../vectors/keys_list.json");
const NETWORKS: &str = include_str!("../vectors/networks.json");
const POOL_FILE: &str = include_str!("../vectors/pool_file.json");
const QRIS_INVOICE: &str = include_str!("../vectors/qris_invoice_paid.json");
const QRIS_PAGE: &str = include_str!("../vectors/qris_invoices_page.json");
const SWEEPS_PAGE: &str = include_str!("../vectors/sweeps_page.json");
const WEBHOOK_BODY: &str = include_str!("../vectors/webhook_deposit_confirmed.json");
const WEBHOOK_QRIS: &str = include_str!("../vectors/webhook_qris_invoice_paid.json");
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
        "error_qris_unavailable.json",
        "keys_list.json",
        "networks.json",
        "pool_file.json",
        "qris_invoice_paid.json",
        "qris_invoices_page.json",
        "sweeps_page.json",
        "webhook_deposit_confirmed.json",
        "webhook_qris_invoice_paid.json",
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

// ---- the QRIS examples ------------------------------------------------------------------------

/// The EMVCo TLV objects of a QRIS payload, in order: a two digit tag, a two digit length, the
/// value. Written here from the format, so the examples are checked by code that is not the
/// gateway's.
fn tlv(payload: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = payload;
    while !rest.is_empty() {
        assert!(rest.len() >= 4, "a truncated TLV head in {payload}");
        let (tag, after) = rest.split_at(2);
        let (len, after) = after.split_at(2);
        let len: usize = len.parse().unwrap();
        assert!(after.len() >= len, "a truncated TLV value in {payload}");
        let (value, after) = after.split_at(len);
        out.push((tag.to_string(), value.to_string()));
        rest = after;
    }
    out
}

/// CRC-16/CCITT-FALSE: poly 0x1021, init 0xFFFF, no reflection, no final xor.
fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0xffff;
    for byte in data {
        crc ^= u16::from(*byte) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}

/// A dynamic QRIS payload that carries `amount_due`: a valid CRC over everything up to and with
/// `6304`, point of initiation `12`, tag 54 the amount in plain digits, the tags ascending.
fn assert_dynamic_qris_of(invoice: &QrisInvoice) {
    let payload = &invoice.qris;
    assert_eq!(
        crc16(b"123456789"),
        0x29b1,
        "the CRC-16/CCITT-FALSE check value"
    );
    let (body, crc) = payload.split_at(payload.len() - 4);
    assert!(body.ends_with("6304"), "{}: tag 63 is last", invoice.id);
    assert_eq!(
        crc,
        format!("{:04X}", crc16(body.as_bytes())),
        "{}: CRC",
        invoice.id
    );
    let objects = tlv(payload);
    let tags: Vec<&str> = objects.iter().map(|(t, _)| t.as_str()).collect();
    let mut sorted = tags.clone();
    sorted.sort();
    assert_eq!(tags, sorted, "{}: tags ascending", invoice.id);
    let value = |tag: &str| {
        objects
            .iter()
            .find(|(t, _)| t == tag)
            .map(|(_, v)| v.as_str())
    };
    assert_eq!(value("00"), Some("01"));
    assert_eq!(value("01"), Some("12"), "{}: dynamic", invoice.id);
    assert_eq!(value("53"), Some("360"), "{}: rupiah", invoice.id);
    assert_eq!(value("58"), Some("ID"));
    assert_eq!(
        value("54"),
        Some(invoice.amount_due.to_string().as_str()),
        "{}: the amount is amount_due",
        invoice.id
    );
    assert_eq!(value("59"), Some("KENARI SANDBOX"));
    // The static QRIS of the brief is the same payload with tag 01 `11` and no tag 54.
    let static_qris = "00020101021126660014ID.CO.QRIS.WWW01189360091400000000010215ID10290000000010303UMI5204573453033605802ID5914KENARI SANDBOX6007JAKARTA6105101106304D485";
    let (static_body, static_crc) = static_qris.split_at(static_qris.len() - 4);
    assert_eq!(static_crc, format!("{:04X}", crc16(static_body.as_bytes())));
    let without_amount: Vec<(String, String)> = objects
        .iter()
        .filter(|(t, _)| t != "54" && t != "63")
        .map(|(t, v)| {
            if t == "01" {
                (t.clone(), "11".to_string())
            } else {
                (t.clone(), v.clone())
            }
        })
        .collect();
    let static_objects: Vec<(String, String)> = tlv(static_qris)
        .into_iter()
        .filter(|(t, _)| t != "63")
        .collect();
    assert_eq!(
        without_amount, static_objects,
        "{}: made from the static QRIS",
        invoice.id
    );
}

#[test]
fn the_qris_examples_decode_round_trip_and_agree_with_each_other() {
    let invoice: QrisInvoice = round_trip("qris_invoice_paid.json", QRIS_INVOICE);
    assert_eq!(invoice.status, QrisInvoiceStatus::Paid);
    assert_eq!(invoice.matched_by, Some(MatchedBy::Feed));
    assert_eq!(invoice.unique_code_mode, UniqueCodeMode::Add);
    assert_eq!(
        invoice.amount_due,
        invoice.base_amount + i64::from(invoice.unique_code)
    );
    assert!(invoice.unique_code <= 999);
    assert_eq!(invoice.provider, QrisProvider::Gopay);
    assert!(invoice.paid_exactly());
    assert!(invoice.livemode);
    assert!(invoice.created_at < invoice.expires_at && invoice.expires_at < invoice.match_until);
    assert_eq!(invoice.payer_issuer.as_deref(), Some("DANA"));
    // The example was paid twice: one repeat of exactly amount_due, after the first payment and
    // inside the window, not reversed.
    assert_eq!(invoice.repeat_payments.len(), 1);
    let repeat = &invoice.repeat_payments[0];
    assert_eq!(repeat.amount, invoice.amount_due);
    assert_eq!(repeat.reversed_at, None);
    assert_eq!(repeat.issuer.as_deref(), Some("DANA"));
    assert!(invoice.paid_at.unwrap() <= repeat.paid_at && repeat.paid_at <= invoice.match_until);
    assert!(repeat.creditable_for(&invoice));
    assert_dynamic_qris_of(&invoice);

    let page: QrisInvoicesPage = round_trip("qris_invoices_page.json", QRIS_PAGE);
    assert_eq!(page.invoices.len(), 2);
    assert_eq!((page.next_seq, page.has_more), (43, false));
    assert_eq!(
        page.invoices[0], invoice,
        "the page holds the single example"
    );
    // The second one is open and carries every nullable field as null.
    let open = &page.invoices[1];
    assert_eq!(open.status, QrisInvoiceStatus::Open);
    assert_eq!(open.provider, QrisProvider::Gopay);
    assert_eq!(
        (
            &open.paid_at,
            &open.paid_amount,
            &open.payer_issuer,
            &open.matched_by
        ),
        (&None, &None, &None, &None)
    );
    assert!(!open.paid_exactly());
    assert!(open.repeat_payments.is_empty());
    assert_eq!(open.unique_code_mode, UniqueCodeMode::Add);
    assert_eq!(
        open.amount_due,
        open.base_amount + i64::from(open.unique_code)
    );
    assert_eq!(page.next_seq, page.invoices.last().unwrap().seq);
    for invoice in &page.invoices {
        assert_dynamic_qris_of(invoice);
    }

    let event: WebhookEvent = round_trip("webhook_qris_invoice_paid.json", WEBHOOK_QRIS);
    assert_eq!(event.event_type, EventType::QrisInvoicePaid);
    assert_eq!(event.deposit, None);
    let refs = event.qris_invoice.as_ref().unwrap();
    assert_eq!(refs.invoice_id, invoice.id);
    assert_eq!(refs.reference, invoice.reference);
    assert_eq!(refs.seq, invoice.seq);

    let error: ErrorBody = round_trip("error_qris_unavailable.json", ERROR_QRIS);
    assert_eq!(error.error.code, "unavailable");
    assert_eq!(
        Api::decode::<QrisInvoice>(503, ERROR_QRIS.as_bytes()),
        Err(ApiError::Status {
            code: 503,
            error_code: Some("unavailable".into())
        })
    );
}

#[test]
fn a_qris_invoice_without_livemode_or_with_another_paid_amount_is_not_creditable() {
    for required in [
        "livemode",
        "provider",
        "status",
        "amount_due",
        "unique_code",
    ] {
        let mut value: serde_json::Value = serde_json::from_str(QRIS_INVOICE).unwrap();
        value.as_object_mut().unwrap().remove(required);
        assert!(
            serde_json::from_value::<QrisInvoice>(value).is_err(),
            "{required} is required"
        );
    }

    let mut invoice: QrisInvoice = serde_json::from_str(QRIS_INVOICE).unwrap();
    assert!(invoice.paid_exactly());
    invoice.paid_amount = Some(invoice.amount_due - 1);
    assert!(!invoice.paid_exactly(), "underpaid");
    invoice.paid_amount = Some(invoice.amount_due + 1);
    assert!(!invoice.paid_exactly(), "overpaid");
    invoice.paid_amount = None;
    assert!(!invoice.paid_exactly(), "no amount");
    invoice.paid_amount = Some(invoice.amount_due);
    for status in [
        QrisInvoiceStatus::Open,
        QrisInvoiceStatus::Expired,
        QrisInvoiceStatus::Unknown,
    ] {
        invoice.status = status;
        assert!(!invoice.paid_exactly(), "{status:?}");
    }

    // A status, a match or a provider this SDK does not know still decodes, as `Unknown`.
    let text = QRIS_INVOICE
        .replace("\"status\": \"paid\"", "\"status\": \"refunded\"")
        .replace("\"matched_by\": \"feed\"", "\"matched_by\": \"guess\"")
        .replace("\"provider\": \"gopay\"", "\"provider\": \"ovo\"");
    for word in ["refunded", "guess", "ovo"] {
        assert!(text.contains(word), "the {word} replacement took");
    }
    let invoice: QrisInvoice = serde_json::from_str(&text).unwrap();
    assert_eq!(invoice.status, QrisInvoiceStatus::Unknown);
    assert_eq!(invoice.matched_by, Some(MatchedBy::Unknown));
    assert_eq!(invoice.provider, QrisProvider::Unknown);
    assert!(!invoice.paid_exactly());
    // A provider the SDK does not know leaves a paid invoice creditable: the status decides.
    let text = QRIS_INVOICE.replace("\"provider\": \"gopay\"", "\"provider\": \"ovo\"");
    let invoice: QrisInvoice = serde_json::from_str(&text).unwrap();
    assert_eq!(invoice.provider, QrisProvider::Unknown);
    assert!(invoice.paid_exactly());

    // A unique code of 0 is a code like any other: the amount due is the base amount.
    let mut value: serde_json::Value = serde_json::from_str(QRIS_INVOICE).unwrap();
    value["unique_code"] = 0.into();
    value["amount_due"] = 50_000.into();
    value["paid_amount"] = 50_000.into();
    let invoice: QrisInvoice = serde_json::from_value(value).unwrap();
    assert_eq!(invoice.unique_code, 0);
    assert!(invoice.paid_exactly());
    // A gateway before the code mode sends no `unique_code_mode`: the invoice reads as `Add`.
    let mut value: serde_json::Value = serde_json::from_str(QRIS_INVOICE).unwrap();
    value.as_object_mut().unwrap().remove("unique_code_mode");
    let older: QrisInvoice = serde_json::from_value(value).unwrap();
    assert_eq!(older.unique_code_mode, UniqueCodeMode::Add);
    assert_eq!(
        older,
        serde_json::from_str::<QrisInvoice>(QRIS_INVOICE).unwrap()
    );
    // A code taken off the base: the amount due is below the base amount.
    let mut value: serde_json::Value = serde_json::from_str(QRIS_INVOICE).unwrap();
    value["unique_code_mode"] = "subtract".into();
    value["amount_due"] = 49_877.into();
    value["paid_amount"] = 49_877.into();
    let invoice: QrisInvoice = serde_json::from_value(value).unwrap();
    assert_eq!(invoice.unique_code_mode, UniqueCodeMode::Subtract);
    assert_eq!(
        invoice.amount_due,
        invoice.base_amount - i64::from(invoice.unique_code)
    );
    assert!(invoice.paid_exactly());
    // A mode this SDK does not know is not read as either.
    let mut value: serde_json::Value = serde_json::from_str(QRIS_INVOICE).unwrap();
    value["unique_code_mode"] = "minus".into();
    assert!(serde_json::from_value::<QrisInvoice>(value).is_err());
    // A negative code is not a code.
    let mut value: serde_json::Value = serde_json::from_str(QRIS_INVOICE).unwrap();
    value["unique_code"] = (-1).into();
    assert!(serde_json::from_value::<QrisInvoice>(value).is_err());
}

#[test]
fn a_tampered_qris_payload_fails_the_crc() {
    let invoice: QrisInvoice = serde_json::from_str(QRIS_INVOICE).unwrap();
    let tampered = invoice.qris.replace("540550123", "540550124");
    assert_ne!(tampered, invoice.qris);
    let (body, crc) = tampered.split_at(tampered.len() - 4);
    assert_ne!(crc, format!("{:04X}", crc16(body.as_bytes())));
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
        .chains(Family::Evm, ["base-sepolia"])
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
