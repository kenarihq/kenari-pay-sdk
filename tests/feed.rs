//! The cursor of the pull feeds and the rule `docs/api.md` gives for it.

use kenari_pay_sdk::api::{Api, Method};
use kenari_pay_sdk::feed::{Cursor, Feed};
use kenari_pay_sdk::wire::{DepositsPage, SweepsPage};

const DEPOSITS_PAGE: &[u8] = include_bytes!("../vectors/deposits_page.json");
const SWEEPS_PAGE: &[u8] = include_bytes!("../vectors/sweeps_page.json");

fn api() -> Api {
    Api::new("https://pay-sandbox.kenari.id", "kp_test_example").unwrap()
}

#[test]
fn each_feed_renders_its_own_request() {
    let deposits = Cursor::deposits(42);
    assert_eq!(deposits.feed(), Feed::Deposits);
    let request = deposits.next(&api(), 100);
    assert_eq!(request.method, Method::Get);
    assert_eq!(
        request.url,
        "https://pay-sandbox.kenari.id/v1/deposits?after_seq=42&limit=100"
    );
    assert_eq!(request, api().deposits(42, 100));

    let sweeps = Cursor::sweeps(7);
    assert_eq!(sweeps.feed(), Feed::Sweeps);
    let request = sweeps.next(&api(), 500);
    assert_eq!(
        request.url,
        "https://pay-sandbox.kenari.id/v1/sweeps?after_seq=7&limit=500"
    );
    assert_eq!(request, api().sweeps(7, 500));
}

#[test]
fn the_cursor_only_moves_forward() {
    let mut cursor = Cursor::deposits(10);
    cursor.advance_to(9);
    assert_eq!(cursor.after_seq(), 10, "below");
    cursor.advance_to(10);
    assert_eq!(cursor.after_seq(), 10, "equal");
    cursor.advance_to(11);
    assert_eq!(cursor.after_seq(), 11);
    cursor.advance_to(i64::MIN);
    assert_eq!(cursor.after_seq(), 11);
    cursor.advance_to(i64::MAX);
    assert_eq!(cursor.after_seq(), i64::MAX);
}

#[test]
fn a_negative_stored_cursor_counts_as_the_start() {
    assert_eq!(Cursor::deposits(-5).after_seq(), 0);
    assert_eq!(Cursor::sweeps(i64::MIN).after_seq(), 0);
    assert_eq!(Cursor::deposits(0).after_seq(), 0);
    let mut cursor = Cursor::deposits(-1);
    cursor.advance_to(0);
    assert_eq!(cursor.after_seq(), 0);
}

#[test]
fn a_failure_stops_the_walk_and_the_failed_item_is_read_again() {
    let page: DepositsPage = Api::decode(200, DEPOSITS_PAGE).unwrap();
    assert_eq!(page.deposits.len(), 2);
    let mut cursor = Cursor::deposits(0);

    // The first deposit is processed, the second fails: the cursor stays on the first.
    let mut processed = Vec::new();
    for deposit in &page.deposits {
        if deposit.seq == 43 {
            break;
        }
        processed.push(deposit.key.clone());
        cursor.advance_to(deposit.seq);
    }
    assert_eq!(processed.len(), 1);
    assert_eq!(cursor.after_seq(), 42);

    // The next request asks for everything after 42, so the failed deposit comes again.
    let again = cursor.next(&api(), 100);
    assert!(
        again.url.ends_with("after_seq=42&limit=100"),
        "{}",
        again.url
    );

    // Once it works the cursor passes it.
    let after = cursor.after_seq();
    for deposit in page.deposits.iter().filter(|d| d.seq > after) {
        cursor.advance_to(deposit.seq);
    }
    assert_eq!(cursor.after_seq(), page.next_seq);
}

#[test]
fn a_deposit_that_comes_again_with_a_higher_seq_moves_the_cursor_again() {
    // The same deposit after a state change: same key, higher seq.
    let page: DepositsPage = Api::decode(200, DEPOSITS_PAGE).unwrap();
    let mut later = page.deposits[0].clone();
    later.seq = 50;
    let mut cursor = Cursor::deposits(0);
    cursor.advance_to(page.deposits[0].seq);
    cursor.advance_to(later.seq);
    assert_eq!(cursor.after_seq(), 50);
    assert_eq!(later.key, page.deposits[0].key);
}

#[test]
fn an_empty_page_leaves_the_cursor_where_it_is() {
    let body = br#"{"deposits":[],"next_seq":43,"has_more":false}"#;
    let page: DepositsPage = Api::decode(200, body).unwrap();
    let mut cursor = Cursor::deposits(43);
    for deposit in &page.deposits {
        cursor.advance_to(deposit.seq);
    }
    assert_eq!(cursor.after_seq(), page.next_seq);
}

#[test]
fn the_sweep_feed_walks_the_same_way() {
    let page: SweepsPage = Api::decode(200, SWEEPS_PAGE).unwrap();
    let mut cursor = Cursor::sweeps(0);
    for item in &page.sweeps {
        cursor.advance_to(item.seq);
    }
    assert_eq!(cursor.after_seq(), page.next_seq);
    assert_eq!(cursor.after_seq(), 43);
    // Several items share a sweep and a transaction and each has its own seq.
    assert_eq!(page.sweeps[0].sweep_id, page.sweeps[1].sweep_id);
    assert_ne!(page.sweeps[0].seq, page.sweeps[1].seq);
}

#[test]
fn a_cursor_is_plain_data_to_store_and_rebuild() {
    let mut cursor = Cursor::deposits(0);
    cursor.advance_to(42);
    let stored = cursor.after_seq().to_string();
    let rebuilt = Cursor::deposits(stored.parse().unwrap());
    assert_eq!(rebuilt, cursor);
}
