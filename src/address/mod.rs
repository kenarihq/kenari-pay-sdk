//! The check a merchant runs before it shows an address to a customer or stores it.
//!
//! [`verify`] takes the [`Address`] the gateway answered, what the merchant [`Expect`]s of it and
//! what the merchant [`Trust`]s. Everything in [`Trust`] is the merchant's own, never anything the
//! gateway sent, and an address the gateway made up cannot pass by agreeing with itself.
//!
//! A **forwarder** address passes only when
//!
//! 1. its `forwarder` block is present,
//! 2. every EVM chain in [`Trust::chains`] has a pin equal to the block's factory and
//!    implementation (the gateway's own `networks` list is its claim and decides nothing, and the
//!    chains of another family are not looked at),
//! 3. the payout is one of the merchant's own wallets, compared in canonical form,
//! 4. `fee_bps` is at most [`Trust::max_fee_bps`], and
//! 5. the CREATE2 address recomputed from the factory, the salt, the implementation, the payout
//!    and the fee equals the address.
//!
//! The salt is not recomputed, because the address commits to the factory, the implementation, the
//! payout and the fee whatever the salt is.
//!
//! A **pool** address passes only when its attestation verifies under the key the merchant holds
//! for its key epoch. Any other kind is refused.
//!
//! ```
//! use kenari_pay_sdk::address::{self, pins, Expect, Refusal, Trust};
//! use kenari_pay_sdk::wire::{Address, Family};
//!
//! // The forwarder example of the gateway, as it answers `POST /v1/addresses` on the sandbox.
//! let text = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/vectors/address_evm_forwarder.json"));
//! let answer: Address = serde_json::from_str(text).unwrap();
//!
//! let trust = Trust::new()
//!     .chains(Family::Evm, ["base-sepolia"])
//!     .pin("base-sepolia", pins::BASE_SEPOLIA)
//!     .payouts(Family::Evm, ["0xeb1aaeefe4370f06e25bc4bf41e6dc3807fa6dc1"])
//!     .unwrap();
//! let expect = Expect::new("usr_acc_7f3a", Family::Evm, false);
//!
//! let verified = address::verify(&answer, &expect, &trust).unwrap();
//! assert_eq!(verified.address, "0x2793975f02ea0402393e0c3ea30c51264276927e");
//! assert_eq!(verified.fee_bps(), Some(50));
//! assert!(verified.sealed_to_current());
//!
//! // The same answer for a customer the merchant did not ask about is refused.
//! let other = Expect::new("usr_somebody_else", Family::Evm, false);
//! assert_eq!(address::verify(&answer, &other, &trust), Err(Refusal::CustomerRef));
//! ```

use std::collections::BTreeMap;

use ed25519_dalek::VerifyingKey;

use crate::wire::{Address, AddressKind, Family};

pub mod attest;
pub mod forwarder;
pub mod pins;

pub use attest::{attestation_message, verify_attestation};

/// Why a [`Trust`] entry cannot be built. It is the merchant's own configuration that is wrong, so
/// fix it before the merchant starts and do not run without it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustError {
    /// Not an address of the family.
    Address,
    /// A mixed case EVM address whose case is not its EIP-55 checksum.
    Checksum,
    /// Not a 32 byte ed25519 public key in hex.
    Key,
    /// A small order public key, which verifies nothing.
    WeakKey,
}

impl std::fmt::Display for TrustError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            TrustError::Address => "not a valid address for its family",
            TrustError::Checksum => "the mixed case spelling does not match the EIP-55 checksum",
            TrustError::Key => "not an ed25519 public key, it is 64 hex characters",
            TrustError::WeakKey => "a small order public key is refused",
        })
    }
}

impl std::error::Error for TrustError {}

/// The factory and the implementation a merchant trusts for one chain, as 20 bytes each.
///
/// A merchant pins these itself and never takes them from the gateway, so a gateway that is wrong
/// or hostile cannot make an address look right by sending its own factory. [`pins`] has the
/// ones kenari-pay has deployed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ForwarderPin {
    /// The `ForwarderFactory` that deploys the forwarders of the chain.
    pub factory: forwarder::EvmAddress,
    /// The one `Forwarder` implementation that factory clones.
    pub implementation: forwarder::EvmAddress,
}

