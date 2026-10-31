//! The `reqwest` client against a small server on the loopback interface. The client under test is
//! the one a merchant gets from `Client::new`, with the same timeouts and redirect rule, and not
//! a test double.

#![cfg(feature = "reqwest")]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use kenari_pay_sdk::api::ApiError;
use kenari_pay_sdk::client::{default_http, Client, ClientError, CONNECT_TIMEOUT, TOTAL_TIMEOUT};
use kenari_pay_sdk::wire::{
    AlipayInvoiceStatus, CreateAlipayInvoice, CreateQrisInvoice, Family, QrisCheck,
    QrisCheckVerdict, QrisInvoiceStatus, QrisWatch,
};

const KEY: &str = "kp_test_SECRETKEYSECRETKEYSECRETKEYSECRETKEYSECRETK";

struct Reply {
    status: u16,
    headers: Vec<(&'static str, String)>,
    body: Vec<u8>,
}

fn reply(status: u16, body: &str) -> Reply {
    Reply {
        status,
        headers: Vec::new(),
        body: body.as_bytes().to_vec(),
    }
}

/// Read one HTTP request: the head and, when it has a `Content-Length`, the body.
fn read_request(stream: &mut TcpStream) -> String {
    let mut data = Vec::new();
    let mut buf = [0u8; 4096];
    let (head_end, length) = loop {
        let n = stream.read(&mut buf).unwrap();
        assert!(n > 0, "the client closed before it sent a whole request");
        data.extend_from_slice(&buf[..n]);
        if let Some(pos) = data.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&data[..pos]).to_ascii_lowercase();
            let length = head
                .lines()
                .find_map(|l| l.strip_prefix("content-length:"))
                .map(|v| v.trim().parse::<usize>().unwrap())
                .unwrap_or(0);
            break (pos + 4, length);
        }
    };
    while data.len() < head_end + length {
        let n = stream.read(&mut buf).unwrap();
        assert!(n > 0, "the client closed before it sent its body");
        data.extend_from_slice(&buf[..n]);
    }
    String::from_utf8(data).unwrap()
}

/// Answer each connection with the next reply and hand back the raw requests.
fn serve(replies: Vec<Reply>) -> (String, JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let handle = thread::spawn(move || {
        let mut seen = Vec::new();
        for reply in replies {
            let (mut stream, _) = listener.accept().unwrap();
            seen.push(read_request(&mut stream));
            let mut head = format!(
                "HTTP/1.1 {} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
                reply.status,
                reply.body.len()
            );
            for (name, value) in &reply.headers {
                head.push_str(&format!("{name}: {value}\r\n"));
            }
            head.push_str("\r\n");
            stream.write_all(head.as_bytes()).unwrap();
            stream.write_all(&reply.body).unwrap();
        }
        seen
    });
    (base, handle)
}

fn head_lines(request: &str) -> Vec<String> {
    request
        .split("\r\n\r\n")
        .next()
        .unwrap()
        .lines()
        .map(|l| l.to_ascii_lowercase())
        .collect()
}

#[tokio::test]
async fn a_read_sends_the_request_line_the_key_and_the_agent() {
    let (base, server) = serve(vec![reply(200, include_str!("../vectors/networks.json"))]);
    let client = Client::new(&format!("{base}/"), KEY).unwrap();
    let networks = client.networks().await.unwrap();
    assert_eq!(networks.networks.len(), 2);
    let seen = server.join().unwrap();
    let lines = head_lines(&seen[0]);
    assert_eq!(lines[0], "get /v1/networks http/1.1");
    assert!(lines.contains(&format!(
        "authorization: bearer {}",
        KEY.to_ascii_lowercase()
    )));
    assert!(lines
        .iter()
        .any(|l| l.starts_with("user-agent: kenari-pay-sdk/")));
    assert!(!lines.iter().any(|l| l.starts_with("content-type:")));
}

