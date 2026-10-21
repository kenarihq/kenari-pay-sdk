//! The check a merchant runs before it shows an address to a customer or stores it.

pub mod forwarder;
pub mod pins;

/// The factory and the implementation a merchant trusts for one chain, as 20 bytes each.
///
/// A merchant pins these itself and never takes them from the gateway, so a gateway that is
/// wrong or hostile cannot make an address look right by sending its own factory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ForwarderPin {
    /// The `ForwarderFactory` that deploys the forwarders of the chain.
    pub factory: forwarder::EvmAddress,
    /// The one `Forwarder` implementation that factory clones.
    pub implementation: forwarder::EvmAddress,
}