impl ForwarderPin {
    /// A pin from the 20 bytes of each.
    pub const fn new(
        factory: forwarder::EvmAddress,
        implementation: forwarder::EvmAddress,
    ) -> ForwarderPin {
        ForwarderPin {
            factory,
            implementation,
        }
    }

    /// A pin from the text of each address, for a merchant that keeps its pins in configuration. A
    /// mixed case spelling must carry its EIP-55 checksum.
    pub fn parse(factory: &str, implementation: &str) -> Result<ForwarderPin, TrustError> {
        Ok(ForwarderPin {
            factory: parse_evm(factory)?,
            implementation: parse_evm(implementation)?,
        })
    }
}

/// A wallet address in canonical form: `0x` and lower case hex on EVM, the base58 text on Solana.
///
/// It can only be made by [`Wallet::parse`], so a list of wallets holds no spelling that would
/// fail to compare equal to the same wallet in another case.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Wallet(String);

impl Wallet {
    /// Parse and canonicalize a wallet of `family`. An EVM spelling that mixes case must carry its
    /// EIP-55 checksum, so a typo is caught here and not at the first deposit.
    pub fn parse(family: Family, text: &str) -> Result<Wallet, TrustError> {
        match family {
            Family::Evm => Ok(Wallet(forwarder::format_address(&parse_evm(text)?))),
            Family::Solana => canonical_address(Family::Solana, text)
                .map(Wallet)
                .ok_or(TrustError::Address),
            Family::Unknown => Err(TrustError::Address),
        }
    }

    /// The canonical text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Wallet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

fn parse_evm(text: &str) -> Result<forwarder::EvmAddress, TrustError> {
    forwarder::parse_address(text).map_err(|e| match e {
        forwarder::AddressTextError::Malformed => TrustError::Address,
        forwarder::AddressTextError::BadChecksum => TrustError::Checksum,
    })
}

/// Canonical address form for comparisons, attestations and keys.
///
/// EVM: `0x` and 40 lower case hex characters, with surrounding white space and an upper case
/// `0X` accepted. Solana: the base58 text unchanged, 32 to 44 characters. `None` when the input is
/// not a well formed address of that family, and always `None` for [`Family::Unknown`].
pub fn canonical_address(family: Family, raw: &str) -> Option<String> {
    let s = raw.trim();
    match family {
        Family::Evm => {
            let hex_part = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X"))?;
            if hex_part.len() != 40 || !hex_part.chars().all(|c| c.is_ascii_hexdigit()) {
                return None;
            }
            Some(format!("0x{}", hex_part.to_ascii_lowercase()))
        }
        Family::Solana => {
            const B58: &str = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
            if !(32..=44).contains(&s.len()) || !s.chars().all(|c| B58.contains(c)) {
                return None;
            }
            Some(s.to_string())
        }
        Family::Unknown => None,
    }
}

/// What a merchant trusts. Every field is the merchant's own, never anything the gateway sent.
///
/// A new [`Trust`] trusts nothing, so [`verify`] refuses every forwarder address until the EVM
/// chains, their pins and the EVM payout wallets are set, and every pool address until the
/// attestation keys are. Build it once at start up, and treat an `Err` from a builder method as a
/// reason not to start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trust {
    /// The chain ids the merchant accepts deposits on, per family, as `payouts` is keyed. Only the
    /// [`Family::Evm`] list is read, by the forwarder check, which needs at least one chain in it.
    /// A chain of another family listed here changes nothing.
    pub chains: BTreeMap<Family, Vec<String>>,
    /// Per chain id, the factory and the implementation the merchant trusts. Every chain in the
    /// EVM list of `chains` needs one.
    pub forwarder_pins: BTreeMap<String, ForwarderPin>,
    /// The merchant's own payout wallets per family, the wallets it still controls. The first of a
    /// family is the one new addresses must be sealed to. Removing a lost wallet from this list is
    /// how a merchant stops accepting addresses sealed to it.
    pub payouts: BTreeMap<Family, Vec<Wallet>>,
    /// The most the merchant agreed to pay, in basis points. 50 unless set.
    pub max_fee_bps: u16,
    /// The attestation public key of each key epoch, for pool addresses.
    pub attest_pubkeys: BTreeMap<u32, VerifyingKey>,
}