#[tokio::test]
async fn a_write_sends_its_json_body_and_decodes_the_address() {
    let (base, server) = serve(vec![reply(
        201,
        include_str!("../vectors/address_evm_forwarder.json"),
    )]);
    let client = Client::new(&base, KEY).unwrap();
    let address = client
        .create_address("usr_acc_7f3a", Family::Evm)
        .await
        .unwrap();
    assert_eq!(address.customer_ref, "usr_acc_7f3a");
    let seen = server.join().unwrap();
    let lines = head_lines(&seen[0]);
    assert_eq!(lines[0], "post /v1/addresses http/1.1");
    assert!(lines.contains(&"content-type: application/json".to_string()));
    let body = seen[0].split("\r\n\r\n").nth(1).unwrap();
    let sent: serde_json::Value = serde_json::from_str(body).unwrap();
    assert_eq!(
        sent,
        serde_json::json!({"customer_ref": "usr_acc_7f3a", "family": "evm"})
    );
}

#[tokio::test]
async fn every_call_reaches_its_route() {
    let page = include_str!("../vectors/deposits_page.json");
    let sweeps = include_str!("../vectors/sweeps_page.json");
    let (base, server) = serve(vec![
        reply(200, page),
        reply(200, sweeps),
        reply(200, include_str!("../vectors/deposit_confirmed.json")),
        reply(200, &serde_json::json!({"deposits": []}).to_string()),
        reply(200, include_str!("../vectors/address_evm.json")),
        reply(200, include_str!("../vectors/keys_list.json")),
        reply(
            200,
            r#"{"key_epoch":2,"attest_pubkey":"00","status":"active","created_at":1}"#,
        ),
        reply(
            200,
            r#"{"imported":2,"family":"evm","key_epoch":1,"next_index":2}"#,
        ),
        reply(201, include_str!("../vectors/qris_invoice_paid.json")),
        reply(200, include_str!("../vectors/qris_invoice_paid.json")),
        reply(200, include_str!("../vectors/qris_invoices_page.json")),
        reply(200, include_str!("../vectors/alipay_packs.json")),
        reply(201, include_str!("../vectors/alipay_invoice_paid.json")),
        reply(200, include_str!("../vectors/alipay_invoice_paid.json")),
        reply(200, include_str!("../vectors/alipay_invoices_page.json")),
        reply(200, include_str!("../vectors/alipay_invoice_paid.json")),
    ]);
    let client = Client::new(&base, KEY).unwrap();
    assert_eq!(client.deposits(42, 100).await.unwrap().next_seq, 43);
    assert_eq!(client.sweeps(0, 500).await.unwrap().next_seq, 43);
    let key = "base:0xabc:12";
    assert!(client.deposit(key).await.unwrap().is_some());
    assert!(client
        .customer_deposits("usr 1", 10)
        .await
        .unwrap()
        .deposits
        .is_empty());
    client.reissue("adr_1").await.unwrap();
    assert_eq!(client.keys().await.unwrap().keys.len(), 2);
    assert_eq!(client.register_key(2, "00").await.unwrap().key_epoch, 2);
    let file = serde_json::from_str(include_str!("../vectors/pool_file.json")).unwrap();
    assert_eq!(client.import_pool(&file).await.unwrap().imported, 2);
    let invoice = CreateQrisInvoice {
        max_unique_code: Some(99),
        ..CreateQrisInvoice::new("pay_01JAXF2Q8N3M6P9R1T4V7X0Z2C", "usr_acc_7f3a", 50_000)
    };
    let made = client.create_qris_invoice(&invoice).await.unwrap();
    assert_eq!(made.reference, invoice.reference);
    let again = client.qris_invoice(&invoice.reference).await.unwrap();
    assert_eq!(again.as_ref(), Some(&made));
    let page = client.qris_invoices(42, 100).await.unwrap();
    assert_eq!(page.invoices[1].status, QrisInvoiceStatus::Open);
    let packs = client.alipay_packs().await.unwrap();
    assert!(packs.available);
    let alipay = CreateAlipayInvoice::new(
        "pay_01JAXF2Q8N3M6P9R1T4V7X0Z2C",
        "usr_acc_7f3a",
        &packs.packs[1].sku_id,
    );
    let made = client.create_alipay_invoice(&alipay).await.unwrap();
    assert_eq!(made.reference, alipay.reference);
    let again = client.alipay_invoice(&alipay.reference).await.unwrap();
    assert_eq!(again.as_ref(), Some(&made));
    let page = client.alipay_invoices(42, 100).await.unwrap();
    assert_eq!(page.invoices[1].status, AlipayInvoiceStatus::Open);
    let cancelled = client.cancel_alipay_invoice(&made.id).await.unwrap();
    assert_eq!(cancelled.id, made.id);
    let seen = server.join().unwrap();
    let request_lines: Vec<String> = seen.iter().map(|r| head_lines(r)[0].clone()).collect();
    assert_eq!(
        request_lines,
        [
            "get /v1/deposits?after_seq=42&limit=100 http/1.1",
            "get /v1/sweeps?after_seq=0&limit=500 http/1.1",
            "get /v1/deposits/base%3a0xabc%3a12 http/1.1",
            "get /v1/customers/usr%201/deposits?limit=10 http/1.1",
            "post /v1/addresses/adr_1/reissue http/1.1",
            "get /v1/keys http/1.1",
            "post /v1/keys http/1.1",
            "post /v1/pool http/1.1",
            "post /v1/qris/invoices http/1.1",
            "get /v1/qris/invoices/pay_01jaxf2q8n3m6p9r1t4v7x0z2c http/1.1",
            "get /v1/qris/invoices?after_seq=42&limit=100 http/1.1",
            "get /v1/alipay/packs http/1.1",
            "post /v1/alipay/invoices http/1.1",
            "get /v1/alipay/invoices/pay_01jaxf2q8n3m6p9r1t4v7x0z2c http/1.1",
            "get /v1/alipay/invoices?after_seq=42&limit=100 http/1.1",
            "post /v1/alipay/invoices/ali_01jaxf3k9v0qh2m7z5r8t4w6yc/cancel http/1.1",
        ]
    );
    let body = seen[8].split("\r\n\r\n").nth(1).unwrap();
    let sent: serde_json::Value = serde_json::from_str(body).unwrap();
    assert_eq!(
        sent,
        serde_json::json!({
            "reference": "pay_01JAXF2Q8N3M6P9R1T4V7X0Z2C",
            "customer_ref": "usr_acc_7f3a",
            "amount_idr": 50000,
            "max_unique_code": 99
        })
    );
    assert!(head_lines(&seen[8]).contains(&"content-type: application/json".to_string()));
    let body = seen[12].split("\r\n\r\n").nth(1).unwrap();
    let sent: serde_json::Value = serde_json::from_str(body).unwrap();
    assert_eq!(
        sent,
        serde_json::json!({
            "reference": "pay_01JAXF2Q8N3M6P9R1T4V7X0Z2C",
            "customer_ref": "usr_acc_7f3a",
            "sku_id": "sku_100k"
        })
    );
    assert!(head_lines(&seen[12]).contains(&"content-type: application/json".to_string()));
    // The cancel has no body, so no content type.
    assert!(!head_lines(&seen[15])
        .iter()
        .any(|l| l.starts_with("content-type:")));
    assert_eq!(seen[15].split("\r\n\r\n").nth(1).unwrap(), "");
}

