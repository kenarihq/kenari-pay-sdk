//! The request building and the answer decoding, with no IO at all.

use kenari_pay_sdk::api::{Api, ApiError, HttpRequest, Method};
use kenari_pay_sdk::wire::{
    Address, CustomerDeposits, Deposit, DepositsPage, Family, Key, Keys, Networks, PoolFile,
    PoolImported, SweepsPage,
};

const KEY: &str = "kp_test_SECRETKEYSECRETKEYSECRETKEYSECRETKEYSECRETK";
const BASE: &str = "https://pay-sandbox.kenari.id";

fn api() -> Api {
    Api::new(BASE, KEY).unwrap()
}

fn bearer() -> (String, String) {
    ("Authorization".to_string(), format!("Bearer {KEY}"))
}

fn json_header() -> (String, String) {
    ("Content-Type".to_string(), "application/json".to_string())
}

fn body_json(request: &HttpRequest) -> serde_json::Value {
    serde_json::from_slice(request.body.as_deref().unwrap()).unwrap()
}

// ---- rendering ------------------------------------------------------------------------------

#[test]
fn a_read_is_a_get_with_the_key_and_no_body() {
    let calls = [
        (api().networks(), "/v1/networks"),
        (api().keys(), "/v1/keys"),
        (
            api().deposit("dep_01JAXF2Q8N3M6P9R1T4V7X0Z2B").unwrap(),
            "/v1/deposits/dep_01JAXF2Q8N3M6P9R1T4V7X0Z2B",
        ),
        (
            api().deposits(42, 100),
            "/v1/deposits?after_seq=42&limit=100",
        ),
        (api().sweeps(0, 500), "/v1/sweeps?after_seq=0&limit=500"),
        (
            api().customer_deposits("usr_acc_7f3a", 10).unwrap(),
            "/v1/customers/usr_acc_7f3a/deposits?limit=10",
        ),
    ];
    for (request, path) in calls {
        assert_eq!(request.method, Method::Get, "{path}");
        assert_eq!(request.url, format!("{BASE}{path}"));
        assert_eq!(request.headers, vec![bearer()], "{path}");
        assert_eq!(request.body, None, "{path}");
    }
}

#[test]
fn a_write_is_a_post_with_a_json_body_and_its_content_type() {
    let request = api().create_address("usr_acc_7f3a", Family::Evm);
    assert_eq!(request.method, Method::Post);
    assert_eq!(request.url, format!("{BASE}/v1/addresses"));
    assert_eq!(request.headers, vec![bearer(), json_header()]);
    assert_eq!(
        body_json(&request),
        serde_json::json!({"customer_ref": "usr_acc_7f3a", "family": "evm"})
    );
    let solana = api().create_address("usr_acc_7f3a", Family::Solana);
    assert_eq!(body_json(&solana)["family"], "solana");

    let request = api().register_key(2, &"ab".repeat(32));
    assert_eq!(request.method, Method::Post);
    assert_eq!(request.url, format!("{BASE}/v1/keys"));
    assert_eq!(request.headers, vec![bearer(), json_header()]);
    assert_eq!(
        body_json(&request),
        serde_json::json!({"key_epoch": 2, "attest_pubkey": "ab".repeat(32)})
    );

    let file: PoolFile = serde_json::from_str(include_str!("../vectors/pool_file.json")).unwrap();
    let request = api().import_pool(&file);
    assert_eq!(request.method, Method::Post);
    assert_eq!(request.url, format!("{BASE}/v1/pool"));
    assert_eq!(request.headers, vec![bearer(), json_header()]);
    let sent: PoolFile = serde_json::from_slice(request.body.as_deref().unwrap()).unwrap();
    assert_eq!(sent, file);
}

#[test]
fn a_reissue_is_a_post_with_no_body_and_so_no_content_type() {
    let request = api().reissue("adr_01JAXEZ7Q4K2M8N5P1R3T6V9WB").unwrap();
    assert_eq!(request.method, Method::Post);
    assert_eq!(
        request.url,
        format!("{BASE}/v1/addresses/adr_01JAXEZ7Q4K2M8N5P1R3T6V9WB/reissue")
    );
    assert_eq!(request.headers, vec![bearer()]);
    assert_eq!(request.body, None);
}

