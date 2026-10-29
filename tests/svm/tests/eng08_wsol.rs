//! ADR-0004: the SOL/USDC pool is a WSOL/USDC market. The program sees two
//! ordinary SPL mints; wrapping (transfer + SyncNative) and unwrapping
//! (CloseAccount) happen on the client side, in the same transaction. These
//! tests run that flow against the real token program with native vaults.

mod common;

use common::*;
use solana_account::Account;
use solana_instruction::{AccountMeta, Instruction};
use solana_keypair::Keypair;
use solana_signer::Signer;

const SYSTEM: Pubkey = anchor_lang::system_program::ID;

fn native_mint_data() -> Vec<u8> {
    let mut d = vec![0u8; 82];
    d[44] = 9; // decimals
    d[45] = 1; // initialized; no mint or freeze authority
    d
}

fn transfer_lamports(from: &Pubkey, to: &Pubkey, lamports: u64) -> Instruction {
    let mut data = 2u32.to_le_bytes().to_vec();
    data.extend_from_slice(&lamports.to_le_bytes());
    Instruction {
        program_id: SYSTEM,
        accounts: vec![AccountMeta::new(*from, true), AccountMeta::new(*to, false)],
        data,
    }
}

fn sync_native(account: &Pubkey) -> Instruction {
    Instruction {
        program_id: TOKEN_PROGRAM,
        accounts: vec![AccountMeta::new(*account, false)],
        data: vec![17],
    }
}

fn close_account(account: &Pubkey, dest: &Pubkey, owner: &Pubkey) -> Instruction {
    Instruction {
        program_id: TOKEN_PROGRAM,
        accounts: vec![
            AccountMeta::new(*account, false),
            AccountMeta::new(*dest, false),
            AccountMeta::new_readonly(*owner, true),
        ],
        data: vec![9],
    }
}

fn wsol_usdc_market(env: &mut Env) -> (Market, Lp) {
    if env.raw(&NATIVE_MINT).is_none() {
        let data = native_mint_data();
        let lamports = env.svm.minimum_balance_for_rent_exemption(82);
        env.svm
            .set_account(
                NATIVE_MINT,
                Account {
                    lamports,
                    data,
                    owner: TOKEN_PROGRAM,
                    executable: false,
                    rent_epoch: 0,
                },
            )
            .unwrap();
    }
    let usdc = env.create_mint(6, TOKEN_PROGRAM);
    let (a, b) = if NATIVE_MINT.to_bytes() < usdc.to_bytes() {
        (NATIVE_MINT, usdc)
    } else {
        (usdc, NATIVE_MINT)
    };
    let m = env.market_with(a, b, 1);
    let lp = env.seed_initial_shards(&m, 1_000_000_000_000);
    let keeper = env.keeper.insecure_clone();
    let i = env.ix_open_market(&m, &keeper.pubkey(), &(0..8).collect::<Vec<_>>());
    env.send(&[i], &[&keeper]).expect("open");
    (m, lp)
}

/// Native accounts must always hold rent + amount lamports.
fn assert_native_consistent(env: &Env, key: &Pubkey) {
    let acc = env.raw(key).unwrap();
    let (mint, _, amount) = token_account_fields(&acc.data);
    assert_eq!(mint, NATIVE_MINT);
    assert_eq!(&acc.data[109..113], &1u32.to_le_bytes(), "is_native set");
    let rent = u64::from_le_bytes(acc.data[113..121].try_into().unwrap());
    assert_eq!(acc.lamports, rent + amount, "lamports = rent + amount");
}

#[test]
fn vaults_are_native_and_stay_consistent() {
    let mut env = Env::new();
    let (m, _) = wsol_usdc_market(&mut env);
    let sol_is_a = m.mint_a == NATIVE_MINT;
    let t = env.user(&m, 1_000_000_000_000);
    for k in 0..10u64 {
        env.swap(&m, &t, (k % 8) as u8, k % 2 == 0, 1_000_000 + k * 7_919)
            .unwrap();
    }
    for id in 0..8 {
        let a = shard_addrs(&m, id, 0);
        assert_native_consistent(&env, if sol_is_a { &a.vault_a } else { &a.vault_b });
    }
}

