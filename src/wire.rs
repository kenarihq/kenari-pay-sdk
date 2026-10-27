//! The merchant API's types, as the gateway sends and reads them (`docs/api.md`).
//!
//! The rules every type here follows:
//!
//! - Every field the API says may be null is an [`Option`], and a field that is missing decodes
//!   the same as a null one.
//! - A field the SDK does not know is ignored, because a newer gateway may add fields.
//! - An unknown `kind`, `status`, `family` or webhook `type` decodes to the `Unknown` variant of
//!   its enum instead of failing, so a merchant on an older SDK never stops decoding the feed. The
//!   variant keeps nothing of the text, so match it as "something I do not understand yet" and
//!   hold the item for a person rather than guessing.
//! - `livemode` and `simulated` are required, with no default. An answer that does not say which
//!   network it comes from cannot be told apart from a sandbox one, so it fails to decode.
//! - Token amounts stay decimal strings, because a token amount can exceed 2^53.
//!   [`Deposit::amount_units`] parses one. QRIS amounts are whole rupiah as JSON integers, which
//!   never come near 2^53.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Address family. It decides the address format and the watcher code of the gateway.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Family {
    /// `0x` and 40 hex characters, one address for every EVM chain.
    Evm,
    /// A base58 public key.
    Solana,
    /// A family this SDK does not know.
    #[serde(other)]
    Unknown,
}

impl Family {
    /// The name on the wire. `unknown` for [`Family::Unknown`], which no gateway accepts.
    pub fn as_str(self) -> &'static str {
        match self {
            Family::Evm => "evm",
            Family::Solana => "solana",
            Family::Unknown => "unknown",
        }
    }
}

/// Which model an address belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AddressKind {
    /// Derived offline by the merchant and attested by the merchant's key.
    Pool,
    /// Computed by the gateway from a factory, the merchant's payout wallet and a salt.
    Forwarder,
    /// A kind this SDK does not know.
    #[serde(other)]
    Unknown,
}

impl AddressKind {
    /// The name on the wire.
    pub fn as_str(self) -> &'static str {
        match self {
            AddressKind::Pool => "pool",
            AddressKind::Forwarder => "forwarder",
            AddressKind::Unknown => "unknown",
        }
    }
}

/// Where a deposit is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DepositStatus {
    /// Seen at the chain head, not final. Do not credit.
    Detected,
    /// Final enough to credit.
    Confirmed,
    /// Dropped before confirmation. It can return to `detected`.
    Orphaned,
    /// Was confirmed, then vanished before the chain finalized. Never expected. Alert a person
    /// and never debit automatically.
    Reverted,
    /// A status this SDK does not know. Never credit it.
    #[serde(other)]
    Unknown,
}

impl DepositStatus {
    /// The name on the wire.
    pub fn as_str(self) -> &'static str {
        match self {
            DepositStatus::Detected => "detected",
            DepositStatus::Confirmed => "confirmed",
            DepositStatus::Orphaned => "orphaned",
            DepositStatus::Reverted => "reverted",
            DepositStatus::Unknown => "unknown",
        }
    }
}

/// The `type` of a webhook event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EventType {
    /// `deposit.detected`
    #[serde(rename = "deposit.detected")]
    DepositDetected,
    /// `deposit.confirmed`
    #[serde(rename = "deposit.confirmed")]
    DepositConfirmed,
    /// `deposit.orphaned`
    #[serde(rename = "deposit.orphaned")]
    DepositOrphaned,
    /// `deposit.reverted`
    #[serde(rename = "deposit.reverted")]
    DepositReverted,
    /// `qris_invoice.paid`
    #[serde(rename = "qris_invoice.paid")]
    QrisInvoicePaid,
    /// `qris_invoice.expired`
    #[serde(rename = "qris_invoice.expired")]
    QrisInvoiceExpired,
    /// An event type this SDK does not know. Answer 2xx and ignore it.
    #[serde(other, rename = "unknown")]
    Unknown,
}

