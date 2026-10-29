//! `rebalance_pair` — capital-free rebalancing between two shards, by the
//! market's listed rebalancers only (ADR-0006).
//!
//! Accounts (R/W/signer):
//! - `caller` signer, one of `market.rebalancers`; `market` R; `registry` R
//! - for shard x and shard y: `shard_*` W, `vault_*_a`, `vault_*_b` W,
//!   `vault_authority_*` R (PDA signer)
//! - `caller_token` W — receives the profit, mint of the input token
//! - `token_program`
//! - remaining: `arrays_x` tick arrays of shard x, then those of shard y,
//!   each in that shard's leg direction (A is bought on the cheaper shard
//!   and sold on the dearer one)
//!
//! The program orders x and y by price itself. Funds move only between the
//! two shards' vaults, plus the profit to the caller; each shard ends as if
//! it had executed one ordinary swap at its own fee.

use anchor_lang::prelude::*;
use anchor_spl::token::{Token, TokenAccount};

use crate::constants::{MAX_SWAP_TICK_ARRAYS, REGISTRY_SEED, VAULT_AUTH_SEED};
use crate::engine::{dynamic_fee, rebalance, Pool, RebalanceParams};
use crate::errors::AmmError;
use crate::events::RebalanceExecuted;
use crate::state::{MarketConfig, ShardRegistry, ShardState, ShardStatus, TickArray};

use super::token_util::{borrow_all, load_tick_arrays, transfer_out};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug)]
pub struct RebalanceArgs {
    /// `true`: sell `amount_in` A on the dearer shard first, profit in A.
    /// `false`: buy A with `amount_in` B on the cheaper shard first, profit in B.
    pub input_is_a: bool,
    pub amount_in: u64,
    pub min_profit: u64,
    /// Tick arrays of shard x at the start of the remaining accounts.
    pub arrays_x: u8,
    /// Unix seconds; fails after this time.
    pub deadline: i64,
}

#[derive(Accounts)]
pub struct RebalancePair<'info> {
    pub caller: Signer<'info>,
    #[account(has_one = token_program)]
    pub market: Box<Account<'info, MarketConfig>>,
    #[account(seeds = [REGISTRY_SEED, market.key().as_ref()], bump = registry.bump, has_one = market)]
    pub registry: Box<Account<'info, ShardRegistry>>,
    #[account(mut, has_one = market, constraint = shard_x.vault_a == vault_x_a.key() @ AmmError::ShardMismatch, constraint = shard_x.vault_b == vault_x_b.key() @ AmmError::ShardMismatch)]
    pub shard_x: Box<Account<'info, ShardState>>,
    #[account(mut)]
    pub vault_x_a: Box<Account<'info, TokenAccount>>,
    #[account(mut)]
    pub vault_x_b: Box<Account<'info, TokenAccount>>,
    /// CHECK: PDA signer for shard x's vaults; address fixed by seeds.
    #[account(seeds = [VAULT_AUTH_SEED, shard_x.key().as_ref()], bump = shard_x.vault_authority_bump)]
    pub vault_authority_x: UncheckedAccount<'info>,
    #[account(mut, has_one = market, constraint = shard_y.vault_a == vault_y_a.key() @ AmmError::ShardMismatch, constraint = shard_y.vault_b == vault_y_b.key() @ AmmError::ShardMismatch)]
    pub shard_y: Box<Account<'info, ShardState>>,
    #[account(mut)]
    pub vault_y_a: Box<Account<'info, TokenAccount>>,
    #[account(mut)]
    pub vault_y_b: Box<Account<'info, TokenAccount>>,
    /// CHECK: PDA signer for shard y's vaults; address fixed by seeds.
    #[account(seeds = [VAULT_AUTH_SEED, shard_y.key().as_ref()], bump = shard_y.vault_authority_bump)]
    pub vault_authority_y: UncheckedAccount<'info>,
    #[account(mut)]
    pub caller_token: Box<Account<'info, TokenAccount>>,
    pub token_program: Program<'info, Token>,
}

