//! Prapavijat SIMD për shumëzimin Montgomery.
//!
//! | Arkitektura | Moduli       | Gjendja                                                  |
//! |-------------|--------------|----------------------------------------------------------|
//! | x86_64      | [`avx512`]   | Implementim real me IFMA (radix 2^52), i testuar kundër CIOS |
//! | aarch64     | [`neon`]     | Delegon te CIOS skalar (MUL/UMULH); shih dokumentimin    |
//! | të tjera    | –            | CIOS portabël                                            |

#[cfg(target_arch = "x86_64")]
pub mod avx512;
#[cfg(target_arch = "aarch64")]
pub mod neon;

/// Prapavija që do të përdorë [`crate::MontgomeryEngine::exp`] në këtë makinë.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Portable,
    Avx512Ifma,
    Neon,
}

impl core::fmt::Display for Backend {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Backend::Portable => "portable-cios",
            Backend::Avx512Ifma => "avx512-ifma",
            Backend::Neon => "neon(scalar-umulh)",
        })
    }
}

/// Zbulon prapavijën aktive.
pub fn detect() -> Backend {
    #[cfg(all(target_arch = "x86_64", feature = "std"))]
    if avx512::is_available() {
        return Backend::Avx512Ifma;
    }
    #[cfg(all(
        target_arch = "x86_64",
        not(feature = "std"),
        target_feature = "avx512ifma"
    ))]
    return Backend::Avx512Ifma;
    #[cfg(target_arch = "aarch64")]
    return Backend::Neon;
    #[allow(unreachable_code)]
    Backend::Portable
}
