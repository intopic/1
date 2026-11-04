//! Përshpejtimi x86_64 me AVX-512 IFMA (`vpmadd52luq` / `vpmadd52huq`).
//!
//! Numrat ruhen në fjalë 52-bit (radix 2^52) brenda korsive 64-bit; një regjistër
//! ZMM mban 8 fjalë. Shumëzimi Montgomery ndjek skemën "Almost Montgomery
//! Multiplication" (Gueron–Krasnov): për çdo fjalë `a_i`, akumulohen pjesët e ulëta
//! `lo52(a_i·B)` dhe `lo52(m·N)`, akumulatori zhvendoset me një korsi, pastaj
//! shtohen pjesët e larta `hi52(·)`. Korsitë 64-bit lejojnë akumulim pa normalizim
//! deri në 79 iteracione (≤ 2^61), pra 4096 bit mbulohen.
//!
//! Domeni Montgomery këtu është `R' = 2^(52·L)`, ku `L = ⌈bits/52⌉`; prandaj
//! API publike ([`Avx512Montgomery::exp`]) merr dhe kthen numra në formë normale
//! (fjalë 64-bit), njëlloj si [`crate::MontgomeryEngine::exp`].

#![allow(clippy::needless_range_loop)]

use alloc::vec::Vec;
use subtle::{Choice, ConditionallySelectable, ConstantTimeEq};

use crate::bigint;
use crate::euclid::EuclidCore;
use crate::montgomery::pow2_mod;

const MASK52: u64 = (1u64 << 52) - 1;
/// 4096 bit / 52 = 79 fjalë → 10 regjistra ZMM.
pub const MAX_VECS: usize = 10;
pub const MAX_L52: usize = MAX_VECS * 8;
const WINDOW: usize = 4;

/// A mbështet CPU-ja AVX-512F + AVX-512 IFMA? (zbulim në kohë ekzekutimi)
#[cfg(feature = "std")]
pub fn is_available() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        std::is_x86_feature_detected!("avx512f") && std::is_x86_feature_detected!("avx512ifma")
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        false
    }
}

type Limbs52 = [u64; MAX_L52];

/// Konteksti Montgomery në radix 2^52.
pub struct Avx512Montgomery {
    n64: usize,
    l: usize,
    v: usize,
    n52: Limbs52,
    k0: u64,
    /// R'^2 mod N në radix 2^52.
    rr52: Limbs52,
}

/// Fjalë 64-bit → fjalë 52-bit.
pub fn to_radix52(x: &[u64], out: &mut [u64]) {
    out.fill(0);
    let bits = x.len() * 64;
    let mut i = 0;
    while i * 52 < bits && i < out.len() {
        let bitpos = i * 52;
        let w = bitpos / 64;
        let off = bitpos % 64;
        let mut val = x[w] >> off;
        if off > 12 && w + 1 < x.len() {
            val |= x[w + 1] << (64 - off);
        }
        out[i] = val & MASK52;
        i += 1;
    }
}

/// Fjalë 52-bit (të normalizuara) → fjalë 64-bit.
pub fn from_radix52(x: &[u64], out: &mut [u64]) {
    out.fill(0);
    for (i, &limb) in x.iter().enumerate() {
        let bitpos = i * 52;
        let w = bitpos / 64;
        let off = bitpos % 64;
        if w < out.len() {
            out[w] |= limb << off;
        }
        if off > 12 && w + 1 < out.len() {
            out[w + 1] |= limb >> (64 - off);
        }
    }
}

impl Avx512Montgomery {
    pub fn new(modulus: &[u64]) -> Self {
        let n64 = modulus.len();
        assert!(n64 > 0 && n64 <= 64 && modulus[0] & 1 == 1);
        let l = (n64 * 64).div_ceil(52);
        let v = l.div_ceil(8);
        let mut n52 = [0u64; MAX_L52];
        to_radix52(modulus, &mut n52[..l]);
        let k0 = EuclidCore::montgomery_constant(modulus[0]) & MASK52;
        let rr = pow2_mod(2 * 52 * l, modulus);
        let mut rr52 = [0u64; MAX_L52];
        to_radix52(&rr, &mut rr52[..l]);
        Self {
            n64,
            l,
            v,
            n52,
            k0,
            rr52,
        }
    }

