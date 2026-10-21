# Changelog

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
