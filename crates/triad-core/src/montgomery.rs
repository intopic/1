//! MOD-06: Shumëzimi Montgomery CIOS për numra 2048/4096-bit.
//!
//! * [`MontgomeryEngine::mul`] – CIOS portabël (radix 2^64) në kohë konstante,
//!   pa alokime në heap (buferi i punës është në stack, maksimumi 4096 bit).
//! * [`MontgomeryEngine::exp`] – fuqizim me dritare fikse 4-bit në kohë konstante
//!   (eksponent sekret). Me feature `std` në x86_64 me AVX-512 IFMA, përdor
//!   automatikisht [`crate::simd::avx512`] (radix 2^52, `vpmadd52{lo,hi}uq`).
//! * [`MontgomeryEngine::exp_vartime`] – dritare rrëshqitëse 5-bit për eksponentë
//!   publikë (p.sh. `e = 65537` gjatë verifikimit RSA).
//!
//! Pse `mul` nuk thërret AVX-512 drejtpërdrejt: rruga IFMA punon me fjalë 52-bit,
//! pra me `R' = 2^(52·L) ≠ R = 2^(64·n)`. Paraqitjet Montgomery nuk janë të
//! përputhshme, ndaj SIMD përdoret brenda `exp`, ku hyrja/dalja janë në formë
//! normale dhe domeni i brendshëm nuk duket nga jashtë.

use alloc::vec::Vec;
use subtle::{Choice, ConditionallySelectable, ConstantTimeEq};

use crate::bigint::{self, adc, mac, sbb};
use crate::euclid::EuclidCore;

/// Numri maksimal i fjalëve 64-bit (4096 bit).
pub const MAX_LIMBS: usize = 64;
/// Gjerësia e dritares për fuqizimin në kohë konstante.
const CT_WINDOW: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MontgomeryError {
    EmptyModulus,
    EvenModulus,
    TooLarge,
    /// Fjala më e lartë e modulit është zero (gjatësia duhet të jetë minimale).
    NotNormalized,
}

#[derive(Clone, Debug)]
pub struct MontgomeryEngine {
    /// Moduli N (little-endian), p.sh. 32 fjalë për RSA-2048.
    pub modulus: Vec<u64>,
    /// R^2 mod N, ku R = 2^(64·n).
    pub r_squared: Vec<u64>,
    /// -N^-1 mod 2^64.
    pub n_prime: u64,
}

impl MontgomeryEngine {
    /// Krijon motorin. Panik nëse moduli është i pavlefshëm; shih [`Self::try_new`].
    pub fn new(modulus: &[u64]) -> Self {
        Self::try_new(modulus).expect("modul i pavlefshëm për Montgomery")
    }

    pub fn try_new(modulus: &[u64]) -> Result<Self, MontgomeryError> {
        let n = modulus.len();
        if n == 0 {
            return Err(MontgomeryError::EmptyModulus);
        }
        if n > MAX_LIMBS {
            return Err(MontgomeryError::TooLarge);
        }
        if modulus[0] & 1 == 0 {
            return Err(MontgomeryError::EvenModulus);
        }
        if modulus[n - 1] == 0 {
            return Err(MontgomeryError::NotNormalized);
        }
        let n_prime = EuclidCore::montgomery_constant(modulus[0]);
        let r_squared = pow2_mod(2 * 64 * n, modulus);
        Ok(Self {
            modulus: modulus.to_vec(),
            r_squared,
            n_prime,
        })
    }

    /// Numri i fjalëve 64-bit të modulit.
    #[inline]
    pub fn limbs(&self) -> usize {
        self.modulus.len()
    }

    /// Shumëzimi modular: `out = a · b · R^-1 mod N`. Kërkon `a, b < N`.
    #[inline]
    pub fn mul(&self, a: &[u64], b: &[u64], out: &mut [u64]) {
        self.mul_cios_portable(a, b, out);
    }