impl Default for Trust {
    fn default() -> Trust {
        Trust::new()
    }
}

impl Trust {
    /// A trust with nothing in it and `max_fee_bps` at [`forwarder::DEFAULT_FEE_BPS`].
    pub fn new() -> Trust {
        Trust {
            chains: BTreeMap::new(),
            forwarder_pins: BTreeMap::new(),
            payouts: BTreeMap::new(),
            max_fee_bps: forwarder::DEFAULT_FEE_BPS,
            attest_pubkeys: BTreeMap::new(),
        }
    }

    /// Add the chain ids of `family` the merchant accepts deposits on. A second call for the same
    /// family adds to its list, so a chain is never dropped by a later call. A forwarder address
    /// is checked against the [`Family::Evm`] list only.
    pub fn chains<I, S>(mut self, family: Family, chains: I) -> Trust
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.chains
            .entry(family)
            .or_default()
            .extend(chains.into_iter().map(Into::into));
        self
    }

    /// Pin the factory and the implementation of a chain.
    pub fn pin(mut self, chain: impl Into<String>, pin: ForwarderPin) -> Trust {
        self.forwarder_pins.insert(chain.into(), pin);
        self
    }

    /// Set the merchant's payout wallets of a family, the first being the current one. It replaces
    /// the family's list.
    pub fn payouts<I, S>(mut self, family: Family, wallets: I) -> Result<Trust, TrustError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let wallets = wallets
            .into_iter()
            .map(|w| Wallet::parse(family, w.as_ref()))
            .collect::<Result<Vec<_>, _>>()?;
        self.payouts.insert(family, wallets);
        Ok(self)
    }

    /// Set the most the merchant agreed to pay, in basis points.
    pub fn max_fee_bps(mut self, max_fee_bps: u16) -> Trust {
        self.max_fee_bps = max_fee_bps;
        self
    }

    /// Set the attestation public key of a key epoch, as 64 hex characters.
    pub fn attest_key(mut self, key_epoch: u32, pubkey_hex: &str) -> Result<Trust, TrustError> {
        self.attest_pubkeys
            .insert(key_epoch, attest::parse_key(pubkey_hex)?);
        Ok(self)
    }
}

/// What the merchant asked for, each of which the answer must equal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Expect<'a> {
    /// The `customer_ref` the merchant sent.
    pub customer_ref: &'a str,
    /// The family the merchant asked for.
    pub family: Family,
    /// The network the merchant runs on, `true` live and `false` sandbox.
    pub livemode: bool,
}

impl<'a> Expect<'a> {
    /// What a merchant asked for with `POST /v1/addresses`.
    pub fn new(customer_ref: &'a str, family: Family, livemode: bool) -> Expect<'a> {
        Expect {
            customer_ref,
            family,
            livemode,
        }
    }
}

/// The model of a verified address and what was checked about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Model {
    /// A pool address whose attestation verified.
    Pool {
        /// The key epoch whose key verified it.
        key_epoch: u32,
        /// The index within the epoch.
        pool_index: u32,
    },
    /// A forwarder address that recomputed.
    Forwarder {
        /// The payout wallet it is sealed to, canonical, one of the merchant's own.
        payout: String,
        /// The fee sealed into it, at most the merchant's maximum.
        fee_bps: u16,
    },
}

/// An address that passed [`verify`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verified {
    /// The address in canonical form. Store and show this spelling, the one that was checked.
    pub address: String,
    /// The model and what the check found.
    pub model: Model,
    /// The chains a forwarder was checked against, which are the EVM list of [`Trust::chains`].
    /// Empty for a pool address, whose check has nothing to do with a chain.
    pub chains: Vec<String>,
    sealed_to_current: bool,
}

