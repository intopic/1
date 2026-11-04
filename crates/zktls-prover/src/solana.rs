//! Kodimi i provave dhe çelësave Groth16 në formatin që pranon programi
//! `solana-zk-verifier` (EIP-197 / syscall-et `alt_bn128` të Solana-s):
//!
//! * G1 = `x ‖ y` (32 + 32 bajte, big-endian) → 64 bajte
//! * G2 = `x.c1 ‖ x.c0 ‖ y.c1 ‖ y.c0` (pjesa imagjinare së pari) → 128 bajte
//! * Fr = 32 bajte big-endian
//!
//! Prova = A (64) + B (128) + C (64) = 256 bajte e pakompresuar
//! (128 bajte në formë të kompresuar – Solana kërkon formën e pakompresuar për syscall).
//! Programi on-chain e mohon vetë `A`, ndaj klienti dërgon provën ashtu siç është.

use ark_bn254::{Bn254, Fq, Fq2, Fr, G1Affine, G2Affine};
use ark_ec::{pairing::Pairing, AffineRepr, CurveGroup};
use ark_ff::{BigInteger, PrimeField, Zero};
use ark_groth16::{Proof, VerifyingKey};
use num_bigint::BigUint;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

use crate::ProverError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SolanaProof {
    pub a: [u8; 64],
    pub b: [u8; 128],
    pub c: [u8; 64],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SolanaVerifyingKey {
    pub alpha_g1: [u8; 64],
    pub beta_g2: [u8; 128],
    pub gamma_g2: [u8; 128],
    pub delta_g2: [u8; 128],
    pub ic: Vec<[u8; 64]>,
}

/// Forma JSON (hex) për CLI dhe testet TypeScript.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProofJson {
    pub proof_a: String,
    pub proof_b: String,
    pub proof_c: String,
    pub public_inputs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VerifyingKeyJson {
    pub alpha_g1: String,
    pub beta_g2: String,
    pub gamma_g2: String,
    pub delta_g2: String,
    pub ic: Vec<String>,
}

fn fq_be(x: &Fq) -> [u8; 32] {
    let mut o = [0u8; 32];
    o.copy_from_slice(&x.into_bigint().to_bytes_be());
    o
}

pub fn fr_to_be(x: &Fr) -> [u8; 32] {
    let mut o = [0u8; 32];
    o.copy_from_slice(&x.into_bigint().to_bytes_be());
    o
}

pub fn fr_from_be(b: &[u8; 32]) -> Result<Fr, ProverError> {
    let x = BigUint::from_bytes_be(b);
    let r: BigUint = Fr::MODULUS.into();
    if x >= r {
        return Err(ProverError::Encoding("hyrja publike >= r".into()));
    }
    Ok(Fr::from_be_bytes_mod_order(b))
}

pub fn g1_to_bytes(p: &G1Affine) -> [u8; 64] {
    let mut o = [0u8; 64];
    if p.is_zero() {
        return o;
    }
    o[..32].copy_from_slice(&fq_be(&p.x));
    o[32..].copy_from_slice(&fq_be(&p.y));
    o
}

pub fn g2_to_bytes(p: &G2Affine) -> [u8; 128] {
    let mut o = [0u8; 128];
    if p.is_zero() {
        return o;
    }
    o[0..32].copy_from_slice(&fq_be(&p.x.c1));
    o[32..64].copy_from_slice(&fq_be(&p.x.c0));
    o[64..96].copy_from_slice(&fq_be(&p.y.c1));
    o[96..128].copy_from_slice(&fq_be(&p.y.c0));
    o
}

fn fq_from_be(b: &[u8]) -> Result<Fq, ProverError> {
    let x = BigUint::from_bytes_be(b);
    let q: BigUint = Fq::MODULUS.into();
    if x >= q {
        return Err(ProverError::Encoding("koordinata >= q".into()));
    }
    Ok(Fq::from_be_bytes_mod_order(b))
}

pub fn g1_from_bytes(b: &[u8; 64]) -> Result<G1Affine, ProverError> {
    if b.iter().all(|&x| x == 0) {
        return Ok(G1Affine::zero());
    }
    let p = G1Affine::new_unchecked(fq_from_be(&b[..32])?, fq_from_be(&b[32..])?);
    if !p.is_on_curve() {
        return Err(ProverError::Encoding("pika G1 jashtë kurbës".into()));
    }
    Ok(p)
}

pub fn g2_from_bytes(b: &[u8; 128]) -> Result<G2Affine, ProverError> {
    if b.iter().all(|&x| x == 0) {
        return Ok(G2Affine::zero());
    }
    let x = Fq2::new(fq_from_be(&b[32..64])?, fq_from_be(&b[0..32])?);
    let y = Fq2::new(fq_from_be(&b[96..128])?, fq_from_be(&b[64..96])?);
    let p = G2Affine::new_unchecked(x, y);
    if !p.is_on_curve() || !p.is_in_correct_subgroup_assuming_on_curve() {
        return Err(ProverError::Encoding("pika G2 e pavlefshme".into()));
    }
    Ok(p)
}

impl SolanaProof {
    pub fn from_ark(p: &Proof<Bn254>) -> Self {
        Self {
            a: g1_to_bytes(&p.a),
            b: g2_to_bytes(&p.b),
            c: g1_to_bytes(&p.c),
        }
    }
    pub fn to_ark(&self) -> Result<Proof<Bn254>, ProverError> {
        Ok(Proof {
            a: g1_from_bytes(&self.a)?,
            b: g2_from_bytes(&self.b)?,
            c: g1_from_bytes(&self.c)?,
        })
    }
    pub fn to_json(&self, inputs: &[[u8; 32]]) -> ProofJson {
        ProofJson {
            proof_a: hex::encode(self.a),
            proof_b: hex::encode(self.b),
            proof_c: hex::encode(self.c),
            public_inputs: inputs.iter().map(hex::encode).collect(),
        }
    }
}

impl ProofJson {
    pub fn decode(&self) -> Result<(SolanaProof, Vec<[u8; 32]>), ProverError> {
        let arr = |s: &str, n: usize| -> Result<Vec<u8>, ProverError> {
            let v = hex::decode(s)?;
            if v.len() != n {
                return Err(ProverError::Encoding(format!("gjatësi {} != {n}", v.len())));
            }
            Ok(v)
        };
        let proof = SolanaProof {
            a: arr(&self.proof_a, 64)?.try_into().unwrap(),
            b: arr(&self.proof_b, 128)?.try_into().unwrap(),
            c: arr(&self.proof_c, 64)?.try_into().unwrap(),
        };
        let inputs = self
            .public_inputs
            .iter()
            .map(|s| Ok(arr(s, 32)?.try_into().unwrap()))
            .collect::<Result<Vec<[u8; 32]>, ProverError>>()?;
        Ok((proof, inputs))
    }
}

impl SolanaVerifyingKey {
    pub fn from_ark(vk: &VerifyingKey<Bn254>) -> Self {
        Self {
            alpha_g1: g1_to_bytes(&vk.alpha_g1),
            beta_g2: g2_to_bytes(&vk.beta_g2),
            gamma_g2: g2_to_bytes(&vk.gamma_g2),
            delta_g2: g2_to_bytes(&vk.delta_g2),
            ic: vk.gamma_abc_g1.iter().map(g1_to_bytes).collect(),
        }
    }
    pub fn to_ark(&self) -> Result<VerifyingKey<Bn254>, ProverError> {
        Ok(VerifyingKey {
            alpha_g1: g1_from_bytes(&self.alpha_g1)?,
            beta_g2: g2_from_bytes(&self.beta_g2)?,
            gamma_g2: g2_from_bytes(&self.gamma_g2)?,
            delta_g2: g2_from_bytes(&self.delta_g2)?,
            gamma_abc_g1: self
                .ic
                .iter()
                .map(g1_from_bytes)
                .collect::<Result<_, _>>()?,
        })
    }
    pub fn to_json(&self) -> VerifyingKeyJson {
        VerifyingKeyJson {
            alpha_g1: hex::encode(self.alpha_g1),
            beta_g2: hex::encode(self.beta_g2),
            gamma_g2: hex::encode(self.gamma_g2),
            delta_g2: hex::encode(self.delta_g2),
            ic: self.ic.iter().map(hex::encode).collect(),
        }
    }
    pub fn from_json(j: &VerifyingKeyJson) -> Result<Self, ProverError> {
        let d = |s: &str| hex::decode(s);
        Ok(Self {
            alpha_g1: d(&j.alpha_g1)?
                .try_into()
                .map_err(|_| ProverError::Encoding("alpha".into()))?,
            beta_g2: d(&j.beta_g2)?
                .try_into()
                .map_err(|_| ProverError::Encoding("beta".into()))?,
            gamma_g2: d(&j.gamma_g2)?
                .try_into()
                .map_err(|_| ProverError::Encoding("gamma".into()))?,
            delta_g2: d(&j.delta_g2)?
                .try_into()
                .map_err(|_| ProverError::Encoding("delta".into()))?,
            ic: j
                .ic
                .iter()
                .map(|s| {
                    d(s)?
                        .try_into()
                        .map_err(|_| ProverError::Encoding("ic".into()))
                })
                .collect::<Result<_, ProverError>>()?,
        })
    }
}

/// Implementim referencë i verifikimit on-chain (i njëjti ekuacion që programi
/// llogarit me `alt_bn128_pairing`):
/// `e(−A, B) · e(α, β) · e(vk_x, γ) · e(C, δ) == 1`.
pub fn verify_reference(
    vk: &SolanaVerifyingKey,
    proof: &SolanaProof,
    inputs: &[[u8; 32]],
) -> Result<bool, ProverError> {
    if inputs.len() + 1 != vk.ic.len() {
        return Err(ProverError::Encoding("numri i hyrjeve publike".into()));
    }
    let mut vk_x = g1_from_bytes(&vk.ic[0])?.into_group();
    for (i, inp) in inputs.iter().enumerate() {
        vk_x += g1_from_bytes(&vk.ic[i + 1])? * fr_from_be(inp)?;
    }
    let neg_a = -g1_from_bytes(&proof.a)?;
    let r = Bn254::multi_pairing(
        [
            neg_a,
            g1_from_bytes(&vk.alpha_g1)?,
            vk_x.into_affine(),
            g1_from_bytes(&proof.c)?,
        ],
        [
            g2_from_bytes(&proof.b)?,
            g2_from_bytes(&vk.beta_g2)?,
            g2_from_bytes(&vk.gamma_g2)?,
            g2_from_bytes(&vk.delta_g2)?,
        ],
    );
    Ok(r.is_zero())
}

// ---------- snarkjs JSON (proof.json / verification_key.json / public.json) ----------

#[derive(Debug, Deserialize, Serialize)]
pub struct SnarkjsProof {
    pub pi_a: Vec<String>,
    pub pi_b: Vec<Vec<String>>,
    pub pi_c: Vec<String>,
    #[serde(default)]
    pub protocol: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct SnarkjsVk {
    pub vk_alpha_1: Vec<String>,
    pub vk_beta_2: Vec<Vec<String>>,
    pub vk_gamma_2: Vec<Vec<String>>,
    pub vk_delta_2: Vec<Vec<String>>,
    #[serde(rename = "IC")]
    pub ic: Vec<Vec<String>>,
}

fn dec_fq(s: &str) -> Result<Fq, ProverError> {
    Fq::from_str(s).map_err(|_| ProverError::Encoding(format!("Fq: {s}")))
}

fn sj_g1(v: &[String]) -> Result<G1Affine, ProverError> {
    if v.len() < 2 {
        return Err(ProverError::Encoding("G1 snarkjs".into()));
    }
    let p = G1Affine::new_unchecked(dec_fq(&v[0])?, dec_fq(&v[1])?);
    if !p.is_on_curve() {
        return Err(ProverError::Encoding("G1 snarkjs jashtë kurbës".into()));
    }
    Ok(p)
}

/// snarkjs ruan G2 si `[[x.c0, x.c1], [y.c0, y.c1], [1, 0]]`.
fn sj_g2(v: &[Vec<String>]) -> Result<G2Affine, ProverError> {
    if v.len() < 2 || v[0].len() != 2 || v[1].len() != 2 {
        return Err(ProverError::Encoding("G2 snarkjs".into()));
    }
    let p = G2Affine::new_unchecked(
        Fq2::new(dec_fq(&v[0][0])?, dec_fq(&v[0][1])?),
        Fq2::new(dec_fq(&v[1][0])?, dec_fq(&v[1][1])?),
    );
    if !p.is_on_curve() {
        return Err(ProverError::Encoding("G2 snarkjs jashtë kurbës".into()));
    }
    Ok(p)
}

pub fn proof_from_snarkjs(json: &str) -> Result<Proof<Bn254>, ProverError> {
    let p: SnarkjsProof = serde_json::from_str(json)?;
    Ok(Proof {
        a: sj_g1(&p.pi_a)?,
        b: sj_g2(&p.pi_b)?,
        c: sj_g1(&p.pi_c)?,
    })
}

pub fn vk_from_snarkjs(json: &str) -> Result<VerifyingKey<Bn254>, ProverError> {
    let v: SnarkjsVk = serde_json::from_str(json)?;
    Ok(VerifyingKey {
        alpha_g1: sj_g1(&v.vk_alpha_1)?,
        beta_g2: sj_g2(&v.vk_beta_2)?,
        gamma_g2: sj_g2(&v.vk_gamma_2)?,
        delta_g2: sj_g2(&v.vk_delta_2)?,
        gamma_abc_g1: v.ic.iter().map(|p| sj_g1(p)).collect::<Result<_, _>>()?,
    })
}

pub fn public_from_snarkjs(json: &str) -> Result<Vec<[u8; 32]>, ProverError> {
    let v: Vec<String> = serde_json::from_str(json)?;
    v.iter()
        .map(|s| {
            let x = Fr::from_str(s).map_err(|_| ProverError::Encoding(format!("Fr: {s}")))?;
            Ok(fr_to_be(&x))
        })
        .collect()
}

/// Konverton një provë arkworks në JSON snarkjs (për testim të përputhshmërisë).
pub fn proof_to_snarkjs(p: &Proof<Bn254>) -> SnarkjsProof {
    let s = |x: &Fq| x.into_bigint().to_string();
    SnarkjsProof {
        pi_a: vec![s(&p.a.x), s(&p.a.y), "1".into()],
        pi_b: vec![
            vec![s(&p.b.x.c0), s(&p.b.x.c1)],
            vec![s(&p.b.y.c0), s(&p.b.y.c1)],
            vec!["1".into(), "0".into()],
        ],
        pi_c: vec![s(&p.c.x), s(&p.c.y), "1".into()],
        protocol: "groth16".into(),
    }
}
