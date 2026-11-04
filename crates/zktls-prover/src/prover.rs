//! Ekzekutimi i provës Groth16 (BN254) me `ark-groth16` ose `snarkjs`.

use std::path::{Path, PathBuf};
use std::process::Command;

use ark_bn254::{Bn254, Fr};
use ark_ff::{BigInteger, PrimeField};
use ark_groth16::{Groth16, PreparedVerifyingKey, Proof, ProvingKey, VerifyingKey};
use ark_r1cs_std::fields::fp::FpVar;
use ark_r1cs_std::prelude::*;
use ark_relations::r1cs::{ConstraintSynthesizer, ConstraintSystemRef, SynthesisError};
use ark_serialize::{CanonicalDeserialize, CanonicalSerialize};
use ark_snark::SNARK;
use rand::{CryptoRng, RngCore};
use serde::{Deserialize, Serialize};

use crate::solana::{self, ProofJson, SolanaProof, SolanaVerifyingKey};
use crate::witness::ZkWebProofWitness;
use crate::{poseidon, public_layout as L, ProverError};

/// Qarku i zhvillimit (arkworks vendas) me të njëjtat 7 hyrje publike si
/// `circuits/main.circom`. Provon:
/// * `value ∈ [0, 2^64)`, `threshold ∈ [0, 2^64)`, `value − threshold ∈ [0, 2^64)`
/// * `commitment = Poseidon(value, salt)`
///
/// NUK provon nënshkrimin RSA as SHA-256 – këto verifikohen natively nga
/// [`crate::tls_parser`] para provimit. Përdoret për teste, CI dhe demo në
/// localnet; për prodhim regjistroni VK-në e qarkut Circom.
#[derive(Clone, Default)]
pub struct DevClaimCircuit {
    pub value: Option<u64>,
    pub salt: Option<Fr>,
    pub public: Option<[Fr; L::COUNT]>,
}

fn fr_to_u64(x: &Fr) -> Option<u64> {
    let b = x.into_bigint();
    let limbs = b.as_ref();
    if limbs[1..].iter().any(|&l| l != 0) {
        return None;
    }
    Some(limbs[0])
}

