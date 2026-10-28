//! Environment probe only. No AMM, custody, PDA, or token operations.
#![allow(unexpected_cfgs)] // Solana entrypoint macro uses target-specific cfg values.

use solana_program::{
    account_info::AccountInfo, entrypoint, entrypoint::ProgramResult, msg,
    program_error::ProgramError, pubkey::Pubkey,
};

entrypoint!(process_instruction);

pub fn process_instruction(
    _program_id: &Pubkey,
    _accounts: &[AccountInfo],
    data: &[u8],
) -> ProgramResult {
    if data != b"ping" {
        return Err(ProgramError::InvalidInstructionData);
    }
    msg!("environment-smoke: pong");
    Ok(())
}
