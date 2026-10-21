//! The address check against the gateway's own examples: the happy paths and one test per refusal.
//! Every refusal test starts from the real example and changes one thing, so the refusal it names
//! is the only thing wrong.

use kenari_pay_sdk::address::{
    self, forwarder, pins, Expect, ForwarderPin, Model, Refusal, Trust, TrustError, Wallet,
};
use kenari_pay_sdk::wire::{Address, AddressKind, Family};

const FORWARDER_EXAMPLE: &str = include_str!("../vectors/address_evm_forwarder.json");
const POOL_EXAMPLE: &str = include_str!("../vectors/address_evm.json");
const ATTESTATION_VECTOR: &str = include_str!("../vectors/attestation_vector.json");
const KEYS_LIST: &str = include_str!("../vectors/keys_list.json");

/// The payout wallet of the forwarder example, the merchant's own in these tests.
const PAYOUT: &str = "0xeb1aaeefe4370f06e25bc4bf41e6dc3807fa6dc1";
/// Another wallet the merchant controls.
const OLD_PAYOUT: &str = "0x00000000000000000000000000000000000000a1";
/// A wallet that is not the merchant's.
const STRANGER: &str = "0x00000000000000000000000000000000000000b2";

fn forwarder_answer() -> Address {
    serde_json::from_str(FORWARDER_EXAMPLE).unwrap()
}

fn pool_answer() -> Address {
    serde_json::from_str(POOL_EXAMPLE).unwrap()
}

fn expect(answer: &Address) -> Expect<'_> {
    Expect::new(&answer.customer_ref, answer.family, answer.livemode)
}

fn trust() -> Trust {
    Trust::new()
        .chains(Family::Evm, ["base-sepolia"])
        .pin("base-sepolia", pins::BASE_SEPOLIA)
        .payouts(Family::Evm, [PAYOUT])
        .unwrap()
}

fn attest_pubkey() -> String {
    let v: serde_json::Value = serde_json::from_str(ATTESTATION_VECTOR).unwrap();
    v["attest_pubkey_hex"].as_str().unwrap().to_string()
}

fn pool_trust() -> Trust {
    Trust::new().attest_key(1, &attest_pubkey()).unwrap()
}

/// The same text with its last hex digit changed.
fn flip_last(text: &str) -> String {
    let mut out = text.to_string();
    let last = out.pop().unwrap();
    out.push(if last == '0' { '1' } else { '0' });
    out
}

fn block(answer: &mut Address) -> &mut kenari_pay_sdk::wire::ForwarderBlock {
    answer.forwarder.as_mut().unwrap()
}

// ---- forwarder: the happy paths ---------------------------------------------------------------

#[test]
fn the_real_forwarder_example_passes_with_the_base_sepolia_pin() {
    let answer = forwarder_answer();
    let verified = address::verify(&answer, &expect(&answer), &trust()).unwrap();
    assert_eq!(verified.address, answer.address);
    assert_eq!(verified.kind(), AddressKind::Forwarder);
    assert_eq!(verified.payout(), Some(PAYOUT));
    assert_eq!(verified.fee_bps(), Some(50));
    assert_eq!(verified.chains, vec!["base-sepolia".to_string()]);
    assert!(verified.sealed_to_current());
    assert_eq!(
        verified.model,
        Model::Forwarder {
            payout: PAYOUT.to_string(),
            fee_bps: 50
        }
    );
}

#[test]
fn the_pin_constant_is_the_deployed_base_sepolia_factory() {
    let parsed = ForwarderPin::parse(
        "0x7bd6793b424c8e01dd7dce50364db20a9587db54",
        "0x7539fadbe11298643551a501358b138e01efe3f0",
    )
    .unwrap();
    assert_eq!(pins::BASE_SEPOLIA, parsed);
    // The EIP-55 spellings of `contracts/README.md` are the same pin.
    let checksummed = ForwarderPin::parse(
        "0x7Bd6793b424C8E01Dd7dCE50364Db20A9587DB54",
        "0x7539FadbE11298643551A501358b138e01EFe3F0",
    )
    .unwrap();
    assert_eq!(pins::BASE_SEPOLIA, checksummed);
    // The example's own block names the same pair.
    let answer = forwarder_answer();
    let block = answer.forwarder.unwrap();
    assert_eq!(
        forwarder::parse_address(&block.factory).unwrap(),
        pins::BASE_SEPOLIA.factory
    );
    assert_eq!(
        forwarder::parse_address(&block.implementation).unwrap(),
        pins::BASE_SEPOLIA.implementation
    );
}