#[tokio::test]
async fn a_404_on_a_deposit_is_none() {
    let (base, server) = serve(vec![
        reply(
            404,
            r#"{"error":{"code":"not_found","message":"no such deposit"}}"#,
        ),
        reply(200, include_str!("../vectors/deposit_confirmed.json")),
    ]);
    let client = Client::new(&base, KEY).unwrap();
    assert_eq!(client.deposit("dep_missing").await.unwrap(), None);
    let found = client
        .deposit("dep_01JAXF2Q8N3M6P9R1T4V7X0Z2B")
        .await
        .unwrap();
    assert_eq!(found.unwrap().seq, 42);
    server.join().unwrap();
}

#[tokio::test]
async fn a_404_on_a_qris_invoice_is_none_and_a_503_keeps_its_code() {
    let (base, server) = serve(vec![
        reply(
            404,
            r#"{"error":{"code":"not_found","message":"no such invoice"}}"#,
        ),
        reply(503, include_str!("../vectors/error_qris_unavailable.json")),
    ]);
    let client = Client::new(&base, KEY).unwrap();
    assert_eq!(client.qris_invoice("qri_missing").await.unwrap(), None);
    let invoice = CreateQrisInvoice::new("pay_1", "usr_1", 1000);
    let error = client.create_qris_invoice(&invoice).await.unwrap_err();
    assert_eq!(
        error,
        ClientError::Api(ApiError::Status {
            code: 503,
            error_code: Some("unavailable".into())
        })
    );
    assert_eq!(error.to_string(), "gateway answered 503");
    server.join().unwrap();
}

