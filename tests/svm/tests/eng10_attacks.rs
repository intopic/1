//! Adversarial scenarios on the deployed program: each one is something an
//! attacker would try, and asserts that it either fails or gains nothing.

mod common;

use common::*;
use sharded_amm::errors::AmmError;
use sharded_amm::math::liquidity_math::max_liquidity_per_tick;
use sharded_amm::state::array_span;
use solana_instruction::{AccountMeta, Instruction};
use solana_signer::Signer;

fn spl_transfer(from: &Pubkey, to: &Pubkey, owner: &Pubkey, amount: u64) -> Instruction {
    let mut data = vec![3u8];
    data.extend_from_slice(&amount.to_le_bytes());
    Instruction {
        program_id: TOKEN_PROGRAM,
        accounts: vec![
            AccountMeta::new(*from, false),
            AccountMeta::new(*to, false),
            AccountMeta::new_readonly(*owner, true),
        ],
        data,
    }
}

/// Value of `(a, b)` in B units at the shard's current price.
fn value(env: &Env, m: &Market, id: u8, a: u64, b: u64) -> f64 {
    let s = env.shard_state(m, id).sqrt_price_x64 as f64 / 18_446_744_073_709_551_616.0;
    a as f64 * s * s + b as f64
}

#[test]
fn vault_donation_changes_no_outcome() {
    // Two identical markets; one gets a large direct donation into a vault.
    // Every later swap and exit must give the same amounts in both.
    let mut plain = Env::new();
    let mut donated = Env::new();
    let (m1, lp1) = plain.open_seeded_market();
    let (m2, lp2) = donated.open_seeded_market();
    let attacker = donated.user(&m2, 1_000_000_000_000);
    let vault = shard_addrs(&m2, 0, 0).vault_b;
    donated
        .send(
            &[spl_transfer(
                &attacker.token_b,
                &vault,
                &attacker.kp.pubkey(),
                777_777_777,
            )],
            &[&attacker.kp],
        )
        .unwrap();
    let t1 = plain.user(&m1, 1_000_000_000_000);
    let t2 = donated.user(&m2, 1_000_000_000_000);
    for k in 0..20u64 {
        let a_to_b = k % 3 != 0;
        let amount = 10_000_000 + k * 1_234_567;
        let b1 = (plain.balance(&t1.token_a), plain.balance(&t1.token_b));
        let b2 = (donated.balance(&t2.token_a), donated.balance(&t2.token_b));
        plain.swap(&m1, &t1, 0, a_to_b, amount).unwrap();
        donated.swap(&m2, &t2, 0, a_to_b, amount).unwrap();
        let d1 = (
            plain.balance(&t1.token_a) as i128 - b1.0 as i128,
            plain.balance(&t1.token_b) as i128 - b1.1 as i128,
        );
        let d2 = (
            donated.balance(&t2.token_a) as i128 - b2.0 as i128,
            donated.balance(&t2.token_b) as i128 - b2.1 as i128,
        );
        assert_eq!(d1, d2, "swap {k}");
    }
    let s1 = plain.shard_state(&m1, 0);
    let s2 = donated.shard_state(&m2, 0);
    assert_eq!(
        (s1.sqrt_price_x64, s1.liquidity, s1.fee_growth_global_a_x128),
        (s2.sqrt_price_x64, s2.liquidity, s2.fee_growth_global_a_x128)
    );
    // LP exits get the same amounts; the donation stays in the vault.
    let get = |env: &mut Env, m: &Market, lp: &Lp| {
        let before = (env.balance(&lp.token_a), env.balance(&lp.token_b));
        let i = env.ix_decrease(m, lp, 0, -1200, 1200, 1_000_000_000_000);
        let c = env.ix_collect(m, lp, 0, -1200, 1200);
        let kp = lp.kp.insecure_clone();
        env.send(&[i, c], &[&kp]).unwrap();
        (
            env.balance(&lp.token_a) - before.0,
            env.balance(&lp.token_b) - before.1,
        )
    };
    assert_eq!(get(&mut plain, &m1, &lp1), get(&mut donated, &m2, &lp2));
    assert!(donated.balance(&vault) >= 777_777_777);
}

#[test]
fn dust_swaps_never_profit() {
    let mut env = Env::new();
    let (m, _) = env.open_seeded_market();
    let attacker = env.user(&m, 10_000_000);
    let start = value(
        &env,
        &m,
        0,
        env.balance(&attacker.token_a),
        env.balance(&attacker.token_b),
    );
    let (mut ok, mut failed) = (0, 0);
    for k in 0..1_500u64 {
        let amount = 1 + k % 97;
        match env.swap(&m, &attacker, 0, k % 2 == 0, amount) {
            Ok(_) => ok += 1,
            Err(_) => failed += 1,
        }
    }
    let end = value(
        &env,
        &m,
        0,
        env.balance(&attacker.token_a),
        env.balance(&attacker.token_b),
    );
    println!("dust swaps ok={ok} failed={failed} start={start} end={end}");
    assert!(ok > 0);
    assert!(end < start, "dust farming gained {}", end - start);
}