#[test]
fn a_fee_below_the_maximum_and_an_old_wallet_pass_and_are_told_apart() {
    // The merchant moved on to another wallet and kept the old one in the list.
    let answer = forwarder_answer();
    let moved = Trust::new()
        .chains(Family::Evm, ["base-sepolia"])
        .pin("base-sepolia", pins::BASE_SEPOLIA)
        .payouts(Family::Evm, [OLD_PAYOUT, PAYOUT])
        .unwrap();
    let verified = address::verify(&answer, &expect(&answer), &moved).unwrap();
    assert_eq!(verified.payout(), Some(PAYOUT));
    assert!(
        !verified.sealed_to_current(),
        "sealed to the second wallet of the list"
    );
    // A looser maximum accepts the same address.
    let loose = trust().max_fee_bps(300);
    assert!(address::verify(&answer, &expect(&answer), &loose).is_ok());
}

#[test]
fn the_payouts_are_compared_in_canonical_form() {
    let answer = forwarder_answer();
    // The EIP-55 spelling of the payout wallet, and an upper case one, are the same wallet.
    for spelling in [
        "0xEb1AAEefe4370f06e25bC4Bf41e6Dc3807Fa6Dc1",
        "0xEB1AAEEFE4370F06E25BC4BF41E6DC3807FA6DC1",
        "  0xeb1aaeefe4370f06e25bc4bf41e6dc3807fa6dc1\n",
    ] {
        let t = Trust::new()
            .chains(Family::Evm, ["base-sepolia"])
            .pin("base-sepolia", pins::BASE_SEPOLIA)
            .payouts(Family::Evm, [spelling])
            .unwrap();
        let verified = address::verify(&answer, &expect(&answer), &t).unwrap();
        assert_eq!(verified.payout(), Some(PAYOUT), "{spelling}");
    }
    // The gateway's block in another case is the same wallet too.
    let mut shouted = forwarder_answer();
    block(&mut shouted).payout = block(&mut shouted)
        .payout
        .to_ascii_uppercase()
        .replace("0X", "0x");
    assert!(address::verify(&shouted, &expect(&shouted), &trust()).is_ok());
}

#[test]
fn the_answer_may_spell_its_address_in_another_case_and_verified_holds_the_canonical_one() {
    let mut answer = forwarder_answer();
    answer.address = answer.address.to_ascii_uppercase().replace("0X", "0x");
    let verified = address::verify(&answer, &expect(&answer), &trust()).unwrap();
    assert_eq!(verified.address, forwarder_answer().address);
}

#[test]
fn every_chain_of_the_merchant_is_checked_and_the_networks_of_the_answer_decide_nothing() {
    let answer = forwarder_answer();
    // Two chains, both pinned to the same pair.
    let two = Trust::new()
        .chains(Family::Evm, ["base-sepolia", "base-sepolia-b"])
        .pin("base-sepolia", pins::BASE_SEPOLIA)
        .pin("base-sepolia-b", pins::BASE_SEPOLIA)
        .payouts(Family::Evm, [PAYOUT])
        .unwrap();
    let verified = address::verify(&answer, &expect(&answer), &two).unwrap();
    assert_eq!(verified.chains, vec!["base-sepolia", "base-sepolia-b"]);
    // The answer's own network list is ignored: emptied or made to claim another chain, it changes
    // nothing.
    let mut other = forwarder_answer();
    other.networks.clear();
    assert!(address::verify(&other, &expect(&other), &trust()).is_ok());
    other.networks = serde_json::from_str::<Address>(POOL_EXAMPLE)
        .unwrap()
        .networks;
    assert!(address::verify(&other, &expect(&other), &trust()).is_ok());
}

