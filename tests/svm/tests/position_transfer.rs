//! D-16 (mainnet analysis): a position moves to a new owner in two steps,
//! and only its `owner` field changes. The address, liquidity, fee
//! checkpoints and uncollected fees stay; from then on only the new owner
//! can withdraw, collect or close it.

mod common;

use common::*;
use sharded_amm::errors::AmmError;
use sharded_amm::instruction as ixs;
use solana_instruction::Instruction;
use solana_signer::Signer;

const RANGE: (i32, i32) = (-600, 600);

fn ix_propose(position: Pubkey, owner: &Pubkey, new_owner: Pubkey) -> Instruction {
    ix(
        sharded_amm::accounts::ProposePositionTransfer {
            owner: *owner,
            position,
        },
        ixs::ProposePositionTransfer { new_owner },
    )
}

fn ix_accept(position: Pubkey, new_owner: &Pubkey) -> Instruction {
    ix(
        sharded_amm::accounts::AcceptPositionTransfer {
            new_owner: *new_owner,
            position,
        },
        ixs::AcceptPositionTransfer {},
    )
}

/// `i` (built for `from`'s position) signed and paid by `to` instead.
fn as_owner(mut i: Instruction, to: &Lp) -> Instruction {
    i.accounts[0].pubkey = to.kp.pubkey();
    i.accounts[6].pubkey = to.token_a;
    i.accounts[7].pubkey = to.token_b;
    i
}

#[test]
fn position_moves_in_two_steps_with_its_liquidity_and_fees() {
    let mut env = Env::new();
    let (m, _) = env.open_seeded_market();
    let (lo, hi) = RANGE;
    let alice = env.user(&m, 1_000_000_000_000);
    let bob = env.user(&m, 0);
    let mallory = env.user(&m, 0);
    env.add_liquidity(&m, &alice, 0, lo, hi, 1_000_000_000)
        .unwrap();
    let shard = shard_addrs(&m, 0, 0).shard;
    let pos = position_pda(&shard, &alice.kp.pubkey(), lo, hi);
    // Fees accrue to the position before the transfer.
    let t = env.user(&m, 1_000_000_000_000);
    env.swap(&m, &t, 0, true, 50_000_000).unwrap();
    env.swap(&m, &t, 0, false, 50_000_000).unwrap();

    // Only the owner proposes, and not to itself.
    expect_custom(
        env.send(
            &[ix_propose(pos, &mallory.kp.pubkey(), mallory.kp.pubkey())],
            &[&mallory.kp],
        ),
        anchor_codes::CONSTRAINT_HAS_ONE,
    );
    expect_custom(
        env.send(
            &[ix_propose(pos, &alice.kp.pubkey(), alice.kp.pubkey())],
            &[&alice.kp],
        ),
        amm_code(AmmError::Unauthorized),
    );
    expect_custom(
        env.send(&[ix_accept(pos, &bob.kp.pubkey())], &[&bob.kp]),
        amm_code(AmmError::NoPendingChange),
    );
    env.send(
        &[ix_propose(pos, &alice.kp.pubkey(), bob.kp.pubkey())],
        &[&alice.kp],
    )
    .unwrap();
    let before = env.position(&pos);
    assert_eq!(before.pending_owner, bob.kp.pubkey());
    // Proposing changes nothing yet: alice still owns it.
    assert_eq!(before.owner, alice.kp.pubkey());
    // Only the named owner accepts.
    for who in [&mallory, &alice] {
        expect_custom(
            env.send(&[ix_accept(pos, &who.kp.pubkey())], &[&who.kp]),
            amm_code(AmmError::Unauthorized),
        );
    }
    env.send(&[ix_accept(pos, &bob.kp.pubkey())], &[&bob.kp])
        .unwrap();
    let after = env.position(&pos);
    assert_eq!(after.owner, bob.kp.pubkey());
    assert_eq!(after.pending_owner, Pubkey::default());
    // Only the owner field changed.
    assert_eq!(
        sharded_amm::state::Position {
            owner: before.owner,
            pending_owner: before.pending_owner,
            ..after
        },
        before
    );

    // Alice has no power left over it.
    let i = env.ix_decrease(&m, &alice, 0, lo, hi, 1);
    expect_custom(
        env.send(&[i], &[&alice.kp]),
        anchor_codes::CONSTRAINT_HAS_ONE,
    );
    let i = env.ix_collect(&m, &alice, 0, lo, hi);
    expect_custom(
        env.send(&[i], &[&alice.kp]),
        anchor_codes::CONSTRAINT_HAS_ONE,
    );
    // Bob withdraws everything, fees earned before the transfer included.
    let i = as_owner(env.ix_decrease(&m, &alice, 0, lo, hi, 1_000_000_000), &bob);
    let c = as_owner(env.ix_collect(&m, &alice, 0, lo, hi), &bob);
    env.send(&[i, c], &[&bob.kp]).unwrap();
    let (a, b) = (env.balance(&bob.token_a), env.balance(&bob.token_b));
    assert!(a > 0 && b > 0, "principal and fees: {a} {b}");
    let p = env.position(&pos);
    assert_eq!((p.liquidity, p.tokens_owed_a, p.tokens_owed_b), (0, 0, 0));
    // Closing refunds the rent to the new owner.
    let lamports = env.svm.get_balance(&bob.kp.pubkey()).unwrap();
    let close = ix(
        sharded_amm::accounts::ClosePosition {
            owner: bob.kp.pubkey(),
            position: pos,
        },
        ixs::ClosePosition {},
    );
    env.send(&[close], &[&bob.kp]).unwrap();
    assert!(env.svm.get_balance(&bob.kp.pubkey()).unwrap() > lamports - 10_000);
    assert!(env.raw(&pos).is_none_or(|a| a.lamports == 0));
}

#[test]
fn a_proposal_can_be_withdrawn() {
    let mut env = Env::new();
    let (m, _) = env.open_seeded_market();
    let (lo, hi) = RANGE;
    let alice = env.user(&m, 1_000_000_000_000);
    let bob = env.user(&m, 0);
    env.add_liquidity(&m, &alice, 0, lo, hi, 1_000_000).unwrap();
    let pos = position_pda(&shard_addrs(&m, 0, 0).shard, &alice.kp.pubkey(), lo, hi);
    env.send(
        &[ix_propose(pos, &alice.kp.pubkey(), bob.kp.pubkey())],
        &[&alice.kp],
    )
    .unwrap();
    env.send(
        &[ix_propose(pos, &alice.kp.pubkey(), Pubkey::default())],
        &[&alice.kp],
    )
    .unwrap();
    expect_custom(
        env.send(&[ix_accept(pos, &bob.kp.pubkey())], &[&bob.kp]),
        amm_code(AmmError::NoPendingChange),
    );
    assert_eq!(env.position(&pos).owner, alice.kp.pubkey());
    // A pending transfer never blocks the owner's exit.
    env.send(
        &[ix_propose(pos, &alice.kp.pubkey(), bob.kp.pubkey())],
        &[&alice.kp],
    )
    .unwrap();
    let i = env.ix_decrease(&m, &alice, 0, lo, hi, 1_000_000);
    env.send(&[i], &[&alice.kp]).unwrap();
}