#[tokio::test]
async fn a_404_on_an_alipay_invoice_is_none_and_the_refusals_keep_their_codes() {
    let conflict = r#"{"error":{"code":"conflict","message":"open invoice"}}"#;
    let (base, server) = serve(vec![
        reply(
            404,
            r#"{"error":{"code":"not_found","message":"no such invoice"}}"#,
        ),
        reply(
            503,
            include_str!("../vectors/error_alipay_unavailable.json"),
        ),
        reply(409, conflict),
        reply(
            404,
            r#"{"error":{"code":"not_found","message":"no such invoice"}}"#,
        ),
        reply(409, conflict),
    ]);
    let client = Client::new(&base, KEY).unwrap();
    assert_eq!(client.alipay_invoice("ali_missing").await.unwrap(), None);
    let invoice = CreateAlipayInvoice::new("pay_1", "usr_1", "sku_50k");
    let status = |code: u16, word: &str| {
        ClientError::Api(ApiError::Status {
            code,
            error_code: Some(word.into()),
        })
    };
    let error = client.create_alipay_invoice(&invoice).await.unwrap_err();
    assert_eq!(error, status(503, "unavailable"));
    assert_eq!(error.to_string(), "gateway answered 503");
    // One open invoice per customer.
    let error = client.create_alipay_invoice(&invoice).await.unwrap_err();
    assert_eq!(error, status(409, "conflict"));
    // A cancel of an invoice that is not there is an error, not `None`, and of a paid one a
    // conflict.
    let error = client
        .cancel_alipay_invoice("ali_missing")
        .await
        .unwrap_err();
    assert_eq!(error, status(404, "not_found"));
    let error = client.cancel_alipay_invoice("ali_paid").await.unwrap_err();
    assert_eq!(error, status(409, "conflict"));
    server.join().unwrap();
}

