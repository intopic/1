//! ADR-0005: per-shard dynamic fee on the real SBF program.
//!
//! - Off by default: every swap pays the base fee.
//! - Enabled through the fee timelock; bad parameters are refused.
//! - A swap that moves the price raises the next fee on that shard only; the
//!   other shards keep the base fee and their bytes do not change.
//! - The host quote (`dynamic_fee::quote_fee` over the on-chain accounts) is
//!   exactly the rate the program charged, as reported in `SwapExecuted`.
//! - After `decay_slots` without swaps the fee is back to the base.

mod common;

use anchor_lang::{AnchorDeserialize, Discriminator};
use base64::Engine;
use common::*;
use sharded_amm::constants::{FEE_TIMELOCK_SECONDS, MAX_DYNAMIC_FEE_RATE_PPM};
use sharded_amm::engine::dynamic_fee::quote_fee;
use sharded_amm::errors::AmmError;
use sharded_amm::events::SwapExecuted;
use sharded_amm::state::DynamicFeeParams;
use solana_signer::Signer;

const BASE: u32 = 3_000;

fn params() -> DynamicFeeParams {
    DynamicFeeParams {
        max_fee_rate_ppm: MAX_DYNAMIC_FEE_RATE_PPM,
        vol_coeff_ppm: 200,
        load_coeff_ppm: 0,
        max_vol_accumulator: 1_000_000,
        filter_slots: 10,
        decay_slots: 100,
        reduction_bps: 5_000,
    }
}

fn set_slot(env: &mut Env, slot: u64) {
    let mut c = env.svm.get_sysvar::<anchor_lang::prelude::Clock>();
    c.slot = slot;
    env.svm.set_sysvar(&c);
}

fn enable(env: &mut Env, m: &Market, p: DynamicFeeParams) {
    let admin = env.admin.insecure_clone();
    env.send(
        &[Env::ix_update_fees_dynamic(
            m,
            &admin.pubkey(),
            BASE,
            2_000,
            p,
        )],
        &[&admin],
    )
    .unwrap();
    env.warp(FEE_TIMELOCK_SECONDS);
    env.send(&[Env::ix_apply_fees(m, &admin.pubkey())], &[&admin])
        .unwrap();
    assert_eq!(env.market_state(m).dynamic_fees, p);
}

/// Swaps and returns the decoded `SwapExecuted` event.
fn swap(env: &mut Env, m: &Market, t: &Lp, id: u8, a_to_b: bool, amount: u64) -> SwapExecuted {
    let arrays = env.swap_arrays(m, id, a_to_b);
    let i = env.ix_swap(m, t, id, env.swap_args(a_to_b, amount), &arrays);
    let msg = solana_message::Message::new(&[i], Some(&t.kp.pubkey()));
    let tx = solana_transaction::Transaction::new(&[&t.kp], msg, env.svm.latest_blockhash());
    let meta = env.svm.send_transaction(tx).expect("swap");
    env.svm.expire_blockhash();
    let b64 = base64::engine::general_purpose::STANDARD;
    let evs: Vec<SwapExecuted> = meta
        .logs
        .iter()
        .filter_map(|l| l.strip_prefix("Program data: "))
        .map(|d| b64.decode(d).unwrap())
        .filter(|d| d.starts_with(SwapExecuted::DISCRIMINATOR))
        .map(|d| SwapExecuted::deserialize(&mut &d[8..]).unwrap())
        .collect();
    assert_eq!(evs.len(), 1);
    let ev = evs.into_iter().next().unwrap();
    // The fee charged is at least the rate times the input (plus dust).
    assert!(
        (u128::from(ev.total_fee) + 1) * 1_000_000
            >= u128::from(ev.amount_in) * u128::from(ev.fee_rate_ppm)
    );
    assert_eq!(ev.liquidity, env.shard_state(m, id).liquidity);
    ev
}

fn quote(env: &Env, m: &Market, id: u8, slot: u64) -> u32 {
    let mk = env.market_state(m);
    let s = env.shard_state(m, id);
    quote_fee(
        &s.dynamic_fee,
        &mk.dynamic_fees,
        mk.params.fee_rate_ppm,
        mk.params.tick_spacing,
        s.tick_current,
        slot,
    )
}

