//! Token CPI helpers and the remaining-accounts loader for tick arrays.
//!
//! Every transfer checks the real balances afterwards (idea from Raydium):
//! the source must drop and the destination rise by exactly `amount`, or the
//! instruction fails with `TransferAmountMismatch`. With the classic SPL
//! Token program this always holds; the check keeps the accounting exact if
//! a token that moves a different amount (a transfer fee, a hook) ever gets
//! in, and it refuses a transfer to the source account itself.

use std::cell::RefMut;
use std::collections::BTreeSet;

use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};

use crate::constants::VAULT_AUTH_SEED;
use crate::errors::AmmError;
use crate::state::TickArray;

/// The `amount` field of an SPL token account (bytes 64..72), read live.
fn token_amount(ai: &AccountInfo) -> Result<u64> {
    let data = ai.try_borrow_data()?;
    let bytes: [u8; 8] = data
        .get(64..72)
        .and_then(|b| b.try_into().ok())
        .ok_or(AmmError::InvalidTokenAccount)?;
    Ok(u64::from_le_bytes(bytes))
}

/// Runs `cpi` and requires `from` to drop and `to` to rise by exactly
/// `amount`.
fn checked_transfer(
    from: &AccountInfo,
    to: &AccountInfo,
    amount: u64,
    cpi: impl FnOnce() -> Result<()>,
) -> Result<()> {
    let (from0, to0) = (token_amount(from)?, token_amount(to)?);
    cpi()?;
    let (from1, to1) = (token_amount(from)?, token_amount(to)?);
    require!(
        from0.checked_sub(from1) == Some(amount) && to1.checked_sub(to0) == Some(amount),
        AmmError::TransferAmountMismatch
    );
    Ok(())
}

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
    checked_transfer(
        &from.to_account_info(),
        &to.to_account_info(),
        amount,
        || token::transfer(CpiContext::new(token_program.key(), accounts), amount),
    )
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
    checked_transfer(
        &vault.to_account_info(),
        &to.to_account_info(),
        amount,
        || {
            token::transfer(
                CpiContext::new_with_signer(token_program.key(), accounts, signer),
                amount,
            )
        },
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

#[cfg(test)]
mod tests {
    use super::*;

    fn set(ai: &AccountInfo, amount: u64) {
        ai.try_borrow_mut_data().unwrap()[64..72].copy_from_slice(&amount.to_le_bytes());
    }

    fn code(r: Result<()>) -> Option<u32> {
        match r {
            Ok(()) => None,
            Err(Error::AnchorError(e)) => Some(e.error_code_number),
            Err(e) => panic!("{e:?}"),
        }
    }

    /// Two token accounts holding 1 000 each; `cpi` moves balances by hand.
    fn run(amount: u64, cpi: impl FnOnce(&AccountInfo, &AccountInfo)) -> Option<u32> {
        let (k1, k2, owner) = (
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            Pubkey::new_unique(),
        );
        let (mut l1, mut l2) = (0u64, 0u64);
        let (mut d1, mut d2) = (vec![0u8; 165], vec![0u8; 165]);
        let a = AccountInfo::new(&k1, false, true, &mut l1, &mut d1, &owner, false);
        let b = AccountInfo::new(&k2, false, true, &mut l2, &mut d2, &owner, false);
        set(&a, 1_000);
        set(&b, 1_000);
        code(checked_transfer(&a, &b, amount, || {
            cpi(&a, &b);
            Ok(())
        }))
    }

    #[test]
    fn transfers_must_move_exactly_the_amount() {
        let mismatch = Some(u32::from(AmmError::TransferAmountMismatch));
        // Exact.
        assert_eq!(
            run(100, |a, b| {
                set(a, 900);
                set(b, 1_100)
            }),
            None
        );
        // A 1% transfer fee withheld from the destination.
        assert_eq!(
            run(100, |a, b| {
                set(a, 900);
                set(b, 1_099)
            }),
            mismatch
        );
        // The source charged more than the amount.
        assert_eq!(
            run(100, |a, b| {
                set(a, 899);
                set(b, 1_100)
            }),
            mismatch
        );
        // Nothing moved (a transfer to the source account itself).
        assert_eq!(run(100, |_, _| {}), mismatch);
    }

    #[test]
    fn self_transfer_is_refused() {
        let (k, owner) = (Pubkey::new_unique(), Pubkey::new_unique());
        let mut l = 0u64;
        let mut d = vec![0u8; 165];
        let a = AccountInfo::new(&k, false, true, &mut l, &mut d, &owner, false);
        set(&a, 1_000);
        // SPL Token leaves a self-transfer's balance unchanged.
        let r = checked_transfer(&a, &a.clone(), 100, || Ok(()));
        assert_eq!(code(r), Some(u32::from(AmmError::TransferAmountMismatch)));
    }
}
