//! `ShardRegistry`: prepared/active membership for one market.
//!
//! Read-only during swaps and normal LP operations. Written only by control
//! instructions (prepare, finalize, activate), each of which bumps
//! `registry_version` with an overflow check.
//!
//! Every transition computes the complete new state first and commits it only
//! after all checks pass, so a failed call leaves the registry unchanged even
//! when used outside a reverting transaction (e.g. in host tests or the SDK).
//!
//! Readiness (real vault balances, LP claims, liquidity in range) is evaluated
//! by the validation layer against current accounts in the same transaction
//! and passed in as `ready_mask`/`ready`. The registry never trusts a stored
//! "ready" flag, and `prepared_mask` proves identity, not liquidity.

use anchor_lang::prelude::*;

use crate::constants::{INITIAL_MASK, INITIAL_SHARDS};
use crate::errors::AmmError;
use crate::math::bitmap;
use crate::state::MarketStatus;

#[account]
#[derive(InitSpace, Debug, PartialEq, Eq)]
pub struct ShardRegistry {
    pub market: Pubkey,
    pub prepared_mask: u64,
    pub active_mask: u64,
    pub registry_version: u64,
    pub bump: u8,
    /// Shards whose own `PAUSE_SWAPS` is set, mirrored by
    /// `set_shard_pause_flags` and `reopen_shard` so `reopen_shard` knows
    /// how many shards are trading. Not membership: `registry_version` does
    /// not change with it. Never read by swaps.
    pub swap_paused_mask: u64,
}

impl ShardRegistry {
    /// Checks the stored invariants:
    /// no reserved bits, and `active_mask ⊆ prepared_mask`.
    pub fn validate(&self) -> std::result::Result<(), AmmError> {
        bitmap::validate_mask(self.prepared_mask)?;
        bitmap::validate_mask(self.active_mask)?;
        if self.active_mask & !self.prepared_mask != 0 {
            return Err(AmmError::ActiveNotPrepared);
        }
        Ok(())
    }

    pub fn active_count(&self) -> std::result::Result<u32, AmmError> {
        self.validate()?;
        Ok(self.active_mask.count_ones())
    }

    /// Active shards other than `id` that are not swap-paused.
    pub fn open_others(&self, id: u8) -> u64 {
        self.active_mask & !self.swap_paused_mask & !(1u64 << id)
    }

    pub fn is_active(&self, id: u8) -> bool {
        bitmap::is_set(self.active_mask, id)
    }

    /// Swap-time check (read-only): market open and shard bit active.
    pub fn check_swappable(
        &self,
        market: MarketStatus,
        id: u8,
    ) -> std::result::Result<(), AmmError> {
        self.validate()?;
        bitmap::shard_bit(id)?;
        if !market.allows_swap() {
            return Err(AmmError::InvalidMarketStatus);
        }
        if !self.is_active(id) {
            return Err(AmmError::ShardNotActive);
        }
        Ok(())
    }

    fn next_version(&self) -> std::result::Result<u64, AmmError> {
        self.registry_version
            .checked_add(1)
            .ok_or(AmmError::RegistryVersionOverflow)
    }

    /// Records that shard `id`'s accounts and vaults were created and verified.
    ///
    /// While `Initializing` only IDs `0..INITIAL_SHARDS` may be prepared; once
    /// `Open`, any unused ID below 56. Not allowed while `Paused`.
    pub fn record_prepared(
        &mut self,
        market: MarketStatus,
        id: u8,
    ) -> std::result::Result<(), AmmError> {
        self.validate()?;
        bitmap::shard_bit(id)?;
        match market {
            MarketStatus::Initializing if id >= INITIAL_SHARDS => {
                return Err(AmmError::NotAnInitialShard)
            }
            MarketStatus::Initializing | MarketStatus::Open => {}
            MarketStatus::Paused | MarketStatus::Cancelled => {
                return Err(AmmError::InvalidMarketStatus)
            }
        }
        let prepared = bitmap::insert(self.prepared_mask, id, AmmError::ShardAlreadyPrepared)?;
        let version = self.next_version()?;
        self.prepared_mask = prepared;
        self.registry_version = version;
        Ok(())
    }

