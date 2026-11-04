//! Gjenerimi i dëshmitarit (`input.json`) për `circuits/main.circom`.
//!
//! Numrat RSA ndahen në 17 copa × 121 bit (formati i `@zk-email/circuits`
//! `RSAVerifier65537(121, 17)`): produkti i dy copave (242 bit) plus akumulimi
//! mbetet nën modulin e fushës BN254 (254 bit), çka nuk do të ishte e sigurt
//! me fjalë 64-bit pa kontrolle shtesë të bartjeve.

use ark_bn254::Fr;
use ark_ff::{BigInteger, PrimeField, UniformRand};
use num_bigint::BigUint;
use serde_json::{json, Value};

use crate::poseidon;
use crate::solana::fr_to_be;
use crate::tls_parser::VerifiedSession;
use crate::{public_layout as L, ProverError};

pub const RSA_CHUNK_BITS: usize = 121;
pub const RSA_CHUNKS: usize = 17;

#[derive(Debug, Clone)]
pub struct ZkWebProofWitness {
    pub attestation_msg: [u8; 128],
    pub user_secret_salt: Fr,
    pub rsa_signature_be: Vec<u8>,
    pub rsa_modulus_be: Vec<u8>,
    pub min_threshold: u64,
    pub value: u64,
    pub timestamp: u64,
    pub claim_key_id: u64,
    pub wallet: [u8; 32],
}

/// Ndan çelësin publik Solana (32 bajte) në dy gjysma 128-bit (BE) → dy elementë Fr.
pub fn wallet_halves(w: &[u8; 32]) -> (Fr, Fr) {
    (
        Fr::from_be_bytes_mod_order(&w[..16]),
        Fr::from_be_bytes_mod_order(&w[16..]),
    )
}

impl ZkWebProofWitness {
    pub fn from_session(
        vs: &VerifiedSession,
        min_threshold: u64,
        salt: Option<Fr>,
    ) -> Result<Self, ProverError> {
        if vs.message.value < min_threshold {
            return Err(ProverError::ThresholdNotMet);
        }
        let salt = salt.unwrap_or_else(|| Fr::rand(&mut rand::thread_rng()));
        Ok(Self {
            attestation_msg: vs.message_bytes,
            user_secret_salt: salt,
            rsa_signature_be: vs.signature.clone(),
            rsa_modulus_be: vs.notary_key.modulus_be_bytes(),
            min_threshold,
            value: vs.message.value,
            timestamp: vs.message.timestamp,
            claim_key_id: vs.message.claim_key_id,
            wallet: vs.message.wallet,
        })
    }

    pub fn commitment(&self) -> Fr {
        poseidon::hash(&[Fr::from(self.value), self.user_secret_salt])
    }

    pub fn bank_key_hash(&self) -> Fr {
        poseidon::rsa_modulus_hash(&self.rsa_modulus_be)
    }

    /// Hyrjet publike në rendin e `public_layout` (dalje Circom së pari, pastaj hyrje).
    pub fn public_inputs(&self) -> [Fr; L::COUNT] {
        let (hi, lo) = wallet_halves(&self.wallet);
        let mut p = [Fr::from(0u64); L::COUNT];
        p[L::COMMITMENT] = self.commitment();
        p[L::ATTESTED_AT] = Fr::from(self.timestamp);
        p[L::CLAIM_KEY_ID] = Fr::from(self.claim_key_id);
        p[L::BANK_KEY_HASH] = self.bank_key_hash();
        p[L::MIN_THRESHOLD] = Fr::from(self.min_threshold);
        p[L::USER_PUBKEY_HI] = hi;
        p[L::USER_PUBKEY_LO] = lo;
        p
    }

    pub fn public_inputs_be(&self) -> Vec<[u8; 32]> {
        self.public_inputs().iter().map(fr_to_be).collect()
    }

    /// `input.json` për `snarkjs groth16 fullprove` / `circom` witness calculator.
    pub fn to_circom_input(&self) -> Value {
        let dec = |x: &Fr| BigUint::from_bytes_le(&x.into_bigint().to_bytes_le()).to_string();
        let chunks = |be: &[u8]| -> Vec<String> {
            poseidon::to_chunks(be, RSA_CHUNK_BITS, RSA_CHUNKS)
                .iter()
                .map(|c| c.to_string())
                .collect()
        };
        let (hi, lo) = wallet_halves(&self.wallet);
        json!({
            "attestation_msg": self.attestation_msg.iter().map(|b| b.to_string()).collect::<Vec<_>>(),
            "user_secret_salt": dec(&self.user_secret_salt),
            "rsa_signature": chunks(&self.rsa_signature_be),
            "rsa_modulus": chunks(&self.rsa_modulus_be),
            "bank_public_key_hash": dec(&self.bank_key_hash()),
            "min_threshold": self.min_threshold.to_string(),
            "user_pubkey_hi": dec(&hi),
            "user_pubkey_lo": dec(&lo),
        })
    }

    /// Sinjalet publike që pritet të dalin nga snarkjs (`public.json`), si dhjetorë.
    pub fn expected_public_json(&self) -> Value {
        let dec = |x: &Fr| BigUint::from_bytes_le(&x.into_bigint().to_bytes_le()).to_string();
        Value::Array(
            self.public_inputs()
                .iter()
                .map(|x| Value::String(dec(x)))
                .collect(),
        )
    }
}
