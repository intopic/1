//! Token CPI helpers and the remaining-accounts loader for tick arrays.

use std::cell::RefMut;
use std::collections::BTreeSet;

use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};

use crate::constants::VAULT_AUTH_SEED;
use crate::errors::AmmError;
use crate::state::TickArray;

/// User -> vault, signed by the user.
pub fn transfer_in<'info>(
    token_program: &Program<'info, Token>,
    from: &Account<'info, TokenAccount>,
    to: &Account<'info, TokenAccount>,
    authority: &Signer<'info>,
    amount: u64,
) -> Result<()> {
    if amount == 0 {
        return Ok(());
    }
    let accounts = Transfer {
        from: from.to_account_info(),
        to: to.to_account_info(),
        authority: authority.to_account_info(),
    };
    token::transfer(CpiContext::new(token_program.key(), accounts), amount)
}

/// Vault -> user, signed by the shard's vault authority PDA.
#[allow(clippy::too_many_arguments)]
pub fn transfer_out<'info>(
    token_program: &Program<'info, Token>,
    vault: &Account<'info, TokenAccount>,
    to: &Account<'info, TokenAccount>,
    vault_authority: &UncheckedAccount<'info>,
    shard: &Pubkey,
    bump: u8,
    amount: u64,
) -> Result<()> {
    if amount == 0 {
        return Ok(());
    }
    let seeds: &[&[u8]] = &[VAULT_AUTH_SEED, shard.as_ref(), &[bump]];
    let signer = &[seeds];
    let accounts = Transfer {
        from: vault.to_account_info(),
        to: to.to_account_info(),
        authority: vault_authority.to_account_info(),
    };
    token::transfer(
        CpiContext::new_with_signer(token_program.key(), accounts, signer),
        amount,
    )
}

/// Loads `1..=max` writable, distinct tick arrays of `shard` from remaining
/// accounts, in the given order.
pub fn load_tick_arrays<'info>(
    accounts: &'info [AccountInfo<'info>],
    shard: &Pubkey,
    max: usize,
) -> Result<Vec<AccountLoader<'info, TickArray>>> {
    require!(
        !accounts.is_empty() && accounts.len() <= max,
        AmmError::InvalidAccountCount
    );
    let mut seen = BTreeSet::new();
    let mut out = Vec::with_capacity(accounts.len());
    for info in accounts {
        require!(seen.insert(info.key()), AmmError::DuplicateAccount);
        require!(info.is_writable, AmmError::TickArrayMismatch);
        let loader = AccountLoader::<TickArray>::try_from(info)?;
        require!(loader.load()?.shard == *shard, AmmError::TickArrayMismatch);
        out.push(loader);
    }
    Ok(out)
}

/// Owned copies of read-only tick arrays of `shard`, for quotes and probes
/// that must not write the accounts.
pub fn copy_tick_arrays<'info>(
    accounts: &'info [AccountInfo<'info>],
    shard: &Pubkey,
    max: usize,
) -> Result<Vec<TickArray>> {
    require!(
        !accounts.is_empty() && accounts.len() <= max,
        AmmError::InvalidAccountCount
    );
    let mut seen = BTreeSet::new();
    let mut out = Vec::with_capacity(accounts.len());
    for info in accounts {
        require!(seen.insert(info.key()), AmmError::DuplicateAccount);
        let loader = AccountLoader::<TickArray>::try_from(info)?;
        let a = *loader.load()?;
        require!(a.shard == *shard, AmmError::TickArrayMismatch);
        out.push(a);
    }
    Ok(out)
}

pub fn borrow_all<'a, 'info>(
    loaders: &'a [AccountLoader<'info, TickArray>],
) -> Result<Vec<RefMut<'a, TickArray>>> {
    loaders.iter().map(|l| l.load_mut()).collect()
}
