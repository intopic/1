//! ARM64 (Apple Silicon / AWS Graviton).
//!
//! Vendim teknik: Neon nuk ka shumëzim vektorial 64×64→128 bit (maksimumi është
//! `vmull_u32`, 32×32→64). Një CIOS me fjalë 32-bit në Neon kryen 4× më shumë
//! produkte të pjesshme se versioni skalar 64-bit, ku `u128` kompilohet në
//! çiftin `MUL`/`UMULH` (2 instruksione, latencë ~3-4 cikle në M1/Graviton3).
//! Matjet publike (p.sh. OpenSSL/BoringSSL për RSA në aarch64) përdorin gjithashtu
//! rrugën skalare. Prandaj ky modul delegon te CIOS portabël, i cili në aarch64
//! kompilohet tashmë në kod optimal.
//!
//! STATUS: i pa-testuar në hardware ARM në këtë depo (CI vetëm x86_64).

use crate::MontgomeryEngine;

/// `out = a·b·R^-1 mod N` – në aarch64 thjesht CIOS skalar.
#[inline]
pub fn montgomery_mul_neon(engine: &MontgomeryEngine, a: &[u64], b: &[u64], out: &mut [u64]) {
    engine.mul_cios_portable(a, b, out);
}
