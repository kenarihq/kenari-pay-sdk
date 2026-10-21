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
use kenari_pay_sdk::wire::Family;

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
        ]
    );
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