#[test]
fn a_chain_of_another_family_next_to_the_evm_ones_changes_nothing() {
    let answer = forwarder_answer();
    // A merchant on both families lists its Solana chain beside the EVM one. Solana has no
    // forwarder and so no pin, and the forwarder check must not ask for one.
    let both = Trust::new()
        .chains(Family::Evm, ["base-sepolia"])
        .chains(Family::Solana, ["solana-devnet"])
        .pin("base-sepolia", pins::BASE_SEPOLIA)
        .payouts(Family::Evm, [PAYOUT])
        .unwrap();
    assert_eq!(
        both.chains.get(&Family::Solana),
        Some(&vec!["solana-devnet".to_string()])
    );
    let verified = address::verify(&answer, &expect(&answer), &both).unwrap();
    assert_eq!(verified.chains, vec!["base-sepolia".to_string()]);
    // The order of the calls does not matter.
    let reversed = Trust::new()
        .chains(Family::Solana, ["solana-devnet", "solana-mainnet"])
        .chains(Family::Evm, ["base-sepolia"])
        .pin("base-sepolia", pins::BASE_SEPOLIA)
        .payouts(Family::Evm, [PAYOUT])
        .unwrap();
    assert!(address::verify(&answer, &expect(&answer), &reversed).is_ok());
    // A pin that is wrong for the Solana chain name is never read either.
    let pinned = both
        .clone()
        .pin("solana-devnet", ForwarderPin::new([9; 20], [8; 20]));
    assert!(address::verify(&answer, &expect(&answer), &pinned).is_ok());

    // Only a Solana chain listed is no EVM chain at all.
    let solana_only = Trust::new()
        .chains(Family::Solana, ["solana-devnet"])
        .pin("base-sepolia", pins::BASE_SEPOLIA)
        .payouts(Family::Evm, [PAYOUT])
        .unwrap();
    assert_eq!(
        address::verify(&answer, &expect(&answer), &solana_only),
        Err(Refusal::NoChains)
    );
    // A Solana chain listed under the EVM family is an EVM chain to the check, and needs its pin.
    let misfiled = Trust::new()
        .chains(Family::Evm, ["base-sepolia", "solana-devnet"])
        .pin("base-sepolia", pins::BASE_SEPOLIA)
        .payouts(Family::Evm, [PAYOUT])
        .unwrap();
    assert_eq!(
        address::verify(&answer, &expect(&answer), &misfiled),
        Err(Refusal::NoPin {
            chain: "solana-devnet".into()
        })
    );
    // A pool address does not read chains at all.
    let pool = pool_answer();
    assert!(address::verify(
        &pool,
        &expect(&pool),
        &pool_trust().chains(Family::Solana, ["x"])
    )
    .is_ok());
}

#[test]
fn chains_of_a_family_add_up_across_calls() {
    let t = Trust::new()
        .chains(Family::Evm, ["base-sepolia"])
        .chains(Family::Evm, ["base"]);
    assert_eq!(
        t.chains.get(&Family::Evm),
        Some(&vec!["base-sepolia".to_string(), "base".to_string()])
    );
    // So the second call cannot loosen the check by dropping the first: `base` has no pin.
    let t = t
        .pin("base-sepolia", pins::BASE_SEPOLIA)
        .payouts(Family::Evm, [PAYOUT])
        .unwrap();
    let answer = forwarder_answer();
    assert_eq!(
        address::verify(&answer, &expect(&answer), &t),
        Err(Refusal::NoPin {
            chain: "base".into()
        })
    );
}

// ---- forwarder: one test per refusal ---------------------------------------------------------

#[test]
fn refuses_the_other_network() {
    let answer = forwarder_answer();
    assert!(!answer.livemode);
    // The merchant runs live and the gateway answered a sandbox address.
    let live = Expect::new(&answer.customer_ref, Family::Evm, true);
    assert_eq!(
        address::verify(&answer, &live, &trust()),
        Err(Refusal::Livemode)
    );
    // And the other way: the merchant runs on the sandbox and the answer says live.
    let mut live_answer = forwarder_answer();
    live_answer.livemode = true;
    assert_eq!(
        address::verify(&live_answer, &expect(&answer), &trust()),
        Err(Refusal::Livemode)
    );
}

#[test]
fn refuses_a_wrong_customer_ref() {
    let answer = forwarder_answer();
    let other = Expect::new("usr_somebody_else", Family::Evm, false);
    assert_eq!(
        address::verify(&answer, &other, &trust()),
        Err(Refusal::CustomerRef)
    );
    // The comparison is exact, so another case or a trailing space is another customer.
    for shifted in ["USR_ACC_7F3A", "usr_acc_7f3a ", " usr_acc_7f3a", ""] {
        let e = Expect::new(shifted, Family::Evm, false);
        assert_eq!(
            address::verify(&answer, &e, &trust()),
            Err(Refusal::CustomerRef),
            "{shifted:?}"
        );
    }
}

#[test]
fn refuses_a_wrong_family() {
    let answer = forwarder_answer();
    let asked_for_solana = Expect::new(&answer.customer_ref, Family::Solana, false);
    assert_eq!(
        address::verify(&answer, &asked_for_solana, &trust()),
        Err(Refusal::Family)
    );
    // An answer of another family than the one asked for.
    let mut solana = forwarder_answer();
    solana.family = Family::Solana;
    assert_eq!(
        address::verify(&solana, &expect(&forwarder_answer()), &trust()),
        Err(Refusal::Family)
    );
    // Unknown equals unknown is still no family.
    let mut unknown = forwarder_answer();
    unknown.family = Family::Unknown;
    let e = Expect::new(&unknown.customer_ref, Family::Unknown, false);
    assert_eq!(
        address::verify(&unknown, &e, &trust()),
        Err(Refusal::Family)
    );
}

