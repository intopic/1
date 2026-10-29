//! Negative tests against the compiled SBF program, not a replacement model.
//! Frozen token accounts are deliberate runtime fault injection (our test
//! mints have no freeze authority); they are not claimed to be an external
//! attack against these mints. CPI logs prove which transfer failed.
//! Every rejected operation preserves complete account contents, excluding
//! the fee payer's SOL balance because failed transactions still pay fees.

mod common;

use anchor_lang::InstructionData;
use common::*;
use sharded_amm::errors::AmmError;
use sharded_amm::instructions::RebalanceArgs;
use solana_account::Account;
use solana_instruction::Instruction;
use solana_keypair::Keypair;
use solana_message::Message;
use solana_signer::Signer;
use solana_transaction::Transaction;
use solana_transaction_error::TransactionError;

fn snapshot(
    env: &Env,
    m: &Market,
    instructions: &[Instruction],
    payer: Pubkey,
) -> Vec<(Pubkey, Option<Account>)> {
    let mut keys = vec![m.market, m.registry];
    for id in 0..8 {
        let a = shard_addrs(m, id, 0);
        keys.extend([a.shard, a.vault_a, a.vault_b]);
        keys.extend(env.boundary_arrays(m, id, -1200, 1200));
    }
    for instruction in instructions {
        keys.extend(instruction.accounts.iter().map(|a| a.pubkey));
    }
    keys.retain(|k| *k != payer);
    keys.sort_unstable();
    keys.dedup();
    keys.into_iter().map(|k| (k, env.raw(&k))).collect()
}

fn assert_unchanged(env: &Env, before: &[(Pubkey, Option<Account>)]) {
    for (key, account) in before {
        assert_eq!(&env.raw(key), account, "failed transaction changed {key}");
    }
}

fn freeze_fixture(env: &mut Env, key: Pubkey, frozen: bool) {
    let mut account = env.raw(&key).unwrap();
    assert_eq!(account.owner, TOKEN_PROGRAM);
    assert_eq!(account.data.len(), 165);
    account.data[108] = if frozen { 2 } else { 1 };
    env.svm.set_account(key, account).unwrap();
}

/// Unlike a broad `is_err`, this requires the SPL Token AccountFrozen error
/// and proves that the preceding token transfers really executed in SVM.
fn frozen_failure(env: &mut Env, instruction: Instruction, signer: &Keypair, successes: usize) {
    let message = Message::new(&[instruction], Some(&signer.pubkey()));
    let tx = Transaction::new(&[signer], message, env.svm.latest_blockhash());
    let failure = env
        .svm
        .send_transaction(tx)
        .expect_err("frozen CPI must fail");
    env.svm.expire_blockhash();
    assert_eq!(
        failure.err,
        TransactionError::InstructionError(0, InstructionError::Custom(17)),
        "expected SPL Token AccountFrozen; logs: {:?}",
        failure.meta.logs
    );
    let success_line = format!("Program {TOKEN_PROGRAM} success");
    assert_eq!(
        failure
            .meta
            .logs
            .iter()
            .filter(|l| **l == success_line)
            .count(),
        successes,
        "wrong transfer stage: {:?}",
        failure.meta.logs
    );
}

#[test]
fn frozen_swap_input_rolls_back_both_directions() {
    for a_to_b in [true, false] {
        let mut env = Env::new();
        let (m, _) = env.open_seeded_market();
        let trader = env.user(&m, 1_000_000_000_000);
        env.swap(&m, &trader, 0, a_to_b, 1_000_000).unwrap();
        let token = if a_to_b {
            trader.token_a
        } else {
            trader.token_b
        };
        freeze_fixture(&mut env, token, true);
        let arrays = env.swap_arrays(&m, 0, a_to_b);
        let i = env.ix_swap(&m, &trader, 0, env.swap_args(a_to_b, 1_000_000), &arrays);
        let before = snapshot(&env, &m, std::slice::from_ref(&i), trader.kp.pubkey());
        frozen_failure(&mut env, i.clone(), &trader.kp, 0);
        assert_unchanged(&env, &before);
        freeze_fixture(&mut env, token, false);
        env.send(&[i], &[&trader.kp]).unwrap();
    }
}

