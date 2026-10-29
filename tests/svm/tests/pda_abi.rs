//! Audit F-02: one normative PDA ABI, checked end to end.
//!
//! `docs/vectors/pda_fixtures.json` is written by the independent
//! `tools/reference/pda_fixtures.py` from docs/account-layout.md. Here every
//! fixture must match (1) Solana's derivation of the listed seed bytes, (2)
//! the test helpers that build instructions, and (3) the account the deployed
//! program actually creates for the same inputs, including its stored bump.

mod common;

use common::*;
use serde_json::Value;
use solana_account::Account;
use solana_keypair::Keypair;
use solana_signer::Signer;

const FIXTURES: &str = include_str!("../../../docs/vectors/pda_fixtures.json");

fn key(v: &Value) -> Pubkey {
    v.as_str().unwrap().parse().unwrap()
}

fn mint_at(env: &mut Env, key: Pubkey, decimals: u8) {
    let data = mint_data(decimals, &env.admin.pubkey(), None);
    let lamports = env.svm.minimum_balance_for_rent_exemption(data.len());
    env.svm
        .set_account(
            key,
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

#[test]
fn every_pda_matches_the_independent_fixture() {
    let doc: Value = serde_json::from_str(FIXTURES).unwrap();
    assert_eq!(doc["program_id"], sharded_amm::ID.to_string());
    assert_eq!(doc["abi"], sharded_amm::constants::SCHEMA_VERSION);
    let fixtures = doc["fixtures"].as_array().unwrap();
    let first = |kind: &str| fixtures.iter().find(|f| f["kind"] == kind).unwrap();

    // Build the same world on chain: fixed mints and a fixed owner keypair.
    let mut env = Env::new();
    let market_in = &first("market")["inputs"];
    let (mint_a, mint_b) = (key(&market_in["mint_a"]), key(&market_in["mint_b"]));
    mint_at(&mut env, mint_a, 9);
    mint_at(&mut env, mint_b, 6);
    let m = env.market_with(mint_a, mint_b, market_in["market_id"].as_u64().unwrap());
    let keeper = env.keeper.insecure_clone();
    for id in [0, 7] {
        env.prepare(&m, &keeper, id).unwrap();
    }
    env.send(
        &[Env::ix_initialize_price(&m, &keeper.pubkey(), 0, Q64)],
        &[&keeper],
    )
    .unwrap();
    let owner = Keypair::new_from_array([3; 32]);
    env.svm.airdrop(&owner.pubkey(), 10_000_000_000).unwrap();
    let shard0 = shard_addrs(&m, 0, 0);

    let mut checked = 0;
    for f in fixtures {
        let kind = f["kind"].as_str().unwrap();
        let inp = &f["inputs"];
        let expected = key(&f["address"]);
        let bump = f["bump"].as_u64().unwrap() as u8;
        // (1) Solana's derivation of exactly these seed bytes.
        let seeds: Vec<Vec<u8>> = f["seeds_hex"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| {
                let s = s.as_str().unwrap();
                (0..s.len())
                    .step_by(2)
                    .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
                    .collect()
            })
            .collect();
        let refs: Vec<&[u8]> = seeds.iter().map(Vec::as_slice).collect();
        assert_eq!(
            Pubkey::find_program_address(&refs, &sharded_amm::ID),
            (expected, bump),
            "{f}"
        );
        // (2) the helpers, and (3) what the program creates.
        let stored_bump = match kind {
            "market" => {
                assert_eq!(m.market, expected);
                Some(env.market_state(&m).bump)
            }
            "registry" => {
                assert_eq!(m.registry, expected);
                Some(env.registry(&m).bump)
            }
            "shard" => {
                let id = inp["id"].as_u64().unwrap() as u8;
                let generation = inp["generation"].as_u64().unwrap();
                assert_eq!(shard_addrs(&m, id, generation).shard, expected);
                (generation == 0).then(|| env.shard_state(&m, id).bump)
            }
            "vault_auth" => {
                assert_eq!(shard0.vault_authority, expected);
                Some(env.shard_state(&m, 0).vault_authority_bump)
            }
            "vault" => {
                let mint = key(&inp["mint"]);
                let v = if mint == mint_a {
                    shard0.vault_a
                } else {
                    shard0.vault_b
                };
                assert_eq!(v, expected);
                let (mint_of, authority, _) = token_account_fields(&env.raw(&v).unwrap().data);
                assert_eq!((mint_of, authority), (mint, shard0.vault_authority));
                None
            }
            "ticks" => {
                let start = inp["start"].as_i64().unwrap() as i32;
                assert_eq!(tick_array_pda(&shard0.shard, start), expected);
                env.send(
                    &[Env::ix_init_tick_array(
                        &m,
                        &keeper.pubkey(),
                        &shard0.shard,
                        start,
                    )],
                    &[&keeper],
                )
                .unwrap();
                Some(env.tick_array(&expected).bump)
            }
            "position" => {
                assert_eq!(key(&inp["owner"]), owner.pubkey());
                let (lo, hi) = (
                    inp["lower"].as_i64().unwrap() as i32,
                    inp["upper"].as_i64().unwrap() as i32,
                );
                assert_eq!(
                    position_pda(&shard0.shard, &owner.pubkey(), lo, hi),
                    expected
                );
                env.send(
                    &[Env::ix_open_position(
                        &m,
                        &owner.pubkey(),
                        &shard0.shard,
                        lo,
                        hi,
                    )],
                    &[&owner],
                )
                .unwrap();
                let p = env.position(&expected);
                assert_eq!(
                    (p.shard, p.owner, p.tick_lower, p.tick_upper),
                    (shard0.shard, owner.pubkey(), lo, hi)
                );
                Some(p.bump)
            }
            other => panic!("unknown fixture kind {other}"),
        };
        if let Some(b) = stored_bump {
            assert_eq!(b, bump, "{kind}: stored bump");
        }
        checked += 1;
    }
    assert_eq!(checked, fixtures.len());
    assert!(checked >= 14);
}
