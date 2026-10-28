//! Anchor toolchain/IDL probe only; no AMM, accounts, custody or token operations.
use anchor_lang::prelude::*;

// Public fixture identity only. No deployment keypair is shipped or required to build.
declare_id!("Fg6PaFpoGXkYsidMpWxTWqkZVZEUWt8DTkZGwg5pYjmS");

#[program]
pub mod anchor_environment_smoke {
    use super::*;

    pub fn ping(_ctx: Context<Ping>) -> Result<()> {
        msg!("anchor-environment-smoke: pong");
        Ok(())
    }
}

#[derive(Accounts)]
pub struct Ping {}