#[test]
fn refuses_a_kind_it_does_not_know_and_a_forwarder_outside_evm() {
    let mut answer = forwarder_answer();
    answer.kind = Some(AddressKind::Unknown);
    assert_eq!(
        address::verify(&answer, &expect(&answer), &trust()),
        Err(Refusal::Kind)
    );
    // The same from the wire, where a kind from a newer gateway is a word this SDK never saw.
    let json = FORWARDER_EXAMPLE.replace("\"kind\": \"forwarder\"", "\"kind\": \"safe_module\"");
    assert_ne!(json, FORWARDER_EXAMPLE);
    let from_wire: Address = serde_json::from_str(&json).unwrap();
    assert_eq!(from_wire.kind, Some(AddressKind::Unknown));
    assert_eq!(
        address::verify(&from_wire, &expect(&from_wire), &trust()),
        Err(Refusal::Kind)
    );
    // A forwarder of the Solana family does not exist.
    let mut solana = forwarder_answer();
    solana.family = Family::Solana;
    let e = Expect::new(&solana.customer_ref, Family::Solana, false);
    assert_eq!(address::verify(&solana, &e, &trust()), Err(Refusal::Kind));
}

#[test]
fn refuses_a_forwarder_without_its_block() {
    let mut answer = forwarder_answer();
    answer.forwarder = None;
    assert_eq!(
        address::verify(&answer, &expect(&answer), &trust()),
        Err(Refusal::NoForwarderBlock)
    );
}

#[test]
fn refuses_an_empty_chains_list() {
    let answer = forwarder_answer();
    let none = Trust::new()
        .pin("base-sepolia", pins::BASE_SEPOLIA)
        .payouts(Family::Evm, [PAYOUT])
        .unwrap();
    assert!(none.chains.is_empty());
    assert_eq!(none.chains.get(&Family::Evm), None);
    assert_eq!(
        address::verify(&answer, &expect(&answer), &none),
        Err(Refusal::NoChains)
    );
}

#[test]
fn refuses_a_chain_without_a_pin() {
    let answer = forwarder_answer();
    // The merchant offers `base` too and has no pin for it, as kenari on live before the mainnet
    // factory exists.
    let two = Trust::new()
        .chains(Family::Evm, ["base-sepolia", "base"])
        .pin("base-sepolia", pins::BASE_SEPOLIA)
        .payouts(Family::Evm, [PAYOUT])
        .unwrap();
    assert_eq!(
        address::verify(&answer, &expect(&answer), &two),
        Err(Refusal::NoPin {
            chain: "base".into()
        })
    );
    // A pin for a chain the merchant does not list is no pin for the one it does.
    let wrong = Trust::new()
        .chains(Family::Evm, ["base"])
        .pin("base-sepolia", pins::BASE_SEPOLIA)
        .payouts(Family::Evm, [PAYOUT])
        .unwrap();
    assert_eq!(
        address::verify(&answer, &expect(&answer), &wrong),
        Err(Refusal::NoPin {
            chain: "base".into()
        })
    );
}

#[test]
fn refuses_a_pin_that_differs() {
    let answer = forwarder_answer();
    let mut other_factory = pins::BASE_SEPOLIA;
    other_factory.factory[19] ^= 1;
    let mut other_implementation = pins::BASE_SEPOLIA;
    other_implementation.implementation[0] ^= 1;
    let swapped = ForwarderPin::new(
        pins::BASE_SEPOLIA.implementation,
        pins::BASE_SEPOLIA.factory,
    );
    for pin in [other_factory, other_implementation, swapped] {
        let t = Trust::new()
            .chains(Family::Evm, ["base-sepolia"])
            .pin("base-sepolia", pin)
            .payouts(Family::Evm, [PAYOUT])
            .unwrap();
        assert_eq!(
            address::verify(&answer, &expect(&answer), &t),
            Err(Refusal::PinMismatch {
                chain: "base-sepolia".into()
            })
        );
    }
    // One chain right and one wrong is a refusal naming the wrong one.
    let mixed = Trust::new()
        .chains(Family::Evm, ["base-sepolia", "other"])
        .pin("base-sepolia", pins::BASE_SEPOLIA)
        .pin("other", other_factory)
        .payouts(Family::Evm, [PAYOUT])
        .unwrap();
    assert_eq!(
        address::verify(&answer, &expect(&answer), &mixed),
        Err(Refusal::PinMismatch {
            chain: "other".into()
        })
    );
}