#[test]
fn jit_liquidity_earns_at_most_the_victims_fee() {
    let mut env = Env::new();
    let (m, _) = env.open_seeded_market();
    let attacker = env.user(&m, u64::MAX / 8);
    let victim = env.user(&m, 1_000_000_000_000);
    let before = (
        env.balance(&attacker.token_a),
        env.balance(&attacker.token_b),
    );
    // Huge narrow position right at the price, then the victim trades.
    env.add_liquidity(&m, &attacker, 0, -60, 60, 1_000_000_000_000_000)
        .unwrap();
    let arrays = env.swap_arrays(&m, 0, true);
    let args = env.swap_args(true, 5_000_000_000);
    let i = env.ix_swap(&m, &victim, 0, args, &arrays);
    env.send(&[i], &[&victim.kp]).unwrap();
    let protocol = env.shard_state(&m, 0).protocol_fees_a;
    // Total fee is protocol / 20%; LP share is the remaining 80%.
    let total_fee_upper = protocol * 5 + 5;
    let i = env.ix_decrease(&m, &attacker, 0, -60, 60, 1_000_000_000_000_000);
    let c = env.ix_collect(&m, &attacker, 0, -60, 60);
    env.send(&[i, c], &[&attacker.kp]).unwrap();
    let after = (
        env.balance(&attacker.token_a),
        env.balance(&attacker.token_b),
    );
    // Value change at the new price: the fee income minus what the price
    // move did to the position; it can never exceed the LP part of the fee.
    let gain = value(&env, &m, 0, after.0, after.1) - value(&env, &m, 0, before.0, before.1);
    assert!(gain <= (total_fee_upper - protocol) as f64, "gain {gain}");
}

#[test]
fn extreme_amounts_and_bounds() {
    let mut env = Env::new();
    let (m, _) = env.open_seeded_market();
    let whale = env.user(&m, u64::MAX / 2);
    // More than the whole active range can absorb: rejected, not partial.
    let st = env.shard_state(&m, 0);
    for a_to_b in [true, false] {
        let r = env.swap(&m, &whale, 0, a_to_b, u64::MAX / 4);
        assert!(r.is_err());
        assert_eq!(env.shard_state(&m, 0), st);
    }
    // Per-tick liquidity cap.
    let cap = max_liquidity_per_tick(60).unwrap();
    expect_custom(
        env.add_liquidity(&m, &whale, 1, -60, 60, cap + 1),
        amm_code(AmmError::LiquidityPerTickExceeded),
    );
    // Tick arrays: bad starts and re-initialization.
    let keeper = env.keeper.insecure_clone();
    let shard = shard_addrs(&m, 0, 0).shard;
    let span = array_span(60);
    for bad in [
        1,
        span / 2,
        -span - 60,
        sharded_amm::constants::MAX_TICK + span,
    ] {
        expect_custom(
            env.send(
                &[Env::ix_init_tick_array(&m, &keeper.pubkey(), &shard, bad)],
                &[&keeper],
            ),
            amm_code(AmmError::InvalidTickArrayStart),
        );
    }
    let existing = 0;
    let arr = env.tick_array(&tick_array_pda(&shard, existing));
    assert!(env
        .send(
            &[Env::ix_init_tick_array(
                &m,
                &keeper.pubkey(),
                &shard,
                existing
            )],
            &[&keeper]
        )
        .is_err());
    assert_eq!(env.tick_array(&tick_array_pda(&shard, existing)), arr);
    // Positions at the very edges of the domain work and fully exit.
    let lo = (sharded_amm::constants::MIN_TICK / 60) * 60;
    let hi = (sharded_amm::constants::MAX_TICK / 60) * 60;
    env.add_liquidity(&m, &whale, 2, lo, hi, 1_000_000_000)
        .unwrap();
    let kp = whale.kp.insecure_clone();
    let i = env.ix_decrease(&m, &whale, 2, lo, hi, 1_000_000_000);
    env.send(&[i], &[&kp]).unwrap();
}

#[test]
fn routing_output_into_a_vault_drains_nothing() {
    // Paying the output into the pool's own vault only burns the caller's input.
    let mut env = Env::new();
    let (m, _) = env.open_seeded_market();
    let attacker = env.user(&m, 1_000_000_000);
    let a = shard_addrs(&m, 0, 0);
    let vaults_before = env.balance(&a.vault_a) + env.balance(&a.vault_b);
    let arrays = env.swap_arrays(&m, 0, true);
    let mut i = env.ix_swap(&m, &attacker, 0, env.swap_args(true, 1_000_000), &arrays);
    i.accounts[7].pubkey = a.vault_b;
    // Refused before any transfer (the output account is not the user's);
    // even past that, a self-transfer would fail the balance check after
    // every transfer (`token_util`, TransferAmountMismatch).
    assert!(env.send(&[i], &[&attacker.kp]).is_err());
    let vaults_after = env.balance(&a.vault_a) + env.balance(&a.vault_b);
    assert_eq!(vaults_after, vaults_before);
    assert_eq!(env.balance(&attacker.token_a), 1_000_000_000);
    // Paying from a vault without its authority fails.
    let mut i = env.ix_swap(&m, &attacker, 0, env.swap_args(true, 1_000_000), &arrays);
    i.accounts[6].pubkey = a.vault_a;
    assert!(env.send(&[i], &[&attacker.kp]).is_err());
}