#[test]
fn frozen_swap_output_rolls_back_successful_input_transfer() {
    for a_to_b in [true, false] {
        let mut env = Env::new();
        let (m, _) = env.open_seeded_market();
        let trader = env.user(&m, 1_000_000_000_000);
        env.swap(&m, &trader, 0, a_to_b, 1_000_000).unwrap();
        let token = if a_to_b {
            trader.token_b
        } else {
            trader.token_a
        };
        freeze_fixture(&mut env, token, true);
        let arrays = env.swap_arrays(&m, 0, a_to_b);
        let i = env.ix_swap(&m, &trader, 0, env.swap_args(a_to_b, 1_000_000), &arrays);
        let before = snapshot(&env, &m, std::slice::from_ref(&i), trader.kp.pubkey());
        frozen_failure(&mut env, i.clone(), &trader.kp, 1);
        assert_unchanged(&env, &before);
        freeze_fixture(&mut env, token, false);
        env.send(&[i], &[&trader.kp]).unwrap();
    }
}

#[test]
fn lp_maximum_a_and_b_are_independently_enforced_atomically() {
    let mut env = Env::new();
    let (m, lp) = env.open_seeded_market();
    for cap_a in [true, false] {
        let mut i = env.ix_increase(&m, &lp, 0, -1200, 1200, 1_000_000);
        i.data = sharded_amm::instruction::IncreaseLiquidity {
            liquidity: 1_000_000,
            amount_a_max: if cap_a { 0 } else { u64::MAX },
            amount_b_max: if cap_a { u64::MAX } else { 0 },
        }
        .data();
        let before = snapshot(&env, &m, std::slice::from_ref(&i), lp.kp.pubkey());
        expect_custom(
            env.send(&[i], &[&lp.kp]),
            amm_code(AmmError::SlippageExceeded),
        );
        assert_unchanged(&env, &before);
    }
    let i = env.ix_increase(&m, &lp, 0, -1200, 1200, 1_000_000);
    env.send(&[i], &[&lp.kp]).unwrap();
}

#[test]
fn lp_minimum_a_and_b_are_independently_enforced_atomically() {
    let mut env = Env::new();
    let (m, lp) = env.open_seeded_market();
    for floor_a in [true, false] {
        let mut i = env.ix_decrease(&m, &lp, 0, -1200, 1200, 1_000_000);
        i.data = sharded_amm::instruction::DecreaseLiquidity {
            liquidity: 1_000_000,
            amount_a_min: if floor_a { u64::MAX } else { 0 },
            amount_b_min: if floor_a { 0 } else { u64::MAX },
        }
        .data();
        let before = snapshot(&env, &m, std::slice::from_ref(&i), lp.kp.pubkey());
        expect_custom(
            env.send(&[i], &[&lp.kp]),
            amm_code(AmmError::SlippageExceeded),
        );
        assert_unchanged(&env, &before);
    }
    let i = env.ix_decrease(&m, &lp, 0, -1200, 1200, 1_000_000);
    env.send(&[i], &[&lp.kp]).unwrap();
}

#[test]
fn lp_second_transfer_failure_rolls_back_deposit_and_withdrawal() {
    for deposit in [true, false] {
        let mut env = Env::new();
        let (m, lp) = env.open_seeded_market();
        freeze_fixture(&mut env, lp.token_b, true);
        let i = if deposit {
            env.ix_increase(&m, &lp, 0, -1200, 1200, 1_000_000)
        } else {
            env.ix_decrease(&m, &lp, 0, -1200, 1200, 1_000_000)
        };
        let before = snapshot(&env, &m, std::slice::from_ref(&i), lp.kp.pubkey());
        frozen_failure(&mut env, i.clone(), &lp.kp, 1);
        assert_unchanged(&env, &before);
        freeze_fixture(&mut env, lp.token_b, false);
        env.send(&[i], &[&lp.kp]).unwrap();
    }
}

