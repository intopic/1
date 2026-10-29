//! `Position`: one owner's liquidity in `[tick_lower, tick_upper)` of a shard.
//!
//! PDA `["lp_pos", shard, owner, tick_lower_i32_le, tick_upper_i32_le]`.
//! Only `owner` may change it. Fee checkpoints are Q128 U256 limbs of the
//! modular inside growth (see `math::fee_math`).

use anchor_lang::prelude::*;

#[account]
#[derive(InitSpace, Debug, PartialEq, Eq)]
pub struct Position {
    pub shard: Pubkey,
    pub owner: Pubkey,
    pub tick_lower: i32,
    pub tick_upper: i32,
    pub liquidity: u128,
    pub fee_growth_inside_last_a: [u64; 4],
    pub fee_growth_inside_last_b: [u64; 4],
    pub tokens_owed_a: u64,
    pub tokens_owed_b: u64,
    pub bump: u8,
    /// Fees already moved to `tokens_owed_*` since the checkpoint; the
    /// checkpoint moves only when liquidity changes (`fee_math::fees_since`).
    pub fees_credited_a: u64,
    pub fees_credited_b: u64,
}
