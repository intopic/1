//! ANT-05: Euklidi binar (Stein GCD) dhe inversi modular në kohë konstante.

use alloc::vec::Vec;
use subtle::{Choice, ConstantTimeEq};

use crate::bigint::{self, add_assign, ct_assign, ct_lt, ct_swap, shr1, sub_assign};

/// Eksponenti publik standard i RSA-s.
pub const RSA_E_65537: u64 = 65_537;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EuclidError {
    /// Moduli është çift ose zero.
    EvenModulus,
    /// Eksponenti publik nuk është tek ose është < 3.
    InvalidExponent,
    /// `gcd(e, n) != 1`.
    NotCoprime,
}

pub struct EuclidCore;

impl EuclidCore {
    /// Stein GCD për `u64` (kohë e ndryshueshme – vetëm për vlera publike).
    pub fn gcd_u64(mut a: u64, mut b: u64) -> u64 {
        if a == 0 {
            return b;
        }
        if b == 0 {
            return a;
        }
        let shift = (a | b).trailing_zeros();
        a >>= a.trailing_zeros();
        loop {
            b >>= b.trailing_zeros();
            if a > b {
                core::mem::swap(&mut a, &mut b);
            }
            b -= a;
            if b == 0 {
                return a << shift;
            }
        }
    }

    /// Stein GCD për numra të mëdhenj (kohë e ndryshueshme – vetëm për vlera publike).
    pub fn binary_gcd(a: &[u64], b: &[u64]) -> Vec<u64> {
        let n = a.len().max(b.len());
        let mut x = alloc::vec![0u64; n];
        let mut y = alloc::vec![0u64; n];
        x[..a.len()].copy_from_slice(a);
        y[..b.len()].copy_from_slice(b);
        if bigint::bit_len(&x) == 0 {
            return y;
        }
        if bigint::bit_len(&y) == 0 {
            return x;
        }
        let mut shift = 0usize;
        while (x[0] | y[0]) & 1 == 0 {
            shr1(&mut x, 0);
            shr1(&mut y, 0);
            shift += 1;
        }
        while x[0] & 1 == 0 {
            shr1(&mut x, 0);
        }
        loop {
            while y[0] & 1 == 0 {
                shr1(&mut y, 0);
            }
            if bigint::cmp_vartime(&x, &y) == core::cmp::Ordering::Greater {
                core::mem::swap(&mut x, &mut y);
            }
            sub_assign(&mut y, &x);
            if bigint::bit_len(&y) == 0 {
                break;
            }
        }
        for _ in 0..shift {
            let mut carry = 0;
            for w in x.iter_mut() {
                let nc = *w >> 63;
                *w = (*w << 1) | carry;
                carry = nc;
            }
        }
        x
    }

    /// `n mod d` për pjesëtues të vogël (kohë e ndryshueshme).
    pub fn rem_u64(n: &[u64], d: u64) -> u64 {
        let mut r: u128 = 0;
        for &w in n.iter().rev() {
            r = ((r << 64) | w as u128) % d as u128;
        }
        r as u64
    }

    /// Kontrollon që eksponenti publik `e` (p.sh. 65537) është i vlefshëm dhe
    /// relativisht i thjeshtë me modulin `n`.
    ///
    /// Shënim: kushti i plotë i RSA-s është `gcd(e, φ(n)) = 1`, i cili kërkon
    /// faktorët e `n`. Me vetëm çelësin publik, kontrolli i mundshëm është
    /// `gcd(e, n) = 1`, plus `n` tek.
    pub fn validate_public_exponent(e: u64, modulus: &[u64]) -> Result<(), EuclidError> {
        if modulus.is_empty() || modulus[0] & 1 == 0 {
            return Err(EuclidError::EvenModulus);
        }
        if e < 3 || e & 1 == 0 {
            return Err(EuclidError::InvalidExponent);
        }
        if Self::gcd_u64(e, Self::rem_u64(modulus, e)) != 1 {
            return Err(EuclidError::NotCoprime);
        }
        Ok(())
    }

