//! Primitivë për numra të mëdhenj me fjalë 64-bit në renditje little-endian
//! (`limbs[0]` është fjala më pak e rëndësishme).
//!
//! Të gjitha funksionet këtu janë pa degëzime të varura nga vlerat.

use alloc::vec::Vec;
use subtle::{Choice, ConditionallySelectable, ConstantTimeEq};

/// `a + b + carry` → (shuma, bartja)
#[inline(always)]
pub const fn adc(a: u64, b: u64, carry: u64) -> (u64, u64) {
    let t = (a as u128) + (b as u128) + (carry as u128);
    (t as u64, (t >> 64) as u64)
}

/// `a - b - borrow` → (diferenca, huazimi ∈ {0,1})
#[inline(always)]
pub const fn sbb(a: u64, b: u64, borrow: u64) -> (u64, u64) {
    let t = (a as u128).wrapping_sub((b as u128) + (borrow as u128));
    (t as u64, ((t >> 64) as u64) & 1)
}

/// `acc + a * b + carry` → (fjala e ulët, fjala e lartë). Nuk tejmbushet kurrë.
#[inline(always)]
pub const fn mac(acc: u64, a: u64, b: u64, carry: u64) -> (u64, u64) {
    let t = (acc as u128) + (a as u128) * (b as u128) + (carry as u128);
    (t as u64, (t >> 64) as u64)
}

/// `out = a - b` (gjatësi të barabarta). Kthen huazimin përfundimtar.
#[inline]
pub fn sub_into(a: &[u64], b: &[u64], out: &mut [u64]) -> u64 {
    debug_assert!(a.len() == b.len() && a.len() == out.len());
    let mut borrow = 0;
    for i in 0..a.len() {
        let (d, br) = sbb(a[i], b[i], borrow);
        out[i] = d;
        borrow = br;
    }
    borrow
}

/// `a += b` në vend. Kthen bartjen përfundimtare.
#[inline]
pub fn add_assign(a: &mut [u64], b: &[u64]) -> u64 {
    debug_assert_eq!(a.len(), b.len());
    let mut carry = 0;
    for i in 0..a.len() {
        let (s, c) = adc(a[i], b[i], carry);
        a[i] = s;
        carry = c;
    }
    carry
}

/// `a -= b` në vend. Kthen huazimin përfundimtar.
#[inline]
pub fn sub_assign(a: &mut [u64], b: &[u64]) -> u64 {
    debug_assert_eq!(a.len(), b.len());
    let mut borrow = 0;
    for i in 0..a.len() {
        let (d, br) = sbb(a[i], b[i], borrow);
        a[i] = d;
        borrow = br;
    }
    borrow
}

/// Nëse `choice == 1`: `dst = src`, përndryshe `dst` mbetet. Kohë konstante.
#[inline]
pub fn ct_assign(dst: &mut [u64], src: &[u64], choice: Choice) {
    debug_assert_eq!(dst.len(), src.len());
    for i in 0..dst.len() {
        dst[i].conditional_assign(&src[i], choice);
    }
}

/// Shkëmben `a` dhe `b` nëse `choice == 1`. Kohë konstante.
#[inline]
pub fn ct_swap(a: &mut [u64], b: &mut [u64], choice: Choice) {
    debug_assert_eq!(a.len(), b.len());
    for i in 0..a.len() {
        u64::conditional_swap(&mut a[i], &mut b[i], choice);
    }
}

/// `a == 0` në kohë konstante.
#[inline]
pub fn ct_is_zero(a: &[u64]) -> Choice {
    let mut acc = 0u64;
    for &w in a {
        acc |= w;
    }
    acc.ct_eq(&0)
}

/// `a < b` (gjatësi të barabarta) në kohë konstante.
#[inline]
pub fn ct_lt(a: &[u64], b: &[u64]) -> Choice {
    debug_assert_eq!(a.len(), b.len());
    let mut borrow = 0;
    for i in 0..a.len() {
        borrow = sbb(a[i], b[i], borrow).1;
    }
    Choice::from(borrow as u8)
}

/// Krahasim i zakonshëm (kohë e ndryshueshme) – vetëm për vlera publike.
pub fn cmp_vartime(a: &[u64], b: &[u64]) -> core::cmp::Ordering {
    let n = a.len().max(b.len());
    for i in (0..n).rev() {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        if x != y {
            return x.cmp(&y);
        }
    }
    core::cmp::Ordering::Equal
}

/// Zhvendosje djathtas me 1 bit, me `top` si biti hyrës në pozicionin më të lartë.
#[inline]
pub fn shr1(a: &mut [u64], top: u64) {
    let n = a.len();
    for i in 0..n {
        let next = if i + 1 < n { a[i + 1] } else { top };
        a[i] = (a[i] >> 1) | (next << 63);
    }
}

/// Numri i bitëve të vlerës (kohë e ndryshueshme – për vlera publike).
pub fn bit_len(a: &[u64]) -> usize {
    for i in (0..a.len()).rev() {
        if a[i] != 0 {
            return i * 64 + (64 - a[i].leading_zeros() as usize);
        }
    }
    0
}

/// Biti `i` i `a`.
#[inline(always)]
pub fn bit(a: &[u64], i: usize) -> u64 {
    (a[i / 64] >> (i % 64)) & 1
}

/// Bajte big-endian → fjalë little-endian (me `limbs` fjalë; mbushet me zero).
/// Kthen `None` nëse vlera nuk nxë.
pub fn from_be_bytes(bytes: &[u8], limbs: usize) -> Option<Vec<u64>> {
    let mut out = alloc::vec![0u64; limbs];
    for (k, &b) in bytes.iter().rev().enumerate() {
        let limb = k / 8;
        if limb >= limbs {
            if b != 0 {
                return None;
            }
            continue;
        }
        out[limb] |= (b as u64) << (8 * (k % 8));
    }
    Some(out)
}

/// Fjalë little-endian → bajte big-endian me gjatësi fikse `len`.
pub fn to_be_bytes(limbs: &[u64], len: usize) -> Vec<u8> {
    let mut out = alloc::vec![0u8; len];
    for k in 0..len {
        let limb = k / 8;
        if limb < limbs.len() {
            out[len - 1 - k] = (limbs[limb] >> (8 * (k % 8))) as u8;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn be_roundtrip() {
        let bytes = [0x01u8, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09];
        let l = from_be_bytes(&bytes, 2).unwrap();
        assert_eq!(l, [0x0203040506070809, 0x01]);
        assert_eq!(to_be_bytes(&l, 9), bytes);
        assert!(from_be_bytes(&bytes, 1).is_none());
    }

    #[test]
    fn ct_ops() {
        assert!(bool::from(ct_lt(&[1, 0], &[0, 1])));
        assert!(!bool::from(ct_lt(&[0, 1], &[0, 1])));
        let mut a = [1u64, 2];
        let mut b = [3u64, 4];
        ct_swap(&mut a, &mut b, Choice::from(1));
        assert_eq!(a, [3, 4]);
        let mut x = [0u64, 1];
        shr1(&mut x, 1);
        assert_eq!(x, [1 << 63, 1 << 63]);
        assert_eq!(bit_len(&[0, 1]), 65);
    }
}
