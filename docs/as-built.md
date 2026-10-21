# As built

What the crate is and the choices made where `docs/sdk-plan.md` of the gateway repository (section
"The crate `kenari-pay-sdk`") was silent. Where a choice goes beyond or against the plan it says so.

## Shape

```
src/lib.rs            crate docs are README.md, forbid(unsafe_code), deny(missing_docs)
src/wire.rs           the merchant API types
src/api.rs            Api, HttpRequest, Method, ApiError: requests and answers, no IO
src/client.rs         Client over reqwest, feature `reqwest`, off by default
src/address/mod.rs    verify, Trust, Expect, Verified, Refusal, ForwarderPin, Wallet
src/address/forwarder.rs  salt, initcode, CREATE2 address, address text (a copy of the gateway's)
src/address/attest.rs     the attestation message and its check
src/address/pins.rs   BASE_SEPOLIA
src/webhook.rs        verify, verify_signature, sign
src/feed.rs           Cursor
vectors/              byte copies of the gateway's contract/*.json and contracts/vectors/addresses.json
scripts/sync-vectors.sh   refreshes vectors/ from a kenari-pay checkout
tests/                one file per area, all reading vectors/ and nothing outside the repository
examples/poll.rs      a complete poll loop (feature `reqwest`)
```

Gates: `cargo build --locked`, `cargo clippy --all-targets --all-features -- -D warnings`,
`cargo test --all-features` (113 tests, 9 of them doc tests), `cargo test` with no features (102),
and `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features`, all green. Build with
`CARGO_PROFILE_DEV_DEBUG=0` and a target directory of your choice, the full debug build is large.

## Dependencies and the toolchain

- `rust-version = "1.97"` and a `rust-toolchain.toml` for 1.97.0. The gateway has no `rust-version`
  and pins 1.97.0 in its toolchain file, so the SDK says the same Rust.
- Same major versions as kenari's `engine/Cargo.lock` for `ed25519-dalek` (2, resolves to 2.2.0 with
  default features, as kenari uses it), `hmac` (0.12), `sha2` (0.10), `hex` (0.4), `subtle` (2) and
  `serde` (1), and `reqwest` 0.12 with `default-features = false` and `rustls-tls`, as kenari does,
  so kenari's build adds no second copy.
- `sha3` is 0.10, the gateway's version. It shares `digest` 0.10 with `hmac` 0.12 and `sha2` 0.10.
  Kenari's lock has a `sha3` 0.11 from another crate, which the SDK does not follow because it would
  bring a second `digest`.
- No `thiserror`. The error `Display` texts are written by hand so each one is a fixed phrase.
- Dev dependencies: `tokio` for the client tests and `curve25519-dalek` for one test that crafts a
  signature with a small order R, which is what `verify_strict` exists to refuse.

## wire

- `Family` has an `Unknown` variant too, although the plan lists only `kind`, `status` and the
  webhook type. A gateway that adds a family would otherwise make `networks()` and the feed fail to
  decode for a merchant on an older SDK, which is what the `Unknown` rule is for.
- `Unknown` keeps nothing of the text and serializes as `unknown`. A value that is not a string is
  still a decode error.
- `livemode` (on `Networks`, `Address`, `Deposit`, `SweepItem`) and `simulated` have no serde
  default. This is kenari's rule (types.rs E17): an answer that does not say which network it is
  from is refused.
- `Address::kind` is an `Option`, as the plan says "all optional". `None` is read as a pool address,
  which is how the gateway's own type reads an older answer that says nothing.
- `Deposit::amount_units()` returns `Option<u128>` and accepts only ASCII digits (no sign, no space,
  no empty text). `SweepItem` has `amount_units()` and `fee_units()` the same way, `None` for a
  null. There is no `i64` helper, kenari narrows the `u128` itself.
- `ErrorDetail::message` has a serde default so an error body without one still gives its code.
- Pool types: `Key`, `Keys`, `PoolEntry`, `PoolFile`, `PoolStats`, `PoolImported` (the answer of
  `POST /v1/pool`) and `KeyStatus`. `Key::pool` is keyed by the family name as text.
