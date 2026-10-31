# Changelog

## Unreleased

The QRIS payment check: for a customer who says they paid, the merchant asks the gateway to look
for the payment now and to keep watching the invoice for a day.

- `api` and `client`: `check_qris_invoice`, `POST /v1/qris/invoices/{id_or_reference}/check` with no
  body and the same path segment rule as the other calls. The gateway starts or extends a 24 hour
  watch of the invoice, does a fresh read of the merchant's GoPay feed and answers within about
  15 s, inside the default client's 20 s `TOTAL_TIMEOUT`. It answers `409` (code `invalid_request`)
  while payment checks are off, which is how they start, and a 404 is an error, not `None`.
- `wire`: `QrisCheck` (`invoice`, `verdict`, `payment`, `watch`, `read`), `QrisCheckVerdict`
  (`paid`, `needs_operator`, `cannot_read`, `not_found`), `QrisWatch` (`started`, `extended`,
  `full`, `too_old`, `not_needed`), both with `Unknown` for a word the SDK does not know,
  `QrisCheckPayment` (`feed_tx_id`, `amount`, `issuer`, `status`, `transaction_time`) and
  `QrisCheckRead` (`fresh`, `read_at`, `covered_until`, `feed_failing`). A check can pay the
  invoice but never chooses where money goes: credit by the invoice and its `qris_invoice.paid` as
  always.
- `wire`: `QrisInvoice::watch_until`, until when a payment check watches the invoice, `None` when it
  was never watched and when an older gateway leaves it out.
- `vectors/`: the gateway's `qris_check.json`, and its QRIS invoice examples with `watch_until`.

Breaking for code that builds a `QrisInvoice` with a struct literal: the new field.

## 0.5.0 (2026-10-28)

Alipay invoices: the merchant sells a points pack set in Alipay, the gateway drains the customer's
points balance at Alipay into the customer's invoice, and the merchant credits each drain.

- `wire`: `AlipayPack` (`sku_id`, `name`, `price` as Alipay's decimal text, `currency`, `points`),
  `AlipayPacks` (`available`, `packs`), `AlipayInvoice`, `AlipayInvoicesPage`, `AlipayInvoiceStatus`
  (`open`, `expired`, `cancelled`, `paid`, with `Unknown` for a word the SDK does not know) and
  `AlipayDrain` (`id`, `points`, `drained_at`). `AlipayInvoice::drains` is every drain into the
  invoice, oldest first, empty while open or when an answer leaves it out, and
  `AlipayInvoice::drained_points` is their sum, the gateway's `paid_points`. Credit each drain once,
  keyed on its `id`. `CreateAlipayInvoice` (`reference`, `customer_ref`, `sku_id`) with `new`.
- `wire`: `EventType::AlipayInvoicePaid` (`alipay_invoice.paid`, the first drain),
  `AlipayInvoiceDrained` (`alipay_invoice.drained`, a later drain) and `AlipayInvoiceExpired`
  (`alipay_invoice.expired`, expired or cancelled with nothing drained), and
  `EventType::is_alipay_invoice`. Their `data` is an `AlipayInvoiceRef` (`invoice_id`, `reference`,
  `seq`) in the new `WebhookEvent::alipay_invoice`. An `alipay_invoice.*` event whose `data` is not
  that is a body error. An unknown type still verifies with `deposit`, `qris_invoice` and
  `alipay_invoice` all `None`.
- `api` and `client`: `alipay_packs`, `create_alipay_invoice`, `alipay_invoice` (a 404 is `None`),
  `alipay_invoices` and `cancel_alipay_invoice`, with the same path segment rule as the other calls.
- `feed`: `Cursor::alipay_invoices` and `Feed::AlipayInvoices`.
- `vectors/`: the gateway's `alipay_packs.json`, `alipay_invoice_paid.json`,
  `alipay_invoices_page.json`, `webhook_alipay_invoice_paid.json` and `error_alipay_unavailable.json`.

Breaking for code that builds a `WebhookEvent` with a struct literal, or matches `EventType` or
`Feed` exhaustively: the new field and variants.

## 0.4.0 (2026-10-28)

- `wire`: `QrisInvoice::repeat_payments`, the settled payments of exactly `amount_due` that landed in
  the invoice's window besides the payment that paid it (the payer paid the same QR again), oldest
  first, empty when an older gateway leaves it out. An entry's `paid_at` can be earlier than the
  invoice's `paid_at` when the feed showed it later. Each is a `QrisRepeatPayment` (`feed_tx_id`, `amount`,
  `issuer`, `paid_at`, `reversed_at`) with `creditable_for(&invoice)`: exactly `amount_due` and not
  reversed. Credit each one once, keyed on its `feed_tx_id`.
- `wire`: `EventType::QrisInvoiceRepeatPaid` (`qris_invoice.repeat_paid`), sent when an invoice's
  repeat payments change. Its `data` is a `QrisInvoiceRef` like the other QRIS events: re-fetch the
  invoice and act on that.
