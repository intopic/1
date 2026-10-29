//! Protocol roles and rules for new markets (`ProtocolConfig`, D-04/D-05/D-07).
//!
//! - `initialize_protocol` — once, signed by the program's upgrade authority
//!   (checked against the program's ProgramData account), so nobody can
//!   front-run the deploy and take the roles.
//! - `propose_roles` / `apply_roles` / `cancel_roles_change` — admin,
//!   timelocked. A new admin is only pending after `apply_roles`.
//! - `accept_admin` — the pending admin, with its own signature.
//! - `remove_pauser` — admin, immediately (emergency).
//! - `propose_settings` / `apply_settings` / `cancel_settings_change` —
//!   admin, timelocked: allowed profiles, creation fee, freeze allowlist.
//! - `sync_market_roles` — anyone: copies the protocol's current roles into
//!   one market. It cannot choose any value.
//!
//! Accounts: `authority` signer (except `sync_market_roles`); `protocol` W
//! (R for the sync); `market` W for the sync. Moves no funds.

use anchor_lang::prelude::*;

use crate::constants::{PROTOCOL_SEED, SCHEMA_VERSION};
use crate::errors::AmmError;
use crate::events::{
    AdminAccepted, MarketRolesSynced, PauserRemoved, ProtocolInitialized, RolesChangeProposed,
    RolesUpdated, SettingsChangeProposed, SettingsUpdated,
};
use crate::state::{
    MarketConfig, PendingRoles, PendingSettings, ProtocolConfig, ProtocolSettings, Roles,
};

#[derive(Accounts)]
pub struct InitializeProtocol<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,
    #[account(
        init,
        payer = authority,
        space = 8 + ProtocolConfig::INIT_SPACE,
        seeds = [PROTOCOL_SEED],
        bump,
    )]
    pub protocol: Box<Account<'info, ProtocolConfig>>,
    #[account(
        constraint = program.programdata_address()? == Some(program_data.key())
            @ AmmError::NotUpgradeAuthority,
    )]
    pub program: Program<'info, crate::program::ShardedAmm>,
    #[account(
        constraint = program_data.upgrade_authority_address == Some(authority.key())
            @ AmmError::NotUpgradeAuthority,
    )]
    pub program_data: Account<'info, ProgramData>,
    pub system_program: Program<'info, System>,
}

pub fn handle_initialize_protocol(
    ctx: Context<InitializeProtocol>,
    roles: Roles,
    settings: ProtocolSettings,
) -> Result<()> {
    roles.validate()?;
    settings.validate()?;
    ctx.accounts.protocol.set_inner(ProtocolConfig {
        schema_version: SCHEMA_VERSION,
        bump: ctx.bumps.protocol,
        roles,
        pending_admin: Pubkey::default(),
        roles_version: 0,
        pending_roles: PendingRoles::default(),
        settings,
        pending_settings: PendingSettings::default(),
        reserved: [0; 64],
    });
    emit!(ProtocolInitialized {
        roles,
        settings,
        by: ctx.accounts.authority.key(),
    });
    Ok(())
}

#[derive(Accounts)]
pub struct UpdateProtocol<'info> {
    pub authority: Signer<'info>,
    #[account(mut, seeds = [PROTOCOL_SEED], bump = protocol.bump)]
    pub protocol: Box<Account<'info, ProtocolConfig>>,
}

pub fn handle_propose_roles(ctx: Context<UpdateProtocol>, roles: Roles) -> Result<()> {
    let by = ctx.accounts.authority.key();
    let now = Clock::get()?.unix_timestamp;
    let p = ctx.accounts.protocol.propose_roles(&by, roles, now)?;
    emit!(RolesChangeProposed {
        roles,
        eta: p.eta,
        by
    });
    Ok(())
}

pub fn handle_apply_roles(ctx: Context<UpdateProtocol>) -> Result<()> {
    let by = ctx.accounts.authority.key();
    let now = Clock::get()?.unix_timestamp;
    let old = ctx.accounts.protocol.apply_roles(&by, now)?;
    let p = &ctx.accounts.protocol;
    emit!(RolesUpdated {
        old,
        new: p.roles,
        pending_admin: p.pending_admin,
        roles_version: p.roles_version,
        by
    });
    Ok(())
}

pub fn handle_cancel_roles_change(ctx: Context<UpdateProtocol>) -> Result<()> {
    let by = ctx.accounts.authority.key();
    ctx.accounts.protocol.cancel_roles_change(&by)?;
    Ok(())
}

pub fn handle_accept_admin(ctx: Context<UpdateProtocol>) -> Result<()> {
    let by = ctx.accounts.authority.key();
    let old = ctx.accounts.protocol.accept_admin(&by)?;
    emit!(AdminAccepted {
        old,
        new: by,
        roles_version: ctx.accounts.protocol.roles_version,
    });
    Ok(())
}

pub fn handle_remove_pauser(ctx: Context<UpdateProtocol>) -> Result<()> {
    let by = ctx.accounts.authority.key();
    let removed = ctx.accounts.protocol.remove_pauser(&by)?;
    emit!(PauserRemoved {
        removed,
        roles_version: ctx.accounts.protocol.roles_version,
        by
    });
    Ok(())
}

pub fn handle_propose_settings(
    ctx: Context<UpdateProtocol>,
    settings: ProtocolSettings,
) -> Result<()> {
    let by = ctx.accounts.authority.key();
    let now = Clock::get()?.unix_timestamp;
    let p = ctx.accounts.protocol.propose_settings(&by, settings, now)?;
    emit!(SettingsChangeProposed {
        settings,
        eta: p.eta,
        by
    });
    Ok(())
}

pub fn handle_apply_settings(ctx: Context<UpdateProtocol>) -> Result<()> {
    let by = ctx.accounts.authority.key();
    let now = Clock::get()?.unix_timestamp;
    let old = ctx.accounts.protocol.apply_settings(&by, now)?;
    emit!(SettingsUpdated {
        old,
        new: ctx.accounts.protocol.settings,
        by
    });
    Ok(())
}

pub fn handle_cancel_settings_change(ctx: Context<UpdateProtocol>) -> Result<()> {
    let by = ctx.accounts.authority.key();
    ctx.accounts.protocol.cancel_settings_change(&by)?;
    Ok(())
}

#[derive(Accounts)]
pub struct SyncMarketRoles<'info> {
    #[account(seeds = [PROTOCOL_SEED], bump = protocol.bump)]
    pub protocol: Box<Account<'info, ProtocolConfig>>,
    #[account(mut)]
    pub market: Box<Account<'info, MarketConfig>>,
}

pub fn handle_sync_market_roles(ctx: Context<SyncMarketRoles>) -> Result<()> {
    let p = &ctx.accounts.protocol;
    let (roles, version) = (p.roles, p.roles_version);
    require!(
        ctx.accounts.market.roles_version != version,
        AmmError::NoPendingChange
    );
    let old_keeper = ctx.accounts.market.sync_roles(&roles, version);
    emit!(MarketRolesSynced {
        market: ctx.accounts.market.key(),
        roles,
        old_keeper,
        roles_version: version,
    });
    Ok(())
}