#[test]
fn off_by_default() {
    let mut env = Env::new();
    let (m, _) = env.open_seeded_market();
    assert_eq!(
        env.market_state(&m).dynamic_fees,
        DynamicFeeParams::disabled(BASE)
    );
    let t = env.user(&m, 1_000_000_000_000);
    for k in 0..4u64 {
        set_slot(&mut env, 500 + k);
        let ev = swap(&mut env, &m, &t, 0, k % 2 == 0, 30_000_000_000);
        assert_eq!(ev.fee_rate_ppm, BASE);
    }
}

#[test]
fn bad_parameters_are_refused() {
    let mut env = Env::new();
    let (m, _) = env.open_seeded_market();
    let admin = env.admin.insecure_clone();
    for p in [
        DynamicFeeParams {
            max_fee_rate_ppm: MAX_DYNAMIC_FEE_RATE_PPM + 1,
            ..params()
        },
        DynamicFeeParams {
            filter_slots: 0,
            ..params()
        },
        DynamicFeeParams {
            filter_slots: 101,
            ..params()
        },
    ] {
        expect_custom(
            env.send(
                &[Env::ix_update_fees_dynamic(
                    &m,
                    &admin.pubkey(),
                    BASE,
                    2_000,
                    p,
                )],
                &[&admin],
            ),
            amm_code(AmmError::InvalidDynamicFeeParams),
        );
    }
}

#[test]
fn movement_raises_the_fee_on_that_shard_only() {
    let mut env = Env::new();
    let (m, _) = env.open_seeded_market();
    enable(&mut env, &m, params());
    let t = env.user(&m, 1_000_000_000_000);
    let other = |env: &Env| -> Vec<Vec<u8>> {
        (1..8)
            .map(|id| {
                let a = shard_addrs(&m, id, 0);
                [a.shard, a.vault_a, a.vault_b]
                    .iter()
                    .flat_map(|k| env.raw(k).unwrap().data)
                    .collect()
            })
            .collect()
    };
    let before = other(&env);

    let s = 10_000u64;
    set_slot(&mut env, s);
    assert_eq!(quote(&env, &m, 0, s), BASE);
    let first = swap(&mut env, &m, &t, 0, true, 30_000_000_000);
    assert_eq!(first.fee_rate_ppm, BASE, "no history yet");
    let moved = first.tick.unsigned_abs() / 60;
    assert!(moved >= 5, "the swap must move the price: {}", first.tick);

    // Next slot on shard 0: the quote is the charged rate and above the base.
    set_slot(&mut env, s + 1);
    let q = quote(&env, &m, 0, s + 1);
    assert!(q > BASE, "quote {q}");
    let second = swap(&mut env, &m, &t, 0, false, 1_000_000);
    assert_eq!(second.fee_rate_ppm, q);
    // Shards 1..8 were not touched by shard 0's work.
    assert_eq!(other(&env), before);
    // And shard 1 still charges the base fee in the same slot.
    let elsewhere = swap(&mut env, &m, &t, 1, false, 1_000_000);
    assert_eq!(elsewhere.fee_rate_ppm, BASE);

    // After the decay period without swaps, shard 0 is back to the base.
    let later = s + 1 + u64::from(params().decay_slots);
    set_slot(&mut env, later);
    assert_eq!(quote(&env, &m, 0, later), BASE);
    let third = swap(&mut env, &m, &t, 0, true, 1_000_000);
    assert_eq!(third.fee_rate_ppm, BASE);
}

#[test]
fn load_term_and_ceiling() {
    let mut env = Env::new();
    let (m, _) = env.open_seeded_market();
    let p = DynamicFeeParams {
        vol_coeff_ppm: 0,
        load_coeff_ppm: 1_000,
        ..params()
    };
    enable(&mut env, &m, p);
    let t = env.user(&m, 1_000_000_000_000);
    set_slot(&mut env, 777);
    // Base, +1000, +2000, +3000, +4000, then the 0.80% ceiling.
    let expected = [3_000, 4_000, 5_000, 6_000, 7_000, 8_000, 8_000];
    for (k, want) in expected.into_iter().enumerate() {
        let ev = swap(&mut env, &m, &t, 2, k % 2 == 0, 1_000_000 + k as u64);
        assert_eq!(ev.fee_rate_ppm, want, "swap {k}");
    }
    set_slot(&mut env, 778);
    assert_eq!(
        swap(&mut env, &m, &t, 2, true, 1_000_000).fee_rate_ppm,
        BASE
    );
}