#[test]
fn a_path_value_stays_one_path_segment() {
    // A deposit key has colons, a customer ref can hold anything.
    let key = "base:0x5c504ed432cb51138bcf09aa5e8a410dd4a1e204ef84bfed1be16dfba1b22060:12";
    let request = api().deposit(key).unwrap();
    assert_eq!(
        request.url,
        format!(
            "{BASE}/v1/deposits/base%3A0x5c504ed432cb51138bcf09aa5e8a410dd4a1e204ef84bfed1be16dfba1b22060%3A12"
        )
    );
    let request = api().customer_deposits("a/b?c=d#e f", 5).unwrap();
    assert_eq!(
        request.url,
        format!("{BASE}/v1/customers/a%2Fb%3Fc%3Dd%23e%20f/deposits?limit=5")
    );
    let request = api().reissue("../keys").unwrap();
    assert_eq!(
        request.url,
        format!("{BASE}/v1/addresses/..%2Fkeys/reissue")
    );
    // A customer ref with quotes, a newline and non ASCII is escaped in the JSON body.
    let nasty = "a\"b\\c\nd\u{e9}\u{1f600}";
    let request = api().create_address(nasty, Family::Evm);
    assert_eq!(body_json(&request)["customer_ref"], nasty);
}

#[test]
fn a_dot_segment_is_refused_by_every_call_that_takes_a_path_value() {
    // `/v1/customers/../deposits` collapses to `/v1/deposits` in every URL parser, the feed of
    // every customer, and it decodes as a page. So no request is built for it.
    for value in ["", ".", ".."] {
        assert_eq!(
            api().customer_deposits(value, 10),
            Err(ApiError::Argument),
            "customer_deposits {value:?}"
        );
        assert_eq!(
            api().deposit(value),
            Err(ApiError::Argument),
            "deposit {value:?}"
        );
        assert_eq!(
            api().reissue(value),
            Err(ApiError::Argument),
            "reissue {value:?}"
        );
    }
    // The refusal is fixed text and holds no value.
    assert_eq!(
        ApiError::Argument.to_string(),
        "request argument cannot be a path segment"
    );
}

#[test]
fn a_value_that_looks_encoded_stays_one_encoded_segment() {
    // `%2e%2e` is `..` to a URL parser. Its `%` is encoded, so it stays text.
    let request = api().customer_deposits("%2e%2e", 10).unwrap();
    assert_eq!(
        request.url,
        format!("{BASE}/v1/customers/%252e%252e/deposits?limit=10")
    );
    let request = api().deposit("%2E%2E").unwrap();
    assert_eq!(request.url, format!("{BASE}/v1/deposits/%252E%252E"));
    let request = api().reissue(".%2e").unwrap();
    assert_eq!(request.url, format!("{BASE}/v1/addresses/.%252e/reissue"));
    // A value with a slash is one segment, never two.
    let request = api().customer_deposits("a/b", 10).unwrap();
    assert_eq!(
        request.url,
        format!("{BASE}/v1/customers/a%2Fb/deposits?limit=10")
    );
    let request = api().customer_deposits("../..", 10).unwrap();
    assert_eq!(
        request.url,
        format!("{BASE}/v1/customers/..%2F../deposits?limit=10")
    );
}

#[test]
fn the_base_url_and_key_are_trimmed() {
    let api = Api::new("  https://pay.kenari.id///\n", "  kp_live_x \n").unwrap();
    let request = api.networks();
    assert_eq!(request.url, "https://pay.kenari.id/v1/networks");
    assert_eq!(
        request.headers,
        vec![("Authorization".to_string(), "Bearer kp_live_x".to_string())]
    );
}

#[test]
fn the_key_only_goes_to_https_or_to_a_loopback_host() {
    // The two gateways, and what a merchant may reasonably write around them.
    for base in [
        "https://pay.kenari.id",
        "https://pay-sandbox.kenari.id/",
        "HTTPS://pay.kenari.id",
        "https://pay.kenari.id:8443",
        "https://127.0.0.1",
        "https://proxy.example/pay",
    ] {
        assert!(Api::new(base, KEY).is_ok(), "{base} should be accepted");
    }
    // Plain http to the loopback interface, for a test or a local fake.
    for base in [
        "http://localhost",
        "http://LocalHost:8080",
        "http://localhost:8080/",
        "http://127.0.0.1",
        "http://127.0.0.1:34567",
        "http://[::1]",
        "http://[::1]:8080/",
        "HTTP://127.0.0.1:1",
    ] {
        assert!(Api::new(base, KEY).is_ok(), "{base} should be accepted");
    }
    // Plain http anywhere else would send the key in clear text.
    for base in [
        "http://pay.kenari.id",
        "http://pay-sandbox.kenari.id/",
        "http://127.0.0.2",
        "http://0.0.0.0",
        "http://127.1",
        "http://[::2]",
        "http://[::ffff:127.0.0.1]",
        "http://192.168.1.10:8080",
        // A host that only starts like a loopback one.
        "http://127.0.0.1.evil.test",
        "http://localhost.evil.test",
        "http://evil.test/localhost",
        "http://evil.test?127.0.0.1",
        "http://evil.test#localhost",
        // User info, which makes the real host the part after the `@`.
        "http://127.0.0.1@evil.test",
        "http://localhost:80@evil.test",
        "http://user:pw@localhost",
        "http://127.0.0.1\\@evil.test",
        // A port that is not a port.
        "http://localhost:",
        "http://localhost:80x",
        "http://localhost:99999",
        "http://localhost:-1",
        "http://[::1",
        "http://[::1]x",
        "http://[::1]:",
        // Characters a URL parser strips or reads differently.
        "http://local\thost",
        "http://127.0.0.1\n.evil.test",
        "http://127.0.0.1 .evil.test",
    ] {
        assert_eq!(
            Api::new(base, KEY).unwrap_err(),
            ApiError::BaseUrl,
            "{base:?} should be refused"
        );
    }
    // Not an http or https URL at all.
    for base in [
        "",
        "   ",
        "pay.kenari.id",
        "//pay.kenari.id",
        "https://",
        "https:///v1",
        "https://@",
        "https://pay.kenari.id@evil.test",
        "ftp://pay.kenari.id",
        "ws://localhost",
        "file:///etc/passwd",
        "https:pay.kenari.id",
        "localhost:8080",
    ] {
        assert_eq!(
            Api::new(base, KEY).unwrap_err(),
            ApiError::BaseUrl,
            "{base:?} should be refused"
        );
    }
}