impl Verified {
    /// Whether the address is a forwarder sealed to the first of the merchant's payout wallets, as
    /// against one made for an older wallet. A merchant uses it to tell a current address from
    /// one that still pays a wallet it moved on from. Always `false` for a pool address.
    pub fn sealed_to_current(&self) -> bool {
        self.sealed_to_current
    }

    /// The model name.
    pub fn kind(&self) -> AddressKind {
        match self.model {
            Model::Pool { .. } => AddressKind::Pool,
            Model::Forwarder { .. } => AddressKind::Forwarder,
        }
    }

    /// The payout wallet of a forwarder, `None` for a pool address.
    pub fn payout(&self) -> Option<&str> {
        match &self.model {
            Model::Forwarder { payout, .. } => Some(payout),
            Model::Pool { .. } => None,
        }
    }

    /// The sealed fee of a forwarder, `None` for a pool address.
    pub fn fee_bps(&self) -> Option<u16> {
        match self.model {
            Model::Forwarder { fee_bps, .. } => Some(fee_bps),
            Model::Pool { .. } => None,
        }
    }
}

/// The rule an address failed. Each variant has a fixed text, so nothing the gateway sent reaches
/// a log line or a customer through it. Never show an address that was refused, alert a person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The answer is for the other network than the one the merchant runs on.
    Livemode,
    /// The answer is for another customer.
    CustomerRef,
    /// The answer is of another family than the one asked for.
    Family,
    /// The kind is neither pool nor forwarder, or it is a forwarder outside EVM.
    Kind,
    /// The address or one of the fields of its forwarder block is not in the documented form.
    Malformed,
    /// A forwarder address with no forwarder block.
    NoForwarderBlock,
    /// [`Trust::chains`] has no EVM chain, so there is nothing to check the forwarder against.
    NoChains,
    /// A chain of the EVM list of [`Trust::chains`] has no pin.
    NoPin {
        /// The chain id, from the merchant's own list.
        chain: String,
    },
    /// The factory or the implementation of the block is not the one pinned for a chain.
    PinMismatch {
        /// The chain id, from the merchant's own list.
        chain: String,
    },
    /// The payout is not one of the merchant's own wallets.
    Payout,
    /// The sealed fee is above the merchant's maximum, or above what the factory allows.
    Fee,
    /// The address recomputed from the block is not the address.
    AddressMismatch,
    /// A pool address with no attestation, key epoch or pool index.
    NoAttestation,
    /// The merchant holds no attestation key for the key epoch of a pool address.
    UnknownEpoch,
    /// The attestation of a pool address does not verify.
    Attestation,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Refusal::Livemode => "the address is for the other network",
            Refusal::CustomerRef => "the address belongs to another customer",
            Refusal::Family => "the address is of another family",
            Refusal::Kind => "the address kind is not accepted",
            Refusal::Malformed => "the address or its forwarder block is malformed",
            Refusal::NoForwarderBlock => "a forwarder address has no forwarder block",
            Refusal::NoChains => "no chain is configured to check a forwarder against",
            Refusal::NoPin { .. } => "no forwarder pin is configured for a chain",
            Refusal::PinMismatch { .. } => "the factory or implementation is not the pinned one",
            Refusal::Payout => "the payout is not one of the merchant's own wallets",
            Refusal::Fee => "the sealed fee is above the agreed maximum",
            Refusal::AddressMismatch => "the address does not match its forwarder block",
            Refusal::NoAttestation => "a pool address has no attestation",
            Refusal::UnknownEpoch => "no attestation key is held for the key epoch",
            Refusal::Attestation => "the attestation does not verify",
        })
    }
}

impl std::error::Error for Refusal {}

/// Check an address the gateway answered before it is shown to a customer or stored.
///
/// See the [module](self) for what a forwarder and a pool address must satisfy. An answer that
/// says no `kind` is a pool address, as the gateway's own type reads it.
pub fn verify(address: &Address, expect: &Expect<'_>, trust: &Trust) -> Result<Verified, Refusal> {
    if address.livemode != expect.livemode {
        return Err(Refusal::Livemode);
    }
    if address.customer_ref != expect.customer_ref {
        return Err(Refusal::CustomerRef);
    }
    if expect.family == Family::Unknown || address.family != expect.family {
        return Err(Refusal::Family);
    }
    match address.kind.unwrap_or(AddressKind::Pool) {
        AddressKind::Pool => verify_pool(address, trust),
        AddressKind::Forwarder => verify_forwarder(address, trust),
        AddressKind::Unknown => Err(Refusal::Kind),
    }
}