impl EventType {
    /// The name on the wire.
    pub fn as_str(self) -> &'static str {
        match self {
            EventType::DepositDetected => "deposit.detected",
            EventType::DepositConfirmed => "deposit.confirmed",
            EventType::DepositOrphaned => "deposit.orphaned",
            EventType::DepositReverted => "deposit.reverted",
            EventType::QrisInvoicePaid => "qris_invoice.paid",
            EventType::QrisInvoiceExpired => "qris_invoice.expired",
            EventType::Unknown => "unknown",
        }
    }

    /// Whether the event is about a deposit, so its `data` is a [`DepositRef`].
    pub fn is_deposit(self) -> bool {
        matches!(
            self,
            EventType::DepositDetected
                | EventType::DepositConfirmed
                | EventType::DepositOrphaned
                | EventType::DepositReverted
        )
    }

    /// Whether the event is about a QRIS invoice, so its `data` is a [`QrisInvoiceRef`].
    pub fn is_qris_invoice(self) -> bool {
        matches!(
            self,
            EventType::QrisInvoicePaid | EventType::QrisInvoiceExpired
        )
    }
}

/// Where a QRIS invoice is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QrisInvoiceStatus {
    /// Waiting for the payment. Do not credit.
    Open,
    /// A payment was matched to it. See [`QrisInvoice::paid_exactly`] before crediting.
    Paid,
    /// Its time to pay ran out without a match. A late payment can still move it to `paid` until
    /// `match_until`, so keep it pending until then.
    Expired,
    /// A status this SDK does not know. Never credit it.
    #[serde(other)]
    Unknown,
}

impl QrisInvoiceStatus {
    /// The name on the wire.
    pub fn as_str(self) -> &'static str {
        match self {
            QrisInvoiceStatus::Open => "open",
            QrisInvoiceStatus::Paid => "paid",
            QrisInvoiceStatus::Expired => "expired",
            QrisInvoiceStatus::Unknown => "unknown",
        }
    }
}

/// The kind of QRIS account an invoice collects with. The merchant never talks to it: the gateway
/// holds the account and watches its feed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QrisProvider {
    /// A GoPay merchant account (GoBiz).
    Gopay,
    /// A ShopeePay merchant account.
    Shopee,
    /// The sandbox's simulated account. Never on live.
    Simulated,
    /// A provider this SDK does not know. The invoice is still read, and its status still decides.
    /// Never send it in a request, no gateway accepts it.
    #[serde(other)]
    Unknown,
}

impl QrisProvider {
    /// The name on the wire.
    pub fn as_str(self) -> &'static str {
        match self {
            QrisProvider::Gopay => "gopay",
            QrisProvider::Shopee => "shopee",
            QrisProvider::Simulated => "simulated",
            QrisProvider::Unknown => "unknown",
        }
    }
}

/// How a paid QRIS invoice was matched to its payment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchedBy {
    /// The gateway's watcher found exactly one payment of the amount due in the invoice's window.
    Feed,
    /// An operator assigned a payment by hand. `paid_amount` can differ from `amount_due`.
    Operator,
    /// `POST /v1/sandbox/qris/invoices/{id}/pay` on the sandbox. Never on live.
    Simulated,
    /// A way this SDK does not know. Hold the invoice for a person.
    #[serde(other)]
    Unknown,
}

impl MatchedBy {
    /// The name on the wire.
    pub fn as_str(self) -> &'static str {
        match self {
            MatchedBy::Feed => "feed",
            MatchedBy::Operator => "operator",
            MatchedBy::Simulated => "simulated",
            MatchedBy::Unknown => "unknown",
        }
    }
}

/// Whether a key epoch still hands out addresses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyStatus {
    /// New addresses come from it.
    Active,
    /// No new addresses, the old ones are still watched.
    Retired,
    /// A status this SDK does not know.
    #[serde(other)]
    Unknown,
}

/// A token the gateway can receive on a chain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Asset {
    /// `<chain>:<symbol lowercase>`, for example `base:usdc`.
    pub asset: String,
    /// The token's symbol.
    pub symbol: String,
    /// The token contract on the chain (the mint on Solana).
    pub contract: String,
    /// Decimals of the token.
    pub decimals: u8,
    /// The smallest deposit reported as creditable, a decimal string in the smallest unit.
    pub min_units: String,
}

/// One enabled network.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Network {
    /// The chain id, for example `base`.
    pub chain: String,
    /// The chain's family.
    pub family: Family,
    /// A name for people.
    pub name: String,
    /// The assets enabled on the chain.
    pub assets: Vec<Asset>,
    /// About how long a deposit takes to confirm, in seconds.
    pub confirm_hint_secs: u32,
    /// The explorer link with `{tx}` where the transaction hash goes.
    pub explorer_tx_url: String,
}