#[test]
fn the_base_url_refusal_is_fixed_text_and_holds_nothing() {
    let error = Api::new("http://user:SECRETPASS@pay.kenari.id/SECRETPATH", KEY).unwrap_err();
    let shown = error.to_string();
    assert_eq!(shown, "base URL must be https or a loopback address");
    assert!(!shown.contains(';'));
    let debug = format!("{error:?}");
    assert_eq!(debug, "BaseUrl");
    assert!(!debug.contains("SECRET"));
}

#[test]
fn the_key_never_shows_in_debug() {
    let api = api();
    let shown = format!("{api:?}");
    assert!(!shown.contains("SECRETKEY"), "{shown}");
    assert!(shown.contains(BASE));
    let request = api.create_address("usr_1", Family::Evm);
    let shown = format!("{request:?}");
    assert!(!shown.contains("SECRETKEY"), "{shown}");
    assert!(shown.contains("Authorization") && shown.contains("<hidden>"));
    // The headers that are not secret still show, and the body is a length, not its text.
    assert!(shown.contains("application/json"));
    assert!(!shown.contains("usr_1"), "{shown}");
    // A clone is the same request.
    assert_eq!(request.clone(), request);
}

// ---- decoding -------------------------------------------------------------------------------

#[test]
fn every_example_decodes_through_the_call_that_returns_it() {
    let networks: Networks = Api::decode(200, include_bytes!("../vectors/networks.json")).unwrap();
    assert_eq!(networks.networks.len(), 2);
    let address: Address =
        Api::decode(200, include_bytes!("../vectors/address_evm_forwarder.json")).unwrap();
    assert_eq!(address.customer_ref, "usr_acc_7f3a");
    // A 201 is a success too.
    let address: Address = Api::decode(201, include_bytes!("../vectors/address_evm.json")).unwrap();
    assert_eq!(address.key_epoch, Some(1));
    let deposit: Deposit =
        Api::decode(200, include_bytes!("../vectors/deposit_confirmed.json")).unwrap();
    assert_eq!(deposit.seq, 42);
    let page: DepositsPage =
        Api::decode(200, include_bytes!("../vectors/deposits_page.json")).unwrap();
    assert_eq!(
        (page.deposits.len(), page.next_seq, page.has_more),
        (2, 43, false)
    );
    let sweeps: SweepsPage =
        Api::decode(200, include_bytes!("../vectors/sweeps_page.json")).unwrap();
    assert_eq!(sweeps.sweeps.len(), 3);
    let keys: Keys = Api::decode(200, include_bytes!("../vectors/keys_list.json")).unwrap();
    assert_eq!(keys.keys.len(), 2);
    let key: Key = serde_json::from_value(serde_json::to_value(&keys.keys[1]).unwrap()).unwrap();
    assert_eq!(key.key_epoch, 2);
    // The customer's deposits are the page without the cursor.
    let body = serde_json::json!({"deposits": page.deposits}).to_string();
    let customer: CustomerDeposits = Api::decode(200, body.as_bytes()).unwrap();
    assert_eq!(customer.deposits.len(), 2);
    let imported: PoolImported = Api::decode(
        200,
        br#"{"imported":2,"family":"evm","key_epoch":1,"next_index":2}"#,
    )
    .unwrap();
    assert_eq!((imported.imported, imported.next_index), (2, 2));
}

#[test]
fn a_non_2xx_answer_is_a_status_with_the_gateways_error_code() {
    let body = include_bytes!("../vectors/error_pool_exhausted.json");
    assert_eq!(
        Api::decode::<Address>(503, body),
        Err(ApiError::Status {
            code: 503,
            error_code: Some("pool_exhausted".into())
        })
    );
    for status in [400, 401, 403, 404, 409, 422, 429, 500, 503, 301, 199] {
        let error = Api::decode::<Networks>(status, body).unwrap_err();
        assert!(
            matches!(error, ApiError::Status { code, .. } if code == status),
            "{status}"
        );
    }
}

