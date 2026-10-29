//! ADR-0007: the off-chain router against the real SBF program.
//!
//! Books are loaded from the on-chain accounts; the plan is sent as one
//! transaction; the trader receives exactly the planned output and every
//! shard ends where the router's own books say. A small order uses one
//! shard, a large one several. `find_rebalance` finds a profitable
//! `rebalance_pair` that pays exactly the predicted profit to a caller with
//! no tokens.

mod common;

use amm_router::{apply_rebalance, find_rebalance, plan, Book, MarketCfg, RouterConfig};
use common::*;
use sharded_amm::engine::Pool;
use sharded_amm::instructions::RebalanceArgs;
use sharded_amm::state::array_span;
use solana_signer::Signer;

fn compute_limit(units: u32) -> solana_instruction::Instruction {
    let mut data = vec![2u8];
    data.extend_from_slice(&units.to_le_bytes());
    solana_instruction::Instruction {
        program_id: anchor_lang::pubkey!("ComputeBudget111111111111111111111111111111"),
        accounts: vec![],
        data,
    }
}

fn books(env: &Env, m: &Market) -> (Vec<Book>, MarketCfg, u64) {
    let cfg = MarketCfg::from_config(&env.market_state(m));
    let span = array_span(cfg.tick_spacing);
    let books = (0..8u8)
        .map(|id| {
            let shard = shard_addrs(m, id, 0).shard;
            let arrays = (-8..8)
                .filter_map(|i| {
                    let k = tick_array_pda(&shard, i * span);
                    env.raw(&k).map(|_| env.tick_array(&k))
                })
                .collect();
            Book::from_state(id, &env.shard_state(m, id), arrays).unwrap()
        })
        .collect();
    let slot = env.svm.get_sysvar::<anchor_lang::prelude::Clock>().slot;
    (books, cfg, slot)
}

fn market() -> (Env, Market) {
    let mut env = Env::new();
    let (m, _) = env.open_seeded_market();
    let span = array_span(env.market_state(&m).params.tick_spacing);
    for id in 0..8 {
        env.ensure_arrays(&m, id, -4 * span, 4 * span - 1);
    }
    (env, m)
}

#[test]
fn planned_output_is_exact_on_chain() {
    for (amount, a_to_b) in [
        (1_000_000u64, true),
        (40_000_000_000, true),
        (40_000_000_000, false),
    ] {
        let (mut env, m) = market();
        let (mut bs, cfg, slot) = books(&env, &m);
        let p = plan(&bs, &cfg, slot, a_to_b, amount, RouterConfig::default()).unwrap();
        if amount == 1_000_000 {
            assert_eq!(p.legs.len(), 1, "small order on one shard");
        } else {
            assert!(p.legs.len() > 1, "large order split: {:?}", p.legs.len());
        }
        let t = env.user(&m, 1_000_000_000_000);
        let mut ixs = vec![compute_limit(1_400_000)];
        for l in &p.legs {
            let shard = shard_addrs(&m, l.id, 0).shard;
            let arrays: Vec<Pubkey> = l
                .array_starts
                .iter()
                .map(|s| tick_array_pda(&shard, *s))
                .collect();
            ixs.push(env.ix_swap(&m, &t, l.id, env.swap_args(a_to_b, l.amount_in), &arrays));
        }
        let out_acc = if a_to_b { t.token_b } else { t.token_a };
        let before = env.balance(&out_acc);
        let kp = t.kp.insecure_clone();
        env.send(&ixs, &[&kp]).unwrap();
        assert_eq!(env.balance(&out_acc) - before, p.amount_out);
        // The router's books, advanced the same way, match the chain.
        for l in &p.legs {
            let b = bs.iter_mut().find(|b| b.id == l.id).unwrap();
            b.apply(&cfg, slot, a_to_b, l.amount_in, 0).unwrap();
        }
        for b in &bs {
            assert_eq!(
                Pool::load(&env.shard_state(&m, b.id)).unwrap(),
                b.pool,
                "shard {}",
                b.id
            );
        }
    }
}

#[test]
fn found_rebalance_pays_the_predicted_profit() {
    let (mut env, m) = market();
    let t = env.user(&m, 1_000_000_000_000);
    env.swap(&m, &t, 5, false, 30_000_000_000).unwrap();
    let (mut bs, cfg, slot) = books(&env, &m);
    let r = find_rebalance(&bs, &cfg, slot, 0).expect("a gap to close");
    assert_eq!(r.hi, 5, "shard 5 got dearer");
    let caller = env.user(&m, 0);
    let token = if r.input_is_a {
        caller.token_a
    } else {
        caller.token_b
    };
    let (sl, sh) = (
        shard_addrs(&m, r.lo, 0).shard,
        shard_addrs(&m, r.hi, 0).shard,
    );
    let lo_arrays: Vec<Pubkey> = r
        .lo_array_starts
        .iter()
        .map(|s| tick_array_pda(&sl, *s))
        .collect();
    let hi_arrays: Vec<Pubkey> = r
        .hi_array_starts
        .iter()
        .map(|s| tick_array_pda(&sh, *s))
        .collect();
    let (x, y) = (shard_addrs(&m, r.lo, 0), shard_addrs(&m, r.hi, 0));
    let i = ix(
        sharded_amm::accounts::RebalancePair {
            caller: caller.kp.pubkey(),
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
            caller_token: token,
            token_program: TOKEN_PROGRAM,
        },
        sharded_amm::instruction::RebalancePair {
            args: RebalanceArgs {
                input_is_a: r.input_is_a,
                amount_in: r.amount_in,
                min_profit: r.profit,
                arrays_x: lo_arrays.len() as u8,
                deadline: i64::MAX,
            },
        },
    );
    let i = with_remaining(i, &[lo_arrays, hi_arrays].concat());
    let kp = caller.kp.insecure_clone();
    env.send(&[i], &[&kp]).unwrap();
    assert_eq!(env.balance(&token), r.profit);
    let got = apply_rebalance(&mut bs, &cfg, slot, &r).unwrap();
    assert_eq!(got, r.profit);
    for id in [r.lo, r.hi] {
        let b = bs.iter().find(|b| b.id == id).unwrap();
        assert_eq!(Pool::load(&env.shard_state(&m, id)).unwrap(), b.pool);
    }
}