#[tokio::test]
async fn a_payment_check_posts_no_body_and_decodes_the_check_and_its_refusals() {
    let check = include_str!("../vectors/qris_check.json");
    let not_on =
        r#"{"error":{"code":"invalid_request","message":"Payment checks are not turned on"}}"#;
    let (base, server) = serve(vec![
        reply(200, check),
        reply(409, not_on),
        reply(
            404,
            r#"{"error":{"code":"not_found","message":"no such invoice"}}"#,
        ),
    ]);
    let client = Client::new(&base, KEY).unwrap();
    let reference = "pay_01M67Z6QZ4R2K8N5T7V9X1B3D5";
    let answer = client.check_qris_invoice(reference).await.unwrap();
    let want: QrisCheck = serde_json::from_str(check).unwrap();
    assert_eq!(answer, want);
    assert_eq!(answer.verdict, QrisCheckVerdict::NotFound);
    assert_eq!(answer.watch, QrisWatch::Started);
    assert_eq!(answer.invoice.reference, reference);
    assert_eq!(answer.invoice.watch_until, Some(1_793_516_520));
    // While payment checks are off the gateway refuses with a 409, kept with its code.
    let error = client.check_qris_invoice("qri_off").await.unwrap_err();
    assert_eq!(
        error,
        ClientError::Api(ApiError::Status {
            code: 409,
            error_code: Some("invalid_request".into())
        })
    );
    assert_eq!(error.to_string(), "gateway answered 409");
    // Another merchant's invoice is an error here, not `None`.
    let error = client.check_qris_invoice("qri/../x").await.unwrap_err();
    assert_eq!(
        error,
        ClientError::Api(ApiError::Status {
            code: 404,
            error_code: Some("not_found".into())
        })
    );
    let seen = server.join().unwrap();
    let request_lines: Vec<String> = seen.iter().map(|r| head_lines(r)[0].clone()).collect();
    assert_eq!(
        request_lines,
        [
            "post /v1/qris/invoices/pay_01m67z6qz4r2k8n5t7v9x1b3d5/check http/1.1",
            "post /v1/qris/invoices/qri_off/check http/1.1",
            "post /v1/qris/invoices/qri%2f..%2fx/check http/1.1",
        ]
    );
    for request in &seen {
        let lines = head_lines(request);
        assert!(lines.contains(&format!(
            "authorization: bearer {}",
            KEY.to_ascii_lowercase()
        )));
        // No body, so no content type.
        assert!(!lines.iter().any(|l| l.starts_with("content-type:")));
        assert_eq!(request.split("\r\n\r\n").nth(1).unwrap(), "");
    }
}

#[tokio::test]
async fn an_error_answer_is_a_status_with_the_code_and_a_fixed_text() {
    let (base, server) = serve(vec![
        reply(503, include_str!("../vectors/error_pool_exhausted.json")),
        reply(200, "this is not json"),
    ]);
    let client = Client::new(&base, KEY).unwrap();
    let error = client
        .create_address("usr_1", Family::Evm)
        .await
        .unwrap_err();
    assert_eq!(
        error,
        ClientError::Api(ApiError::Status {
            code: 503,
            error_code: Some("pool_exhausted".into())
        })
    );
    assert_eq!(error.to_string(), "gateway answered 503");
    let error = client.networks().await.unwrap_err();
    assert!(
        matches!(error, ClientError::Api(ApiError::Decode(_))),
        "{error:?}"
    );
    assert_eq!(error.to_string(), "gateway answer unreadable");
    server.join().unwrap();
}

#[tokio::test]
async fn a_redirect_is_not_followed_and_the_key_goes_nowhere_else() {
    // The elsewhere server must never be asked.
    let elsewhere = TcpListener::bind("127.0.0.1:0").unwrap();
    elsewhere.set_nonblocking(true).unwrap();
    let target = format!("http://{}/stolen", elsewhere.local_addr().unwrap());
    let mut redirect = reply(301, "");
    redirect.headers.push(("Location", target));
    let (base, server) = serve(vec![redirect]);
    let client = Client::new(&base, KEY).unwrap();
    let error = client.networks().await.unwrap_err();
    assert_eq!(
        error,
        ClientError::Api(ApiError::Status {
            code: 301,
            error_code: None
        })
    );
    server.join().unwrap();
    thread::sleep(Duration::from_millis(100));
    assert!(
        elsewhere.accept().is_err(),
        "the client followed the redirect"
    );
}

#[test]
fn the_key_never_goes_over_plain_http_to_another_host() {
    // Both constructors refuse it, as the same fixed error, before any client is used.
    let want = ClientError::Api(ApiError::BaseUrl);
    for base in [
        "http://pay.kenari.id",
        "http://127.0.0.1@evil.test",
        "ftp://x",
    ] {
        assert_eq!(Client::new(base, KEY).unwrap_err(), want, "{base}");
        let http = reqwest::Client::new();
        assert_eq!(
            Client::with_http(base, KEY, http).unwrap_err(),
            want,
            "{base}"
        );
    }
    assert_eq!(
        want.to_string(),
        "base URL must be https or a loopback address"
    );
    // https is accepted, and so is a loopback host over plain http, which the other tests use.
    assert!(Client::new("https://pay-sandbox.kenari.id", KEY).is_ok());
    assert!(Client::new("http://127.0.0.1:1", KEY).is_ok());
    assert!(Client::new("http://localhost:1", KEY).is_ok());
    assert!(Client::new("http://[::1]:1", KEY).is_ok());
}