/// The answer of `GET /v1/networks`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Networks {
    /// `true` on the live gateway, `false` on the sandbox.
    pub livemode: bool,
    /// The enabled networks.
    pub networks: Vec<Network>,
}

/// What a merchant needs to recompute a forwarder address.
///
/// Every address is lower case and the salt is `0x` and 64 hex characters. None of it is to be
/// trusted before [`crate::address::verify`] has compared it with what the merchant pinned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForwarderBlock {
    /// The factory that deploys the forwarder.
    pub factory: String,
    /// The implementation the factory clones.
    pub implementation: String,
    /// The wallet the forwarder pays out to.
    pub payout: String,
    /// The fee sealed into the address, in basis points.
    pub fee_bps: u16,
    /// The CREATE2 salt.
    pub salt: String,
}

/// A customer's deposit address (`POST /v1/addresses` and `POST /v1/addresses/{id}/reissue`).
///
/// Run it through [`crate::address::verify`] before you show it or store it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Address {
    /// `adr_...`
    pub id: String,
    /// The merchant's id for the customer, echoed.
    pub customer_ref: String,
    /// The address family.
    pub family: Family,
    /// The address, in canonical form.
    pub address: String,
    /// `pool` or `forwarder`. An answer that says nothing is a pool address.
    pub kind: Option<AddressKind>,
    /// For a forwarder, the block the address is recomputed from. Null for a pool address.
    pub forwarder: Option<ForwarderBlock>,
    /// The key epoch of a pool address. Null for a forwarder.
    pub key_epoch: Option<u32>,
    /// The index within the epoch's pool. Null for a forwarder.
    pub pool_index: Option<u32>,
    /// The hex ed25519 attestation of a pool address. Null for a forwarder.
    pub attestation: Option<String>,
    /// The enabled networks of the family. This is the gateway's claim and decides nothing.
    pub networks: Vec<Network>,
    /// `true` on the live gateway, `false` on the sandbox.
    pub livemode: bool,
    /// When the address was first allocated, in unix seconds.
    pub created_at: i64,
}

/// A deposit, as the pull feed and `GET /v1/deposits/{id_or_key}` show it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Deposit {
    /// `dep_...`
    pub id: String,
    /// `{chain}:{tx_hash}:{event_index}`, the natural idempotency key.
    pub key: String,
    /// The feed position, reassigned on every state change.
    pub seq: i64,
    /// The merchant's id for the customer that owns the address.
    pub customer_ref: String,
    /// `adr_...` of the receiving address.
    pub address_id: String,
    /// The family of the receiving address.
    pub family: Family,
    /// The receiving address.
    pub address: String,
    /// The chain id.
    pub chain: String,
    /// The asset id.
    pub asset: String,
    /// The token's symbol.
    pub symbol: String,
    /// The token contract.
    pub contract: String,
    /// Decimals of the token.
    pub decimals: u8,
    /// The amount, a decimal string in the smallest unit. See [`Deposit::amount_units`].
    pub amount_units: String,
    /// The transaction hash or signature.
    pub tx_hash: String,
    /// EVM: the log index as a decimal string. Solana: the receiving token account.
    pub event_index: String,
    /// The sender when known.
    pub from: Option<String>,
    /// The block number or slot.
    pub block_number: i64,
    /// The block hash, null while unknown.
    pub block_hash: Option<String>,
    /// Where the deposit is in its life.
    pub status: DepositStatus,
    /// `true` when `amount_units` is below the asset's `min_units`. Do not credit it.
    pub below_minimum: bool,
    /// When it was first seen, in unix seconds.
    pub detected_at: i64,
    /// When it was confirmed, null until then.
    pub confirmed_at: Option<i64>,
    /// The explorer link, null for a simulated deposit.
    pub explorer_url: Option<String>,
    /// `true` on the live gateway, `false` on the sandbox.
    pub livemode: bool,
    /// `true` only for a sandbox deposit made without a chain.
    pub simulated: bool,
}

impl Deposit {
    /// `amount_units` as an integer. `None` for anything that is not plain decimal digits that fit
    /// in a `u128`, so the caller holds the deposit instead of guessing. A sign, a space or an
    /// empty text is not a number here.
    pub fn amount_units(&self) -> Option<u128> {
        parse_units(&self.amount_units)
    }
}

