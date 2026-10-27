# kenari-pay-sdk

The merchant SDK for [kenari-pay](https://pay.kenari.id), a watch-only deposit gateway. A merchant
asks for a customer's permanent deposit address, the customer sends a stablecoin to it, and the
gateway reports the deposit through a signed webhook and a pull feed.

With forwarder addresses a merchant's whole setup is an API key and a payout wallet. What it still
has to get right is small and easy to get wrong, and this crate does exactly that part:

- refuse an address that does not pay the merchant's own wallet,
- refuse a webhook that is not signed,
- credit only a confirmed deposit,
- advance the feed cursor only after a deposit has been processed.

It holds no secret and opens no connection. It is the wire types, the address check, the webhook
check and the building of requests, tested against the gateway's own examples. The full API is in
the gateway's `docs/api.md`, served at `https://pay.kenari.id/llms-full.txt`.

## Install

The crate is not on crates.io yet. Depend on it by git and pin a revision:

```toml
[dependencies]
kenari-pay-sdk = { git = "https://github.com/kenarihq/kenari-pay-sdk", rev = "<commit>" }
```

The crate never opens a connection by itself, so it works with whatever HTTP client you already
run. The off by default feature `reqwest` adds `client::Client`, the calls ready made over
`reqwest` with a 5 s connect and a 20 s total timeout and no redirects.

## Quickstart

### 1. Get set up

Ask the kenari-pay operator for a merchant account. You get an API key (`kp_test_...` on the
sandbox, `kp_live_...` on live) and a webhook secret (`whsec_...`), each shown once, and the
operator lists your payout wallet. Build against the sandbox, `https://pay-sandbox.kenari.id`, and
move to `https://pay.kenari.id` when it works. Every object carries `livemode`, and the check
below refuses any whose `livemode` is not the mode you run in.

### 2. Say what you trust

Everything in `Trust` is yours. None of it comes from the gateway, which is the point: an address
the gateway made up cannot pass by agreeing with itself.

```rust
use kenari_pay_sdk::address::{pins, Trust};
use kenari_pay_sdk::wire::Family;

fn trust() -> Result<Trust, kenari_pay_sdk::address::TrustError> {
    Ok(Trust::new()
        // The EVM chains you accept deposits on, and the factory you trust on each.
        .chains(Family::Evm, ["base-sepolia"])
        .pin("base-sepolia", pins::BASE_SEPOLIA)
        // Your own payout wallets. The first is the one new addresses must be sealed to.
        .payouts(Family::Evm, ["0xeb1aaeefe4370f06e25bc4bf41e6dc3807fa6dc1"])?
        // The most you agreed to pay, in basis points. 50 is 0.5 percent.
        .max_fee_bps(50))
}
```

Build it once at start up and do not start when it returns an error. A chain with no pin refuses
every forwarder address, so on live, until the mainnet factory is pinned, you stay on pool
addresses.

### 3. Allocate an address, verify it, then show it

`Api` renders each call to plain data and decodes the answer. You send the request with your own
client.

```rust
use kenari_pay_sdk::address::{self, Expect, Trust};
use kenari_pay_sdk::api::{Api, HttpRequest};
use kenari_pay_sdk::wire::{Address, Family};

/// The request that asks for a customer's address. Send it with your HTTP client.
fn ask(api: &Api, customer_ref: &str) -> HttpRequest {
    api.create_address(customer_ref, Family::Evm)
}

/// What to do with the answer. Returns the address to show, or an error to alert on.
fn check(
    trust: &Trust,
    customer_ref: &str,
    status: u16,
    body: &[u8],
) -> Result<String, Box<dyn std::error::Error>> {
    let answer: Address = Api::decode(status, body)?;
    // `false` is the sandbox, `true` is live.
    let expect = Expect::new(customer_ref, Family::Evm, false);
    let verified = address::verify(&answer, &expect, trust)?;
    // Show and store this spelling, the one that was checked.
    Ok(verified.address)
}
```

`verify` passes a forwarder address only when its block is present, the factory and the
implementation equal your pin for every EVM chain you listed, the payout is one of your wallets, the
fee is at most your maximum, and the address recomputes from all of it. A pool address passes only
when its attestation verifies under the key you hold for its epoch. Anything else is a `Refusal`
with a fixed text: never show that address, alert a person.

`Verified::sealed_to_current()` says whether a forwarder is sealed to the first of your payout
wallets, which tells a current address from one made for an older wallet. An address keeps paying
the wallet it was made for, so remove a lost wallet from your list to stop accepting addresses
sealed to it.

### 4. Take deposits from the pull feed

The feed is the truth. Webhooks are a faster hint and can be late, repeated or out of order. The
rule is: **advance the cursor only after a deposit has been processed, and stop at the first one
that fails**, so it is read again next time.

```rust
use kenari_pay_sdk::api::{Api, ApiError, HttpRequest};
use kenari_pay_sdk::feed::Cursor;
use kenari_pay_sdk::wire::{Deposit, DepositStatus, DepositsPage};

/// Your idempotent work for one deposit. Return `Err` to stop and read it again next time.
fn process(deposit: &Deposit) -> Result<(), String> {
    match deposit.status {
        // Credit once per `deposit.key`, only when confirmed and not below the minimum.
        DepositStatus::Confirmed if !deposit.below_minimum => {
            let _units = deposit.amount_units().ok_or("amount is not a number")?;
            // Re-fetch it with `api.deposit(&deposit.key)` first. Check that the re-fetched
            // `livemode` is your mode and that `simulated` is false on live, then credit.
            Ok(())
        }
        // Never debit on a reverted deposit without a person. Alert instead.
        DepositStatus::Reverted => Ok(()),
        _ => Ok(()),
    }
}

/// One pass over the feed. `send` is your HTTP client: it returns the status and the body.
fn poll(
    api: &Api,
    cursor: &mut Cursor,
    mut send: impl FnMut(HttpRequest) -> (u16, Vec<u8>),
    mut store: impl FnMut(i64),
) -> Result<(), ApiError> {
    loop {
        let (status, body) = send(cursor.next(api, 100));
        let page: DepositsPage = Api::decode(status, &body)?;
        for deposit in &page.deposits {
            if process(deposit).is_err() {
                return Ok(()); // stop here, read it again next pass
            }
            cursor.advance_to(deposit.seq);
            store(cursor.after_seq()); // in the same transaction as the credit, when you can
        }
        if !page.has_more {
            return Ok(());
        }
    }
}
```

A complete loop over `reqwest` is in `examples/poll.rs`. Run it with
`KENARI_PAY_API_KEY=kp_test_... cargo run --example poll --features reqwest`.

### 5. Check webhooks, if you want them

```rust
use kenari_pay_sdk::webhook::{self, WebhookError};

/// `signature` is the `Kenari-Pay-Signature` header, `body` the raw request body before any
/// parsing, `secret` your `whsec_...` and `now` your own clock in unix seconds.
fn on_webhook(signature: &str, body: &[u8], secret: &str, now: i64) -> Result<(), WebhookError> {
    let event = webhook::verify(signature, body, secret, now)?;
    if let Some(deposit) = &event.deposit {
        // The body carries ids only. Re-fetch `deposit.key` and act on that.
        let _key = &deposit.key;
    }
    if let Some(invoice) = &event.qris_invoice {
        // The same for a QRIS invoice: re-fetch `invoice.invoice_id` and act on that.
        let _id = &invoice.invoice_id;
    }
    // Both are `None` for an event type this SDK does not know. Ignore it and answer 2xx.
    Ok(()) // answer 2xx fast
}
```

The check is HMAC-SHA256 over `"<t>.<raw body>"`, compared in constant time, with a 300 s
tolerance either way. An empty secret refuses every webhook. A webhook proves the request came from
the gateway and not that the body is true, so never act on the body.

### 6. Credit rules

- Credit a deposit once per `key`, only when its `status` is `confirmed` and `below_minimum` is
  false, and only after a re-fetch. Check the re-fetched deposit, not the page that listed it.
- Its `livemode` must be the mode you run in. An object from the other network is never money of
  this one.
- `simulated` is `true` only for a deposit made with `POST /v1/sandbox/deposits`. It has no
  transaction on any chain, so never credit it on live. The live gateway always says `false`, so a
  `simulated` deposit there means something is wrong: stop and alert a person. On the sandbox you
  may credit it like a real one, which is how you test your flow.
- `detected` and `orphaned` are not creditable. `orphaned` can come back to `detected`.
- `reverted` means a confirmed deposit vanished. Alert a person and never debit automatically.
- A status or event type the SDK does not know decodes to `Unknown`. Never credit it, and put it
  where a person will see it.
- Amounts are decimal strings. `Deposit::amount_units()` parses one and gives `None` for anything
  that is not plain digits.

### 7. QRIS invoices, if the operator enabled them

A merchant the operator enabled QRIS for can also take rupiah. The gateway holds a QRIS merchant
account, adds a unique code of Rp0 to Rp999 to your amount so each open invoice of an amount is
told apart, and marks the invoice paid when exactly that amount lands.

```rust
use kenari_pay_sdk::api::{Api, ApiError, HttpRequest};
use kenari_pay_sdk::wire::{CreateQrisInvoice, MatchedBy, QrisInvoice};

/// The request for an invoice. `reference` is your own payment id: the same one again returns the
/// same invoice, so a retry after a timeout is safe.
fn ask(api: &Api, payment_id: &str, customer_ref: &str, amount_idr: i64) -> HttpRequest {
    let invoice = CreateQrisInvoice {
        // Keep the code small, so the total stays close to your amount.
        max_unique_code: Some(99),
        ..CreateQrisInvoice::new(payment_id, customer_ref, amount_idr)
    };
    api.create_qris_invoice(&invoice)
}

/// Show `qris` as a QR code and `amount_due` as the total, with the code as its own line.
fn show(status: u16, body: &[u8]) -> Result<(String, i64), ApiError> {
    let invoice: QrisInvoice = Api::decode(status, body)?;
    Ok((invoice.qris, invoice.amount_due))
}

/// After a `qris_invoice.paid` webhook or a feed item: re-fetch with `api.qris_invoice(id)` and
/// decide on that answer.
fn creditable(invoice: &QrisInvoice, live: bool) -> bool {
    invoice.livemode == live
        && invoice.paid_exactly()
        && !(live && invoice.matched_by == Some(MatchedBy::Simulated))
}
```

- Credit an invoice once per `id`, after a re-fetch, only when `paid_exactly()` holds: `status` is
  `paid` and `paid_amount` is `amount_due`. A paid invoice with another amount was assigned by an
  operator, so hold it for a person.
- `expired` is not final. A payment that lands before `match_until` still turns it `paid`, so keep
  the payment pending until then.
- `matched_by` `simulated` comes only from `POST /v1/sandbox/qris/invoices/{id}/pay` on the
  sandbox. On live it means something is wrong: stop and alert a person.
- The refusals are `403 forbidden` (QRIS not enabled), `503 unavailable` (no account can take the
  payment right now, or every code of the amount is taken) and `429 rate_limited` (the daily cap).
  Offer the customer another way to pay.
- The feed `GET /v1/qris/invoices` follows the cursor rule above, with `Cursor::qris_invoices` and
  at most 100 invoices a page.

## Tests against the gateway's own examples

`vectors/` holds byte copies of the gateway's `contract/*.json` and of
`contracts/vectors/addresses.json`. The tests decode every example into its wire type, run the 20
forwarder vectors and the webhook and attestation vectors, check the QRIS payload of each invoice
example (its CRC, and the amount it carries is `amount_due`), and refuse a tampered copy of each.
`scripts/sync-vectors.sh <kenari-pay checkout>` refreshes the copy. A language that gets an SDK
later runs the same vectors.

## License

Licensed under either of the Apache License, Version 2.0 (`LICENSE-APACHE`) or the MIT license (`LICENSE-MIT`) at your option.
Copyright 2026 kenari.id.
