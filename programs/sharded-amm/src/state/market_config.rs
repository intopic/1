//! `MarketConfig`: one token pair, its parameters and roles.
//!
//! Read-only during swaps and LP operations. Mint order is canonical
//! (`mint_a < mint_b` by bytes), so a pair has exactly one market per
//! `market_id`. `tick_spacing` is immutable once the market exists: changing
//! it would invalidate every position's tick alignment.

use anchor_lang::prelude::*;

use crate::constants::{MAX_FEE_RATE_PPM, MAX_PROTOCOL_SHARE_BPS, MAX_TICK_SPACING};
use crate::errors::AmmError;
use crate::state::MarketStatus;

#[account]
#[derive(InitSpace, Debug, PartialEq, Eq)]
pub struct MarketConfig {
    pub schema_version: u16,
    pub market_id: u64,
    pub mint_a: Pubkey,
    pub mint_b: Pubkey,
    /// Pinned token program (classic SPL Token in v1).
    pub token_program: Pubkey,
    /// Parameters within bounds. Cannot move principal.
    pub admin: Pubkey,
    /// Prepares/activates shards only when on-chain readiness checks pass.
    pub keeper: Pubkey,
    /// Collects accrued protocol fees only.
    pub revenue_authority: Pubkey,
    /// Pauses risky operations; never blocks valid LP exits.
    pub pauser: Pubkey,
    pub params: MarketParams,
    pub status: MarketStatus,
    pub bump: u8,
    /// Zeroed space for future fields without a realloc.
    pub reserved: [u8; 64],
}

#[derive(AnchorSerialize, AnchorDeserialize, InitSpace, Clone, Copy, Debug, PartialEq, Eq)]
pub struct MarketParams {
    /// Swap fee in parts per million of input.
    pub fee_rate_ppm: u32,
    /// Protocol share of the swap fee, basis points; the rest goes to LPs.
    pub protocol_share_bps: u16,
    pub tick_spacing: u16,
}

impl MarketParams {
    pub fn validate(&self) -> std::result::Result<(), AmmError> {
        if self.fee_rate_ppm > MAX_FEE_RATE_PPM {
            return Err(AmmError::InvalidFeeRate);
        }
        if self.protocol_share_bps > MAX_PROTOCOL_SHARE_BPS {
            return Err(AmmError::InvalidProtocolShare);
        }
        if self.tick_spacing == 0 || self.tick_spacing > MAX_TICK_SPACING {
            return Err(AmmError::InvalidTickSpacing);
        }
        Ok(())
    }
}

/// Canonical order: distinct mints with `mint_a < mint_b` by bytes.
pub fn check_mint_order(mint_a: &Pubkey, mint_b: &Pubkey) -> std::result::Result<(), AmmError> {
    if mint_a.to_bytes() >= mint_b.to_bytes() {
        return Err(AmmError::MintsNotCanonical);
    }
    Ok(())
}

impl MarketConfig {
    /// Signer may prepare shards: the keeper, or the admin during setup.
    pub fn can_prepare(&self, signer: &Pubkey) -> bool {
        *signer == self.keeper
            || (self.status == MarketStatus::Initializing && *signer == self.admin)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn param_bounds() {
        let ok = MarketParams {
            fee_rate_ppm: 3_000,
            protocol_share_bps: 2_000,
            tick_spacing: 60,
        };
        assert_eq!(ok.validate(), Ok(()));
        for fee in [500, 1_000, 3_000, 0, MAX_FEE_RATE_PPM] {
            assert_eq!(
                MarketParams {
                    fee_rate_ppm: fee,
                    ..ok
                }
                .validate(),
                Ok(())
            );
        }
        assert_eq!(
            MarketParams {
                fee_rate_ppm: MAX_FEE_RATE_PPM + 1,
                ..ok
            }
            .validate(),
            Err(AmmError::InvalidFeeRate)
        );
        assert_eq!(
            MarketParams {
                protocol_share_bps: 5_001,
                ..ok
            }
            .validate(),
            Err(AmmError::InvalidProtocolShare)
        );
        for s in [0u16, 1_001, u16::MAX] {
            assert_eq!(
                MarketParams {
                    tick_spacing: s,
                    ..ok
                }
                .validate(),
                Err(AmmError::InvalidTickSpacing)
            );
        }
        for s in [1u16, 10, 60, 1_000] {
            assert_eq!(
                MarketParams {
                    tick_spacing: s,
                    ..ok
                }
                .validate(),
                Ok(())
            );
        }
    }

    #[test]
    fn mint_order() {
        let a = Pubkey::new_from_array([1; 32]);
        let b = Pubkey::new_from_array([2; 32]);
        assert_eq!(check_mint_order(&a, &b), Ok(()));
        assert_eq!(check_mint_order(&b, &a), Err(AmmError::MintsNotCanonical));
        assert_eq!(check_mint_order(&a, &a), Err(AmmError::MintsNotCanonical));
    }
}