/// The answer of `GET /v1/deposits`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DepositsPage {
    /// The deposits with `seq` above the cursor, ascending.
    pub deposits: Vec<Deposit>,
    /// The last `seq` of the page, or the cursor when the page is empty.
    pub next_seq: i64,
    /// Whether more follow.
    pub has_more: bool,
}

/// The answer of `GET /v1/customers/{customer_ref}/deposits`, newest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomerDeposits {
    /// The customer's newest deposits, below minimum ones included.
    pub deposits: Vec<Deposit>,
}

/// One thing a sweep moved into the merchant's payout wallet: a token, or the native currency, of
/// one forwarder address.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SweepItem {
    /// `swi_...`
    pub id: String,
    /// The feed position.
    pub seq: i64,
    /// `swp_...`, the transaction this item was in. Several items share one.
    pub sweep_id: String,
    /// The chain id.
    pub chain: String,
    /// The asset id, null for the chain's native currency.
    pub asset: Option<String>,
    /// The symbol.
    pub symbol: String,
    /// The token contract, null for the native currency.
    pub contract: Option<String>,
    /// Decimals, 18 for the native currency.
    pub decimals: u8,
    /// `adr_...` of the flushed address.
    pub address_id: String,
    /// The merchant's id for the customer.
    pub customer_ref: String,
    /// The flushed deposit address.
    pub address: String,
    /// The payout wallet the address is sealed to.
    pub payout: String,
    /// What reached the payout wallet, a decimal string. Null for a value that did not fit 64 bits.
    pub amount_units: Option<String>,
    /// What reached the fee recipient, a decimal string. Null for a value that did not fit 64 bits.
    pub fee_units: Option<String>,
    /// The transaction hash.
    pub tx_hash: String,
    /// The block number.
    pub block_number: i64,
    /// When the sweep was confirmed, in unix seconds.
    pub confirmed_at: i64,
    /// The explorer link.
    pub explorer_url: String,
    /// `true` on the live gateway, `false` on the sandbox.
    pub livemode: bool,
}

impl SweepItem {
    /// `amount_units` as an integer, `None` when it is null or not plain decimal digits.
    pub fn amount_units(&self) -> Option<u128> {
        self.amount_units.as_deref().and_then(parse_units)
    }

    /// `fee_units` as an integer, `None` when it is null or not plain decimal digits.
    pub fn fee_units(&self) -> Option<u128> {
        self.fee_units.as_deref().and_then(parse_units)
    }
}

/// The answer of `GET /v1/sweeps`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SweepsPage {
    /// The items with `seq` above the cursor, ascending.
    pub sweeps: Vec<SweepItem>,
    /// The last `seq` of the page, or the cursor when the page is empty.
    pub next_seq: i64,
    /// Whether more follow.
    pub has_more: bool,
}

/// Whether a QRIS invoice's unique code is added to its base amount or taken off it. The invoice's
/// `amount_due` is `base_amount + unique_code` (`Add`) or `base_amount - unique_code` (`Subtract`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UniqueCodeMode {
    /// The code rides on top of the base amount: the customer pays up to Rp999 more. The gateway's
    /// default.
    #[default]
    Add,
    /// The code is taken off the base amount, so the customer never pays more than the base amount.
    /// Take the code from your own fee, and ask a `max_unique_code` no larger than that fee.
    Subtract,
}

impl UniqueCodeMode {
    /// The name on the wire.
    pub fn as_str(self) -> &'static str {
        match self {
            UniqueCodeMode::Add => "add",
            UniqueCodeMode::Subtract => "subtract",
        }
    }
}

