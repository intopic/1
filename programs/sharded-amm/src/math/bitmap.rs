//! Shard membership bitmaps (`u64`, bit `i` = shard ID `i`).
//!
//! Invariants enforced here:
//! - an ID is range-checked (`id < MAX_SHARDS`) before any shift, so no input
//!   value (56..=255) can panic, wrap or truncate;
//! - a mask with any reserved bit (56..=63) is rejected, never masked off;
//! - counts are always derived with `count_ones()`, never stored separately.
//!
//! A bitmap only records membership. It carries no liquidity, readiness or
//! authorization meaning; callers must check those before setting a bit.

use crate::constants::{MAX_SHARDS, RESERVED_MASK, VALID_MASK};
use crate::errors::AmmError;

/// Single-bit mask for `id`, or `ShardIdOutOfRange` for `id >= 56`.
pub fn shard_bit(id: u8) -> Result<u64, AmmError> {
    if id >= MAX_SHARDS {
        return Err(AmmError::ShardIdOutOfRange);
    }
    Ok(1u64 << id)
}

/// Rejects masks with reserved bits set.
pub fn validate_mask(mask: u64) -> Result<(), AmmError> {
    if mask & RESERVED_MASK != 0 {
        return Err(AmmError::ReservedBitsSet);
    }
    Ok(())
}

/// `false` for out-of-range IDs instead of an error: membership of an invalid
/// ID is simply absent.
pub fn is_set(mask: u64, id: u8) -> bool {
    shard_bit(id).is_ok_and(|bit| mask & bit != 0)
}

/// Returns `mask` with `id` set. Fails if the bit is already set, so a double
/// prepare/activate is an error rather than a silent no-op.
pub fn insert(mask: u64, id: u8, already_set: AmmError) -> Result<u64, AmmError> {
    validate_mask(mask)?;
    let bit = shard_bit(id)?;
    if mask & bit != 0 {
        return Err(already_set);
    }
    Ok(mask | bit)
}

/// Number of set bits in a valid mask (at most 56).
pub fn count(mask: u64) -> Result<u32, AmmError> {
    validate_mask(mask)?;
    Ok(mask.count_ones())
}

/// `true` when the set bits are exactly `0..n` for some `n` (including 0).
/// Only for such masks may a router use `hash & (n - 1)` with `n` a power of two.
pub fn is_prefix(mask: u64) -> bool {
    mask & RESERVED_MASK == 0 && mask & mask.wrapping_add(1) == 0
}

/// ID of the set bit with the given rank (0 = lowest set bit).
pub fn id_at_rank(mask: u64, rank: u32) -> Result<u8, AmmError> {
    let n = count(mask)?;
    if n == 0 {
        return Err(AmmError::EmptyMask);
    }
    if rank >= n {
        return Err(AmmError::RankOutOfRange);
    }
    let mut remaining = mask;
    for _ in 0..rank {
        remaining &= remaining - 1; // clear lowest set bit; remaining != 0 since rank < n
    }
    // trailing_zeros() < 56 because remaining is a non-zero valid mask.
    Ok(remaining.trailing_zeros() as u8)
}

/// Rank of `id` among set bits, i.e. the number of set bits below it.
pub fn rank_of(mask: u64, id: u8) -> Result<u32, AmmError> {
    validate_mask(mask)?;
    let bit = shard_bit(id)?;
    if mask & bit == 0 {
        return Err(AmmError::ShardNotActive);
    }
    Ok((mask & (bit - 1)).count_ones())
}

/// Offchain load-spreading helper: `rank = hash % count`, then the active bit
/// at that rank. Works for sparse masks. This is not best execution and not a
/// security boundary; the program still validates the chosen ID.
pub fn select_by_hash(mask: u64, hash: u64) -> Result<u8, AmmError> {
    let n = count(mask)?;
    if n == 0 {
        return Err(AmmError::EmptyMask);
    }
    // n <= 56, so the remainder fits in u32.
    id_at_rank(mask, (hash % u64::from(n)) as u32)
}

