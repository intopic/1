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
    /// `Open -> Paused` by the pauser or admin; `Paused -> Open` by the admin
    /// only, so a compromised pauser can halt but not resume. Pausing while
    /// `Initializing` is not a v1 transition.
    pub fn set_paused(
        &mut self,
        signer: &Pubkey,
        paused: bool,
    ) -> std::result::Result<(), AmmError> {
        match (self.status, paused) {
            (MarketStatus::Open, true) => {
                if *signer != self.pauser && *signer != self.admin {
                    return Err(AmmError::Unauthorized);
                }
                self.status = MarketStatus::Paused;
            }
            (MarketStatus::Paused, false) => {
                if *signer != self.admin {
                    return Err(AmmError::Unauthorized);
                }
                self.status = MarketStatus::Open;
            }
            _ => return Err(AmmError::InvalidMarketStatus),
        }
        Ok(())
    }

    /// Admin-only fee update within bounds. `tick_spacing` is immutable and
    /// not an input. Returns the previous params for the event.
    pub fn update_fees(
        &mut self,
        signer: &Pubkey,
        fee_rate_ppm: u32,
        protocol_share_bps: u16,
    ) -> std::result::Result<MarketParams, AmmError> {
        if *signer != self.admin {
            return Err(AmmError::Unauthorized);
        }
        let new = MarketParams {
            fee_rate_ppm,
            protocol_share_bps,
            ..self.params
        };
        new.validate()?;
        let old = self.params;
        self.params = new;
        Ok(old)
    }

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

    fn config(status: MarketStatus) -> MarketConfig {
        MarketConfig {
            schema_version: 1,
            market_id: 1,
            mint_a: Pubkey::new_unique(),
            mint_b: Pubkey::new_unique(),
            token_program: Pubkey::new_unique(),
            admin: Pubkey::new_from_array([1; 32]),
            keeper: Pubkey::new_from_array([2; 32]),
            revenue_authority: Pubkey::new_from_array([3; 32]),
            pauser: Pubkey::new_from_array([4; 32]),
            params: MarketParams {
                fee_rate_ppm: 3_000,
                protocol_share_bps: 2_000,
                tick_spacing: 60,
            },
            status,
            bump: 255,
            reserved: [0; 64],
        }
    }

    #[test]
    fn pause_roles_and_transitions() {
        let mut c = config(MarketStatus::Open);
        let (admin, keeper, revenue, pauser) = (c.admin, c.keeper, c.revenue_authority, c.pauser);
        for who in [keeper, revenue] {
            assert_eq!(
                c.clone().set_paused(&who, true),
                Err(AmmError::Unauthorized)
            );
        }
        assert_eq!(c.set_paused(&pauser, true), Ok(()));
        assert_eq!(c.status, MarketStatus::Paused);
        assert_eq!(
            c.set_paused(&pauser, true),
            Err(AmmError::InvalidMarketStatus)
        );
        assert_eq!(c.set_paused(&pauser, false), Err(AmmError::Unauthorized));
        assert_eq!(c.set_paused(&admin, false), Ok(()));
        assert_eq!(c.status, MarketStatus::Open);
        assert_eq!(c.set_paused(&admin, true), Ok(()));
        let mut c = config(MarketStatus::Initializing);
        assert_eq!(
            c.set_paused(&pauser, true),
            Err(AmmError::InvalidMarketStatus)
        );
        assert_eq!(
            c.set_paused(&admin, false),
            Err(AmmError::InvalidMarketStatus)
        );
    }

    #[test]
    fn fee_update_admin_only_and_bounded() {
        let mut c = config(MarketStatus::Open);
        let admin = c.admin;
        for who in [c.keeper, c.revenue_authority, c.pauser] {
            assert_eq!(c.update_fees(&who, 500, 2_000), Err(AmmError::Unauthorized));
        }
        assert_eq!(
            c.update_fees(&admin, 100_001, 2_000),
            Err(AmmError::InvalidFeeRate)
        );
        assert_eq!(
            c.update_fees(&admin, 500, 5_001),
            Err(AmmError::InvalidProtocolShare)
        );
        assert_eq!(c.params.fee_rate_ppm, 3_000);
        let old = c.update_fees(&admin, 500, 1_000).unwrap();
        assert_eq!(old.fee_rate_ppm, 3_000);
        assert_eq!(
            c.params,
            MarketParams {
                fee_rate_ppm: 500,
                protocol_share_bps: 1_000,
                tick_spacing: 60
            }
        );
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