    /// Opens the market: sets `active_mask = 0xff` and `market = Open`
    /// atomically. `ready_mask` must have exactly bits 0..=7, each derived
    /// from re-validating that shard's current accounts in this transaction.
    pub fn finalize_initial(
        &mut self,
        market: &mut MarketStatus,
        ready_mask: u64,
    ) -> std::result::Result<(), AmmError> {
        self.validate()?;
        bitmap::validate_mask(ready_mask)?;
        if *market != MarketStatus::Initializing || self.active_mask != 0 {
            return Err(AmmError::InvalidMarketStatus);
        }
        if self.prepared_mask != INITIAL_MASK {
            return Err(AmmError::InitialShardsNotPrepared);
        }
        if ready_mask != INITIAL_MASK {
            return Err(AmmError::InitialShardsNotReady);
        }
        let version = self.next_version()?;
        self.active_mask = INITIAL_MASK;
        self.registry_version = version;
        *market = MarketStatus::Open;
        Ok(())
    }

    /// Expansion: activates one prepared, funded shard after open.
    /// `ready` must come from re-validating current state in this transaction.
    pub fn activate(
        &mut self,
        market: MarketStatus,
        id: u8,
        ready: bool,
    ) -> std::result::Result<(), AmmError> {
        self.validate()?;
        bitmap::shard_bit(id)?;
        if market != MarketStatus::Open {
            return Err(AmmError::InvalidMarketStatus);
        }
        if !bitmap::is_set(self.prepared_mask, id) {
            return Err(AmmError::ShardNotPrepared);
        }
        let active = bitmap::insert(self.active_mask, id, AmmError::ShardAlreadyActive)?;
        if !ready {
            return Err(AmmError::ShardNotReady);
        }
        let version = self.next_version()?;
        self.active_mask = active;
        self.registry_version = version;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::{MAX_SHARDS, VALID_MASK};
    use crate::math::bitmap::tests::Rng;

    fn empty() -> ShardRegistry {
        ShardRegistry {
            market: Pubkey::new_unique(),
            prepared_mask: 0,
            active_mask: 0,
            registry_version: 0,
            bump: 255,
            swap_paused_mask: 0,
        }
    }

    /// Registry with 0..=7 prepared and the market still initializing.
    fn prepared_initial() -> ShardRegistry {
        let mut r = empty();
        for id in 0..INITIAL_SHARDS {
            r.record_prepared(MarketStatus::Initializing, id).unwrap();
        }
        r
    }

    fn opened() -> (ShardRegistry, MarketStatus) {
        let mut r = prepared_initial();
        let mut m = MarketStatus::Initializing;
        r.finalize_initial(&mut m, INITIAL_MASK).unwrap();
        (r, m)
    }

    /// Asserts `f` fails with `err` and leaves registry and market untouched.
    fn assert_rejects(
        r: &ShardRegistry,
        m: MarketStatus,
        err: AmmError,
        f: impl FnOnce(&mut ShardRegistry, &mut MarketStatus) -> std::result::Result<(), AmmError>,
    ) {
        let mut r2 = r.clone();
        let mut m2 = m;
        assert_eq!(f(&mut r2, &mut m2), Err(err));
        assert_eq!(&r2, r, "registry mutated on failure");
        assert_eq!(m2, m, "market mutated on failure");
    }

    #[test]
    fn launch_happy_path() {
        let r = prepared_initial();
        assert_eq!(r.prepared_mask, 0xff);
        assert_eq!(r.active_mask, 0);
        assert_eq!(r.registry_version, 8);
        let (r, m) = opened();
        assert_eq!(m, MarketStatus::Open);
        assert_eq!(r.active_mask, 0xff);
        assert_eq!(r.active_count(), Ok(8));
        assert_eq!(r.registry_version, 9);
        for id in 0..8 {
            assert_eq!(r.check_swappable(m, id), Ok(()));
        }
        assert_eq!(r.check_swappable(m, 8), Err(AmmError::ShardNotActive));
    }

    #[test]
    fn no_swaps_before_open() {
        let r = prepared_initial();
        assert_eq!(
            r.check_swappable(MarketStatus::Initializing, 0),
            Err(AmmError::InvalidMarketStatus)
        );
        let (r, _) = opened();
        assert_eq!(
            r.check_swappable(MarketStatus::Paused, 0),
            Err(AmmError::InvalidMarketStatus)
        );
    }

    #[test]
    fn finalize_rejects_partial_preparation() {
        for missing in 0..INITIAL_SHARDS {
            let mut r = empty();
            for id in (0..INITIAL_SHARDS).filter(|&i| i != missing) {
                r.record_prepared(MarketStatus::Initializing, id).unwrap();
            }
            assert_rejects(
                &r,
                MarketStatus::Initializing,
                AmmError::InitialShardsNotPrepared,
                |r, m| r.finalize_initial(m, INITIAL_MASK),
            );
        }
    }

    #[test]
    fn finalize_rejects_partial_readiness() {
        let r = prepared_initial();
        for bad in [0u64, 0x7f, 0xfe, 0x1ff, 0xef, 1 << 55] {
            assert_rejects(
                &r,
                MarketStatus::Initializing,
                AmmError::InitialShardsNotReady,
                |r, m| r.finalize_initial(m, bad),
            );
        }
        assert_rejects(
            &r,
            MarketStatus::Initializing,
            AmmError::ReservedBitsSet,
            |r, m| r.finalize_initial(m, 0xff | 1 << 63),
        );
    }

    #[test]
    fn finalize_only_once() {
        let (r, m) = opened();
        assert_rejects(&r, m, AmmError::InvalidMarketStatus, |r, m| {
            r.finalize_initial(m, INITIAL_MASK)
        });
        assert_rejects(
            &r,
            MarketStatus::Paused,
            AmmError::InvalidMarketStatus,
            |r, m| r.finalize_initial(m, INITIAL_MASK),
        );
    }

    #[test]
    fn prepare_bounds_and_duplicates() {
        let r = empty();
        for id in [8u8, 55, 56, 63, 255] {
            let err = if id < MAX_SHARDS {
                AmmError::NotAnInitialShard
            } else {
                AmmError::ShardIdOutOfRange
            };
            assert_rejects(&r, MarketStatus::Initializing, err, |r, m| {
                r.record_prepared(*m, id)
            });
        }
        let r = prepared_initial();
        assert_rejects(
            &r,
            MarketStatus::Initializing,
            AmmError::ShardAlreadyPrepared,
            |r, m| r.record_prepared(*m, 3),
        );
        let (r, m) = opened();
        assert_rejects(
            &r,
            MarketStatus::Paused,
            AmmError::InvalidMarketStatus,
            |r, m| r.record_prepared(*m, 8),
        );
        assert_rejects(&r, m, AmmError::ShardAlreadyPrepared, |r, m| {
            r.record_prepared(*m, 0)
        });
        for id in [56u8, 63, 255] {
            assert_rejects(&r, m, AmmError::ShardIdOutOfRange, |r, m| {
                r.record_prepared(*m, id)
            });
        }
    }

    #[test]
    fn activation_rules() {
        let (mut r, m) = opened();
        // Not prepared.
        assert_rejects(&r, m, AmmError::ShardNotPrepared, |r, m| {
            r.activate(*m, 8, true)
        });
        r.record_prepared(m, 8).unwrap();
        // Prepared but readiness re-check fails (e.g. LP withdrew after prepare).
        assert_rejects(&r, m, AmmError::ShardNotReady, |r, m| {
            r.activate(*m, 8, false)
        });
        // Wrong market status.
        assert_rejects(
            &r,
            MarketStatus::Paused,
            AmmError::InvalidMarketStatus,
            |r, m| r.activate(*m, 8, true),
        );
        r.activate(m, 8, true).unwrap();
        assert_eq!(r.active_mask, 0x1ff);
        // Double activation, including an initial shard.
        assert_rejects(&r, m, AmmError::ShardAlreadyActive, |r, m| {
            r.activate(*m, 8, true)
        });
        assert_rejects(&r, m, AmmError::ShardAlreadyActive, |r, m| {
            r.activate(*m, 0, true)
        });
        for id in [56u8, 63, 255] {
            assert_rejects(&r, m, AmmError::ShardIdOutOfRange, |r, m| {
                r.activate(*m, id, true)
            });
        }
    }

    #[test]
    fn activation_before_open_rejected() {
        let r = prepared_initial();
        assert_rejects(
            &r,
            MarketStatus::Initializing,
            AmmError::InvalidMarketStatus,
            |r, m| r.activate(*m, 0, true),
        );
    }

    #[test]
    fn expand_to_16_32_56_and_sparse() {
        let (mut r, m) = opened();
        for (target, mask) in [(16u8, 0xffffu64), (32, 0xffff_ffff), (56, VALID_MASK)] {
            for id in r.active_mask.count_ones() as u8..target {
                r.record_prepared(m, id).unwrap();
                r.activate(m, id, true).unwrap();
            }
            assert_eq!(r.active_mask, mask);
            assert_eq!(r.active_count(), Ok(u32::from(target)));
        }
        assert_rejects(&r, m, AmmError::ShardIdOutOfRange, |r, m| {
            r.record_prepared(*m, 56)
        });

        // Sparse expansion: 0..=7 plus 55 only.
        let (mut r, m) = opened();
        r.record_prepared(m, 55).unwrap();
        r.activate(m, 55, true).unwrap();
        assert_eq!(r.active_mask, 0xff | 1 << 55);
        assert_eq!(bitmap::select_by_hash(r.active_mask, 8), Ok(55));
    }

    #[test]
    fn version_overflow_rejected_without_mutation() {
        let mut r = empty();
        r.registry_version = u64::MAX;
        assert_rejects(
            &r,
            MarketStatus::Initializing,
            AmmError::RegistryVersionOverflow,
            |r, m| r.record_prepared(*m, 0),
        );
        let mut r = prepared_initial();
        r.registry_version = u64::MAX;
        assert_rejects(
            &r,
            MarketStatus::Initializing,
            AmmError::RegistryVersionOverflow,
            |r, m| r.finalize_initial(m, INITIAL_MASK),
        );
        let (mut r, m) = opened();
        r.record_prepared(m, 9).unwrap();
        r.registry_version = u64::MAX;
        assert_rejects(&r, m, AmmError::RegistryVersionOverflow, |r, m| {
            r.activate(*m, 9, true)
        });
    }

    #[test]
    fn corrupt_stored_state_rejected() {
        let mut r = empty();
        r.active_mask = 1; // active but not prepared
        assert_eq!(r.validate(), Err(AmmError::ActiveNotPrepared));
        assert_eq!(r.active_count(), Err(AmmError::ActiveNotPrepared));
        assert_rejects(
            &r,
            MarketStatus::Open,
            AmmError::ActiveNotPrepared,
            |r, m| r.record_prepared(*m, 2),
        );
        let mut r = empty();
        r.prepared_mask = 1 << 60;
        assert_eq!(r.validate(), Err(AmmError::ReservedBitsSet));
        assert_eq!(
            r.check_swappable(MarketStatus::Open, 0),
            Err(AmmError::ReservedBitsSet)
        );
    }

    /// Random operation sequences against a naive model; every step must keep
    /// the invariants and either match the model or fail without mutation.
    #[test]
    fn randomized_lifecycle_against_model() {
        let seed = 0xabad_1dea_0000_0056u64;
        let mut rng = Rng(seed);
        for run in 0..300 {
            let mut r = empty();
            let mut m = MarketStatus::Initializing;
            let mut prepared = [false; 64];
            let mut active = [false; 64];
            let mut version = 0u64;
            for step in 0..200 {
                let ctx = format!("seed {seed:#x} run {run} step {step}");
                let id = match rng.next() % 4 {
                    0 => (rng.next() % 256) as u8,
                    _ => (rng.next() % 60) as u8,
                };
                let before = (r.clone(), m);
                let op = rng.next() % 10;
                let res = match op {
                    0..=3 => r.record_prepared(m, id),
                    4 => r.finalize_initial(
                        &mut m,
                        if rng.next().is_multiple_of(3) {
                            rng.next() & 0xff
                        } else {
                            0xff
                        },
                    ),
                    5..=7 => r.activate(m, id, !rng.next().is_multiple_of(4)),
                    8 => {
                        m = match m {
                            MarketStatus::Open => MarketStatus::Paused,
                            MarketStatus::Paused => MarketStatus::Open,
                            s => s,
                        };
                        Ok(())
                    }
                    _ => Ok(()),
                };
                match res {
                    Err(_) => assert_eq!((r.clone(), m), before, "{ctx}: mutated on error"),
                    Ok(()) => match op {
                        0..=3 => {
                            assert!((id as usize) < 56 && !prepared[id as usize], "{ctx}");
                            prepared[id as usize] = true;
                            version += 1;
                        }
                        4 => {
                            assert!((0..8).all(|i| prepared[i]), "{ctx}");
                            (0..8).for_each(|i| active[i] = true);
                            version += 1;
                        }
                        5..=7 => {
                            assert!(prepared[id as usize] && !active[id as usize], "{ctx}");
                            active[id as usize] = true;
                            version += 1;
                        }
                        _ => {}
                    },
                }
                r.validate().unwrap_or_else(|e| panic!("{ctx}: {e:?}"));
                let model_prep = (0..64)
                    .filter(|&i| prepared[i])
                    .fold(0u64, |a, i| a | 1 << i);
                let model_act = (0..64).filter(|&i| active[i]).fold(0u64, |a, i| a | 1 << i);
                assert_eq!(r.prepared_mask, model_prep, "{ctx}");
                assert_eq!(r.active_mask, model_act, "{ctx}");
                assert_eq!(r.registry_version, version, "{ctx}");
                if m == MarketStatus::Initializing {
                    assert_eq!(r.active_mask, 0, "{ctx}: active before open");
                    assert_eq!(r.prepared_mask & !INITIAL_MASK, 0, "{ctx}");
                } else {
                    assert_eq!(r.active_mask & INITIAL_MASK, INITIAL_MASK, "{ctx}");
                }
            }
        }
    }
}
