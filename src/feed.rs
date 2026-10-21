//! Pure helpers for the two pull feeds, `GET /v1/deposits` and `GET /v1/sweeps`.
//!
//! Both are change feeds ordered by `seq`. Because `seq` is reassigned at every state change, a
//! deposit appears again, with a higher `seq`, each time it moves from `detected` to `confirmed`
//! and so on. The feed is the backstop under the webhook, and it is the truth.
//!
//! # The rule
//!
//! **Advance the stored cursor only after an item has been processed, and stop at the first item
//! that fails, so it is read again next time.**
//!
//! - Process the items of a page in the order they come.
//! - After each item is processed and its effect is stored, call [`Cursor::advance_to`] with the
//!   item's `seq` and store [`Cursor::after_seq`], in the same transaction as the effect when you
//!   can.
//! - On the first item that fails, stop. Do not skip it and do not read on, because the cursor
//!   would then pass an item you never processed and the feed would not show it again.
//! - Make the processing idempotent per deposit key, because an item is delivered again after any
//!   later state change and after a crash between the effect and the stored cursor.
//! - An item you can never process (a malformed one, say) is a decision of yours, not the feed's.
//!   Put it aside where a person will see it, then advance past it, or the feed stops there for good.
//! - Credit only a deposit that is `confirmed` and not `below_minimum`, after a re-fetch of it with
//!   `GET /v1/deposits/{key}`. Never debit on `reverted` without a person.
//!
//! ```
//! use kenari_pay_sdk::api::{Api, Method};
//! use kenari_pay_sdk::feed::Cursor;
//! use kenari_pay_sdk::wire::DepositsPage;
//!
//! let api = Api::new("https://pay-sandbox.kenari.id", "kp_test_example").unwrap();
//! let mut cursor = Cursor::deposits(0);
//!
//! // Render the request, send it with your own client, decode the answer.
//! let request = cursor.next(&api, 100);
//! assert_eq!(request.method, Method::Get);
//! assert_eq!(request.url, "https://pay-sandbox.kenari.id/v1/deposits?after_seq=0&limit=100");
//! let answer = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/vectors/deposits_page.json"));
//! let page: DepositsPage = Api::decode(200, answer).unwrap();
//!
//! for deposit in &page.deposits {
//!     let processed: Result<(), &str> = Ok(()); // your idempotent work for `deposit.key`
//!     if processed.is_err() {
//!         break; // read again next time
//!     }
//!     cursor.advance_to(deposit.seq);
//!     // store `cursor.after_seq()` now
//! }
//! assert_eq!(cursor.after_seq(), 43);
//! ```

use crate::api::{Api, HttpRequest};

/// Which feed a [`Cursor`] reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Feed {
    /// `GET /v1/deposits`
    Deposits,
    /// `GET /v1/sweeps`
    Sweeps,
}

/// A position in a feed: the `seq` of the last item processed. It starts at 0.
///
/// Store [`Cursor::after_seq`] and build the cursor from it again with [`Cursor::deposits`] or
/// [`Cursor::sweeps`]. See the [module](self) for the rule that decides when to advance it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cursor {
    feed: Feed,
    after_seq: i64,
}

impl Cursor {
    /// A cursor on the deposit feed after `after_seq`, 0 for the start. A negative stored value
    /// counts as 0, so a bad value re-reads from the start, which idempotent processing allows.
    pub fn deposits(after_seq: i64) -> Cursor {
        Cursor {
            feed: Feed::Deposits,
            after_seq: after_seq.max(0),
        }
    }

    /// A cursor on the sweep feed after `after_seq`, 0 for the start. A negative stored value
    /// counts as 0.
    pub fn sweeps(after_seq: i64) -> Cursor {
        Cursor {
            feed: Feed::Sweeps,
            after_seq: after_seq.max(0),
        }
    }

    /// The feed this cursor reads.
    pub fn feed(&self) -> Feed {
        self.feed
    }

    /// The `seq` of the last item processed, the value to store.
    pub fn after_seq(&self) -> i64 {
        self.after_seq
    }

    /// The request for the next page, `limit` items at most (1 to 500).
    pub fn next(&self, api: &Api, limit: u32) -> HttpRequest {
        match self.feed {
            Feed::Deposits => api.deposits(self.after_seq, limit),
            Feed::Sweeps => api.sweeps(self.after_seq, limit),
        }
    }

    /// Move the cursor to the `seq` of an item that has been processed. It only moves forward, so
    /// a `seq` at or below the cursor changes nothing.
    pub fn advance_to(&mut self, seq: i64) {
        if seq > self.after_seq {
            self.after_seq = seq;
        }
    }
}