- `vectors/`: the gateway's QRIS invoice examples with `repeat_payments`.

Breaking for code that builds a `QrisInvoice` with a struct literal, or matches `EventType`
exhaustively: the new field and variant.

## 0.3.0 (2026-10-27)

- `wire`: `UniqueCodeMode` (`add`, `subtract`). `CreateQrisInvoice::unique_code_mode`, left out of
  the body when `None` (the gateway's default, `add`): `subtract` takes the unique code off
  `amount_idr`, so the customer pays at most the amount, and falls back to adding a code of 1 to 999
  when every code up to `max_unique_code` is held. `QrisInvoice::unique_code_mode`, the mode the
  invoice used, `add` when an older gateway leaves it out. `amount_due` is `base_amount +
  unique_code` or `base_amount - unique_code` by it.
- `vectors/`: the gateway's QRIS invoice examples with `"unique_code_mode": "add"`.

Breaking for code that builds a `CreateQrisInvoice` or a `QrisInvoice` with a struct literal: the
new field.

## 0.2.0 (2026-10-27)

QRIS invoices, the gateway's rupiah rail.

- `wire`: `QrisInvoice`, `QrisInvoicesPage`, `QrisInvoiceStatus` (`open`, `paid`, `expired`),
  `MatchedBy` (`feed`, `operator`, `simulated`) and `QrisProvider` (`gopay`, `shopee`,
  `simulated`), each with `Unknown` for a word the SDK does not know. `CreateQrisInvoice` with the
  optional `max_unique_code` and `provider`, left out of the body when `None`.
  `QrisInvoice::paid_exactly` says whether a paid invoice carries exactly its amount due.
  Amounts are whole rupiah as integers, and `unique_code` is 0 to 999.
- `wire`: the webhook types `qris_invoice.paid` and `qris_invoice.expired`, whose `data` is a
  `QrisInvoiceRef` (`invoice_id`, `reference`, `seq`) in the new `WebhookEvent::qris_invoice`. A
  `qris_invoice.*` event whose `data` is not that is a body error, as for deposits. An unknown type
  still verifies with both `deposit` and `qris_invoice` as `None`. `EventType::is_deposit` and
  `EventType::is_qris_invoice`.
- `api` and `client`: `create_qris_invoice`, `qris_invoice` (a 404 is `None`) and `qris_invoices`.
- `feed`: `Cursor::qris_invoices` and `Feed::QrisInvoices`.
- `vectors/`: the gateway's `qris_invoice_paid.json`, `qris_invoices_page.json`,
  `webhook_qris_invoice_paid.json` and `error_qris_unavailable.json`.

Breaking for code that builds a `WebhookEvent` by hand or matches `EventType` or `Feed`
exhaustively: the new field and variants.

## 0.1.0 (2026-10-21)

The first version of the merchant SDK for kenari-pay.

- `wire`: the merchant API types. A nullable field is an `Option`, an unknown field is ignored,
  and an unknown family, kind, status or webhook type decodes to `Unknown`. `livemode` and
  `simulated` are required.
- `api`: the calls as plain `HttpRequest` data with no IO, `Api::decode` and `Api::decode_optional`
  for the answers, and `ApiError` with a fixed `Display`. It has `networks`, `create_address`,
  `reissue`, `deposit`, `deposits`, `customer_deposits` and `sweeps`, and for pool merchants
  `keys`, `register_key` and `import_pool`.
  `Api::new` refuses a base URL that is not `https://`, except a loopback host, so the key never
  goes over plain http. A value that would be an empty, `.` or `..` path segment is refused as
  `ApiError::Argument`, because a URL parser would collapse it into another route.
- `client` (feature `reqwest`, off by default): the same calls over `reqwest` with a 5 s connect
  timeout, a 20 s total timeout and no redirects.
- `address`: `verify` for forwarder addresses (recomputed from the merchant's own pins, payout
  wallets and maximum fee) and pool addresses (ed25519 attestation, `verify_strict`), with
  `forwarder::address`, `forwarder::salt` and `pins::BASE_SEPOLIA`.
  `Trust::chains(family, [...])` is keyed by family like `payouts`, and the forwarder check reads
  the EVM list only.
- `webhook`: `verify` with the `t=<unix>,v1=<hex>` header, HMAC-SHA256, a constant time compare, a
  300 s tolerance and an empty secret that refuses every webhook.
  An event of a type the SDK does not know verifies whatever its `data` holds, with
  `WebhookEvent::deposit` as `None`. The deposit ids are read only for a `deposit.*` type.
- `feed`: `Cursor` for the deposit and sweep feeds and the rule for advancing it.
- `vectors/`: byte copies of the gateway's `contract/*.json` and `contracts/vectors/addresses.json`,
  and `scripts/sync-vectors.sh` to refresh them.
- `examples/poll.rs`: a complete poll loop over `reqwest`.
