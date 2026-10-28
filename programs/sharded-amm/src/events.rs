use anchor_lang::prelude::*;

use crate::state::MarketParams;

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
