//! Audit F-04: what one transaction can carry when it splits a swap across
//! shards.
//!
//! An address lookup table (ALT) shortens each account reference from 32
//! bytes to 1, so it solves the 1232-byte packet limit. It does **not** raise
//! the limit on accounts a transaction may lock: every account loaded through
//! an ALT still counts. This file has a calculator for both numbers and then
//! builds the real transactions (legacy and v0 + ALT) with real swap
//! instructions, compiles them, measures their size and runs them on the
//! deployed program under mainnet-beta feature gates.
//!
//! `TX_BUDGET_OUT=<file>` writes the measured table as Markdown
//! (docs/tx-budget.md is produced this way).

mod common;

use common::*;
use solana_account::Account;
use solana_address_lookup_table_interface::state::{AddressLookupTable, LookupTableMeta};
use solana_instruction::Instruction;
use solana_message::{v0, AddressLookupTableAccount, Message, VersionedMessage};
use solana_signer::Signer;
use solana_transaction::versioned::VersionedTransaction;
use std::borrow::Cow;

/// Agave's `PACKET_DATA_SIZE`: the largest serialized transaction.
const PACKET_DATA_SIZE: usize = 1232;
/// Account locks per transaction while `increase_tx_account_lock_limit` is
/// inactive, as it is in LiteSVM's mainnet-beta snapshot.
const ACCOUNT_LOCK_LIMIT: usize = 64;
const COMPUTE_BUDGET: Pubkey = anchor_lang::pubkey!("ComputeBudget111111111111111111111111111111");
const ALT_PROGRAM: Pubkey = anchor_lang::pubkey!("AddressLookupTab1e1111111111111111111111111");
/// 8-byte discriminator + Borsh `SwapArgs` (bool, u64, u64, u128, u64, i64).
const SWAP_DATA: usize = 8 + 1 + 8 + 8 + 16 + 8 + 8;
/// Accounts of one `swap_exact_in` before the tick arrays.
const SWAP_FIXED_ACCOUNTS: usize = 10;

fn short_vec(n: usize) -> usize {
    match n {
        0..=0x7f => 1,
        0x80..=0x3fff => 2,
        _ => 3,
    }
}

/// Predicted size of a split swap over `n` shards with `k` tick arrays each.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Budget {
    /// Distinct accounts the transaction locks (static + loaded).
    accounts: usize,
    /// Serialized transaction bytes, one signature.
    bytes: usize,
}

/// The calculator. Shared accounts: payer, market, registry, user token in,
/// user token out, token program and the AMM program (7). Per shard: shard,
/// vault A, vault B, vault authority and `k` tick arrays. `SetComputeUnitLimit`
/// adds the ComputeBudget program. With `alt`, everything except the payer
/// and the invoked programs is loaded from one lookup table.
fn budget(n: usize, k: usize, alt: bool, compute_ix: bool) -> Budget {
    let cb = usize::from(compute_ix);
    let accounts = 7 + n * (4 + k) + cb;
    let swap_ix = 1
        + short_vec(SWAP_FIXED_ACCOUNTS + k)
        + SWAP_FIXED_ACCOUNTS
        + k
        + short_vec(SWAP_DATA)
        + SWAP_DATA;
    let cb_ix = 1 + 1 + 1 + 5;
    let ixs = short_vec(n + cb) + n * swap_ix + cb * cb_ix;
    let signatures = 1 + 64;
    let bytes = if alt {
        let statics = 2 + cb;
        let (writable, readonly) = (2 + n * (3 + k), 3 + n);
        signatures
            + 1
            + 3
            + short_vec(statics)
            + 32 * statics
            + 32
            + ixs
            + 1
            + 32
            + short_vec(writable)
            + writable
            + short_vec(readonly)
            + readonly
    } else {
        signatures + 3 + short_vec(accounts) + 32 * accounts + 32 + ixs
    };
    Budget { accounts, bytes }
}

fn compute_limit(units: u32) -> Instruction {
    let mut data = vec![2u8];
    data.extend_from_slice(&units.to_le_bytes());
    Instruction {
        program_id: COMPUTE_BUDGET,
        accounts: vec![],
        data,
    }
}