#[test]
fn refuses_a_gateway_that_pins_itself() {
    // A block that names its own factory and implementation, and an address recomputed from them,
    // agrees with itself and is still refused because the merchant's pin is another pair.
    let mut answer = forwarder_answer();
    let own_factory = [0x11u8; 20];
    let own_implementation = [0x22u8; 20];
    let payout = forwarder::parse_address(PAYOUT).unwrap();
    let salt = forwarder::parse_salt(&block(&mut answer).salt).unwrap();
    let computed =
        forwarder::address(&own_factory, &own_implementation, &payout, 50, &salt).unwrap();
    {
        let b = block(&mut answer);
        b.factory = forwarder::format_address(&own_factory);
        b.implementation = forwarder::format_address(&own_implementation);
    }
    answer.address = forwarder::format_address(&computed);
    assert_eq!(
        address::verify(&answer, &expect(&answer), &trust()),
        Err(Refusal::PinMismatch {
            chain: "base-sepolia".into()
        })
    );
}

#[test]
fn refuses_a_payout_that_is_not_ours() {
    let mut answer = forwarder_answer();
    // The gateway sealed the address to a wallet the merchant does not hold, and the address is
    // recomputed to match, which is the whole attack.
    let factory = pins::BASE_SEPOLIA.factory;
    let implementation = pins::BASE_SEPOLIA.implementation;
    let stranger = forwarder::parse_address(STRANGER).unwrap();
    let salt = forwarder::parse_salt(&block(&mut answer).salt).unwrap();
    let computed = forwarder::address(&factory, &implementation, &stranger, 50, &salt).unwrap();
    block(&mut answer).payout = STRANGER.to_string();
    answer.address = forwarder::format_address(&computed);
    assert_eq!(
        address::verify(&answer, &expect(&answer), &trust()),
        Err(Refusal::Payout)
    );
    // No wallets at all, wallets of another family only, and an empty list all refuse.
    let real = forwarder_answer();
    let no_payouts = Trust::new()
        .chains(Family::Evm, ["base-sepolia"])
        .pin("base-sepolia", pins::BASE_SEPOLIA);
    assert_eq!(
        address::verify(&real, &expect(&real), &no_payouts),
        Err(Refusal::Payout)
    );
    let solana_only = no_payouts
        .clone()
        .payouts(
            Family::Solana,
            ["HAgk14JpMQLgt6rVgv7cBQFJWFto5Dqxi472uT3DKpqk"],
        )
        .unwrap();
    assert_eq!(
        address::verify(&real, &expect(&real), &solana_only),
        Err(Refusal::Payout)
    );
    let empty = no_payouts
        .payouts(Family::Evm, std::iter::empty::<&str>())
        .unwrap();
    assert_eq!(
        address::verify(&real, &expect(&real), &empty),
        Err(Refusal::Payout)
    );
    // A wallet the merchant removed from its list (it was lost) is no longer accepted.
    let moved_on = Trust::new()
        .chains(Family::Evm, ["base-sepolia"])
        .pin("base-sepolia", pins::BASE_SEPOLIA)
        .payouts(Family::Evm, [OLD_PAYOUT])
        .unwrap();
    assert_eq!(
        address::verify(&real, &expect(&real), &moved_on),
        Err(Refusal::Payout)
    );
}

#[test]
fn refuses_a_fee_above_the_maximum() {
    let mut answer = forwarder_answer();
    block(&mut answer).fee_bps = 51;
    assert_eq!(
        address::verify(&answer, &expect(&answer), &trust()),
        Err(Refusal::Fee)
    );
    // The default is 50 and a merchant that agreed to less is held to less.
    let real = forwarder_answer();
    assert_eq!(
        address::verify(&real, &expect(&real), &trust().max_fee_bps(49)),
        Err(Refusal::Fee)
    );
    // Above what the factory allows is a refusal even when the merchant's maximum is looser.
    let mut huge = forwarder_answer();
    block(&mut huge).fee_bps = 301;
    assert_eq!(
        address::verify(&huge, &expect(&huge), &trust().max_fee_bps(u16::MAX)),
        Err(Refusal::Fee)
    );
}

#[test]
fn refuses_a_changed_salt_or_address() {
    let real = forwarder_answer();
    let mut salt = forwarder_answer();
    block(&mut salt).salt = flip_last(&block(&mut salt).salt);
    assert_eq!(
        address::verify(&salt, &expect(&salt), &trust()),
        Err(Refusal::AddressMismatch)
    );
    let mut changed = forwarder_answer();
    changed.address = flip_last(&changed.address);
    assert_eq!(
        address::verify(&changed, &expect(&changed), &trust()),
        Err(Refusal::AddressMismatch)
    );
    // The address of another customer (the pool example's) under this block.
    let mut swapped = forwarder_answer();
    swapped.address = pool_answer().address;
    assert_eq!(
        address::verify(&swapped, &expect(&swapped), &trust()),
        Err(Refusal::AddressMismatch)
    );
    // A fee that is allowed but is not the one the address was made with.
    let mut cheaper = forwarder_answer();
    block(&mut cheaper).fee_bps = 49;
    assert_eq!(
        address::verify(&cheaper, &expect(&cheaper), &trust()),
        Err(Refusal::AddressMismatch)
    );
    // Another wallet of the merchant's own list under the same address.
    let two = Trust::new()
        .chains(Family::Evm, ["base-sepolia"])
        .pin("base-sepolia", pins::BASE_SEPOLIA)
        .payouts(Family::Evm, [PAYOUT, OLD_PAYOUT])
        .unwrap();
    let mut other_wallet = forwarder_answer();
    block(&mut other_wallet).payout = OLD_PAYOUT.to_string();
    assert_eq!(
        address::verify(&other_wallet, &expect(&other_wallet), &two),
        Err(Refusal::AddressMismatch)
    );
    // The real one still passes, so none of the above is a broken helper.
    assert!(address::verify(&real, &expect(&real), &two).is_ok());
}

