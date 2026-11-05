//! # zktls-prover
//!
//! Rrjedha e plotë off-chain:
//!
//! ```text
//! sesion i noterizuar (JSON) ──► tls_parser ──► VerifiedSession
//!        (HTTP + atestim EdDSA)        │  (BabyJubjub EdDSA-Poseidon,
//!                                    │   normalizer MED-01, kontroll digest-i)
//!                                    ▼
//!                               witness ──► input.json për circuits/main.circom
//!                                    │
//!                                    ▼
//!                               prover ──► Groth16/BN254 ──► solana (bajte për programin)
//! ```
//!
//! Dy prapavija provimi:
//! * [`prover::SnarkjsProver`] – prodhim: qarku i plotë Circom (EdDSA-Poseidon
//!   + kushti) përmes `snarkjs groth16 fullprove`. Kërkon artefaktet e kompiluara.
//! * [`prover::DevGroth16Prover`] – zhvillim: qark arkworks vendas që provon
//!   kushtin `min ≤ value ≤ max`, zotimin dhe nullifier-in Poseidon, me të njëjtat 10 hyrje publike.
//!   EdDSA verifikohet natively (jashtë provës) – **jo i sigurt për prodhim**.

pub mod attestation;
pub mod eddsa;
pub mod error;
pub mod mock;
pub mod poseidon;
pub mod prover;
pub mod solana;
pub mod tls_parser;
pub mod witness;

pub use attestation::AttestationMessage;
pub use error::ProverError;
pub use tls_parser::{NotarizedSession, VerifiedSession};

/// Rendi i hyrjeve publike (i njëjtë për qarkun Circom, qarkun dev dhe programin Solana).
pub mod public_layout {
    // Daljet e qarkut (renditen të parat në public.json)
    pub const COMMITMENT: usize = 0;
    pub const ATTESTED_AT: usize = 1;
    pub const CLAIM_KEY_ID: usize = 2;
    pub const NULLIFIER: usize = 3;
    // Hyrjet publike (sipas rendit të deklarimit në main.circom)
    pub const BANK_KEY_HASH: usize = 4;
    pub const MIN_VALUE: usize = 5;
    pub const MAX_VALUE: usize = 6;
    pub const APP_ID: usize = 7;
    pub const USER_PUBKEY_HI: usize = 8;
    pub const USER_PUBKEY_LO: usize = 9;
    pub const COUNT: usize = 10;
}
