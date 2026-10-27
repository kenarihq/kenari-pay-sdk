//! The webhook check: the gateway's vector, every tamper of it, and the cells of kenari's own
//! `sig.rs` suite ported over, so a merchant that moves from its copy to the SDK loses no guard.

use kenari_pay_sdk::webhook::{self, WebhookError, TOLERANCE_SECS};
use kenari_pay_sdk::wire::{EventType, WebhookEvent};

const VECTOR: &str = include_str!("../vectors/webhook_signature_vector.json");
const BODY_EXAMPLE: &str = include_str!("../vectors/webhook_deposit_confirmed.json");
const QRIS_EXAMPLE: &str = include_str!("../vectors/webhook_qris_invoice_paid.json");

struct Vector {
    secret: String,
    ts: i64,
    body: String,
    header: String,
}

fn vector() -> Vector {
    let v: serde_json::Value = serde_json::from_str(VECTOR).unwrap();
    Vector {
        secret: v["secret"].as_str().unwrap().into(),
        ts: v["timestamp"].as_i64().unwrap(),
        body: v["raw_body"].as_str().unwrap().into(),
        header: v["header"].as_str().unwrap().into(),
    }
}

fn mac_of(v: &Vector) -> String {
    v.header.split("v1=").nth(1).unwrap().to_string()
}

#[test]
fn the_gateways_vector_verifies_and_reads_as_the_example_event() {
    let v = vector();
    let event = webhook::verify(&v.header, v.body.as_bytes(), &v.secret, v.ts).unwrap();
    assert_eq!(event.id, "evt_01JAXF3K9V0QH2M7Z5R8T4W6YB");
    assert_eq!(event.event_type, EventType::DepositConfirmed);
    assert_eq!(event.created_at, 1_760_700_000);
    let deposit = event.deposit.as_ref().unwrap();
    assert_eq!(deposit.deposit_id, "dep_01JAXF2Q8N3M6P9R1T4V7X0Z2B");
    assert_eq!(deposit.seq, 42);
    // The body of the vector is the example file, and both read the same.
    assert_eq!(v.body, BODY_EXAMPLE.trim_end());
    let example: WebhookEvent = serde_json::from_str(BODY_EXAMPLE).unwrap();
    assert_eq!(example, event);
    // And the header is what `sign` makes for it.
    assert_eq!(webhook::sign(&v.secret, v.ts, v.body.as_bytes()), v.header);
}