#[test]
fn refuses_malformed_fields() {
    for (what, change) in [
        ("address", 0),
        ("factory", 1),
        ("implementation", 2),
        ("payout", 3),
        ("salt", 4),
    ] {
        let mut answer = forwarder_answer();
        match change {
            0 => answer.address = "0x1234".into(),
            1 => block(&mut answer).factory = "factory".into(),
            2 => block(&mut answer).implementation = String::new(),
            3 => block(&mut answer).payout = "0xeb1aaeefe4370f06e25bc4bf41e6dc3807fa6dcg".into(),
            _ => block(&mut answer).salt = "0x1234".into(),
        }
        assert_eq!(
            address::verify(&answer, &expect(&answer), &trust()),
            Err(Refusal::Malformed),
            "{what}"
        );
    }
    // A mixed case spelling with a wrong checksum is malformed, not another way to say the address.
    let mut answer = forwarder_answer();
    answer.address = "0x2793975F02Ea0402393E0C3eA30c51264276927e".into();
    assert_eq!(
        address::verify(&answer, &expect(&answer), &trust()),
        Err(Refusal::Malformed)
    );
}

#[test]
fn a_zero_payout_is_never_accepted() {
    let mut answer = forwarder_answer();
    block(&mut answer).payout = "0x0000000000000000000000000000000000000000".into();
    let zero = Trust::new()
        .chains(Family::Evm, ["base-sepolia"])
        .pin("base-sepolia", pins::BASE_SEPOLIA)
        .payouts(Family::Evm, ["0x0000000000000000000000000000000000000000"])
        .unwrap();
    assert_eq!(
        address::verify(&answer, &expect(&answer), &zero),
        Err(Refusal::Payout)
    );
}

#[test]
fn a_new_trust_trusts_nothing() {
    let nothing = Trust::default();
    assert_eq!(nothing, Trust::new());
    assert_eq!(nothing.max_fee_bps, 50);
    let fwd = forwarder_answer();
    assert_eq!(
        address::verify(&fwd, &expect(&fwd), &nothing),
        Err(Refusal::NoChains)
    );
    let pool = pool_answer();
    assert_eq!(
        address::verify(&pool, &expect(&pool), &nothing),
        Err(Refusal::UnknownEpoch)
    );
}

// ---- pool: the happy paths and the refusals --------------------------------------------------

#[test]
fn the_real_pool_example_passes_with_the_attestation_key_of_the_vector() {
    let answer = pool_answer();
    let verified = address::verify(&answer, &expect(&answer), &pool_trust()).unwrap();
    assert_eq!(verified.address, answer.address);
    assert_eq!(verified.kind(), AddressKind::Pool);
    assert_eq!(
        verified.model,
        Model::Pool {
            key_epoch: 1,
            pool_index: 0
        }
    );
    assert_eq!(verified.payout(), None);
    assert_eq!(verified.fee_bps(), None);
    assert!(verified.chains.is_empty());
    assert!(!verified.sealed_to_current());
    // The pool check needs none of the forwarder settings, and it ignores them when they are there.
    let both = trust().attest_key(1, &attest_pubkey()).unwrap();
    assert!(address::verify(&answer, &expect(&answer), &both).is_ok());
}

#[test]
fn an_answer_with_no_kind_is_a_pool_address() {
    let mut answer = pool_answer();
    answer.kind = None;
    assert!(address::verify(&answer, &expect(&answer), &pool_trust()).is_ok());
    // And an old answer that says no kind and carries no attestation is refused, not waved through.
    answer.attestation = None;
    assert_eq!(
        address::verify(&answer, &expect(&answer), &pool_trust()),
        Err(Refusal::NoAttestation)
    );
}

