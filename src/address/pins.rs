//! The forwarder factories kenari-pay has deployed, as constants a merchant can pin.
//!
//! A pin is the factory and the implementation of one chain. Pass it to the trust of a merchant
//! with the id of the chain it belongs to. The mainnet pin is added once the mainnet factory
//! exists, and until then a merchant has none for `base` and refuses forwarder addresses there.

use super::ForwarderPin;

/// Base Sepolia, the sandbox chain (chain id 84532).
///
/// The factory is `0x7bd6793b424c8e01dd7dce50364db20a9587db54` and the implementation is
/// `0x7539fadbe11298643551a501358b138e01efe3f0`, both from `contracts/README.md` of the gateway.
pub const BASE_SEPOLIA: ForwarderPin = ForwarderPin {
    factory: [
        0x7b, 0xd6, 0x79, 0x3b, 0x42, 0x4c, 0x8e, 0x01, 0xdd, 0x7d, 0xce, 0x50, 0x36, 0x4d, 0xb2,
        0x0a, 0x95, 0x87, 0xdb, 0x54,
    ],
    implementation: [
        0x75, 0x39, 0xfa, 0xdb, 0xe1, 0x12, 0x98, 0x64, 0x35, 0x51, 0xa5, 0x01, 0x35, 0x8b, 0x13,
        0x8e, 0x01, 0xef, 0xe3, 0xf0,
    ],
};