#[test]
fn the_vector_verifies_and_every_tamper_is_refused() {
    let v = vector();
    let ok = |secret: &str, header: &str, body: &str, now: i64| {
        webhook::verify_signature(header, body.as_bytes(), secret, now)
    };
    assert_eq!(ok(&v.secret, &v.header, &v.body, v.ts), Ok(()));
    assert_eq!(
        ok(&v.secret, &v.header, &v.body, v.ts + 300),
        Ok(()),
        "300 s late is inside"
    );
    assert_eq!(
        ok(&v.secret, &v.header, &v.body, v.ts - 300),
        Ok(()),
        "300 s early is inside"
    );
    assert_eq!(
        ok(&v.secret, &v.header, &v.body, v.ts + 301),
        Err(WebhookError::Stale),
        "301 s late"
    );
    assert_eq!(
        ok(&v.secret, &v.header, &v.body, v.ts - 301),
        Err(WebhookError::Stale),
        "301 s early"
    );
    let tampered = v.body.replace("\"seq\":42", "\"seq\":43");
    assert_ne!(tampered, v.body, "the tamper must change the body");
    assert_eq!(
        ok(&v.secret, &v.header, &tampered, v.ts),
        Err(WebhookError::Mismatch),
        "body changed"
    );
    assert_eq!(
        ok("whsec_other", &v.header, &v.body, v.ts),
        Err(WebhookError::Mismatch),
        "other secret"
    );
    assert_eq!(
        ok("", &v.header, &v.body, v.ts),
        Err(WebhookError::Mismatch),
        "empty secret fails closed"
    );
    // A valid MAC for another timestamp does not verify under this header's `t`.
    let reused = format!("t={},v1={}", v.ts + 1, mac_of(&v));
    assert_eq!(
        ok(&v.secret, &reused, &v.body, v.ts + 1),
        Err(WebhookError::Mismatch),
        "timestamp is signed"
    );
    // One hex digit of the MAC flipped.
    let mac = mac_of(&v);
    let mut flipped = mac.clone();
    flipped.replace_range(0..1, if mac.starts_with('0') { "1" } else { "0" });
    assert_eq!(
        ok(
            &v.secret,
            &format!("t={},v1={flipped}", v.ts),
            &v.body,
            v.ts
        ),
        Err(WebhookError::Mismatch),
        "one digit flipped"
    );
    // Whitespace appended, the body reserialized, a trailing newline: all other bytes.
    for other in [
        format!("{} ", v.body),
        format!("{}\n", v.body),
        v.body.replace(',', ", "),
        String::new(),
    ] {
        assert_eq!(
            ok(&v.secret, &v.header, &other, v.ts),
            Err(WebhookError::Mismatch),
            "{other:?}"
        );
    }
    // The secret is not trimmed or case folded.
    assert_eq!(
        ok(&format!("{} ", v.secret), &v.header, &v.body, v.ts),
        Err(WebhookError::Mismatch)
    );
    assert_eq!(
        ok(&v.secret.to_uppercase(), &v.header, &v.body, v.ts),
        Err(WebhookError::Mismatch)
    );
}

#[test]
fn an_attacker_who_signs_with_the_empty_key_gets_nowhere() {
    // The case the empty secret guard exists for: a deployment with a blank secret, and a forged
    // request whose MAC was made with the empty key.
    let v = vector();
    let forged = webhook::sign("", v.ts, v.body.as_bytes());
    assert_eq!(
        webhook::verify_signature(&forged, v.body.as_bytes(), "", v.ts),
        Err(WebhookError::Mismatch),
        "blank secret"
    );
    assert_eq!(
        webhook::verify(&forged, v.body.as_bytes(), "", v.ts),
        Err(WebhookError::Mismatch)
    );
    assert_eq!(
        webhook::verify_signature(&forged, v.body.as_bytes(), &v.secret, v.ts),
        Err(WebhookError::Mismatch),
        "the real secret against the forged MAC"
    );
}

#[test]
fn header_shapes() {
    let v = vector();
    let mac = mac_of(&v);
    let cases: &[(&str, String, Result<(), WebhookError>)] = &[
        ("empty", String::new(), Err(WebhookError::Malformed)),
        ("no v1", format!("t={}", v.ts), Err(WebhookError::Malformed)),
        ("no t", format!("v1={mac}"), Err(WebhookError::Malformed)),
        (
            "t not a number",
            format!("t=abc,v1={mac}"),
            Err(WebhookError::Malformed),
        ),
        (
            "v1 not hex",
            format!("t={},v1=zz", v.ts),
            Err(WebhookError::Malformed),
        ),
        (
            "v1 odd length",
            format!("t={},v1=abc", v.ts),
            Err(WebhookError::Malformed),
        ),
        ("no equals", "garbage".into(), Err(WebhookError::Malformed)),
        (
            "short mac",
            format!("t={},v1=00", v.ts),
            Err(WebhookError::Mismatch),
        ),
        (
            "empty mac",
            format!("t={},v1=", v.ts),
            Err(WebhookError::Mismatch),
        ),
        (
            "long mac",
            format!("t={},v1={mac}00", v.ts),
            Err(WebhookError::Mismatch),
        ),
        (
            "hostile t",
            format!("t={},v1={mac}", i64::MIN),
            Err(WebhookError::Stale),
        ),
        (
            "hostile t max",
            format!("t={},v1={mac}", i64::MAX),
            Err(WebhookError::Stale),
        ),
        (
            "t beyond i64",
            format!("t=99999999999999999999,v1={mac}"),
            Err(WebhookError::Malformed),
        ),
        (
            "spaces around parts",
            format!("t={} , v1={mac}", v.ts),
            Ok(()),
        ),
        (
            "unknown key ignored",
            format!("t={},v0=aa,v1={mac}", v.ts),
            Ok(()),
        ),
        (
            "order does not matter",
            format!("v1={mac},t={}", v.ts),
            Ok(()),
        ),
        (
            "upper case hex",
            format!("t={},v1={}", v.ts, mac.to_uppercase()),
            Ok(()),
        ),
    ];
    for (name, header, want) in cases {
        assert_eq!(
            webhook::verify_signature(header, v.body.as_bytes(), &v.secret, v.ts),
            *want,
            "header cell `{name}`"
        );
    }
}