    /// CIOS (Coarsely Integrated Operand Scanning), Koç–Acar–Kaliski 1996.
    /// Kohë konstante: nuk ka degëzime sipas të dhënave; zbritja përfundimtare
    /// bëhet me përzgjedhje të kushtëzuar (`subtle`).
    pub fn mul_cios_portable(&self, a: &[u64], b: &[u64], out: &mut [u64]) {
        let n = self.limbs();
        assert!(
            a.len() == n && b.len() == n && out.len() == n,
            "gjatësi të papërputhshme"
        );
        let p = &self.modulus;
        let mut t = [0u64; MAX_LIMBS + 2];
        for i in 0..n {
            // t += a * b[i]
            let mut c = 0;
            for j in 0..n {
                let (lo, hi) = mac(t[j], a[j], b[i], c);
                t[j] = lo;
                c = hi;
            }
            let (s, c2) = adc(t[n], c, 0);
            t[n] = s;
            t[n + 1] = c2;
            // t = (t + m*N) / 2^64
            let m = t[0].wrapping_mul(self.n_prime);
            let (_, mut c) = mac(t[0], m, p[0], 0);
            for j in 1..n {
                let (lo, hi) = mac(t[j], m, p[j], c);
                t[j - 1] = lo;
                c = hi;
            }
            let (s, c2) = adc(t[n], c, 0);
            t[n - 1] = s;
            t[n] = t[n + 1] + c2;
        }
        // t < 2N: zbrit N nëse t >= N
        let mut borrow = 0;
        for j in 0..n {
            let (d, br) = sbb(t[j], p[j], borrow);
            out[j] = d;
            borrow = br;
        }
        // mbaj t nëse (t[n] == 0 && borrow == 1)
        let keep = t[n].ct_eq(&0) & borrow.ct_eq(&1);
        for j in 0..n {
            out[j].conditional_assign(&t[j], keep);
        }
    }

    /// `out = a · R mod N`.
    pub fn to_montgomery(&self, a: &[u64], out: &mut [u64]) {
        self.mul(a, &self.r_squared, out);
    }

    /// `out = a · R^-1 mod N`.
    pub fn from_montgomery(&self, a: &[u64], out: &mut [u64]) {
        let mut one = [0u64; MAX_LIMBS];
        one[0] = 1;
        self.mul(a, &one[..self.limbs()], out);
    }

    /// Shumëzim modular i plotë: `out = a · b mod N` (hyrje/dalje në formë normale).
    pub fn mul_mod(&self, a: &[u64], b: &[u64], out: &mut [u64]) {
        let n = self.limbs();
        let mut am = [0u64; MAX_LIMBS];
        self.to_montgomery(a, &mut am[..n]);
        self.mul(&am[..n], b, out);
    }