#[test]
fn a_body_that_is_not_the_error_shape_gives_no_code() {
    for body in [
        &b""[..],
        b"<html>Bad gateway</html>",
        b"{}",
        b"{\"error\":\"nope\"}",
        b"{\"error\":{\"message\":\"no code\"}}",
        b"{\"error\":{\"code\":7}}",
        b"null",
    ] {
        assert_eq!(
            Api::decode::<Networks>(502, body),
            Err(ApiError::Status {
                code: 502,
                error_code: None
            }),
            "{}",
            String::from_utf8_lossy(body)
        );
    }
    // A code that is not a plain word is dropped, so text from a hostile party never rides along.
    for code in ["Not Found", "<b>x</b>", "a-b", ""] {
        let body = serde_json::json!({"error": {"code": code, "message": "m"}}).to_string();
        assert_eq!(
            Api::decode::<Networks>(400, body.as_bytes()),
            Err(ApiError::Status {
                code: 400,
                error_code: None
            }),
            "{code:?}"
        );
    }
    // The message is optional on the wire and the code is still read.
    assert_eq!(
        Api::decode::<Networks>(404, br#"{"error":{"code":"not_found"}}"#),
        Err(ApiError::Status {
            code: 404,
            error_code: Some("not_found".into())
        })
    );
}

#[test]
fn a_2xx_body_that_is_not_the_shape_is_a_decode_error() {
    for body in [&b""[..], b"not json", b"{}", b"[]", b"null"] {
        let error = Api::decode::<Networks>(200, body).unwrap_err();
        assert!(matches!(error, ApiError::Decode(_)), "{error:?}");
    }
    // A gateway that forgot `livemode` is refused, not read as the sandbox.
    let error = Api::decode::<Networks>(200, br#"{"networks":[]}"#).unwrap_err();
    assert!(matches!(error, ApiError::Decode(_)));
    let error = Api::decode::<Deposit>(
        200,
        include_str!("../vectors/deposit_confirmed.json")
            .replace("\"simulated\": false", "\"x\": 1")
            .as_bytes(),
    )
    .unwrap_err();
    assert!(matches!(error, ApiError::Decode(_)));
}

#[test]
fn a_404_on_a_deposit_is_none_and_the_other_statuses_are_not() {
    let missing = br#"{"error":{"code":"not_found","message":"no such deposit"}}"#;
    assert_eq!(Api::decode_optional::<Deposit>(404, missing), Ok(None));
    assert_eq!(Api::decode_optional::<Deposit>(404, b""), Ok(None));
    let found: Option<Deposit> =
        Api::decode_optional(200, include_bytes!("../vectors/deposit_confirmed.json")).unwrap();
    assert_eq!(found.unwrap().id, "dep_01JAXF2Q8N3M6P9R1T4V7X0Z2B");
    assert_eq!(
        Api::decode_optional::<Deposit>(500, b"{}"),
        Err(ApiError::Status {
            code: 500,
            error_code: None
        })
    );
    assert_eq!(
        Api::decode_optional::<Deposit>(401, br#"{"error":{"code":"unauthorized"}}"#),
        Err(ApiError::Status {
            code: 401,
            error_code: Some("unauthorized".into())
        })
    );
    assert!(matches!(
        Api::decode_optional::<Deposit>(200, b"junk"),
        Err(ApiError::Decode(_))
    ));
    // `decode` itself never turns a 404 into a success.
    assert!(Api::decode::<Deposit>(404, missing).is_err());
}

#[test]
fn display_never_carries_a_body_or_a_parser_fragment() {
    let hostile = br#"{"error":{"code":"internal","message":"customer-secret at db host-7"}}"#;
    let errors = [
        Api::decode::<Networks>(503, hostile).unwrap_err(),
        Api::decode::<Networks>(200, br#"{"livemode":"customer-secret"}"#).unwrap_err(),
        ApiError::Decode("invalid type: string \"customer-secret\"".into()),
    ];
    for error in &errors {
        let shown = error.to_string();
        assert!(
            !shown.contains("secret") && !shown.contains("host-7"),
            "{error:?}: {shown}"
        );
        assert!(!shown.contains(';'), "{shown}");
    }
    assert_eq!(errors[0].to_string(), "gateway answered 503");
    assert_eq!(errors[1].to_string(), "gateway answer unreadable");
    // The detail is in Debug for a log, and it never has the gateway's message in it.
    let debug = format!("{:?}", errors[0]);
    assert!(
        debug.contains("internal") && !debug.contains("customer-secret"),
        "{debug}"
    );
}