- Every wire type derives `Serialize` too, so the tests can check that an example round-trips and a
  merchant can store one.

## api

- `Api::new` trims white space around the URL and the key and the trailing slashes of the URL, as
  kenari's `from_config` does.
- `Api::new` returns a `Result` and refuses a base URL that is not `https://`, so the bearer key
  never crosses the network in clear text. The one exception is `http://` to a loopback host:
  `localhost`, `127.0.0.1` or `[::1]`, with or without a port, for tests and local fakes. The
  refusal is `ApiError::BaseUrl`, with a fixed text that holds no part of the URL. Choices where
  the brief left room:
  - The host must be exactly one of the three (a host that only starts like one, such as
    `127.0.0.1.evil.test`, is not a loopback host), `localhost` in any case, the port decimal and
    at most 65535. `127.0.0.2`, `0.0.0.0` and the other spellings of loopback are refused.
  - An authority with user info (`@`), a backslash, white space or a control character is refused
    for `https://` too, because a URL parser reads those differently from a plain split and the
    check must name the host the key goes to. The scheme may be upper case.
  - Nothing else about an `https://` URL is checked, its host and path are the merchant's own.
- A value that becomes a path segment (the `id_or_key` of `deposit`, the `customer_ref` of
  `customer_deposits`, the `address_id` of `reissue`) is refused when it is empty, `.` or `..`, as
  `ApiError::Argument` with a fixed text that holds no value. Those three calls return
  `Result<HttpRequest, ApiError>`. Every URL parser collapses a dot segment, so `..` would render
  `/v1/customers/../deposits`, which the gateway reads as `/v1/deposits`, the feed of every
  customer, and its page decodes as `CustomerDeposits`. Percent encoding the dots does not help
  (the WHATWG parser reads `%2e%2e` as `..`), so a `%` in a value is itself encoded: `%2e%2e`
  stays one segment, `%252e%252e`, and is not a dot segment. Those three values are the only ones
  that are a dot segment after encoding. `create_address` stays infallible: its `customer_ref`
  goes in the JSON body and not in a path (the gateway refuses `.` and `..` there as well).
- The `Authorization: Bearer` header is sent on every call, `networks` included, as kenari does.
  The gateway ignores it there. `Content-Type: application/json` is added only for a request with a
  body, so a reissue (a `POST` with no body) has none. No `Accept` header, the plan lists none.
- `Api::decode::<T>` has one return type, so the 404 rule of `deposit` is a second function,
  `Api::decode_optional::<T>`. Any 404 counts as "no such deposit", including one from a proxy in
  front of the gateway, so check the base URL with `networks()` at start up.
- `ApiError::Status { code, error_code }` keeps the gateway's `error.code` only when it is 1 to 64
  characters of lower case letters, digits and underscores. Anything else is `None`, so text from
  a hostile or broken party cannot ride along in `Debug`. The message is never kept.
- `ApiError::Decode` holds the parser's text, as kenari's does, for `Debug` and a log. `Display` is
  fixed: `gateway answered 503` and `gateway answer unreadable`.
- `limit` is passed as given. The gateway's ranges are in the doc of each call, the SDK does not
  clamp them.
- `HttpRequest` and `Api` print `<hidden>` for the bearer key in `Debug`.
- `limit` and `after_seq` are `u32` and `i64`, the types kenari uses.
- The retire call (`POST /v1/keys/{epoch}/retire`) is not in the SDK. The plan names three pool
  calls and this is a fourth.

## client

- `Client::new` returns a `Result` because building a `reqwest::Client` can fail, and because the
  base URL can be refused (see `Api::new`). `Client::with_http` returns a `Result` for the second
  reason.
- The default client has the 5 s connect and 20 s total timeouts, and follows no redirect. A
  redirect would carry the bearer key to wherever it points, and the gateway never redirects `/v1`.
  A 3xx therefore comes back as `ApiError::Status`. It also sets a user agent, `kenari-pay-sdk/<version>`.