#[test]
fn the_tolerance_is_300_seconds_and_a_hostile_now_cannot_overflow() {
    assert_eq!(TOLERANCE_SECS, 300);
    let v = vector();
    // `now` at the extremes with a sane `t`: no panic, and a stale answer.
    for now in [i64::MIN, i64::MAX] {
        assert_eq!(
            webhook::verify_signature(&v.header, v.body.as_bytes(), &v.secret, now),
            Err(WebhookError::Stale)
        );
    }
    // The distance between the two extremes does not fit in an i64 and is still one comparison.
    let header = webhook::sign(&v.secret, i64::MIN, v.body.as_bytes());
    assert_eq!(
        webhook::verify_signature(&header, v.body.as_bytes(), &v.secret, i64::MAX),
        Err(WebhookError::Stale)
    );
    // A correctly signed header at the edge of the range verifies for a `now` next to it.
    assert_eq!(
        webhook::verify_signature(&header, v.body.as_bytes(), &v.secret, i64::MIN + 300),
        Ok(())
    );
    assert_eq!(
        webhook::verify_signature(&header, v.body.as_bytes(), &v.secret, i64::MIN + 301),
        Err(WebhookError::Stale)
    );
}

#[test]
fn the_signature_is_checked_before_the_body_is_read() {
    let v = vector();
    // A body that is not JSON, signed by the real secret, is a body error.
    let junk = b"not json at all";
    let header = webhook::sign(&v.secret, v.ts, junk);
    assert_eq!(
        webhook::verify(&header, junk, &v.secret, v.ts),
        Err(WebhookError::Body)
    );
    // The same junk with a wrong signature is a signature error, so an unsigned body is never read.
    assert_eq!(
        webhook::verify(&v.header, junk, &v.secret, v.ts),
        Err(WebhookError::Mismatch)
    );
    // A body that is JSON and not an event, and one with a field missing, are body errors.
    for body in [
        r#"{"hello":"world"}"#,
        r#"{"id":"evt_1","type":"deposit.confirmed","created_at":1}"#,
        r#"{"id":"evt_1","type":"deposit.confirmed","created_at":1,"data":{"key":"k","seq":1}}"#,
        r#"[]"#,
        r#""#,
    ] {
        let header = webhook::sign(&v.secret, v.ts, body.as_bytes());
        assert_eq!(
            webhook::verify(&header, body.as_bytes(), &v.secret, v.ts),
            Err(WebhookError::Body),
            "{body}"
        );
    }
}

/// Verify a body signed with the vector's secret.
fn verified(body: &str) -> Result<WebhookEvent, WebhookError> {
    let v = vector();
    let header = webhook::sign(&v.secret, v.ts, body.as_bytes());
    webhook::verify(&header, body.as_bytes(), &v.secret, v.ts)
}