fn verify_pool(address: &Address, trust: &Trust) -> Result<Verified, Refusal> {
    let (Some(key_epoch), Some(pool_index), Some(attestation)) = (
        address.key_epoch,
        address.pool_index,
        address.attestation.as_deref(),
    ) else {
        return Err(Refusal::NoAttestation);
    };
    let canonical =
        canonical_address(address.family, &address.address).ok_or(Refusal::Malformed)?;
    let key = trust
        .attest_pubkeys
        .get(&key_epoch)
        .ok_or(Refusal::UnknownEpoch)?;
    if !attest::verify_with(
        key,
        key_epoch,
        address.family,
        pool_index,
        &canonical,
        attestation,
    ) {
        return Err(Refusal::Attestation);
    }
    Ok(Verified {
        address: canonical,
        model: Model::Pool {
            key_epoch,
            pool_index,
        },
        chains: Vec::new(),
        sealed_to_current: false,
    })
}

fn verify_forwarder(address: &Address, trust: &Trust) -> Result<Verified, Refusal> {
    // Forwarders exist on EVM only. Anything else under this kind is not an address to show.
    if address.family != Family::Evm {
        return Err(Refusal::Kind);
    }
    let block = address
        .forwarder
        .as_ref()
        .ok_or(Refusal::NoForwarderBlock)?;
    // Only the EVM chains count. A forwarder is an EVM address, and a chain of another family
    // that the merchant lists for its own reasons has no factory to pin.
    let chains = trust
        .chains
        .get(&Family::Evm)
        .map(Vec::as_slice)
        .unwrap_or_default();
    if chains.is_empty() {
        return Err(Refusal::NoChains);
    }
    let factory = forwarder::parse_address(&block.factory).map_err(|_| Refusal::Malformed)?;
    let implementation =
        forwarder::parse_address(&block.implementation).map_err(|_| Refusal::Malformed)?;
    // The merchant's chains and pins decide, never the networks the gateway listed.
    for chain in chains {
        let pin = trust
            .forwarder_pins
            .get(chain)
            .ok_or_else(|| Refusal::NoPin {
                chain: chain.clone(),
            })?;
        if pin.factory != factory || pin.implementation != implementation {
            return Err(Refusal::PinMismatch {
                chain: chain.clone(),
            });
        }
    }
    let payout = forwarder::parse_address(&block.payout).map_err(|_| Refusal::Malformed)?;
    let payout_text = forwarder::format_address(&payout);
    let own = trust
        .payouts
        .get(&Family::Evm)
        .map(Vec::as_slice)
        .unwrap_or_default();
    if !own.iter().any(|wallet| wallet.as_str() == payout_text) {
        return Err(Refusal::Payout);
    }
    if block.fee_bps > trust.max_fee_bps {
        return Err(Refusal::Fee);
    }
    let salt = forwarder::parse_salt(&block.salt).ok_or(Refusal::Malformed)?;
    let computed = forwarder::address(&factory, &implementation, &payout, block.fee_bps, &salt)
        .map_err(|e| match e {
            forwarder::ForwarderError::FeeTooHigh(_) => Refusal::Fee,
            forwarder::ForwarderError::ZeroPayout => Refusal::Payout,
        })?;
    let claimed = forwarder::parse_address(&address.address).map_err(|_| Refusal::Malformed)?;
    if computed != claimed {
        return Err(Refusal::AddressMismatch);
    }
    let sealed_to_current = own
        .first()
        .is_some_and(|wallet| wallet.as_str() == payout_text);
    Ok(Verified {
        address: forwarder::format_address(&computed),
        model: Model::Forwarder {
            payout: payout_text,
            fee_bps: block.fee_bps,
        },
        chains: chains.to_vec(),
        sealed_to_current,
    })
}