fn alloc_u64_bits(
    cs: &ConstraintSystemRef<Fr>,
    v: Option<u64>,
) -> Result<(Vec<Boolean<Fr>>, FpVar<Fr>), SynthesisError> {
    let bits = (0..64)
        .map(|i| {
            Boolean::new_witness(cs.clone(), || {
                v.map(|x| (x >> i) & 1 == 1)
                    .ok_or(SynthesisError::AssignmentMissing)
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let num = Boolean::le_bits_to_fp(&bits)?;
    Ok((bits, num))
}

impl ConstraintSynthesizer<Fr> for DevClaimCircuit {
    fn generate_constraints(self, cs: ConstraintSystemRef<Fr>) -> Result<(), SynthesisError> {
        let pubs = (0..L::COUNT)
            .map(|i| {
                FpVar::new_input(cs.clone(), || {
                    self.public
                        .map(|p| p[i])
                        .ok_or(SynthesisError::AssignmentMissing)
                })
            })
            .collect::<Result<Vec<_>, _>>()?;

        let (_, value) = alloc_u64_bits(&cs, self.value)?;
        let threshold_val = self.public.and_then(|p| fr_to_u64(&p[L::MIN_THRESHOLD]));
        let (_, threshold) = alloc_u64_bits(&cs, threshold_val)?;
        threshold.enforce_equal(&pubs[L::MIN_THRESHOLD])?;
        let diff_val = match (self.value, threshold_val) {
            (Some(v), Some(t)) => Some(v.checked_sub(t).ok_or(SynthesisError::Unsatisfiable)?),
            _ => None,
        };
        let (_, diff) = alloc_u64_bits(&cs, diff_val)?;
        diff.enforce_equal(&(&value - &threshold))?;

        let salt = FpVar::new_witness(cs.clone(), || {
            self.salt.ok_or(SynthesisError::AssignmentMissing)
        })?;
        poseidon::hash_gadget(&[value, salt])?.enforce_equal(&pubs[L::COMMITMENT])?;

        // Lidh hyrjet e tjera publike në sistem (parandalon "malleability" të tyre).
        for i in [
            L::ATTESTED_AT,
            L::CLAIM_KEY_ID,
            L::BANK_KEY_HASH,
            L::USER_PUBKEY_HI,
            L::USER_PUBKEY_LO,
        ] {
            let _ = pubs[i].square()?;
        }
        Ok(())
    }
}

/// Pakoja e provës që ruhet/dërgohet te CLI dhe programi Solana.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofBundle {
    pub backend: String,
    /// Çelësi publik Solana (base58) i lidhur me provën.
    pub wallet: String,
    pub min_threshold: u64,
    pub attested_at: u64,
    #[serde(flatten)]
    pub proof: ProofJson,
}

impl ProofBundle {
    pub fn new(
        backend: &str,
        w: &ZkWebProofWitness,
        proof: &SolanaProof,
        inputs: &[[u8; 32]],
    ) -> Self {
        Self {
            backend: backend.into(),
            wallet: bs58::encode(w.wallet).into_string(),
            min_threshold: w.min_threshold,
            attested_at: w.timestamp,
            proof: proof.to_json(inputs),
        }
    }
}

pub struct DevGroth16Prover {
    pub pk: ProvingKey<Bn254>,
}

impl DevGroth16Prover {
    /// Setup specifik për qarkun (vetëm për zhvillim: toksina njihet nga kush e ekzekuton).
    pub fn setup<R: RngCore + CryptoRng>(rng: &mut R) -> Result<Self, ProverError> {
        let (pk, _vk) = Groth16::<Bn254>::circuit_specific_setup(DevClaimCircuit::default(), rng)?;
        Ok(Self { pk })
    }

    pub fn vk(&self) -> &VerifyingKey<Bn254> {
        &self.pk.vk
    }

    pub fn solana_vk(&self) -> SolanaVerifyingKey {
        SolanaVerifyingKey::from_ark(&self.pk.vk)
    }

    pub fn save(&self, path: &Path) -> Result<(), ProverError> {
        let mut buf = Vec::new();
        self.pk
            .serialize_compressed(&mut buf)
            .map_err(|e| ProverError::Encoding(e.to_string()))?;
        std::fs::write(path, buf)?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self, ProverError> {
        let buf = std::fs::read(path)?;
        let pk = ProvingKey::<Bn254>::deserialize_compressed(&buf[..])
            .map_err(|e| ProverError::Encoding(e.to_string()))?;
        Ok(Self { pk })
    }

    pub fn prove<R: RngCore + CryptoRng>(
        &self,
        w: &ZkWebProofWitness,
        rng: &mut R,
    ) -> Result<(SolanaProof, Vec<[u8; 32]>), ProverError> {
        if w.value < w.min_threshold {
            return Err(ProverError::ThresholdNotMet);
        }
        let public = w.public_inputs();
        let circuit = DevClaimCircuit {
            value: Some(w.value),
            salt: Some(w.user_secret_salt),
            public: Some(public),
        };
        let proof: Proof<Bn254> = Groth16::<Bn254>::prove(&self.pk, circuit, rng)?;
        Ok((SolanaProof::from_ark(&proof), w.public_inputs_be()))
    }
}

/// Verifikim off-chain me arkworks (i pavarur nga formati Solana).
pub fn verify_ark(
    vk: &VerifyingKey<Bn254>,
    proof: &SolanaProof,
    inputs: &[[u8; 32]],
) -> Result<bool, ProverError> {
    let pvk: PreparedVerifyingKey<Bn254> = Groth16::<Bn254>::process_vk(vk)?;
    let inputs = inputs
        .iter()
        .map(solana::fr_from_be)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Groth16::<Bn254>::verify_with_processed_vk(
        &pvk,
        &inputs,
        &proof.to_ark()?,
    )?)
}

/// Prapavija e prodhimit: thërret `snarkjs groth16 fullprove` mbi qarkun Circom.
///
/// STATUS: e pa-testuar në CI (kërkon `circom`, `snarkjs` dhe një `.zkey` të
/// prodhuar nga `circuits/scripts/setup_keys.sh`).
pub struct SnarkjsProver {
    pub wasm: PathBuf,
    pub zkey: PathBuf,
    pub snarkjs_bin: String,
    pub work_dir: PathBuf,
}

impl SnarkjsProver {
    pub fn prove(
        &self,
        w: &ZkWebProofWitness,
    ) -> Result<(SolanaProof, Vec<[u8; 32]>), ProverError> {
        std::fs::create_dir_all(&self.work_dir)?;
        let input = self.work_dir.join("input.json");
        let proof_p = self.work_dir.join("proof.json");
        let public_p = self.work_dir.join("public.json");
        std::fs::write(&input, serde_json::to_vec_pretty(&w.to_circom_input())?)?;
        let out = Command::new(&self.snarkjs_bin)
            .args(["groth16", "fullprove"])
            .arg(&input)
            .arg(&self.wasm)
            .arg(&self.zkey)
            .arg(&proof_p)
            .arg(&public_p)
            .output()
            .map_err(|e| ProverError::Snarkjs(format!("nuk u nis `{}`: {e}", self.snarkjs_bin)))?;
        if !out.status.success() {
            return Err(ProverError::Snarkjs(
                String::from_utf8_lossy(&out.stderr).into_owned(),
            ));
        }
        let proof = solana::proof_from_snarkjs(&std::fs::read_to_string(&proof_p)?)?;
        let public = solana::public_from_snarkjs(&std::fs::read_to_string(&public_p)?)?;
        if public != w.public_inputs_be() {
            return Err(ProverError::Snarkjs(
                "public.json nuk përputhet me hyrjet e pritura".into(),
            ));
        }
        Ok((SolanaProof::from_ark(&proof), public))
    }
}

/// Konverton `Fr` në dhjetor (si në JSON-in e snarkjs).
pub fn fr_to_decimal(x: &Fr) -> String {
    num_bigint::BigUint::from_bytes_le(&x.into_bigint().to_bytes_le()).to_string()
}
