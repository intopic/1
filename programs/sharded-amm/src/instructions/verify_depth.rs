//! `verify_depth` — proves a prepared shard has usable depth both ways
//! (rule 4) and marks it so; `open_market` and `activate_shard` require the
//! mark. Anyone may call it: it only reads the pool and its tick arrays and
//! sets a flag when the check passes.
//!
//! Accounts: `market` R; `shard` W (`has_one = market`, `Prepared`).
//! Remaining: `arrays_a_to_b` tick arrays of the shard from the current tick
//! downwards, then the arrays upwards (read-only). The probes run on copies.
//! Any later liquidity decrease clears the mark.

use anchor_lang::prelude::*;

use crate::constants::MAX_SWAP_TICK_ARRAYS;
use crate::engine::{probe_depth, DepthProbe, Pool};
use crate::errors::AmmError;
use crate::events::DepthVerified;
use crate::state::{MarketConfig, ShardState, ShardStatus};

use super::token_util::copy_tick_arrays;

#[derive(Accounts)]
pub struct VerifyDepth<'info> {
    pub market: Box<Account<'info, MarketConfig>>,
    #[account(mut, has_one = market)]
    pub shard: Box<Account<'info, ShardState>>,
}

pub fn handle_verify_depth<'info>(
    ctx: Context<'info, VerifyDepth<'info>>,
    arrays_a_to_b: u8,
) -> Result<()> {
    let shard = &ctx.accounts.shard;
    require!(
        shard.status == ShardStatus::Prepared,
        AmmError::ShardAlreadyActive
    );
    require!(shard.sqrt_price_x64 != 0, AmmError::ShardMismatch);
    let split = usize::from(arrays_a_to_b);
    require!(
        split <= ctx.remaining_accounts.len(),
        AmmError::InvalidAccountCount
    );
    let (down, up) = ctx.remaining_accounts.split_at(split);
    let key = shard.key();
    let mut down = copy_tick_arrays(down, &key, MAX_SWAP_TICK_ARRAYS)?;
    let mut up = copy_tick_arrays(up, &key, MAX_SWAP_TICK_ARRAYS)?;
    let m = &ctx.accounts.market;
    let r = m.readiness;
    let pool = Pool::load(shard)?;
    let (tick_after_a, tick_after_b) = probe_depth(
        &pool,
        &mut down,
        &mut up,
        DepthProbe {
            probe_a: r.depth_probe_a,
            probe_b: r.depth_probe_b,
            max_impact_ticks: r.max_depth_impact_bps,
            fee_rate_ppm: m.params.fee_rate_ppm,
            protocol_share_bps: m.params.protocol_share_bps,
            tick_spacing: m.params.tick_spacing,
        },
    )?;
    ctx.accounts.shard.depth_verified = true;
    ctx.accounts.shard.depth_config_version = ctx.accounts.market.config_version;
    emit!(DepthVerified {
        shard: key,
        tick: pool.tick,
        tick_after_a,
        tick_after_b,
        probe_a: r.depth_probe_a,
        probe_b: r.depth_probe_b,
    });
    Ok(())
}