    /// Fuqizim në kohë konstante (dritare fikse 4-bit). `base < N`.
    ///
    /// # Safety
    /// Thirrësi duhet të garantojë që CPU mbështet `avx512f` dhe `avx512ifma`.
    #[target_feature(enable = "avx512f,avx512ifma")]
    pub unsafe fn exp(&self, base: &[u64], exponent: &[u64], out: &mut [u64]) {
        let l = self.l;
        let mut b52 = [0u64; MAX_L52];
        to_radix52(base, &mut b52[..l]);
        let mut one = [0u64; MAX_L52];
        one[0] = 1;

        const T: usize = 1 << WINDOW;
        let mut table = [[0u64; MAX_L52]; T];
        // SAFETY (për të gjitha thirrjet `amm` më poshtë): kemi të njëjtat veçori.
        unsafe {
            self.amm(&one, &self.rr52, &mut table[0]); // 1·R'
            self.amm(&b52, &self.rr52, &mut table[1]); // base·R'
            for i in 2..T {
                let (lo, hi) = table.split_at_mut(i);
                self.amm(&lo[i - 1], &lo[1], &mut hi[0]);
            }
            let mut acc = table[0];
            let mut tmp = [0u64; MAX_L52];
            let mut sel = [0u64; MAX_L52];
            let bits = exponent.len() * 64;
            let windows = bits.div_ceil(WINDOW);
            for w in (0..windows).rev() {
                for _ in 0..WINDOW {
                    self.amm(&acc, &acc, &mut tmp);
                    acc = tmp;
                }
                let mut idx = 0u64;
                for k in 0..WINDOW {
                    let b = w * WINDOW + k;
                    if b < bits {
                        idx |= bigint::bit(exponent, b) << k;
                    }
                }
                sel[..l].fill(0);
                for (k, entry) in table.iter().enumerate() {
                    let c: Choice = (k as u64).ct_eq(&idx);
                    for j in 0..l {
                        sel[j].conditional_assign(&entry[j], c);
                    }
                }
                self.amm(&acc, &sel, &mut tmp);
                acc = tmp;
            }
            self.amm(&acc, &one, &mut tmp);
            from_radix52(&tmp[..l], &mut out[..self.n64]);
        }
    }

    /// Shumëzim i plotë modular `a·b mod N` (formë normale) – kryesisht për teste.
    ///
    /// # Safety
    /// Si te [`Self::exp`].
    #[target_feature(enable = "avx512f,avx512ifma")]
    pub unsafe fn mul_mod(&self, a: &[u64], b: &[u64]) -> Vec<u64> {
        let l = self.l;
        let (mut a52, mut b52, mut t, mut r) = (
            [0u64; MAX_L52],
            [0u64; MAX_L52],
            [0u64; MAX_L52],
            [0u64; MAX_L52],
        );
        to_radix52(a, &mut a52[..l]);
        to_radix52(b, &mut b52[..l]);
        unsafe {
            self.amm(&a52, &self.rr52, &mut t); // a·R'
            self.amm(&t, &b52, &mut r); // a·b
        }
        let mut out = alloc::vec![0u64; self.n64];
        from_radix52(&r[..l], &mut out);
        out
    }