#[test]
fn wrap_swap_unwrap_in_one_transaction() {
    let mut env = Env::new();
    let (m, _) = wsol_usdc_market(&mut env);
    let sol_is_a = m.mint_a == NATIVE_MINT;
    let usdc = if sol_is_a { m.mint_b } else { m.mint_a };
    let trader = Keypair::new();
    env.svm.airdrop(&trader.pubkey(), 10_000_000_000).unwrap();
    // An empty WSOL account and an empty USDC account.
    let wsol = env.token_account(&NATIVE_MINT, &trader.pubkey(), 0);
    let usdc_acc = env.token_account(&usdc, &trader.pubkey(), 0);
    let user = Lp {
        kp: trader.insecure_clone(),
        token_a: if sol_is_a { wsol } else { usdc_acc },
        token_b: if sol_is_a { usdc_acc } else { wsol },
    };

    // SOL -> USDC: wrap 2 SOL, swap it all, close the WSOL account.
    let sol_in = 2_000_000_000u64;
    let wallet0 = env.svm.get_account(&trader.pubkey()).unwrap().lamports;
    let wsol_rent = env.raw(&wsol).unwrap().lamports;
    let args = env.swap_args(sol_is_a, sol_in);
    let arrays = env.swap_arrays(&m, 0, sol_is_a);
    let swap = env.ix_swap(&m, &user, 0, args, &arrays);
    env.send(
        &[
            transfer_lamports(&trader.pubkey(), &wsol, sol_in),
            sync_native(&wsol),
            swap,
            close_account(&wsol, &trader.pubkey(), &trader.pubkey()),
        ],
        &[&trader],
    )
    .expect("wrap + swap + unwrap");
    let usdc_out = env.balance(&usdc_acc);
    assert!(usdc_out > 0);
    let wallet1 = env.svm.get_account(&trader.pubkey()).unwrap().lamports;
    // Wallet paid exactly the input and the tx fee, and got the rent back.
    let fee = 5_000;
    assert_eq!(wallet0 + wsol_rent - sol_in - fee, wallet1);
    assert!(env.raw(&wsol).is_none_or(|a| a.lamports == 0), "closed");

    // USDC -> SOL: a fresh WSOL account receives, then is closed to the wallet.
    let wsol2 = env.token_account(&NATIVE_MINT, &trader.pubkey(), 0);
    let rent2 = env.raw(&wsol2).unwrap().lamports;
    let user2 = Lp {
        kp: trader.insecure_clone(),
        token_a: if sol_is_a { wsol2 } else { usdc_acc },
        token_b: if sol_is_a { usdc_acc } else { wsol2 },
    };
    let args = env.swap_args(!sol_is_a, usdc_out);
    let arrays = env.swap_arrays(&m, 0, !sol_is_a);
    let swap = env.ix_swap(&m, &user2, 0, args, &arrays);
    let sol_before = env.svm.get_account(&trader.pubkey()).unwrap().lamports;
    env.send(
        &[
            swap,
            close_account(&wsol2, &trader.pubkey(), &trader.pubkey()),
        ],
        &[&trader],
    )
    .expect("swap + unwrap");
    let sol_after = env.svm.get_account(&trader.pubkey()).unwrap().lamports;
    let sol_back = sol_after + fee - sol_before - rent2;
    assert_eq!(env.balance(&usdc_acc), 0);
    // Round trip loses fees and rounding, never gains.
    assert!(
        sol_back < sol_in && sol_back > sol_in * 99 / 100,
        "{sol_back}"
    );
    for id in 0..8 {
        let a = shard_addrs(&m, id, 0);
        assert_native_consistent(&env, if sol_is_a { &a.vault_a } else { &a.vault_b });
    }
}

#[test]
fn unsynced_lamports_are_not_tradeable() {
    // Lamports sent to a WSOL account without SyncNative do not count: the
    // swap sees amount 0 and the token transfer fails; nothing moves.
    let mut env = Env::new();
    let (m, _) = wsol_usdc_market(&mut env);
    let sol_is_a = m.mint_a == NATIVE_MINT;
    let usdc = if sol_is_a { m.mint_b } else { m.mint_a };
    let trader = Keypair::new();
    env.svm.airdrop(&trader.pubkey(), 10_000_000_000).unwrap();
    let wsol = env.token_account(&NATIVE_MINT, &trader.pubkey(), 0);
    let usdc_acc = env.token_account(&usdc, &trader.pubkey(), 0);
    let user = Lp {
        kp: trader.insecure_clone(),
        token_a: if sol_is_a { wsol } else { usdc_acc },
        token_b: if sol_is_a { usdc_acc } else { wsol },
    };
    let args = env.swap_args(sol_is_a, 1_000_000_000);
    let arrays = env.swap_arrays(&m, 0, sol_is_a);
    let swap = env.ix_swap(&m, &user, 0, args, &arrays);
    let st = env.shard_state(&m, 0);
    assert!(env
        .send(
            &[
                transfer_lamports(&trader.pubkey(), &wsol, 1_000_000_000),
                swap
            ],
            &[&trader],
        )
        .is_err());
    assert_eq!(env.shard_state(&m, 0), st);
    assert_eq!(env.balance(&usdc_acc), 0);
}