/// Iterates set IDs in ascending order. The caller must have validated `mask`;
/// reserved bits are skipped defensively.
pub fn ids(mask: u64) -> impl Iterator<Item = u8> {
    let mut remaining = mask & VALID_MASK;
    core::iter::from_fn(move || {
        if remaining == 0 {
            return None;
        }
        let id = remaining.trailing_zeros() as u8;
        remaining &= remaining - 1;
        Some(id)
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::constants::INITIAL_MASK;

    /// Deterministic xorshift64*; seeds are printed on failure for replay.
    pub(crate) struct Rng(pub u64);
    impl Rng {
        pub fn next(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
        }
    }

    fn reference_ids(mask: u64) -> Vec<u8> {
        (0..64u8).filter(|&i| mask >> i & 1 == 1).collect()
    }

    #[test]
    fn constants_match_spec() {
        assert_eq!(INITIAL_MASK, 0xff);
        assert_eq!(VALID_MASK, 0x00ff_ffff_ffff_ffff);
        assert_eq!(RESERVED_MASK, 0xff00_0000_0000_0000);
        assert_eq!(VALID_MASK.count_ones(), 56);
    }

    #[test]
    fn shard_bit_bounds_for_every_u8() {
        for id in 0..=u8::MAX {
            let r = shard_bit(id);
            if id < 56 {
                assert_eq!(r, Ok(1u64 << id), "id {id}");
            } else {
                assert_eq!(r, Err(AmmError::ShardIdOutOfRange), "id {id}");
                assert!(!is_set(u64::MAX, id), "id {id}");
            }
        }
        assert_eq!(shard_bit(55), Ok(1u64 << 55));
        assert_eq!(shard_bit(56), Err(AmmError::ShardIdOutOfRange));
        assert_eq!(shard_bit(63), Err(AmmError::ShardIdOutOfRange));
        assert_eq!(shard_bit(255), Err(AmmError::ShardIdOutOfRange));
    }

    #[test]
    fn reserved_bits_rejected_individually() {
        for bit in 56..64 {
            let mask = 1u64 << bit;
            assert_eq!(validate_mask(mask), Err(AmmError::ReservedBitsSet));
            assert_eq!(count(mask), Err(AmmError::ReservedBitsSet));
            assert_eq!(id_at_rank(mask | 1, 0), Err(AmmError::ReservedBitsSet));
            assert_eq!(select_by_hash(mask | 1, 0), Err(AmmError::ReservedBitsSet));
            assert_eq!(
                insert(mask, 0, AmmError::ShardAlreadyActive),
                Err(AmmError::ReservedBitsSet)
            );
            assert!(!is_prefix(mask | 0xff));
        }
        assert_eq!(validate_mask(VALID_MASK), Ok(()));
        assert_eq!(validate_mask(u64::MAX), Err(AmmError::ReservedBitsSet));
    }

    #[test]
    fn insert_rejects_duplicate_and_out_of_range() {
        let m = insert(0, 7, AmmError::ShardAlreadyActive).unwrap();
        assert_eq!(m, 0x80);
        assert_eq!(
            insert(m, 7, AmmError::ShardAlreadyActive),
            Err(AmmError::ShardAlreadyActive)
        );
        assert_eq!(
            insert(m, 56, AmmError::ShardAlreadyActive),
            Err(AmmError::ShardIdOutOfRange)
        );
        let full = insert(VALID_MASK & !(1 << 55), 55, AmmError::ShardAlreadyActive).unwrap();
        assert_eq!(full, VALID_MASK);
    }

    #[test]
    fn zero_mask_and_rank_errors() {
        assert_eq!(count(0), Ok(0));
        assert_eq!(id_at_rank(0, 0), Err(AmmError::EmptyMask));
        assert_eq!(select_by_hash(0, 12345), Err(AmmError::EmptyMask));
        assert_eq!(id_at_rank(0xff, 8), Err(AmmError::RankOutOfRange));
        assert_eq!(id_at_rank(VALID_MASK, 56), Err(AmmError::RankOutOfRange));
        assert_eq!(rank_of(0xff, 8), Err(AmmError::ShardNotActive));
        assert_eq!(rank_of(0xff, 56), Err(AmmError::ShardIdOutOfRange));
    }

    #[test]
    fn rank_selection_on_named_masks() {
        // Only bit 55.
        let m = 1u64 << 55;
        assert_eq!(id_at_rank(m, 0), Ok(55));
        assert_eq!(select_by_hash(m, u64::MAX), Ok(55));
        // Full valid mask: rank == id.
        for r in 0..56 {
            assert_eq!(id_at_rank(VALID_MASK, r), Ok(r as u8));
        }
        // Sparse with gaps.
        let sparse = (1 << 0) | (1 << 9) | (1 << 30) | (1 << 55);
        assert_eq!(ids(sparse).collect::<Vec<_>>(), vec![0, 9, 30, 55]);
        assert_eq!(id_at_rank(sparse, 2), Ok(30));
        assert_eq!(rank_of(sparse, 55), Ok(3));
        assert_eq!(select_by_hash(sparse, 5), Ok(9)); // 5 % 4 = 1
    }

    #[test]
    fn hash_and_55_is_not_a_valid_selector() {
        // Documents why `hash & 55` is forbidden: 55 = 0b110111, so it only ever
        // yields the 32 IDs whose bits are a subset of that pattern (never 8, 24..).
        let reachable: std::collections::BTreeSet<u64> = (0..1024u64).map(|h| h & 55).collect();
        assert!(!reachable.contains(&8));
        assert!(reachable.len() < 56);
        // The rank selector reaches every active ID.
        let reached: std::collections::BTreeSet<u8> = (0..56u64)
            .map(|h| select_by_hash(VALID_MASK, h).unwrap())
            .collect();
        assert_eq!(reached.len(), 56);
    }

    #[test]
    fn prefix_masks() {
        assert!(is_prefix(0));
        assert!(is_prefix(0xff));
        assert!(is_prefix(0xffff));
        assert!(is_prefix(VALID_MASK));
        assert!(!is_prefix(0xfe));
        assert!(!is_prefix(0x1ff & !0x10));
        assert!(!is_prefix(u64::MAX));
        // For the 0xff prefix, `hash & 7` equals rank selection.
        for h in 0..1000u64 {
            assert_eq!(select_by_hash(0xff, h), Ok((h & 7) as u8));
        }
    }

    #[test]
    fn randomized_against_reference() {
        let seed = 0x5eed_0000_0000_0001u64;
        let mut rng = Rng(seed);
        for iter in 0..20_000 {
            let mask = rng.next() & VALID_MASK & rng.next(); // varied densities
            let expected = reference_ids(mask);
            assert_eq!(
                ids(mask).collect::<Vec<_>>(),
                expected,
                "seed {seed:#x} iter {iter}"
            );
            assert_eq!(count(mask), Ok(expected.len() as u32));
            for (r, &id) in expected.iter().enumerate() {
                assert_eq!(
                    id_at_rank(mask, r as u32),
                    Ok(id),
                    "seed {seed:#x} iter {iter}"
                );
                assert_eq!(rank_of(mask, id), Ok(r as u32));
                assert!(is_set(mask, id));
            }
            let h = rng.next();
            if expected.is_empty() {
                assert_eq!(select_by_hash(mask, h), Err(AmmError::EmptyMask));
            } else {
                let id = select_by_hash(mask, h).unwrap();
                assert_eq!(id, expected[(h % expected.len() as u64) as usize]);
                assert!(is_set(mask, id));
            }
            let reserved = mask | (1u64 << (56 + (rng.next() % 8)));
            assert_eq!(validate_mask(reserved), Err(AmmError::ReservedBitsSet));
        }
    }
}