    /// Llogarit inversin modular `a^-1 mod m` me XGCD binar në kohë konstante.
    ///
    /// Algoritmi ruan invariantet `A ≡ U·a`, `B ≡ V·a (mod m)` dhe kryen saktësisht
    /// `2·64·len` iteracione me operacione të kushtëzuara pa degëzime (`subtle`).
    /// Kërkon `m` tek. Kthen `None` nëse `gcd(a, m) != 1` ose `m` është çift.
    pub fn modular_inverse(a: &[u64], modulus: &[u64]) -> Option<Vec<u64>> {
        let n = modulus.len();
        if n == 0 || modulus[0] & 1 == 0 || a.len() > n {
            return None;
        }
        let mut aa = alloc::vec![0u64; n];
        aa[..a.len()].copy_from_slice(a);
        let mut bb = modulus.to_vec();
        let mut u = alloc::vec![0u64; n];
        u[0] = 1;
        let mut v = alloc::vec![0u64; n];
        let mut tmp = alloc::vec![0u64; n];
        let mut tmp2 = alloc::vec![0u64; n];

        // U duhet të jetë i reduktuar mod m; nëse a >= m, A ende funksionon
        // sepse invarianti është kongruencë.
        for _ in 0..(2 * 64 * n) {
            let a_odd = Choice::from((aa[0] & 1) as u8);
            // nëse A tek dhe A < B: shkëmbe (A,U) <-> (B,V)
            let swap = a_odd & ct_lt(&aa, &bb);
            ct_swap(&mut aa, &mut bb, swap);
            ct_swap(&mut u, &mut v, swap);
            // nëse A tek: A -= B; U -= V (mod m)
            tmp.copy_from_slice(&aa);
            sub_assign(&mut tmp, &bb);
            ct_assign(&mut aa, &tmp, a_odd);
            tmp.copy_from_slice(&u);
            let borrow = sub_assign(&mut tmp, &v);
            tmp2.copy_from_slice(&tmp);
            add_assign(&mut tmp2, modulus);
            ct_assign(&mut tmp, &tmp2, Choice::from(borrow as u8));
            ct_assign(&mut u, &tmp, a_odd);
            // A /= 2 (A është tani çift); U /= 2 mod m
            shr1(&mut aa, 0);
            let u_odd = Choice::from((u[0] & 1) as u8);
            tmp.copy_from_slice(&u);
            let carry = add_assign(&mut tmp, modulus);
            ct_assign(&mut u, &tmp, u_odd);
            let top = carry & (u_odd.unwrap_u8() as u64);
            shr1(&mut u, top);
        }
        // B = gcd(a, m); V = a^-1 nëse B == 1
        let mut one = alloc::vec![0u64; n];
        one[0] = 1;
        let mut eq = Choice::from(1);
        for i in 0..n {
            eq &= bb[i].ct_eq(&one[i]);
        }
        if bool::from(eq) {
            Some(v)
        } else {
            None
        }
    }

    /// Llogarit konstantën negative të Montgomery-t: `mu = -N^-1 mod 2^64`
    /// (iteracion Newton/Hensel: çdo hap dyfishon bitët e saktë, 1→64 në 6 hapa).
    pub fn montgomery_constant(modulus_least_word: u64) -> u64 {
        let mut inv = 1u64;
        for _ in 0..6 {
            inv = inv.wrapping_mul(2u64.wrapping_sub(modulus_least_word.wrapping_mul(inv)));
        }
        inv.wrapping_neg()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use num_bigint::BigUint;
    use rand::{Rng, SeedableRng};

    fn to_big(x: &[u64]) -> BigUint {
        let mut b = BigUint::default();
        for &w in x.iter().rev() {
            b = (b << 64) + BigUint::from(w);
        }
        b
    }

    #[test]
    fn mont_constant() {
        let mut rng = rand::rngs::StdRng::seed_from_u64(1);
        for _ in 0..1000 {
            let n: u64 = rng.gen::<u64>() | 1;
            let mu = EuclidCore::montgomery_constant(n);
            assert_eq!(n.wrapping_mul(mu), u64::MAX); // n * mu ≡ -1
        }
    }

    #[test]
    fn gcd_small() {
        assert_eq!(EuclidCore::gcd_u64(48, 18), 6);
        assert_eq!(EuclidCore::gcd_u64(65537, 65537 * 3), 65537);
        assert_eq!(EuclidCore::binary_gcd(&[48, 0], &[18]), [6, 0]);
        assert_eq!(EuclidCore::binary_gcd(&[0, 4], &[0, 6]), [0, 2]);
    }

    #[test]
    fn inverse_random() {
        let mut rng = rand::rngs::StdRng::seed_from_u64(7);
        for limbs in [1usize, 2, 4, 8, 32] {
            for _ in 0..5 {
                let mut m: Vec<u64> = (0..limbs).map(|_| rng.gen()).collect();
                m[0] |= 1;
                m[limbs - 1] |= 1 << 63;
                let a: Vec<u64> = (0..limbs).map(|_| rng.gen()).collect();
                let (bm, ba) = (to_big(&m), to_big(&a) % to_big(&m));
                let g = num_integer_gcd(&ba, &bm);
                match EuclidCore::modular_inverse(&a, &m) {
                    Some(inv) => {
                        assert_eq!(g, BigUint::from(1u8));
                        assert_eq!((to_big(&inv) * &ba) % &bm, BigUint::from(1u8));
                        assert!(to_big(&inv) < bm);
                    }
                    None => assert_ne!(g, BigUint::from(1u8)),
                }
            }
        }
        assert!(EuclidCore::modular_inverse(&[3], &[9]).is_none());
        assert!(EuclidCore::modular_inverse(&[3], &[10]).is_none());
        assert_eq!(EuclidCore::modular_inverse(&[3], &[7]).unwrap(), [5]);
    }

    fn num_integer_gcd(a: &BigUint, b: &BigUint) -> BigUint {
        let (mut a, mut b) = (a.clone(), b.clone());
        while b != BigUint::default() {
            let r = &a % &b;
            a = b;
            b = r;
        }
        a
    }

    #[test]
    fn exponent_validation() {
        assert!(EuclidCore::validate_public_exponent(RSA_E_65537, &[65537 * 7]).is_err());
        assert!(EuclidCore::validate_public_exponent(RSA_E_65537, &[1_000_003]).is_ok());
        assert_eq!(
            EuclidCore::validate_public_exponent(RSA_E_65537, &[10]),
            Err(EuclidError::EvenModulus)
        );
        assert_eq!(
            EuclidCore::validate_public_exponent(4, &[11]),
            Err(EuclidError::InvalidExponent)
        );
    }
}