    /// Fuqizimi modular `out = base^exponent mod N` në kohë konstante
    /// (dritare fikse 4-bit, kërkim në tabelë pa degëzime). `base < N`.
    /// Zgjedh automatikisht AVX-512 IFMA kur është e disponueshme.
    pub fn exp(&self, base: &[u64], exponent: &[u64], out: &mut [u64]) {
        #[cfg(all(target_arch = "x86_64", feature = "std"))]
        {
            if crate::simd::avx512::is_available() {
                // SAFETY: veçoritë e CPU-së u verifikuan në kohë ekzekutimi.
                let eng = crate::simd::avx512::Avx512Montgomery::new(&self.modulus);
                unsafe { eng.exp(base, exponent, out) };
                return;
            }
        }
        #[cfg(all(
            target_arch = "x86_64",
            not(feature = "std"),
            target_feature = "avx512ifma"
        ))]
        {
            let eng = crate::simd::avx512::Avx512Montgomery::new(&self.modulus);
            unsafe { eng.exp(base, exponent, out) };
            return;
        }
        #[allow(unreachable_code)]
        self.exp_portable(base, exponent, out)
    }

    /// Fuqizimi në kohë konstante vetëm me rrugën portabël.
    pub fn exp_portable(&self, base: &[u64], exponent: &[u64], out: &mut [u64]) {
        let n = self.limbs();
        assert!(base.len() == n && out.len() == n);
        const T: usize = 1 << CT_WINDOW;
        let mut table = [[0u64; MAX_LIMBS]; T];
        let mut one = [0u64; MAX_LIMBS];
        one[0] = 1;
        self.to_montgomery(&one[..n], &mut table[0][..n]);
        self.to_montgomery(base, &mut table[1][..n]);
        for i in 2..T {
            let (lo, hi) = table.split_at_mut(i);
            self.mul(&lo[i - 1][..n], &lo[1][..n], &mut hi[0][..n]);
        }
        let mut acc = table[0];
        let mut tmp = [0u64; MAX_LIMBS];
        let mut sel = [0u64; MAX_LIMBS];
        let bits = exponent.len() * 64;
        let windows = bits.div_ceil(CT_WINDOW);
        for w in (0..windows).rev() {
            for _ in 0..CT_WINDOW {
                self.mul(&acc[..n], &acc[..n], &mut tmp[..n]);
                acc[..n].copy_from_slice(&tmp[..n]);
            }
            let mut idx = 0u64;
            for k in 0..CT_WINDOW {
                let b = w * CT_WINDOW + k;
                if b < bits {
                    idx |= bigint::bit(exponent, b) << k;
                }
            }
            ct_table_select(&table, idx, n, &mut sel);
            self.mul(&acc[..n], &sel[..n], &mut tmp[..n]);
            acc[..n].copy_from_slice(&tmp[..n]);
        }
        self.from_montgomery(&acc[..n], out);
    }

    /// Fuqizim me dritare rrëshqitëse (5-bit) – KOHË E NDRYSHUESHME.
    /// Vetëm për eksponentë publikë (verifikim RSA me `e = 65537`).
    pub fn exp_vartime(&self, base: &[u64], exponent: &[u64], out: &mut [u64]) {
        let n = self.limbs();
        assert!(base.len() == n && out.len() == n);
        const W: usize = 5;
        let mut odd = [[0u64; MAX_LIMBS]; 1 << (W - 1)]; // base^1, base^3, ..., base^31
        let mut b2 = [0u64; MAX_LIMBS];
        self.to_montgomery(base, &mut odd[0][..n]);
        self.mul(&odd[0][..n], &odd[0][..n], &mut b2[..n]);
        for i in 1..odd.len() {
            let (lo, hi) = odd.split_at_mut(i);
            self.mul(&lo[i - 1][..n], &b2[..n], &mut hi[0][..n]);
        }
        let mut one = [0u64; MAX_LIMBS];
        one[0] = 1;
        let mut acc = [0u64; MAX_LIMBS];
        self.to_montgomery(&one[..n], &mut acc[..n]);
        let mut tmp = [0u64; MAX_LIMBS];
        let mut i = bigint::bit_len(exponent) as isize - 1;
        while i >= 0 {
            if bigint::bit(exponent, i as usize) == 0 {
                self.mul(&acc[..n], &acc[..n], &mut tmp[..n]);
                acc[..n].copy_from_slice(&tmp[..n]);
                i -= 1;
                continue;
            }
            // gjej dritaren më të gjatë [l, i] me gjatësi <= W që mbaron me bit 1
            let mut l = (i - W as isize + 1).max(0);
            while bigint::bit(exponent, l as usize) == 0 {
                l += 1;
            }
            let mut val = 0usize;
            for k in (l..=i).rev() {
                val = (val << 1) | bigint::bit(exponent, k as usize) as usize;
                self.mul(&acc[..n], &acc[..n], &mut tmp[..n]);
                acc[..n].copy_from_slice(&tmp[..n]);
            }
            self.mul(&acc[..n], &odd[val >> 1][..n], &mut tmp[..n]);
            acc[..n].copy_from_slice(&tmp[..n]);
            i = l - 1;
        }
        self.from_montgomery(&acc[..n], out);
    }
}

/// Përzgjedh `table[idx]` duke lexuar çdo hyrje (pa akses të varur nga sekreti).
#[inline]
fn ct_table_select<const T: usize>(
    table: &[[u64; MAX_LIMBS]; T],
    idx: u64,
    n: usize,
    out: &mut [u64; MAX_LIMBS],
) {
    out[..n].fill(0);
    for (k, entry) in table.iter().enumerate() {
        let c: Choice = (k as u64).ct_eq(&idx);
        for j in 0..n {
            out[j].conditional_assign(&entry[j], c);
        }
    }
}

