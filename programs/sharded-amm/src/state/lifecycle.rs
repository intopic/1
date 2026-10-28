//! Market and shard status machines.
//!
//! Market: `Initializing -> Open`, `Open <-> Paused`. There is no transition
//! back to `Initializing`. Shard: `Prepared -> Active`. Deactivation, ID reuse
//! and shrinking are out of scope for v1 (see docs/adr/0002).
//!
//! Exit policy: an LP may always withdraw/collect its own claims from a
//! prepared or active shard, in every market status including `Paused`.
//! Pausing blocks new risk (swaps, new liquidity), never exits.

use anchor_lang::prelude::*;

#[derive(AnchorSerialize, AnchorDeserialize, InitSpace, Clone, Copy, Debug, PartialEq, Eq)]
#[borsh(use_discriminant = true)]
#[repr(u8)]
pub enum MarketStatus {
    /// Shards 0..=7 are being prepared and seeded; no swaps.
    Initializing = 0,
    /// Normal operation.
    Open = 1,
    /// Swaps and new liquidity blocked; exits allowed.
    Paused = 2,
}

#[derive(AnchorSerialize, AnchorDeserialize, InitSpace, Clone, Copy, Debug, PartialEq, Eq)]
#[borsh(use_discriminant = true)]
#[repr(u8)]
pub enum ShardStatus {
    /// Accounts and vaults exist; may hold seeded liquidity; not routable.
    /// Zero so that Anchor's `init` can deserialize the zeroed account.
    Prepared = 0,
    /// Routable: its bit is set in `ShardRegistry::active_mask`.
    Active = 1,
}

impl MarketStatus {
    pub fn allows_swap(self) -> bool {
        self == MarketStatus::Open
    }

    /// Seeding before open and adding liquidity after open; never while paused.
    pub fn allows_add_liquidity(self) -> bool {
        matches!(self, MarketStatus::Initializing | MarketStatus::Open)
    }

    pub fn allows_lp_exit(self) -> bool {
        true
    }
}

impl ShardStatus {
    pub fn allows_swap(self) -> bool {
        self == ShardStatus::Active
    }

    pub fn allows_lp_exit(self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exits_never_blocked() {
        for m in [
            MarketStatus::Initializing,
            MarketStatus::Open,
            MarketStatus::Paused,
        ] {
            for s in [ShardStatus::Prepared, ShardStatus::Active] {
                assert!(m.allows_lp_exit() && s.allows_lp_exit());
            }
        }
    }

    #[test]
    fn swaps_only_open_and_active() {
        assert!(MarketStatus::Open.allows_swap() && ShardStatus::Active.allows_swap());
        assert!(!MarketStatus::Initializing.allows_swap());
        assert!(!MarketStatus::Paused.allows_swap());
        assert!(!ShardStatus::Prepared.allows_swap());
        assert!(!MarketStatus::Paused.allows_add_liquidity());
    }
}