pub fn handle_rebalance_pair<'info>(
    ctx: Context<'info, RebalancePair<'info>>,
    args: RebalanceArgs,
) -> Result<()> {
    let clock = Clock::get()?;
    require!(
        clock.unix_timestamp <= args.deadline,
        AmmError::DeadlineExceeded
    );
    let acc = &ctx.accounts;
    require!(
        acc.shard_x.key() != acc.shard_y.key(),
        AmmError::DuplicateAccount
    );
    require!(
        acc.market.is_rebalancer(&acc.caller.key()),
        AmmError::NotRebalancer
    );
    acc.market.check_rebalance()?;
    for s in [&acc.shard_x, &acc.shard_y] {
        acc.registry.check_swappable(acc.market.status, s.id)?;
        require!(s.status == ShardStatus::Active, AmmError::ShardNotActive);
    }
    let mint_in = if args.input_is_a {
        acc.market.mint_a
    } else {
        acc.market.mint_b
    };
    require!(
        acc.caller_token.mint == mint_in,
        AmmError::InvalidTokenAccount
    );

    let (kx, ky) = (acc.shard_x.key(), acc.shard_y.key());
    let params = acc.market.params;
    let dynamic = acc.market.dynamic_fees;
    let split = usize::from(args.arrays_x);
    require!(
        split <= ctx.remaining_accounts.len(),
        AmmError::InvalidAccountCount
    );
    let (rx, ry) = ctx.remaining_accounts.split_at(split);
    let lx = load_tick_arrays(rx, &kx, MAX_SWAP_TICK_ARRAYS)?;
    let ly = load_tick_arrays(ry, &ky, MAX_SWAP_TICK_ARRAYS)?;
    let mut bx = borrow_all(&lx)?;
    let mut by = borrow_all(&ly)?;
    let mut ax: Vec<&mut TickArray> = bx.iter_mut().map(|r| &mut **r).collect();
    let mut ay: Vec<&mut TickArray> = by.iter_mut().map(|r| &mut **r).collect();

    let (mut px, mut py) = (Pool::load(&acc.shard_x)?, Pool::load(&acc.shard_y)?);
    let (mut fx, mut fy) = (acc.shard_x.dynamic_fee, acc.shard_y.dynamic_fee);
    let fee = |st: &mut _, pool: &Pool| {
        dynamic_fee::begin(
            st,
            &dynamic,
            params.fee_rate_ppm,
            params.tick_spacing,
            pool.tick,
            clock.slot,
        )
    };
    let (rate_x, rate_y) = (fee(&mut fx, &px), fee(&mut fy, &py));
    // x is `lo` when A is cheaper there.
    let x_is_lo = px.sqrt_price < py.sqrt_price;
    let out = if x_is_lo {
        rebalance(
            &mut px,
            &mut ax,
            &mut py,
            &mut ay,
            RebalanceParams {
                input_is_a: args.input_is_a,
                amount_in: args.amount_in,
                min_profit: args.min_profit,
                fee_lo_ppm: rate_x,
                fee_hi_ppm: rate_y,
                protocol_share_bps: params.protocol_share_bps,
                tick_spacing: params.tick_spacing,
            },
        )?
    } else {
        rebalance(
            &mut py,
            &mut ay,
            &mut px,
            &mut ax,
            RebalanceParams {
                input_is_a: args.input_is_a,
                amount_in: args.amount_in,
                min_profit: args.min_profit,
                fee_lo_ppm: rate_y,
                fee_hi_ppm: rate_x,
                protocol_share_bps: params.protocol_share_bps,
                tick_spacing: params.tick_spacing,
            },
        )?
    };
    drop((ax, ay));
    drop((bx, by));
    for (st, pool) in [(&mut fx, &px), (&mut fy, &py)] {
        dynamic_fee::finish(st, &dynamic, params.tick_spacing, pool.tick, clock.slot);
    }
    px.store(&mut ctx.accounts.shard_x);
    py.store(&mut ctx.accounts.shard_y);
    ctx.accounts.shard_x.dynamic_fee = fx;
    ctx.accounts.shard_y.dynamic_fee = fy;

    // The first leg runs on `hi` when the input is A, on `lo` otherwise.
    let first_is_x = x_is_lo != args.input_is_a;
    let acc = &ctx.accounts;
    let side = |is_x: bool| {
        if is_x {
            (
                &acc.vault_x_a,
                &acc.vault_x_b,
                &acc.vault_authority_x,
                kx,
                acc.shard_x.vault_authority_bump,
            )
        } else {
            (
                &acc.vault_y_a,
                &acc.vault_y_b,
                &acc.vault_authority_y,
                ky,
                acc.shard_y.vault_authority_bump,
            )
        }
    };
    let (f_a, f_b, f_auth, f_key, f_bump) = side(first_is_x);
    let (s_a, s_b, s_auth, s_key, s_bump) = side(!first_is_x);
    let (f_in, f_mid, s_in, s_mid) = if args.input_is_a {
        (f_a, f_b, s_a, s_b)
    } else {
        (f_b, f_a, s_b, s_a)
    };
    // First shard is paid its input by the second shard, which received
    // that much (and the profit) as its output.
    transfer_out(
        &acc.token_program,
        s_in,
        f_in,
        s_auth,
        &s_key,
        s_bump,
        out.first.amount_in,
    )?;
    // The intermediate token goes from the first shard to the second.
    transfer_out(
        &acc.token_program,
        f_mid,
        s_mid,
        f_auth,
        &f_key,
        f_bump,
        out.first.amount_out,
    )?;
    transfer_out(
        &acc.token_program,
        s_in,
        &acc.caller_token,
        s_auth,
        &s_key,
        s_bump,
        out.profit,
    )?;

    let (lo, hi, plo, phi, flo, fhi) = if x_is_lo {
        (kx, ky, px.sqrt_price, py.sqrt_price, rate_x, rate_y)
    } else {
        (ky, kx, py.sqrt_price, px.sqrt_price, rate_y, rate_x)
    };
    emit!(RebalanceExecuted {
        caller: acc.caller.key(),
        shard_lo: lo,
        shard_hi: hi,
        input_is_a: args.input_is_a,
        amount_in: out.first.amount_in,
        amount_mid: out.first.amount_out,
        amount_out: out.second.amount_out,
        profit: out.profit,
        fee_lo_ppm: flo,
        fee_hi_ppm: fhi,
        sqrt_price_lo_x64: plo,
        sqrt_price_hi_x64: phi,
    });
    Ok(())
}
