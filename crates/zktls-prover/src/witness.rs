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

/// Diapazoni i pretenduar `min ≤ value ≤ max`.
///
/// * bilanc ≥ 10 000: `ValueRange::at_least(10_000)`
/// * mosha ≥ 18: `ValueRange::born_on_or_before(20081104)` (data si YYYYMMDD)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValueRange {
    pub min: u64,
    pub max: u64,
}

impl ValueRange {
    pub fn at_least(min: u64) -> Self {
        Self { min, max: u64::MAX }
    }
    pub fn at_most(max: u64) -> Self {
        Self { min: 0, max }
    }
    pub fn born_on_or_before(yyyymmdd: u64) -> Self {
        Self::at_most(yyyymmdd)
    }
}

/// Mosha maksimale që pranohet në `--min-age`.
pub const MAX_AGE_YEARS: u64 = 150;

/// Çelësat JSON që mbajnë datën e lindjes: vetëm këto lejohen për prova moshe.
pub const BIRTH_DATE_KEYS: [&str; 4] = ["date_of_birth", "birth_date", "birthdate", "dob"];

/// Data limit e lindjes për moshë minimale: `today − years` si YYYYMMDD.
/// (29 shkurti trajtohet si 28 shkurt kur viti limit nuk është i brishtë.)
/// `None` për data/mosha të pavlefshme (p.sh. `years > 150`), pa panik.
pub fn birth_cutoff(today_yyyymmdd: u64, years: u64) -> Option<u64> {
    let (y, m, d) = (
        today_yyyymmdd / 10_000,
        (today_yyyymmdd / 100) % 100,
        today_yyyymmdd % 100,
    );
    if years > MAX_AGE_YEARS || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let cy = y.checked_sub(years).filter(|&c| c > 0)?;
    let leap = (cy.is_multiple_of(4) && !cy.is_multiple_of(100)) || cy.is_multiple_of(400);
    let d = if m == 2 && d == 29 && !leap { 28 } else { d };
    Some(cy * 10_000 + m * 100 + d)
}

/// `app_id` nga një emër tekst (p.sh. "airdrop-jup-2026"): SHA-256 me bajtin e parë 0.
pub fn app_id_from_label(label: &str) -> Fr {
    use sha2::{Digest, Sha256};
    let mut h: [u8; 32] = Sha256::digest(format!("zktls/app/{label}").as_bytes()).into();
    h[0] = 0;
    Fr::from_be_bytes_mod_order(&h)
}

#[derive(Debug, Clone)]
pub struct ZkWebProofWitness {
    pub attestation_msg: [u8; 128],
    pub user_secret_salt: Fr,
    pub rsa_signature_be: Vec<u8>,
    pub rsa_modulus_be: Vec<u8>,
    pub min_value: u64,
    pub max_value: u64,
    /// Identifikuesi i aplikacionit/kampanjës (element Fr) – i njëjti nullifier
    /// përsëritet vetëm brenda të njëjtit `app_id`.
    pub app_id: Fr,
    pub account_tag: [u8; 32],
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
        range: ValueRange,
        app_id: Fr,
        salt: Option<Fr>,
    ) -> Result<Self, ProverError> {
        if vs.message.value < range.min || vs.message.value > range.max {
            return Err(ProverError::ThresholdNotMet);
        }
        let salt = salt.unwrap_or_else(|| Fr::rand(&mut rand::thread_rng()));
        Ok(Self {
            attestation_msg: vs.message_bytes,
            user_secret_salt: salt,
            rsa_signature_be: vs.signature.clone(),
            rsa_modulus_be: vs.notary_key.modulus_be_bytes(),
            min_value: range.min,
            max_value: range.max,
            app_id,
            account_tag: vs.message.account_tag,
            value: vs.message.value,
            timestamp: vs.message.timestamp,
            claim_key_id: vs.message.claim_key_id,
            wallet: vs.message.wallet,
        })
    }

    pub fn commitment(&self) -> Fr {
        poseidon::hash(&[Fr::from(self.value), self.user_secret_salt])
    }

    pub fn account_tag_fr(&self) -> Fr {
        Fr::from_be_bytes_mod_order(&self.account_tag)
    }

    /// `Poseidon(account_tag, app_id)` – unik për (llogari, aplikacion).
    pub fn nullifier(&self) -> Fr {
        poseidon::hash(&[self.account_tag_fr(), self.app_id])
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
        p[L::NULLIFIER] = self.nullifier();
        p[L::BANK_KEY_HASH] = self.bank_key_hash();
        p[L::MIN_VALUE] = Fr::from(self.min_value);
        p[L::MAX_VALUE] = Fr::from(self.max_value);
        p[L::APP_ID] = self.app_id;
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
            "min_value": self.min_value.to_string(),
            "max_value": self.max_value.to_string(),
            "app_id": dec(&self.app_id),
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cutoff() {
        assert_eq!(birth_cutoff(20261104, 18), Some(20081104));
        assert_eq!(birth_cutoff(20240229, 18), Some(20060228));
        assert_eq!(birth_cutoff(20240229, 20), Some(20040229));
        // pa tejmbushje/panik
        assert_eq!(birth_cutoff(20261104, 2027), None);
        assert_eq!(birth_cutoff(20261104, u64::MAX), None);
        assert_eq!(birth_cutoff(0, 18), None);
        assert_ne!(app_id_from_label("a"), app_id_from_label("b"));
    }
}