#[test]
fn second_shard_slippage_rolls_back_the_entire_split_transaction() {
    let mut env = Env::new();
    let (m, _) = env.open_seeded_market();
    let trader = env.user(&m, 1_000_000_000_000);
    let make = |env: &Env, id, min_amount_out| {
        let mut args = env.swap_args(true, 1_000_000);
        args.min_amount_out = min_amount_out;
        env.ix_swap(&m, &trader, id, args, &env.swap_arrays(&m, id, true))
    };
    let first = make(&env, 0, 0);
    // Positive control for leg 1 before testing transaction-wide rollback.
    env.send(std::slice::from_ref(&first), &[&trader.kp])
        .unwrap();
    let first = make(&env, 0, 0);
    let failed = make(&env, 1, u64::MAX);
    let batch = [first.clone(), failed];
    let before = snapshot(&env, &m, &batch, trader.kp.pubkey());
    assert_eq!(
        env.send(&batch, &[&trader.kp]).unwrap_err(),
        TransactionError::InstructionError(
            1,
            InstructionError::Custom(amm_code(AmmError::SlippageExceeded))
        )
    );
    assert_unchanged(&env, &before);
    let second = make(&env, 1, 0);
    env.send(&[first, second], &[&trader.kp]).unwrap();
}

#[test]
fn rebalance_second_and_third_cpi_failures_roll_back_both_shards() {
    for fail_third in [false, true] {
        let mut env = Env::new();
        let (m, _) = env.open_seeded_market();
        let trader = env.user(&m, 1_000_000_000_000);
        env.swap(&m, &trader, 0, true, 20_000_000_000).unwrap();
        let keeper = env.keeper.insecure_clone();
        let profit = env.token_account(&m.mint_b, &keeper.pubkey(), 0);
        let (x, y) = (shard_addrs(&m, 0, 0), shard_addrs(&m, 1, 0));
        let ax = env.swap_arrays(&m, 0, false);
        let ay = env.swap_arrays(&m, 1, true);
        let i = with_remaining(
            ix(
                sharded_amm::accounts::RebalancePair {
                    caller: keeper.pubkey(),
                    market: m.market,
                    registry: m.registry,
                    shard_x: x.shard,
                    vault_x_a: x.vault_a,
                    vault_x_b: x.vault_b,
                    vault_authority_x: x.vault_authority,
                    shard_y: y.shard,
                    vault_y_a: y.vault_a,
                    vault_y_b: y.vault_b,
                    vault_authority_y: y.vault_authority,
                    caller_token: profit,
                    token_program: TOKEN_PROGRAM,
                },
                sharded_amm::instruction::RebalancePair {
                    args: RebalanceArgs {
                        input_is_a: false,
                        amount_in: 2_000_000_000,
                        min_profit: 1,
                        arrays_x: ax.len() as u8,
                        deadline: env.now() + 60,
                    },
                },
            ),
            &[ax, ay].concat(),
        );
        // B flows y -> x, then A flows x -> y, then B flows y -> caller.
        let frozen = if fail_third { profit } else { y.vault_a };
        freeze_fixture(&mut env, frozen, true);
        let before = snapshot(&env, &m, std::slice::from_ref(&i), keeper.pubkey());
        frozen_failure(&mut env, i.clone(), &keeper, if fail_third { 2 } else { 1 });
        assert_unchanged(&env, &before);
        freeze_fixture(&mut env, frozen, false);
        env.send(&[i], &[&keeper]).unwrap();
        assert!(
            env.balance(&profit) > 0,
            "positive control must settle profit"
        );
    }
}