#[tokio::test]
async fn a_dot_segment_is_refused_before_anything_is_sent() {
    // The server must never be asked: `/v1/customers/../deposits` would reach the feed of every
    // customer once a URL parser collapsed it.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let client = Client::new(&format!("http://{}", listener.local_addr().unwrap()), KEY).unwrap();
    let want = ClientError::Api(ApiError::Argument);
    for value in ["", ".", ".."] {
        assert_eq!(client.customer_deposits(value, 10).await.unwrap_err(), want);
        assert_eq!(client.deposit(value).await.unwrap_err(), want);
        assert_eq!(client.reissue(value).await.unwrap_err(), want);
        assert_eq!(client.qris_invoice(value).await.unwrap_err(), want);
        assert_eq!(client.alipay_invoice(value).await.unwrap_err(), want);
        assert_eq!(client.cancel_alipay_invoice(value).await.unwrap_err(), want);
        assert_eq!(client.check_qris_invoice(value).await.unwrap_err(), want);
    }
    assert_eq!(
        want.to_string(),
        "request argument cannot be a path segment"
    );
    thread::sleep(Duration::from_millis(100));
    assert!(listener.accept().is_err(), "a request was sent");
}

#[tokio::test]
async fn a_connection_that_cannot_be_made_is_a_transport_error_without_the_key_or_the_url() {
    // A port nobody listens on.
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let client = Client::new(&format!("http://127.0.0.1:{port}"), KEY).unwrap();
    let error = client
        .customer_deposits("customer-name-SECRET", 10)
        .await
        .unwrap_err();
    assert!(matches!(error, ClientError::Transport(_)), "{error:?}");
    assert_eq!(error.to_string(), "request to the gateway failed");
    let debug = format!("{error:?} {client:?}");
    assert!(!debug.contains("SECRETKEY"), "{debug}");
    assert!(!debug.contains("customer-name-SECRET"), "{debug}");
}

#[tokio::test]
async fn a_client_the_merchant_built_is_used_as_it_is() {
    // A server that accepts and never answers, and a client with a short timeout of the
    // merchant's own: the call ends by that timeout.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let hang = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        thread::sleep(Duration::from_millis(1500));
        drop(stream);
    });
    let http = reqwest::Client::builder()
        .timeout(Duration::from_millis(200))
        .build()
        .unwrap();
    let client = Client::with_http(&base, KEY, http).unwrap();
    let started = std::time::Instant::now();
    let error = client.networks().await.unwrap_err();
    assert!(matches!(error, ClientError::Transport(_)), "{error:?}");
    assert!(started.elapsed() < Duration::from_secs(1));
    hang.join().unwrap();
}

#[test]
fn the_default_client_has_the_documented_timeouts_and_follows_no_redirect() {
    assert_eq!(CONNECT_TIMEOUT, Duration::from_secs(5));
    assert_eq!(TOTAL_TIMEOUT, Duration::from_secs(20));
    // The Debug text of a `reqwest::Client` lists its total timeout and its redirect policy, and
    // not its connect timeout, which lives in the connector. It also lists the proxies of the
    // environment, so the text is checked and never printed.
    let shown = format!("{:?}", default_http().unwrap());
    assert!(shown.contains("TotalTimeout: 20s"), "no 20 s total timeout");
    assert!(shown.contains("Policy(None)"), "redirects are not off");
    assert!(shown.contains("kenari-pay-sdk/"), "no user agent");
}

#[test]
fn the_key_never_shows_in_the_debug_of_a_client() {
    let client = Client::new("http://127.0.0.1:1", KEY).unwrap();
    let shown = format!("{client:?}");
    assert!(!shown.contains("SECRETKEY"), "{shown}");
    assert!(shown.contains("<hidden>"));
}