#[test]
fn an_event_type_from_a_newer_gateway_still_verifies_and_reads_as_unknown() {
    // The case the plan promises: any `data`, because the data of a type is that type's own.
    let body = r#"{"id":"evt_9","type":"sweep.confirmed","created_at":5,"data":{"sweep_id":"swp_1","seq":9}}"#;
    let event = verified(body).unwrap();
    assert_eq!(event.event_type, EventType::Unknown);
    assert_eq!(event.id, "evt_9");
    assert_eq!(event.created_at, 5);
    assert_eq!(event.deposit, None);
    assert_eq!(event.qris_invoice, None);

    // Whatever else the data of an unknown type is, or whether it is there at all.
    for data in [
        r#","data":{}"#,
        r#","data":null"#,
        r#","data":5"#,
        r#","data":"text""#,
        r#","data":[1,2]"#,
        r#","data":{"deposit_id":7,"key":null}"#,
        "",
    ] {
        let body = format!(
            r#"{{"id":"evt_9","type":"sweep.confirmed","created_at":5{data},"added":true}}"#
        );
        let event = verified(&body).unwrap_or_else(|e| panic!("{body}: {e:?}"));
        assert_eq!(event.event_type, EventType::Unknown, "{body}");
        assert_eq!(event.deposit, None, "{body}");
        assert_eq!(event.qris_invoice, None, "{body}");
    }

    // Data that looks like a deposit's, under a type this SDK does not know, is still not read as
    // one: only a `deposit.*` type says its data is deposit ids.
    let body = r#"{"id":"evt_9","type":"deposit.refunded","created_at":5,"data":{"deposit_id":"dep_9","key":"k","seq":9}}"#;
    let event = verified(body).unwrap();
    assert_eq!(event.event_type, EventType::Unknown);
    assert_eq!(event.deposit, None);
    // The same for data that looks like a QRIS invoice's under a `qris_invoice.*` type this SDK
    // does not know.
    let body = r#"{"id":"evt_9","type":"qris_invoice.refunded","created_at":5,"data":{"invoice_id":"qri_9","reference":"pay_9","seq":9}}"#;
    let event = verified(body).unwrap();
    assert_eq!(event.event_type, EventType::Unknown);
    assert_eq!((event.deposit, event.qris_invoice), (None, None));

    // The envelope is still needed for an event of any type.
    for body in [
        r#"{"type":"sweep.confirmed","created_at":5,"data":{}}"#,
        r#"{"id":"evt_9","type":"sweep.confirmed","data":{}}"#,
        r#"{"id":"evt_9","created_at":5,"data":{}}"#,
        r#"{"id":9,"type":"sweep.confirmed","created_at":5}"#,
        r#"{"id":"evt_9","type":"sweep.confirmed","created_at":"5"}"#,
        r#"{"id":"evt_9","type":7,"created_at":5}"#,
    ] {
        assert_eq!(verified(body), Err(WebhookError::Body), "{body}");
    }
}