/// Compiles `ixs` as a legacy message, or as v0 with one fresh lookup table
/// that holds every account except the payer and the invoked programs.
fn compile(
    env: &mut Env,
    payer: &Pubkey,
    ixs: &[Instruction],
    alt: bool,
    alt_slot: &mut u64,
) -> VersionedMessage {
    let blockhash = env.svm.latest_blockhash();
    if !alt {
        return VersionedMessage::Legacy(Message::new_with_blockhash(ixs, Some(payer), &blockhash));
    }
    let mut keys: Vec<Pubkey> = Vec::new();
    for ix in ixs {
        for a in &ix.accounts {
            if a.pubkey != *payer && !keys.contains(&a.pubkey) {
                keys.push(a.pubkey);
            }
        }
    }
    let table = Pubkey::new_unique();
    let data = AddressLookupTable {
        meta: LookupTableMeta {
            deactivation_slot: u64::MAX,
            last_extended_slot: 0,
            last_extended_slot_start_index: 0,
            authority: Some(*payer),
            _padding: 0,
        },
        addresses: Cow::Owned(keys.clone()),
    }
    .serialize_for_tests()
    .unwrap();
    let lamports = env.svm.minimum_balance_for_rent_exemption(data.len());
    env.svm
        .set_account(
            table,
            Account {
                lamports,
                data,
                owner: ALT_PROGRAM,
                executable: false,
                rent_epoch: 0,
            },
        )
        .unwrap();
    // Tables are usable from the slot after their last extension.
    *alt_slot += 1;
    env.svm.warp_to_slot(*alt_slot);
    let lookup = AddressLookupTableAccount {
        key: table,
        addresses: keys,
    };
    VersionedMessage::V0(v0::Message::try_compile(payer, ixs, &[lookup], blockhash).unwrap())
}

/// Accounts the message locks (static + loaded) and the signed size.
fn measure(message: &VersionedMessage) -> Budget {
    let loaded: usize = match message {
        VersionedMessage::V0(v) => v
            .address_table_lookups
            .iter()
            .map(|l| l.writable_indexes.len() + l.readonly_indexes.len())
            .sum(),
        _ => 0,
    };
    Budget {
        accounts: message.static_account_keys().len() + loaded,
        bytes: 1 + 64 + message.serialize().len(),
    }
}

struct Row {
    n: usize,
    k: usize,
    alt: bool,
    predicted: Budget,
    measured: Budget,
    result: Result<u64, String>,
}