- `default_http()` is public so a test can build the client the way the merchant gets it.
- `ClientError` is `Api(ApiError)` or `Transport(String)`. The transport text has the URL taken out
  (a customer ref sits in some paths). `Display` is fixed.
- `Client::send` takes a rendered `HttpRequest`, for a call the typed methods do not cover, and
  `Client::api()` gives the builder.
- The default client reads the proxy settings of the environment, as `reqwest` does. The tests talk
  to the loopback interface and so need it not to be proxied, which `NO_PROXY` normally says.

## address

- `Trust` is built with typed values so a bad entry cannot reach `verify`. `Trust::new()` trusts
  nothing, so every forwarder address is refused until the chains, pins and payout wallets are set.
  The fields are public and named as in the plan (`chains`, `forwarder_pins`, `payouts`,
  `max_fee_bps`, `attest_pubkeys`), with builder methods: `chains`, `pin`, `payouts`, `max_fee_bps`,
  `attest_key`. The ones that parse text return `Result<_, TrustError>`.
- `Trust::chains` is keyed by family, as `payouts` is: `Trust::chains(Family::Evm, [...])`, and the
  field is a `BTreeMap<Family, Vec<String>>`. The forwarder check reads the EVM list only, so a
  Solana chain listed next to the EVM one changes nothing (Solana has no forwarder and so no pin
  to ask for). A call adds to the family's list and does not replace it, which differs from
  `payouts` on purpose: a later call can then only make the check stricter (one more chain needing
  its pin) and never drop a chain. `Verified::chains` is the EVM list.
- `Wallet` is a canonical wallet address that can only be made by `Wallet::parse(family, text)`.
  EVM text is parsed strictly (a mixed case spelling must carry its EIP-55 checksum, as the gateway
  does for payout wallets) and kept lower case. Wallets compare in canonical form.
- `ForwarderPin::parse` does the same for a pin from configuration.
- A public key that is a small order point is refused when the trust is built
  (`TrustError::WeakKey`), as well as by `verify_strict` at every check.
- `Expect` borrows the `customer_ref`.
- An answer with no `kind` is checked as a pool address, so it needs a valid attestation. A pool
  address ignores the forwarder settings of the trust and a forwarder ignores the attestation keys.
- A forwarder in a family other than EVM is refused as `Refusal::Kind`.
- `Verified` has the canonical `address`, a `Model` (`Pool { key_epoch, pool_index }` or
  `Forwarder { payout, fee_bps }`), the `chains`, and `sealed_to_current()`, plus `kind()`,
  `payout()` and `fee_bps()`. For a pool address `chains` is empty (the pool check has nothing to do
  with a chain) and `sealed_to_current()` is `false`. The canonical `address` is an addition, so a
  merchant stores the spelling that was checked.
- A `Trust` with no EVM chain is refused for a forwarder only (`Refusal::NoChains`). The plan says
  "at least one" and the pool check does not use chains.
- The order of the checks, which decides the `Refusal` named when several fail: `livemode`,
  `customer_ref`, `family`, then the kind. Forwarder: family is EVM, block present, chains not
  empty, block factory and implementation parse, per chain a pin that equals them, payout parses and
  is one of the merchant's, fee at most the maximum, salt parses, the recomputed address equals the
  claimed one. Pool: the three attestation fields present, address canonical, a key for the epoch,
  the signature.
- `Refusal` has 15 variants, each with a fixed text. `NoPin` and `PinMismatch` carry the chain id
  from the merchant's own list, never anything the gateway sent.
- A fee above 300 is refused even when the merchant's maximum is higher, because the factory's
  `addressOf` refuses it.
- `forwarder` is a copy of the gateway's `crates/core/src/forwarder.rs` without `thiserror`, held to
  the same 20 vectors. `EvmAddress` is the name of its 20 byte array type.
