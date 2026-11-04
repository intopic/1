//! # triad-core
//!
//! Motori kriptografik bazë i `solana-zktls-coprocessor`. Përmban tre module
//! ("Treshja Algoritmike"):
//!
//! | Kodi   | Moduli          | Përmbajtja                                              |
//! |--------|-----------------|---------------------------------------------------------|
//! | MED-01 | [`normalizer`]  | Al-Jabr: maskim + nxjerrje kanonike e pretendimit Web2  |
//! | ANT-05 | [`euclid`]      | Stein GCD, inversi modular në kohë konstante, `-N^-1`   |
//! | MOD-06 | [`montgomery`]  | Montgomery CIOS 2048/4096-bit (portabël + AVX-512 IFMA) |
//!
//! Crate-i është `no_std` (me `alloc`) kur hiqet feature `std`. Me `std`,
//! [`montgomery::MontgomeryEngine::exp`] zbulon AVX-512 IFMA në kohë ekzekutimi.
//!
//! Shënim mbi kohën konstante: shumëzimi, zbritja përfundimtare, fuqizimi me
//! dritare fikse dhe inversi modular nuk degëzohen sipas vlerave sekrete
//! (përdoret `subtle`). Moduli `N` dhe gjatësitë konsiderohen publike.

#![cfg_attr(not(feature = "std"), no_std)]
#![deny(unsafe_op_in_unsafe_fn)]
#![allow(clippy::needless_range_loop)]

extern crate alloc;

pub mod bigint;
pub mod euclid;
pub mod montgomery;
pub mod normalizer;
pub mod rsa;
pub mod simd;

pub use euclid::EuclidCore;
pub use montgomery::MontgomeryEngine;
pub use normalizer::{AlgebraicNormalizer, CanonicalClaim, NormalizerError};