/// `2^k mod N` me dyfishim të përsëritur (kohë e ndryshueshme; N është publik).
pub(crate) fn pow2_mod(k: usize, modulus: &[u64]) -> Vec<u64> {
    let n = modulus.len();
    let mut x = alloc::vec![0u64; n];
    x[0] = 1;
    if n == 1 && modulus[0] == 1 {
        return alloc::vec![0];
    }
    for _ in 0..k {
        let mut carry = 0;
        for w in x.iter_mut() {
            let nc = *w >> 63;
            *w = (*w << 1) | carry;
            carry = nc;
        }
        if carry == 1 || bigint::cmp_vartime(&x, modulus) != core::cmp::Ordering::Less {
            bigint::sub_assign(&mut x, modulus);
        }
    }
    x
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
    fn from_big(b: &BigUint, n: usize) -> Vec<u64> {
        let mut d = b.to_u64_digits();
        d.resize(n, 0);
        d
    }
    fn rand_mod(rng: &mut impl Rng, n: usize) -> Vec<u64> {
        let mut m: Vec<u64> = (0..n).map(|_| rng.gen()).collect();
        m[0] |= 1;
        m[n - 1] |= 1 << 63;
        m
    }
    fn rand_below(rng: &mut impl Rng, m: &[u64]) -> Vec<u64> {
        let x: Vec<u64> = (0..m.len()).map(|_| rng.gen()).collect();
        from_big(&(to_big(&x) % to_big(m)), m.len())
    }

    #[test]
    fn mul_matches_bigint() {
        let mut rng = rand::rngs::StdRng::seed_from_u64(42);
        for n in [1usize, 2, 3, 17, 32, 64] {
            let m = rand_mod(&mut rng, n);
            let eng = MontgomeryEngine::new(&m);
            let bm = to_big(&m);
            let rinv = to_big(
                &EuclidCore::modular_inverse(
                    &from_big(&((BigUint::from(1u8) << (64 * n)) % &bm), n),
                    &m,
                )
                .unwrap(),
            );
            for _ in 0..20 {
                let a = rand_below(&mut rng, &m);
                let b = rand_below(&mut rng, &m);
                let mut out = vec![0; n];
                eng.mul(&a, &b, &mut out);
                assert_eq!(to_big(&out), to_big(&a) * to_big(&b) * &rinv % &bm);
                eng.mul_mod(&a, &b, &mut out);
                assert_eq!(to_big(&out), to_big(&a) * to_big(&b) % &bm);
            }
            // rasti kufitar: N-1
            let mut nm1 = m.clone();
            nm1[0] -= 1;
            let mut out = vec![0; n];
            eng.mul_mod(&nm1, &nm1, &mut out);
            assert_eq!(to_big(&out), BigUint::from(1u8));
        }
    }

    #[test]
    fn exp_matches_bigint() {
        let mut rng = rand::rngs::StdRng::seed_from_u64(9);
        for n in [1usize, 4, 32, 64] {
            let m = rand_mod(&mut rng, n);
            let eng = MontgomeryEngine::new(&m);
            let base = rand_below(&mut rng, &m);
            let e: Vec<u64> = (0..n.min(4)).map(|_| rng.gen()).collect();
            let expect = to_big(&base).modpow(&to_big(&e), &to_big(&m));
            let mut out = vec![0; n];
            eng.exp_portable(&base, &e, &mut out);
            assert_eq!(to_big(&out), expect);
            eng.exp_vartime(&base, &e, &mut out);
            assert_eq!(to_big(&out), expect);
            eng.exp(&base, &e, &mut out);
            assert_eq!(to_big(&out), expect);
            eng.exp_vartime(&base, &[65537], &mut out);
            assert_eq!(
                to_big(&out),
                to_big(&base).modpow(&BigUint::from(65537u32), &to_big(&m))
            );
            eng.exp(&base, &[0], &mut out);
            assert_eq!(to_big(&out), BigUint::from(1u8));
        }
    }

    #[test]
    fn rejects_bad_modulus() {
        assert_eq!(
            MontgomeryEngine::try_new(&[4]).unwrap_err(),
            MontgomeryError::EvenModulus
        );
        assert_eq!(
            MontgomeryEngine::try_new(&[]).unwrap_err(),
            MontgomeryError::EmptyModulus
        );
        assert_eq!(
            MontgomeryEngine::try_new(&[3, 0]).unwrap_err(),
            MontgomeryError::NotNormalized
        );
    }
}