#[test]
fn a_deposit_type_whose_data_is_not_deposit_shaped_is_still_a_body_error() {
    for word in [
        "deposit.detected",
        "deposit.confirmed",
        "deposit.orphaned",
        "deposit.reverted",
    ] {
        for data in [
            r#","data":{"sweep_id":"swp_1","seq":9}"#,
            r#","data":{"deposit_id":"dep_1","seq":1}"#,
            r#","data":{"deposit_id":"dep_1","key":"k"}"#,
            r#","data":{"deposit_id":1,"key":"k","seq":1}"#,
            r#","data":{"deposit_id":"dep_1","key":"k","seq":"1"}"#,
            r#","data":{"invoice_id":"qri_1","reference":"pay_1","seq":1}"#,
            r#","data":{}"#,
            r#","data":null"#,
            r#","data":[]"#,
            r#","data":"dep_1""#,
            "",
        ] {
            let body = format!(r#"{{"id":"evt_1","type":"{word}","created_at":1{data}}}"#);
            assert_eq!(verified(&body), Err(WebhookError::Body), "{body}");
        }
    }
}

#[test]
fn a_qris_invoice_type_whose_data_is_not_invoice_shaped_is_a_body_error() {
    for word in ["qris_invoice.paid", "qris_invoice.expired"] {
        for data in [
            r#","data":{"deposit_id":"dep_1","key":"k","seq":1}"#,
            r#","data":{"invoice_id":"qri_1","seq":1}"#,
            r#","data":{"invoice_id":"qri_1","reference":"pay_1"}"#,
            r#","data":{"reference":"pay_1","seq":1}"#,
            r#","data":{"invoice_id":1,"reference":"pay_1","seq":1}"#,
            r#","data":{"invoice_id":"qri_1","reference":null,"seq":1}"#,
            r#","data":{"invoice_id":"qri_1","reference":"pay_1","seq":"1"}"#,
            r#","data":{}"#,
            r#","data":null"#,
            r#","data":[]"#,
            r#","data":"qri_1""#,
            "",
        ] {
            let body = format!(r#"{{"id":"evt_1","type":"{word}","created_at":1{data}}}"#);
            assert_eq!(verified(&body), Err(WebhookError::Body), "{body}");
        }
    }
}

#[test]
fn the_qris_example_verifies_when_signed_and_reads_its_invoice_ids() {
    let event = verified(QRIS_EXAMPLE.trim_end()).unwrap();
    assert_eq!(event.id, "evt_01JAXF3K9V0QH2M7Z5R8T4W6YD");
    assert_eq!(event.event_type, EventType::QrisInvoicePaid);
    assert_eq!(event.created_at, 1_760_700_400);
    assert_eq!(event.deposit, None);
    let invoice = event.qris_invoice.as_ref().unwrap();
    assert_eq!(invoice.invoice_id, "qri_01JAXF3K9V0QH2M7Z5R8T4W6YC");
    assert_eq!(invoice.reference, "pay_01JAXF2Q8N3M6P9R1T4V7X0Z2C");
    assert_eq!(invoice.seq, 42);
    // A field a newer gateway adds to the data is ignored.
    let body = QRIS_EXAMPLE
        .trim_end()
        .replace(r#""seq":42}"#, r#""seq":42,"added":true}"#);
    assert_ne!(body, QRIS_EXAMPLE.trim_end());
    assert_eq!(verified(&body).unwrap().qris_invoice, event.qris_invoice);
    // And the body is signed like any other: one changed byte is refused.
    let v = vector();
    let header = webhook::sign(&v.secret, v.ts, QRIS_EXAMPLE.as_bytes());
    let tampered = QRIS_EXAMPLE.replace(r#""seq":42"#, r#""seq":43"#);
    assert_eq!(
        webhook::verify(&header, tampered.as_bytes(), &v.secret, v.ts),
        Err(WebhookError::Mismatch)
    );
}

#[test]
fn a_qris_event_serializes_back_to_its_body() {
    let example: WebhookEvent = serde_json::from_str(QRIS_EXAMPLE).unwrap();
    let value = serde_json::to_value(&example).unwrap();
    assert_eq!(
        value,
        serde_json::from_str::<serde_json::Value>(QRIS_EXAMPLE).unwrap()
    );
    assert_eq!(
        value["data"]["invoice_id"],
        "qri_01JAXF3K9V0QH2M7Z5R8T4W6YC"
    );
    let again: WebhookEvent = serde_json::from_value(value).unwrap();
    assert_eq!(again, example);
    let expired = verified(
        r#"{"id":"evt_2","type":"qris_invoice.expired","created_at":2,"data":{"invoice_id":"qri_2","reference":"pay_2","seq":7}}"#,
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(&expired).unwrap(),
        serde_json::json!({"id": "evt_2", "type": "qris_invoice.expired", "created_at": 2,
            "data": {"invoice_id": "qri_2", "reference": "pay_2", "seq": 7}})
    );
}

#[test]
fn an_unknown_event_serializes_as_its_envelope_and_a_deposit_event_keeps_its_data() {
    let unknown =
        verified(r#"{"id":"evt_9","type":"sweep.confirmed","created_at":5,"data":{"x":1}}"#)
            .unwrap();
    assert_eq!(
        serde_json::to_value(&unknown).unwrap(),
        serde_json::json!({"id": "evt_9", "type": "unknown", "created_at": 5})
    );
    let example: WebhookEvent = serde_json::from_str(BODY_EXAMPLE).unwrap();
    let value = serde_json::to_value(&example).unwrap();
    assert_eq!(
        value,
        serde_json::from_str::<serde_json::Value>(BODY_EXAMPLE).unwrap()
    );
    assert_eq!(value["data"]["seq"], 42);
}

#[test]
fn every_event_type_of_the_docs_reads() {
    let v = vector();
    for (word, want) in [
        ("deposit.detected", EventType::DepositDetected),
        ("deposit.confirmed", EventType::DepositConfirmed),
        ("deposit.orphaned", EventType::DepositOrphaned),
        ("deposit.reverted", EventType::DepositReverted),
    ] {
        let body = format!(
            r#"{{"id":"evt_1","type":"{word}","created_at":1,"data":{{"deposit_id":"dep_1","key":"k","seq":1}}}}"#
        );
        let header = webhook::sign(&v.secret, v.ts, body.as_bytes());
        let event = webhook::verify(&header, body.as_bytes(), &v.secret, v.ts).unwrap();
        assert_eq!(event.event_type, want, "{word}");
        // Every deposit type carries the ids, and only those types do.
        let deposit = event.deposit.as_ref().unwrap();
        assert_eq!(
            (
                deposit.deposit_id.as_str(),
                deposit.key.as_str(),
                deposit.seq
            ),
            ("dep_1", "k", 1),
            "{word}"
        );
        assert_eq!(event.qris_invoice, None, "{word}");
    }
    for (word, want) in [
        ("qris_invoice.paid", EventType::QrisInvoicePaid),
        ("qris_invoice.expired", EventType::QrisInvoiceExpired),
    ] {
        let body = format!(
            r#"{{"id":"evt_1","type":"{word}","created_at":1,"data":{{"invoice_id":"qri_1","reference":"pay_1","seq":1}}}}"#
        );
        let header = webhook::sign(&v.secret, v.ts, body.as_bytes());
        let event = webhook::verify(&header, body.as_bytes(), &v.secret, v.ts).unwrap();
        assert_eq!(event.event_type, want, "{word}");
        // Every QRIS invoice type carries the invoice ids, and only those types do.
        assert_eq!(event.deposit, None, "{word}");
        let invoice = event.qris_invoice.as_ref().unwrap();
        assert_eq!(
            (
                invoice.invoice_id.as_str(),
                invoice.reference.as_str(),
                invoice.seq
            ),
            ("qri_1", "pay_1", 1),
            "{word}"
        );
    }
}

#[test]
fn the_error_texts_are_fixed_and_hold_nothing_of_the_request() {
    let v = vector();
    let body = r#"{"id":"LEAK","type":"deposit.confirmed","created_at":1,"data":{}}"#;
    let header = webhook::sign(&v.secret, v.ts, body.as_bytes());
    let errors = [
        webhook::verify(&header, body.as_bytes(), &v.secret, v.ts).unwrap_err(),
        webhook::verify("LEAK", body.as_bytes(), &v.secret, v.ts).unwrap_err(),
        webhook::verify(&v.header, body.as_bytes(), &v.secret, v.ts).unwrap_err(),
        webhook::verify(&v.header, v.body.as_bytes(), &v.secret, 0).unwrap_err(),
    ];
    assert_eq!(
        errors,
        [
            WebhookError::Body,
            WebhookError::Malformed,
            WebhookError::Mismatch,
            WebhookError::Stale
        ]
    );
    for error in errors {
        let text = error.to_string();
        assert!(!text.contains("LEAK") && !text.contains(';'), "{text}");
    }
}