#[test]
fn the_solana_entry_of_the_attestation_vector_passes() {
    let v: serde_json::Value = serde_json::from_str(ATTESTATION_VECTOR).unwrap();
    let entry = &v["entries"][1];
    assert_eq!(entry["family"], "solana");
    let mut answer = pool_answer();
    answer.family = Family::Solana;
    answer.address = entry["address"].as_str().unwrap().to_string();
    answer.attestation = Some(entry["signature_hex"].as_str().unwrap().to_string());
    answer.networks.clear();
    let verified = address::verify(&answer, &expect(&answer), &pool_trust()).unwrap();
    assert_eq!(verified.address, answer.address);
    // The Solana signature does not carry over to the EVM family.
    let mut evm = pool_answer();
    evm.attestation = answer.attestation.clone();
    assert_eq!(
        address::verify(&evm, &expect(&evm), &pool_trust()),
        Err(Refusal::Attestation)
    );
}

#[test]
fn a_pool_address_in_another_case_verifies_and_comes_back_canonical() {
    let mut answer = pool_answer();
    answer.address = answer.address.to_ascii_uppercase().replace("0X", "0x");
    let verified = address::verify(&answer, &expect(&answer), &pool_trust()).unwrap();
    assert_eq!(verified.address, pool_answer().address);
}

#[test]
fn refuses_a_pool_address_without_its_attestation_fields() {
    for part in 0..3 {
        let mut answer = pool_answer();
        match part {
            0 => answer.attestation = None,
            1 => answer.key_epoch = None,
            _ => answer.pool_index = None,
        }
        assert_eq!(
            address::verify(&answer, &expect(&answer), &pool_trust()),
            Err(Refusal::NoAttestation),
            "part {part}"
        );
    }
}

#[test]
fn refuses_an_epoch_with_no_key() {
    let mut answer = pool_answer();
    answer.key_epoch = Some(2);
    assert_eq!(
        address::verify(&answer, &expect(&answer), &pool_trust()),
        Err(Refusal::UnknownEpoch)
    );
    // The key of epoch 1 must be the merchant's own, so with no key at all it is the same.
    let real = pool_answer();
    assert_eq!(
        address::verify(&real, &expect(&real), &Trust::new()),
        Err(Refusal::UnknownEpoch)
    );
}

#[test]
fn refuses_an_attestation_that_does_not_verify() {
    let real = pool_answer();
    let mut flipped = pool_answer();
    flipped.attestation = Some(flip_last(real.attestation.as_deref().unwrap()));
    let mut other_index = pool_answer();
    other_index.pool_index = Some(1);
    let mut other_address = pool_answer();
    other_address.address = "0x6fac4d18c912343bf86fa7049364dd4e424ab9c0".into();
    let mut short = pool_answer();
    short.attestation = Some(real.attestation.clone().unwrap()[..126].to_string());
    let mut not_hex = pool_answer();
    not_hex.attestation = Some("zz".into());
    let mut empty = pool_answer();
    empty.attestation = Some(String::new());
    for (what, answer) in [
        ("flipped signature", flipped),
        ("another index", other_index),
        ("another address", other_address),
        ("short signature", short),
        ("not hex", not_hex),
        ("empty", empty),
    ] {
        assert_eq!(
            address::verify(&answer, &expect(&answer), &pool_trust()),
            Err(Refusal::Attestation),
            "{what}"
        );
    }
    // The key the merchant holds for epoch 2 does not verify an epoch 1 signature, even when it is
    // filed under epoch 1.
    let keys: serde_json::Value = serde_json::from_str(KEYS_LIST).unwrap();
    let epoch2 = keys["keys"][1]["attest_pubkey"].as_str().unwrap();
    let wrong_key = Trust::new().attest_key(1, epoch2).unwrap();
    assert_eq!(
        address::verify(&real, &expect(&real), &wrong_key),
        Err(Refusal::Attestation)
    );
}

#[test]
fn refuses_a_pool_address_that_is_not_an_address() {
    for bad in ["", "0x1234", "9858effd232b4033e47d90003d41ec34ecaeda94"] {
        let mut answer = pool_answer();
        answer.address = bad.into();
        assert_eq!(
            address::verify(&answer, &expect(&answer), &pool_trust()),
            Err(Refusal::Malformed),
            "{bad:?}"
        );
    }
}

#[test]
fn the_pool_checks_of_livemode_customer_and_family_hold_too() {
    let answer = pool_answer();
    assert_eq!(
        address::verify(
            &answer,
            &Expect::new(&answer.customer_ref, Family::Evm, false),
            &pool_trust()
        ),
        Err(Refusal::Livemode)
    );
    assert_eq!(
        address::verify(
            &answer,
            &Expect::new("someone", Family::Evm, true),
            &pool_trust()
        ),
        Err(Refusal::CustomerRef)
    );
    assert_eq!(
        address::verify(
            &answer,
            &Expect::new(&answer.customer_ref, Family::Solana, true),
            &pool_trust()
        ),
        Err(Refusal::Family)
    );
}

