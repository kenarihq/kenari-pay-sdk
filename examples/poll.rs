//! A complete poll loop over the deposit feed, with the `reqwest` client.
//!
//! ```text
//! KENARI_PAY_API_KEY=kp_test_... cargo run --example poll --features reqwest
//! ```
//!
//! Settings, all from the environment:
//!
//! - `KENARI_PAY_API_KEY`: the merchant API key. Required. It is never printed.
//! - `KENARI_PAY_URL`: the gateway, `https://pay-sandbox.kenari.id` unless set.
//! - `KENARI_PAY_CURSOR_FILE`: where the cursor is kept, `kenari-pay-cursor` unless set.
//! - `KENARI_PAY_ONCE`: set it to anything to catch up and exit instead of polling for ever.
//!
//! The rules it follows are the ones of `kenari_pay_sdk::feed`: the cursor advances only after a
//! deposit has been handled, and the loop stops at the first deposit that fails, so the next pass
//! reads it again. Only a deposit that is `confirmed` and not `below_minimum` is credited, and only
//! after a re-fetch of it, whose `livemode` must be the mode of the key and whose `simulated` must
//! be false on live. A `reverted` deposit is never debited, a person is told.
//!
//! This is an example. It keeps the cursor in a file and the credited keys in memory. A real
//! merchant stores both in its database, the key under a unique constraint and the cursor in the
//! same transaction as the credit, so a crash between the two cannot credit twice.

use std::collections::HashSet;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use kenari_pay_sdk::client::Client;
use kenari_pay_sdk::feed::Cursor;
use kenari_pay_sdk::wire::{Deposit, DepositStatus};

const PAGE: u32 = 100;
const IDLE: Duration = Duration::from_secs(10);

/// What handling a deposit ended in.
enum Handled {
    /// Done with it, move the cursor past it.
    Done,
    /// Could not handle it now, stop here and read it again next pass.
    Retry(String),
}

/// Why a deposit is not money of the network this merchant runs on, when it is not. A deposit from
/// the other network never is. A simulated one (made with `POST /v1/sandbox/deposits`, no
/// transaction on any chain) is never credited on live, and the live gateway never makes one.
fn not_ours(deposit: &Deposit, livemode: bool) -> Option<&'static str> {
    if deposit.livemode != livemode {
        Some("is from the other network")
    } else if deposit.simulated && livemode {
        Some("is simulated and this is live")
    } else {
        None
    }
}

fn read_cursor(path: &PathBuf) -> i64 {
    // A missing or unreadable file is the start of the feed, which idempotent crediting allows.
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| text.trim().parse().ok())
        .unwrap_or(0)
}

fn write_cursor(path: &PathBuf, seq: i64) -> std::io::Result<()> {
    let mut temp = path.clone().into_os_string();
    temp.push(".tmp");
    let temp = PathBuf::from(temp);
    std::fs::write(&temp, seq.to_string())?;
    std::fs::rename(&temp, path)
}

async fn handle(
    client: &Client,
    livemode: bool,
    credited: &mut HashSet<String>,
    deposit: &Deposit,
) -> Handled {
    // An object from the other network is never money of this one.
    if let Some(why) = not_ours(deposit, livemode) {
        return Handled::Retry(format!("{} {why}", deposit.key));
    }
    match deposit.status {
        DepositStatus::Detected => {
            println!("{} detected, waiting for it to confirm", deposit.key);
            Handled::Done
        }
        DepositStatus::Orphaned => {
            println!("{} orphaned, nothing to credit", deposit.key);
            Handled::Done
        }
        DepositStatus::Reverted => {
            // Never debit on this without a person.
            eprintln!(
                "ALERT {} was reverted, a person must look at it",
                deposit.key
            );
            Handled::Done
        }
        DepositStatus::Unknown => {
            eprintln!(
                "ALERT {} has a status this SDK does not know, put aside",
                deposit.key
            );
            Handled::Done
        }
        DepositStatus::Confirmed if deposit.below_minimum => {
            println!("{} is below the minimum, not credited", deposit.key);
            Handled::Done
        }
        DepositStatus::Confirmed => {
            // The feed says confirmed. Act on the gateway's own answer for the key, not on the page.
            let fresh = match client.deposit(&deposit.key).await {
                Ok(Some(fresh)) => fresh,
                Ok(None) => {
                    return Handled::Retry(format!("{} was not found on re-fetch", deposit.key))
                }
                Err(error) => {
                    return Handled::Retry(format!("re-fetch of {} failed: {error}", deposit.key))
                }
            };
            // The re-fetched object is the one to act on, so it gets the same check as the page.
            if let Some(why) = not_ours(&fresh, livemode) {
                return Handled::Retry(format!("{} {why} on re-fetch", deposit.key));
            }
            if fresh.status != DepositStatus::Confirmed || fresh.below_minimum {
                // It moved since the page was made. A later feed item says where it went.
                println!("{} is no longer creditable, skipping", deposit.key);
                return Handled::Done;
            }
            let Some(units) = fresh.amount_units() else {
                return Handled::Retry(format!(
                    "{} has an amount that is not a number",
                    deposit.key
                ));
            };
            if credited.insert(fresh.key.clone()) {
                println!(
                    "credit {units} units of {} to {} for {}",
                    fresh.symbol, fresh.customer_ref, fresh.key
                );
            } else {
                println!("{} was credited before, nothing to do", fresh.key);
            }
            Handled::Done
        }
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let Ok(api_key) = std::env::var("KENARI_PAY_API_KEY") else {
        eprintln!("set KENARI_PAY_API_KEY to the merchant API key");
        return ExitCode::FAILURE;
    };
    let url = std::env::var("KENARI_PAY_URL")
        .unwrap_or_else(|_| "https://pay-sandbox.kenari.id".to_string());
    let cursor_file = PathBuf::from(
        std::env::var("KENARI_PAY_CURSOR_FILE").unwrap_or_else(|_| "kenari-pay-cursor".to_string()),
    );
    let once = std::env::var_os("KENARI_PAY_ONCE").is_some();
    // A live key says kp_live_ and a sandbox key says kp_test_, and the gateway refuses the other.
    let livemode = api_key.trim().starts_with("kp_live_");

    let client = match Client::new(&url, &api_key) {
        Ok(client) => client,
        Err(error) => {
            eprintln!("cannot build the client: {error}");
            return ExitCode::FAILURE;
        }
    };
    let mut cursor = Cursor::deposits(read_cursor(&cursor_file));
    let mut credited = HashSet::new();

    loop {
        let page = match client.deposits(cursor.after_seq(), PAGE).await {
            Ok(page) => page,
            Err(error) => {
                eprintln!("feed failed, trying again: {error}");
                tokio::time::sleep(IDLE).await;
                continue;
            }
        };
        let mut stopped = false;
        for deposit in &page.deposits {
            match handle(&client, livemode, &mut credited, deposit).await {
                Handled::Done => {
                    cursor.advance_to(deposit.seq);
                    if let Err(error) = write_cursor(&cursor_file, cursor.after_seq()) {
                        eprintln!("cannot store the cursor, stopping: {error}");
                        return ExitCode::FAILURE;
                    }
                }
                Handled::Retry(why) => {
                    // Stop at the first failure so this deposit is read again.
                    eprintln!("stopping at seq {}: {why}", deposit.seq);
                    stopped = true;
                    break;
                }
            }
        }
        if stopped || !page.has_more {
            if once {
                return if stopped {
                    ExitCode::FAILURE
                } else {
                    ExitCode::SUCCESS
                };
            }
            tokio::time::sleep(IDLE).await;
        }
    }
}