/// The body of `POST /v1/qris/invoices`: ask for a QRIS invoice.
///
/// The same `reference` again returns the invoice made the first time, whatever the rest of the
/// body says, so a retry after a timeout never makes a second invoice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateQrisInvoice {
    /// The merchant's own id for this payment, 1 to 128 characters, unique per merchant.
    pub reference: String,
    /// The merchant's id for the customer.
    pub customer_ref: String,
    /// The base amount in whole rupiah, 1000 to 9999000. The customer pays this plus a unique
    /// code of Rp0 to Rp999 (or minus it, see `unique_code_mode`), the invoice's `amount_due`.
    pub amount_idr: i64,
    /// The largest unique code wanted, 0 to 999. The gateway picks the smallest free code up to
    /// it, and above it up to 999 only when every code up to it is taken. `None` leaves it out of
    /// the body, which the gateway reads as 999.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_unique_code: Option<u16>,
    /// Add the code to `amount_idr` or take it off. With `Subtract` the code is the smallest free
    /// one up to `max_unique_code` taken off the amount, and when every one is held the gateway
    /// falls back to adding a code of 1 to 999 (the invoice then says `Add`). `None` leaves it out
    /// of the body, which the gateway reads as `Add`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unique_code_mode: Option<UniqueCodeMode>,
    /// The kind of QRIS account to collect with. `None` leaves it out of the body, and the gateway
    /// picks the merchant's usable account of the lowest priority.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<QrisProvider>,
}

impl CreateQrisInvoice {
    /// The request for `amount_idr` with no `max_unique_code`, no `unique_code_mode` and no
    /// `provider`, so the gateway's defaults apply.
    pub fn new(reference: &str, customer_ref: &str, amount_idr: i64) -> CreateQrisInvoice {
        CreateQrisInvoice {
            reference: reference.to_string(),
            customer_ref: customer_ref.to_string(),
            amount_idr,
            max_unique_code: None,
            unique_code_mode: None,
            provider: None,
        }
    }
}

/// A QRIS invoice, as `POST /v1/qris/invoices`, `GET /v1/qris/invoices/{id_or_reference}` and the
/// QRIS feed show it.
///
/// Amounts are whole rupiah, as JSON integers (no smallest unit strings as for tokens).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QrisInvoice {
    /// `qri_...`
    pub id: String,
    /// The merchant's own id for the payment, echoed.
    pub reference: String,
    /// The merchant's id for the customer, echoed.
    pub customer_ref: String,
    /// The kind of QRIS account the invoice collects with.
    pub provider: QrisProvider,
    /// Where the invoice is in its life.
    pub status: QrisInvoiceStatus,
    /// The base amount the merchant asked for, in rupiah.
    pub base_amount: i64,
    /// The unique code added to the base amount or taken off it (`unique_code_mode`), Rp0 to Rp999.
    pub unique_code: u16,
    /// The mode the invoice used: a `Subtract` request that fell back to adding says `Add`. An
    /// answer without it (a gateway before 0.3.0 of this SDK) reads as `Add`.
    #[serde(default)]
    pub unique_code_mode: UniqueCodeMode,
    /// What the customer pays, `base_amount + unique_code` (`Add`) or `base_amount - unique_code`
    /// (`Subtract`), in rupiah. Show it with the code as its own line.
    pub amount_due: i64,
    /// The dynamic QRIS payload to render as a QR code. It carries `amount_due`.
    pub qris: String,
    /// When the invoice was made, in unix seconds.
    pub created_at: i64,
    /// When the time to pay ends and the invoice turns `expired`, in unix seconds.
    pub expires_at: i64,
    /// Until when a late payment of `amount_due` is still matched to this invoice, in unix seconds.
    pub match_until: i64,
    /// When it was paid, null until then.
    pub paid_at: Option<i64>,
    /// What was paid, in rupiah, null until then.
    pub paid_amount: Option<i64>,
    /// The payer's QRIS issuer when the feed names it, such as `DANA`.
    pub payer_issuer: Option<String>,
    /// How the payment was matched, null until paid.
    pub matched_by: Option<MatchedBy>,
    /// The feed position, reassigned on every state change.
    pub seq: i64,
    /// `true` on the live gateway, `false` on the sandbox.
    pub livemode: bool,
}

impl QrisInvoice {
    /// `true` when the invoice is `paid` and `paid_amount` is exactly `amount_due`. Only such an
    /// invoice is safe to credit without a person. A paid invoice with another amount was
    /// assigned by an operator: hold it for a person.
    pub fn paid_exactly(&self) -> bool {
        self.status == QrisInvoiceStatus::Paid && self.paid_amount == Some(self.amount_due)
    }
}

/// The answer of `GET /v1/qris/invoices`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QrisInvoicesPage {
    /// The invoices with `seq` above the cursor, ascending.
    pub invoices: Vec<QrisInvoice>,
    /// The last `seq` of the page, or the cursor when the page is empty.
    pub next_seq: i64,
    /// Whether more follow.
    pub has_more: bool,
}

