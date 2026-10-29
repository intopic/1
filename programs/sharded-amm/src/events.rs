use anchor_lang::prelude::*;

use crate::state::{DynamicFeeParams, MarketParams};

#[event]
pub struct MarketInitialized {
    pub market: Pubkey,
    pub mint_a: Pubkey,
    pub mint_b: Pubkey,
    pub market_id: u64,
    pub params: MarketParams,
}

#[event]
pub struct ShardPrepared {
    pub market: Pubkey,
    pub shard: Pubkey,
    pub id: u8,
    pub generation: u64,
    pub registry_version: u64,
}

#[event]
pub struct MarketPauseChanged {
    pub market: Pubkey,
    pub paused: bool,
    pub by: Pubkey,
}

#[event]
pub struct FeesUpdated {
    pub market: Pubkey,
    pub old: MarketParams,
    pub new: MarketParams,
    pub old_dynamic: DynamicFeeParams,
    pub new_dynamic: DynamicFeeParams,
    pub by: Pubkey,
}

#[event]
pub struct FeeChangeProposed {
    pub market: Pubkey,
    pub fee_rate_ppm: u32,
    pub protocol_share_bps: u16,
    pub dynamic: DynamicFeeParams,
    pub eta: i64,
    pub by: Pubkey,
}

#[event]
pub struct PriceInitialized {
    pub shard: Pubkey,
    pub sqrt_price_x64: u128,
    pub tick: i32,
}

#[event]
pub struct LiquidityChanged {
    pub shard: Pubkey,
    pub position: Pubkey,
    pub owner: Pubkey,
    /// Positive when added, negative when removed.
    pub liquidity_delta: i128,
    pub amount_a: u64,
    pub amount_b: u64,
}

#[event]
pub struct FeesCollected {
    pub shard: Pubkey,
    pub position: Pubkey,
    pub owner: Pubkey,
    pub amount_a: u64,
    pub amount_b: u64,
}

#[event]
pub struct MarketOpened {
    pub market: Pubkey,
    pub active_mask: u64,
    pub registry_version: u64,
}

#[event]
pub struct MarketCancelled {
    pub market: Pubkey,
    pub by: Pubkey,
}

#[event]
pub struct ShardActivated {
    pub market: Pubkey,
    pub shard: Pubkey,
    pub id: u8,
    pub active_mask: u64,
    pub registry_version: u64,
}

#[event]
pub struct SwapExecuted {
    pub shard: Pubkey,
    pub user: Pubkey,
    pub a_to_b: bool,
    pub amount_in: u64,
    pub amount_out: u64,
    /// Nominal fee plus rounding dust; split below.
    pub total_fee: u64,
    pub protocol_fee: u64,
    pub rounding_dust: u64,
    pub sqrt_price_x64: u128,
    pub tick: i32,
    pub ticks_crossed: u32,
    /// Fee rate this swap paid: the base fee or the shard's dynamic fee.
    pub fee_rate_ppm: u32,
    /// Active liquidity after the swap.
    pub liquidity: u128,
}

#[event]
pub struct ProtocolFeesCollected {
    pub shard: Pubkey,
    pub amount_a: u64,
    pub amount_b: u64,
    pub by: Pubkey,
}
