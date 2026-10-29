//! Collecting fees often pays exactly what one collect at the end pays
//! (idea from Orca's careful fee checkpoint): the position checkpoint
//! advances only by the growth the paid fee used up, so the rounding
//! remainder carries over instead of being lost at every collect.
//!
//! Two LPs hold the same liquidity on the same range of the same shard. One
//! collects after every swap, the other once at the end; they receive the
//! same amounts of both tokens, to the unit.

mod common;

use common::*;

#[test]
fn frequent_collect_pays_what_one_collect_pays() {
    let mut env = Env::new();
    let (m, _) = env.open_seeded_market();
    let often = env.user(&m, 1_000_000_000_000);
    let once = env.user(&m, 1_000_000_000_000);
    // Small next to the seed liquidity, so each swap earns them a fraction
    // of a unit: without the carried remainder, `often` would get nothing.
    let l = 300_000_007u128;
    for p in [&often, &once] {
        env.add_liquidity(&m, p, 0, -600, 600, l).unwrap();
    }
    let balances = |env: &Env, p: &Lp| (env.balance(&p.token_a), env.balance(&p.token_b));
    let (a0, b0) = balances(&env, &often);
    let (c0, d0) = balances(&env, &once);
    let t = env.user(&m, 1_000_000_000_000);
    let kp = often.kp.insecure_clone();
    for i in 0..200 {
        env.swap(&m, &t, 0, i % 2 == 0, 1_000_000 + i).unwrap();
        let c = env.ix_collect(&m, &often, 0, -600, 600);
        env.send(&[c], &[&kp]).unwrap();
    }
    let c = env.ix_collect(&m, &once, 0, -600, 600);
    let kp2 = once.kp.insecure_clone();
    env.send(&[c], &[&kp2]).unwrap();
    let (a1, b1) = balances(&env, &often);
    let (c1, d1) = balances(&env, &once);
    let (often_a, often_b, once_a, once_b) = (a1 - a0, b1 - b0, c1 - c0, d1 - d0);
    println!("often: {often_a} A {often_b} B; once: {once_a} A {once_b} B");
    assert!(once_a > 0 && once_b > 0, "the range earned fees");
    assert_eq!((often_a, often_b), (once_a, once_b));
}