#[test]
fn split_swap_transaction_budget() {
    let mut env = Env::mainnet();
    let (m, _) = env.open_seeded_market();
    for id in 0..8 {
        env.ensure_arrays(&m, id, 0, 12_000);
    }
    let trader = env.user(&m, 1_000_000_000_000);
    let payer = trader.kp.pubkey();
    let mut rows = Vec::new();
    let mut alt_slot = 1u64;

    for k in 1..=3usize {
        for n in 1..=8usize {
            for alt in [false, true] {
                let mut ixs = vec![compute_limit(1_400_000)];
                for id in 0..n as u8 {
                    let arrays = env.swap_arrays(&m, id, false);
                    assert!(arrays.len() >= k, "shard {id} has {} arrays", arrays.len());
                    ixs.push(env.ix_swap(
                        &m,
                        &trader,
                        id,
                        env.swap_args(false, 1_000),
                        &arrays[..k],
                    ));
                }
                let message = compile(&mut env, &payer, &ixs, alt, &mut alt_slot);
                let measured = measure(&message);
                let tx = VersionedTransaction::try_new(message, &[&trader.kp]).unwrap();
                let result = env
                    .svm
                    .send_transaction(tx)
                    .map(|meta| meta.compute_units_consumed)
                    .map_err(|e| format!("{:?}", e.err));
                env.svm.expire_blockhash();
                rows.push(Row {
                    n,
                    k,
                    alt,
                    predicted: budget(n, k, alt, true),
                    measured,
                    result,
                });
            }
        }
    }

    let mut md = String::from(
        "| shard-e | tick arrays/shard | mesazhi | llogari | bajt | rezultati (LiteSVM, mainnet features) | CU |\n|---:|---:|---|---:|---:|---|---:|\n",
    );
    for r in &rows {
        // The calculator is exact.
        assert_eq!(r.predicted, r.measured, "n={} k={} alt={}", r.n, r.k, r.alt);
        let fits_locks = r.measured.accounts <= ACCOUNT_LOCK_LIMIT;
        let fits_packet = r.measured.bytes <= PACKET_DATA_SIZE;
        // LiteSVM enforces the lock limit; the packet limit is enforced by
        // the network before execution, so it is reported, not executed.
        match &r.result {
            Ok(_) => assert!(
                fits_locks,
                "n={} k={} executed over the lock limit",
                r.n, r.k
            ),
            Err(e) => {
                assert!(!fits_locks, "n={} k={} alt={} failed: {e}", r.n, r.k, r.alt);
                assert!(e.contains("TooManyAccountLocks"), "{e}");
            }
        }
        let verdict = match (&r.result, fits_packet) {
            (Err(_), _) => format!(
                "refuzohet: {} > {ACCOUNT_LOCK_LIMIT} llogari",
                r.measured.accounts
            ),
            (Ok(_), false) => format!(
                "ekzekutohet, por {} > {PACKET_DATA_SIZE} bajt: s'dërgohet",
                r.measured.bytes
            ),
            (Ok(_), true) => "OK".into(),
        };
        let cu = r
            .result
            .as_ref()
            .map(|c| c.to_string())
            .unwrap_or_else(|_| "—".into());
        md += &format!(
            "| {} | {} | {} | {} | {} | {} | {} |\n",
            r.n,
            r.k,
            if r.alt { "v0 + ALT" } else { "legacy" },
            r.measured.accounts,
            r.measured.bytes,
            verdict,
            cu
        );
    }
    print!("{md}");
    if let Ok(path) = std::env::var("TX_BUDGET_OUT") {
        std::fs::write(path, &md).unwrap();
    }

    // The findings the docs rely on.
    let ok = |n, k, alt| {
        rows.iter()
            .find(|r| (r.n, r.k, r.alt) == (n, k, alt))
            .map(|r| r.result.is_ok() && r.measured.bytes <= PACKET_DATA_SIZE)
            .unwrap()
    };
    // Without an ALT the packet, not the lock limit, stops a split swap.
    assert!(ok(3, 1, false) && !ok(4, 1, false));
    assert!(ok(2, 3, false) && !ok(3, 3, false));
    // With an ALT all 8 shards fit for k = 1..=3 ...
    for k in 1..=3 {
        assert!(ok(8, k, true));
    }
    // ... but 8 shards x 3 arrays + ComputeBudget is exactly 64 accounts.
    // One more account (here the System Program, which a SOL wrap needs) is
    // refused even though every account comes from the lookup table.
    let mut ixs = vec![compute_limit(1_400_000)];
    for id in 0..8u8 {
        let arrays = env.swap_arrays(&m, id, false);
        ixs.push(env.ix_swap(&m, &trader, id, env.swap_args(false, 1_000), &arrays[..3]));
    }
    ixs.push(solana_instruction::Instruction {
        program_id: SYSTEM_PROGRAM,
        accounts: vec![
            solana_instruction::AccountMeta::new(payer, true),
            solana_instruction::AccountMeta::new(payer, true),
        ],
        data: [2u32.to_le_bytes().as_slice(), &0u64.to_le_bytes()].concat(),
    });
    let message = compile(&mut env, &payer, &ixs, true, &mut alt_slot);
    let b = measure(&message);
    assert_eq!(b.accounts, 65);
    assert!(b.bytes <= PACKET_DATA_SIZE);
    let tx = VersionedTransaction::try_new(message, &[&trader.kp]).unwrap();
    let err = env.svm.send_transaction(tx).unwrap_err().err;
    assert_eq!(
        err,
        solana_transaction_error::TransactionError::TooManyAccountLocks
    );
}