- The free functions `canonical_address`, `attestation_message` and `verify_attestation` have the
  signatures of kenari's `sig.rs`, hex in and `bool` out, with its guards (the address is
  canonicalized first, white space around the hex is ignored, any malformed input is `false`).
- `pins::BASE_SEPOLIA` is the factory `0x7bd6793b424c8e01dd7dce50364db20a9587db54` and the
  implementation `0x7539fadbe11298643551a501358b138e01efe3f0`. There is no `pins::BASE`, since
  the mainnet factory does not exist yet.

## webhook

- The order of the checks is kenari's: header shape, then the tolerance (`abs_diff`), then the empty
  secret, then the MAC. An empty secret is `WebhookError::Mismatch`, not a variant of its own, so
  kenari's cells port unchanged.
- The MAC is compared with `subtle`'s `ConstantTimeEq` over the finished tag.
- A key that appears twice in the header is taken at its last value, as kenari does.
- `WebhookError::Body` is added for a body that is signed and is not an event. The signature is
  checked first, so an unsigned body is never parsed.
- An event type the SDK does not know is a success with `EventType::Unknown`, whatever its `data`
  holds, as `docs/api.md` and the plan promise (answer 2xx and ignore it). The envelope (`id`,
  `type`, `created_at`) is decoded first and the deposit ids only for a `deposit.*` type.
  `WebhookEvent::deposit` is `Option<DepositRef>` (the former `WebhookData`, renamed because it is
  the ids of a deposit and not the data of every event), `Some` for the four deposit types and
  `None` for `Unknown`. A deposit type whose `data` is missing, null or not `deposit_id`, `key`
  and `seq` is still `WebhookError::Body`. An unknown type that carries deposit shaped data is
  `None` too, because only a `deposit.*` type says its data is deposit ids. `WebhookEvent` reads
  through a private envelope type (`#[serde(try_from)]`) and still derives `Serialize`: a deposit
  event writes its ids as `data`, an `Unknown` event writes the envelope with the type `unknown`
  and no `data`, since it keeps nothing of either. `EventType::Unknown` now serializes as
  `unknown` like the other `Unknown` variants (it wrote `Unknown` before).
- `verify_signature` (the check alone) and `sign` (the header for a body, to test a handler) are
  public additions. A header signed with an empty secret is refused whatever it is checked against.

## feed

- `Cursor` has two constructors, `deposits(after_seq)` and `sweeps(after_seq)`, so one type serves
  both feeds, and `next(&Api, limit)` renders the request of the one it belongs to.
- A negative stored value counts as 0, as kenari reads its cursor. `advance_to` only moves forward.
- The module doc is the rule: advance after an item is processed, stop at the first failure.

## vectors and the gateway

- `scripts/sync-vectors.sh` removes `vectors/*.json` first, so a file the gateway deleted goes too.
- `tests/vectors.rs` fails when the set of files in `vectors/` changes, so a new example gets a
  test before it is accepted.
- The gateway repository has `crates/core/tests/sdk_vectors.rs`, which compares `vectors/` with
  `contract/` and `contracts/vectors/addresses.json` byte for byte when this checkout sits next to it.

## How it was checked

- The ported cells of kenari's `sig.rs` (webhook vector and tampers, header shapes, attestation
  vector and tampers, canonical address cells) are in `tests/webhook.rs` and `tests/attestation.rs`.
- Every example decodes and serializes back to the same JSON (`tests/vectors.rs`).
- One test per `Refusal` starts from the real example and changes one thing (`tests/address.rs`).
- A mutation pass removed each guard in turn (the empty secret check, the tolerance, the MAC compare,
  `verify_strict`, the livemode, customer, pin, payout, fee and recompute checks, the attestation,
  the epoch key, percent encoding, the 404 rule, the cursor rules, the `livemode` requirement and
  more, 25 in all). Every one made a test fail.
- The README snippets were run once against the vectors and `examples/poll.rs` against a local
  fake gateway (one credit, the cursor stored, a sandbox key refusing live objects). The README
  snippets are compiled as doc tests on every run and not executed.
