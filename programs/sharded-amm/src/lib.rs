//! Sharded-tick CLMM for Solana: one market, 8 initial shards, capacity 56.
//!
//! Status: ENG-02. Constants, shard bitmaps and the registry lifecycle are
//! implemented as pure, host-tested logic. No instruction handlers, custody or
//! swap math exist yet; see STATUS.md. Not audited.
#![allow(unexpected_cfgs)]

use anchor_lang::prelude::*;

pub mod constants;
pub mod errors;
pub mod math;
pub mod state;

// Placeholder program ID for local builds. The deploy keypair is not in the
// repository; a devnet/mainnet ID is assigned at release (ENG-10).
declare_id!("B6rXDCwiXe1CyXyCS4JS1Tn9J5yud87cREWapUoD5hEy");

#[program]
pub mod sharded_amm {}