/// The body of a non-2xx answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorBody {
    /// The error.
    pub error: ErrorDetail,
}

/// The `error` object of an [`ErrorBody`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorDetail {
    /// A snake case code such as `not_found` or `pool_exhausted`. Match on this.
    pub code: String,
    /// A text for a log. Never show it to a customer.
    #[serde(default)]
    pub message: String,
}

/// The ids a deposit webhook carries. It carries nothing else on purpose, so act on a re-fetch of
/// the deposit and never on the webhook.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DepositRef {
    /// `dep_...`
    pub deposit_id: String,
    /// The deposit key to re-fetch with `GET /v1/deposits/{key}`.
    pub key: String,
    /// The feed position when the event was made.
    pub seq: i64,
}

/// The ids a QRIS invoice webhook carries. Like [`DepositRef`] it carries nothing else on purpose,
/// so act on a re-fetch of the invoice and never on the webhook.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QrisInvoiceRef {
    /// `qri_...`, the id to re-fetch with `GET /v1/qris/invoices/{id}`.
    pub invoice_id: String,
    /// The merchant's own id for the payment.
    pub reference: String,
    /// The feed position when the event was made.
    pub seq: i64,
}

/// A webhook event, after [`crate::webhook::verify`] has checked its signature.
///
/// The envelope (`id`, `type`, `created_at`) is read first, and `data` only for a type this SDK
/// knows: the ids of a deposit for a `deposit.*` type, the ids of a QRIS invoice for a
/// `qris_invoice.*` type. So an event of a type this SDK does not know is still an event, with
/// [`EventType::Unknown`] and neither `deposit` nor `qris_invoice`, whatever its `data` holds.
/// Answer it 2xx and ignore it. A known type whose `data` is not the ids of that type is not an
/// event at all, and fails to decode.
///
/// An `Unknown` event keeps nothing of its type and its `data`, so it serializes as the envelope
/// with the type `unknown` and no `data`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(try_from = "RawWebhookEvent")]
pub struct WebhookEvent {
    /// `evt_...`
    pub id: String,
    /// The event type.
    pub event_type: EventType,
    /// When the event was made, in unix seconds.
    pub created_at: i64,
    /// The ids of the deposit, `Some` for every deposit type and `None` for every other type. On
    /// the wire this is `data`.
    pub deposit: Option<DepositRef>,
    /// The ids of the QRIS invoice, `Some` for every `qris_invoice.*` type and `None` for every
    /// other type. On the wire this is `data`.
    pub qris_invoice: Option<QrisInvoiceRef>,
}

impl Serialize for WebhookEvent {
    /// The envelope, and `data` from whichever of `deposit` and `qris_invoice` is set. When both
    /// are set, which decoding never makes, `deposit` wins.
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let has_data = self.deposit.is_some() || self.qris_invoice.is_some();
        let mut out = serializer.serialize_struct("WebhookEvent", if has_data { 4 } else { 3 })?;
        out.serialize_field("id", &self.id)?;
        out.serialize_field("type", &self.event_type)?;
        out.serialize_field("created_at", &self.created_at)?;
        if let Some(deposit) = &self.deposit {
            out.serialize_field("data", deposit)?;
        } else if let Some(invoice) = &self.qris_invoice {
            out.serialize_field("data", invoice)?;
        }
        out.end()
    }
}

/// The envelope of a webhook with its `data` still unread, so the type decides how `data` is read.
#[derive(Deserialize)]
struct RawWebhookEvent {
    id: String,
    #[serde(rename = "type")]
    event_type: EventType,
    created_at: i64,
    #[serde(default)]
    data: serde_json::Value,
}

impl TryFrom<RawWebhookEvent> for WebhookEvent {
    type Error = &'static str;

    fn try_from(raw: RawWebhookEvent) -> Result<WebhookEvent, Self::Error> {
        let (mut deposit, mut qris_invoice) = (None, None);
        if raw.event_type.is_deposit() {
            deposit = Some(
                serde_json::from_value(raw.data)
                    .map_err(|_| "a deposit event needs data with deposit_id, key and seq")?,
            );
        } else if raw.event_type.is_qris_invoice() {
            qris_invoice = Some(serde_json::from_value(raw.data).map_err(|_| {
                "a QRIS invoice event needs data with invoice_id, reference and seq"
            })?);
        }
        Ok(WebhookEvent {
            id: raw.id,
            event_type: raw.event_type,
            created_at: raw.created_at,
            deposit,
            qris_invoice,
        })
    }
}