    /// `out = a·b·R'^-1 mod N`, me hyrje të normalizuara 52-bit `< N`.
    #[target_feature(enable = "avx512f,avx512ifma")]
    unsafe fn amm(&self, a: &Limbs52, b: &Limbs52, out: &mut Limbs52) {
        use core::arch::x86_64::*;
        let (l, v) = (self.l, self.v);
        let zero = _mm512_setzero_si512();
        let mut bv = [zero; MAX_VECS];
        let mut nv = [zero; MAX_VECS];
        let mut x = [zero; MAX_VECS];
        for k in 0..v {
            // SAFETY: indekset k*8..k*8+8 <= MAX_L52.
            unsafe {
                bv[k] = _mm512_loadu_si512(b.as_ptr().add(k * 8) as *const _);
                nv[k] = _mm512_loadu_si512(self.n52.as_ptr().add(k * 8) as *const _);
            }
        }
        for i in 0..l {
            let ai = _mm512_set1_epi64(a[i] as i64);
            for k in 0..v {
                x[k] = _mm512_madd52lo_epu64(x[k], ai, bv[k]);
            }
            let x0 = _mm_cvtsi128_si64(_mm512_castsi512_si128(x[0])) as u64;
            let m = x0.wrapping_mul(self.k0) & MASK52;
            let mv = _mm512_set1_epi64(m as i64);
            for k in 0..v {
                x[k] = _mm512_madd52lo_epu64(x[k], mv, nv[k]);
            }
            let x0 = _mm_cvtsi128_si64(_mm512_castsi512_si128(x[0])) as u64;
            let carry = x0 >> 52; // 52 bitët e ulët janë zero
            for k in 0..v {
                let next = if k + 1 < v { x[k + 1] } else { zero };
                x[k] = _mm512_alignr_epi64::<1>(next, x[k]);
            }
            x[0] = _mm512_add_epi64(x[0], _mm512_set_epi64(0, 0, 0, 0, 0, 0, 0, carry as i64));
            for k in 0..v {
                x[k] = _mm512_madd52hi_epu64(x[k], ai, bv[k]);
                x[k] = _mm512_madd52hi_epu64(x[k], mv, nv[k]);
            }
        }
        let mut raw = [0u64; MAX_L52];
        for k in 0..v {
            // SAFETY: si më lart.
            unsafe { _mm512_storeu_si512(raw.as_mut_ptr().add(k * 8) as *mut _, x[k]) };
        }
        // normalizim i bartjeve → 52-bit
        let mut c = 0u64;
        for j in 0..l {
            let t = raw[j] + c;
            out[j] = t & MASK52;
            c = t >> 52;
        }
        for j in l..MAX_L52 {
            out[j] = 0;
        }
        // rezultati < 2N; zbrit N në kohë konstante
        let mut d = [0u64; MAX_L52];
        let mut borrow = 0u64;
        for j in 0..l {
            let t = out[j].wrapping_sub(self.n52[j]).wrapping_sub(borrow);
            d[j] = t & MASK52;
            borrow = t >> 63;
        }
        let sub = Choice::from((c | (borrow ^ 1)) as u8 & 1);
        for j in 0..l {
            out[j].conditional_assign(&d[j], sub);
        }
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use crate::MontgomeryEngine;
    use rand::{Rng, SeedableRng};

    #[test]
    fn radix_roundtrip() {
        let mut rng = rand::rngs::StdRng::seed_from_u64(3);
        for n in [1usize, 5, 32, 64] {
            let x: Vec<u64> = (0..n).map(|_| rng.gen()).collect();
            let l = (n * 64).div_ceil(52);
            let mut r = vec![0u64; l];
            to_radix52(&x, &mut r);
            let mut back = vec![0u64; n];
            from_radix52(&r, &mut back);
            assert_eq!(x, back);
        }
    }

    #[test]
    fn avx512_matches_portable() {
        if !is_available() {
            eprintln!("AVX-512 IFMA mungon në këtë CPU – testi anashkalohet");
            return;
        }
        let mut rng = rand::rngs::StdRng::seed_from_u64(11);
        for n in [1usize, 2, 7, 16, 32, 48, 64] {
            let mut m: Vec<u64> = (0..n).map(|_| rng.gen()).collect();
            m[0] |= 1;
            m[n - 1] |= 1 << 63;
            let portable = MontgomeryEngine::new(&m);
            let avx = Avx512Montgomery::new(&m);
            for _ in 0..4 {
                let mut a: Vec<u64> = (0..n).map(|_| rng.gen()).collect();
                let mut b: Vec<u64> = (0..n).map(|_| rng.gen()).collect();
                a[n - 1] >>= 1; // < N sepse biti i lartë i N është 1
                b[n - 1] >>= 1;
                let mut expect = vec![0u64; n];
                portable.mul_mod(&a, &b, &mut expect);
                assert_eq!(unsafe { avx.mul_mod(&a, &b) }, expect, "mul_mod n={n}");
                let e: Vec<u64> = (0..n.min(3)).map(|_| rng.gen()).collect();
                portable.exp_portable(&a, &e, &mut expect);
                let mut got = vec![0u64; n];
                unsafe { avx.exp(&a, &e, &mut got) };
                assert_eq!(got, expect, "exp n={n}");
            }
        }
    }
}