// ---- the trust itself ------------------------------------------------------------------------

#[test]
fn a_trust_entry_that_cannot_be_parsed_is_an_error_at_start_up() {
    let evm = |text| Wallet::parse(Family::Evm, text);
    assert!(evm(PAYOUT).is_ok());
    assert_eq!(evm("").unwrap_err(), TrustError::Address);
    assert_eq!(evm("0x1234").unwrap_err(), TrustError::Address);
    assert_eq!(
        evm("0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAEd").unwrap_err(),
        TrustError::Checksum
    );
    assert_eq!(
        Wallet::parse(Family::Unknown, PAYOUT).unwrap_err(),
        TrustError::Address
    );
    assert_eq!(
        Wallet::parse(Family::Solana, PAYOUT).unwrap_err(),
        TrustError::Address
    );
    assert!(Wallet::parse(
        Family::Solana,
        "HAgk14JpMQLgt6rVgv7cBQFJWFto5Dqxi472uT3DKpqk"
    )
    .is_ok());
    // One bad wallet in a list fails the whole list.
    assert_eq!(
        Trust::new()
            .payouts(Family::Evm, [PAYOUT, "nope"])
            .unwrap_err(),
        TrustError::Address
    );
    assert_eq!(
        ForwarderPin::parse("0x1234", "0x7539fadbe11298643551a501358b138e01efe3f0").unwrap_err(),
        TrustError::Address
    );
    assert_eq!(
        ForwarderPin::parse(
            "0x7bd6793b424c8e01dd7dce50364db20a9587db54",
            "0x7539FadbE11298643551A501358b138e01EFe3f0"
        )
        .unwrap_err(),
        TrustError::Checksum
    );
}

#[test]
fn an_attestation_key_must_be_a_real_public_key() {
    assert_eq!(Trust::new().attest_key(1, "").unwrap_err(), TrustError::Key);
    assert_eq!(
        Trust::new().attest_key(1, "00").unwrap_err(),
        TrustError::Key
    );
    assert_eq!(
        Trust::new().attest_key(1, &"zz".repeat(32)).unwrap_err(),
        TrustError::Key
    );
    // The all zero key is a small order point, which verifies any signature of a certain shape.
    assert_eq!(
        Trust::new().attest_key(1, &"00".repeat(32)).unwrap_err(),
        TrustError::WeakKey
    );
    // White space around the hex of a key kept in configuration is ignored.
    let padded = format!(" {}\n", attest_pubkey());
    let answer = pool_answer();
    let t = Trust::new().attest_key(1, &padded).unwrap();
    assert!(address::verify(&answer, &expect(&answer), &t).is_ok());
}

#[test]
fn a_refusal_names_the_rule_with_a_fixed_text_and_never_the_gateways_words() {
    // Every field the gateway controls carries a marker, and no refusal text may repeat it.
    let mut answer = forwarder_answer();
    answer.customer_ref = "LEAK_customer".into();
    {
        let b = block(&mut answer);
        b.factory = "LEAK_factory".into();
    }
    let e = Expect::new("another", Family::Evm, false);
    let refusals = vec![
        address::verify(&answer, &e, &trust()).unwrap_err(),
        address::verify(&answer, &expect(&answer), &trust()).unwrap_err(),
        Refusal::NoPin {
            chain: "LEAK_chain".into(),
        },
        Refusal::PinMismatch {
            chain: "LEAK_chain".into(),
        },
        Refusal::Livemode,
        Refusal::Family,
        Refusal::Kind,
        Refusal::NoForwarderBlock,
        Refusal::NoChains,
        Refusal::Payout,
        Refusal::Fee,
        Refusal::AddressMismatch,
        Refusal::NoAttestation,
        Refusal::UnknownEpoch,
        Refusal::Attestation,
        Refusal::Malformed,
        Refusal::CustomerRef,
    ];
    let mut texts = std::collections::BTreeSet::new();
    for refusal in &refusals {
        let text = refusal.to_string();
        assert!(!text.contains("LEAK"), "{refusal:?}: {text}");
        assert!(!text.contains(';'), "{refusal:?}: {text}");
        assert!(!text.is_empty());
        texts.insert(text);
    }
    // The 15 rules read as 15 different texts.
    assert_eq!(texts.len(), 15);
}

#[test]
fn the_trust_error_texts_are_fixed_and_carry_no_semicolon() {
    for error in [
        TrustError::Address,
        TrustError::Checksum,
        TrustError::Key,
        TrustError::WeakKey,
    ] {
        let text = error.to_string();
        assert!(!text.is_empty() && !text.contains(';'), "{text}");
    }
}