/// Size and use of one family's pool within a key epoch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PoolStats {
    /// How many entries were imported.
    pub size: i64,
    /// How many were handed out.
    pub allocated: i64,
}

/// One key epoch of a pool merchant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Key {
    /// The epoch number, from 1.
    pub key_epoch: u32,
    /// The attestation public key, 64 hex characters.
    pub attest_pubkey: String,
    /// Whether the epoch still hands out addresses.
    pub status: KeyStatus,
    /// When the epoch was registered, in unix seconds.
    pub created_at: i64,
    /// Pool use per family name. A family with no imported entries is absent.
    #[serde(default)]
    pub pool: BTreeMap<String, PoolStats>,
}

/// The answer of `GET /v1/keys`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Keys {
    /// The merchant's key epochs, oldest first.
    pub keys: Vec<Key>,
}

/// One signed entry of a pool file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PoolEntry {
    /// The index within the epoch.
    pub pool_index: u32,
    /// The address in canonical form.
    pub address: String,
    /// The hex ed25519 attestation.
    pub attestation: String,
}

/// The file `kenari-pay-keys pool` writes and `POST /v1/pool` reads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PoolFile {
    /// The file format version, 2.
    pub version: u32,
    /// The family of every entry.
    pub family: Family,
    /// The key epoch that signed the entries.
    pub key_epoch: u32,
    /// The epoch's attestation public key, 64 hex characters.
    pub attest_pubkey: String,
    /// The entries, in index order.
    pub entries: Vec<PoolEntry>,
}

/// The answer of `POST /v1/pool`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PoolImported {
    /// How many entries were imported.
    pub imported: u64,
    /// The family of the entries.
    pub family: Family,
    /// The key epoch of the entries.
    pub key_epoch: u32,
    /// The index the next import must start at.
    pub next_index: u64,
}

/// A decimal amount: ASCII digits only, at least one, and it fits in a `u128`.
fn parse_units(text: &str) -> Option<u128> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn units_are_plain_decimal_digits() {
        assert_eq!(parse_units("0"), Some(0));
        assert_eq!(parse_units("25000000"), Some(25_000_000));
        assert_eq!(parse_units("007"), Some(7));
        assert_eq!(
            parse_units("340282366920938463463374607431768211455"),
            Some(u128::MAX)
        );
        for bad in [
            "",
            " 5",
            "5 ",
            "+5",
            "-5",
            "1.5",
            "1e6",
            "0x10",
            "١٢",
            "340282366920938463463374607431768211456",
        ] {
            assert_eq!(parse_units(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn an_unknown_word_decodes_to_unknown() {
        let family: Family = serde_json::from_str("\"tron\"").unwrap();
        assert_eq!(family, Family::Unknown);
        let kind: AddressKind = serde_json::from_str("\"safe\"").unwrap();
        assert_eq!(kind, AddressKind::Unknown);
        let status: DepositStatus = serde_json::from_str("\"settled\"").unwrap();
        assert_eq!(status, DepositStatus::Unknown);
        let event: EventType = serde_json::from_str("\"sweep.confirmed\"").unwrap();
        assert_eq!(event, EventType::Unknown);
        let key: KeyStatus = serde_json::from_str("\"paused\"").unwrap();
        assert_eq!(key, KeyStatus::Unknown);
        let invoice: QrisInvoiceStatus = serde_json::from_str("\"refunded\"").unwrap();
        assert_eq!(invoice, QrisInvoiceStatus::Unknown);
        let matched: MatchedBy = serde_json::from_str("\"guess\"").unwrap();
        assert_eq!(matched, MatchedBy::Unknown);
        let provider: QrisProvider = serde_json::from_str("\"ovo\"").unwrap();
        assert_eq!(provider, QrisProvider::Unknown);
        let event: EventType = serde_json::from_str("\"qris_invoice.refunded\"").unwrap();
        assert_eq!(event, EventType::Unknown);
        assert!(serde_json::from_str::<QrisInvoiceStatus>("1").is_err());
        // A value that is not text is still an error, not an `Unknown`.
        assert!(serde_json::from_str::<DepositStatus>("7").is_err());
        assert!(serde_json::from_str::<DepositStatus>("null").is_err());
    }

    #[test]
    fn the_known_words_round_trip() {
        for (word, status) in [
            ("detected", DepositStatus::Detected),
            ("confirmed", DepositStatus::Confirmed),
            ("orphaned", DepositStatus::Orphaned),
            ("reverted", DepositStatus::Reverted),
        ] {
            let json = format!("\"{word}\"");
            assert_eq!(
                serde_json::from_str::<DepositStatus>(&json).unwrap(),
                status
            );
            assert_eq!(serde_json::to_string(&status).unwrap(), json);
            assert_eq!(status.as_str(), word);
        }
        for (word, event) in [
            ("deposit.detected", EventType::DepositDetected),
            ("deposit.confirmed", EventType::DepositConfirmed),
            ("deposit.orphaned", EventType::DepositOrphaned),
            ("deposit.reverted", EventType::DepositReverted),
            ("qris_invoice.paid", EventType::QrisInvoicePaid),
            ("qris_invoice.expired", EventType::QrisInvoiceExpired),
        ] {
            let json = format!("\"{word}\"");
            assert_eq!(serde_json::from_str::<EventType>(&json).unwrap(), event);
            assert_eq!(serde_json::to_string(&event).unwrap(), json);
            assert_eq!(event.as_str(), word);
            assert_eq!(event.is_deposit(), word.starts_with("deposit."), "{word}");
            assert_eq!(
                event.is_qris_invoice(),
                word.starts_with("qris_invoice."),
                "{word}"
            );
        }
        assert!(!EventType::Unknown.is_deposit() && !EventType::Unknown.is_qris_invoice());
        for (word, status) in [
            ("open", QrisInvoiceStatus::Open),
            ("paid", QrisInvoiceStatus::Paid),
            ("expired", QrisInvoiceStatus::Expired),
        ] {
            let json = format!("\"{word}\"");
            assert_eq!(
                serde_json::from_str::<QrisInvoiceStatus>(&json).unwrap(),
                status
            );
            assert_eq!(serde_json::to_string(&status).unwrap(), json);
            assert_eq!(status.as_str(), word);
        }
        for (word, matched) in [
            ("feed", MatchedBy::Feed),
            ("operator", MatchedBy::Operator),
            ("simulated", MatchedBy::Simulated),
        ] {
            let json = format!("\"{word}\"");
            assert_eq!(serde_json::from_str::<MatchedBy>(&json).unwrap(), matched);
            assert_eq!(serde_json::to_string(&matched).unwrap(), json);
            assert_eq!(matched.as_str(), word);
        }
        for (word, provider) in [
            ("gopay", QrisProvider::Gopay),
            ("shopee", QrisProvider::Shopee),
            ("simulated", QrisProvider::Simulated),
        ] {
            let json = format!("\"{word}\"");
            assert_eq!(
                serde_json::from_str::<QrisProvider>(&json).unwrap(),
                provider
            );
            assert_eq!(serde_json::to_string(&provider).unwrap(), json);
            assert_eq!(provider.as_str(), word);
        }
        assert_eq!(Family::Evm.as_str(), "evm");
        assert_eq!(Family::Solana.as_str(), "solana");
        assert_eq!(AddressKind::Pool.as_str(), "pool");
        assert_eq!(AddressKind::Forwarder.as_str(), "forwarder");
    }

    #[test]
    fn a_missing_optional_field_is_none_and_an_unknown_field_is_ignored() {
        let json = r#"{"id":"adr_1","customer_ref":"u","family":"evm","address":"0x00",
            "networks":[],"livemode":false,"created_at":1,"added_by_a_newer_gateway":{"a":1}}"#;
        let address: Address = serde_json::from_str(json).unwrap();
        assert_eq!(address.kind, None);
        assert_eq!(address.forwarder, None);
        assert_eq!(address.key_epoch, None);
        assert_eq!(address.pool_index, None);
        assert_eq!(address.attestation, None);
    }

    #[test]
    fn livemode_and_simulated_are_required() {
        let nets = r#"{"networks":[]}"#;
        assert!(serde_json::from_str::<Networks>(nets).is_err());
        let address = r#"{"id":"adr_1","customer_ref":"u","family":"evm","address":"0x00",
            "networks":[],"created_at":1}"#;
        assert!(serde_json::from_str::<Address>(address).is_err());
    }
}
