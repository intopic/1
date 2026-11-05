//! circomlib EdDSA-Poseidon: BabyJubjub (a=168700), BLAKE-512 seed expansion.
//! Wire coordinates/scalars are canonical decimal strings, never reduced on input.
use ark_bn254::Fr;
use ark_ec::{
    twisted_edwards::{Affine, MontCurveConfig, TECurveConfig},
    AffineRepr, CurveConfig, CurveGroup,
};
use ark_ed_on_bn254::Fr as Scalar;
use ark_ff::{BigInteger, MontFp, PrimeField};
use num_bigint::BigUint;
use serde::{Deserialize, Serialize};

use crate::{poseidon, ProverError};

pub const SCHEME: &str = "babyjubjub-eddsa-poseidon-v3";
pub const MESSAGE_DOMAIN: &[u8] = b"zktls/attestation/v3";
pub const KEY_DOMAIN: &[u8] = b"zktls/notary-key/v3";

// ark-ed-on-bn254 uses isomorphic coordinates with a=1. Use circomlib's
// original coordinates and Base8, retaining arkworks' subgroup scalar field.
#[derive(Clone, Default, PartialEq, Eq)]
struct BabyJub;
impl CurveConfig for BabyJub {
    type BaseField = Fr;
    type ScalarField = Scalar;
    const COFACTOR: &'static [u64] = &[8];
    const COFACTOR_INV: Scalar =
        MontFp!("2394026564107420727433200628387514462817212225638746351800188703329891451411");
}
impl TECurveConfig for BabyJub {
    const COEFF_A: Fr = MontFp!("168700");
    const COEFF_D: Fr = MontFp!("168696");
    const GENERATOR: Affine<Self> = Affine::new_unchecked(
        MontFp!("5299619240641551281634865583518297030282874472190772894086521144482721001553"),
        MontFp!("16950150798460657717958625567821834550301663161624707787222815936182638968203"),
    );
    type MontCurveConfig = Self;
}
impl MontCurveConfig for BabyJub {
    const COEFF_A: Fr = MontFp!("168698");
    const COEFF_B: Fr = MontFp!("1");
    type TECurveConfig = Self;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicKey {
    pub x: String,
    pub y: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Signature {
    pub r8x: String,
    pub r8y: String,
    pub s: String,
}

pub fn decimal<F: PrimeField>(v: &F) -> String {
    BigUint::from_bytes_le(&v.into_bigint().to_bytes_le()).to_string()
}

fn parse<F: PrimeField>(s: &str) -> Result<F, ProverError> {
    let v = s
        .parse::<F>()
        .map_err(|_| ProverError::Key("invalid field element".into()))?;
    if decimal(&v) != s {
        return Err(ProverError::Key("non-canonical field element".into()));
    }
    Ok(v)
}

fn point(x: &str, y: &str) -> Result<Affine<BabyJub>, ProverError> {
    let p = Affine::<BabyJub>::new_unchecked(parse(x)?, parse(y)?);
    if p.is_zero() || !p.is_on_curve() || !p.is_in_correct_subgroup_assuming_on_curve() {
        return Err(ProverError::Key("invalid BabyJubjub subgroup point".into()));
    }
    Ok(p)
}

impl PublicKey {
    pub fn coordinates(&self) -> Result<[Fr; 2], ProverError> {
        let p = point(&self.x, &self.y)?;
        Ok([p.x, p.y])
    }
    pub fn hash(&self) -> Result<Fr, ProverError> {
        let [x, y] = self.coordinates()?;
        Ok(poseidon::hash(&[
            Fr::from_be_bytes_mod_order(KEY_DOMAIN),
            x,
            y,
        ]))
    }
    pub fn verify(&self, message: &[u8; 128], sig: &Signature) -> Result<(), ProverError> {
        let a = point(&self.x, &self.y)?;
        let r = point(&sig.r8x, &sig.r8y)?;
        let s = parse::<Scalar>(&sig.s)?;
        let h = poseidon::hash(&[r.x, r.y, a.x, a.y, message_hash(message)]);
        let h = Scalar::from_le_bytes_mod_order(&h.into_bigint().to_bytes_le());
        let left = BabyJub::GENERATOR * s;
        let right = r.into_group() + a * (h * Scalar::from(8u64));
        if left != right {
            return Err(ProverError::Attestation(
                "invalid EdDSA-Poseidon signature".into(),
            ));
        }
        Ok(())
    }
}

/// Domain + five BE limbs of at most 31 bytes. No limb reaches the field modulus;
/// all 128 bytes (including magic and payload digest) are bound injectively.
pub fn message_hash(message: &[u8; 128]) -> Fr {
    let mut limbs = vec![Fr::from_be_bytes_mod_order(MESSAGE_DOMAIN)];
    limbs.extend(message.chunks(31).map(Fr::from_be_bytes_mod_order));
    poseidon::hash(&limbs)
}

pub struct SigningKey([u8; 32]);
impl SigningKey {
    pub fn from_hex(s: &str) -> Result<Self, ProverError> {
        let seed = hex::decode(s.trim())?;
        Ok(Self(seed.try_into().map_err(|_| {
            ProverError::Key("EdDSA seed must be 32 bytes".into())
        })?))
    }
    pub fn from_seed(seed: [u8; 32]) -> Self {
        Self(seed)
    }
    fn expanded(&self) -> [u8; 64] {
        use blake_hash::Digest;
        let mut h: [u8; 64] = blake_hash::Blake512::digest(&self.0).into();
        h[0] &= 0xf8;
        h[31] &= 0x7f;
        h[31] |= 0x40;
        h
    }
    pub fn public(&self) -> PublicKey {
        let scalar = BigUint::from_bytes_le(&self.expanded()[..32]) >> 3usize;
        let a = (BabyJub::GENERATOR * Scalar::from_le_bytes_mod_order(&scalar.to_bytes_le()))
            .into_affine();
        PublicKey {
            x: decimal(&a.x),
            y: decimal(&a.y),
        }
    }
    pub fn sign(&self, message: &[u8; 128]) -> Signature {
        use blake_hash::Digest;
        let expanded = self.expanded();
        let s = Scalar::from_le_bytes_mod_order(&expanded[..32]);
        let m = message_hash(message);
        let mut nonce_input = expanded[32..].to_vec();
        let mut m_le = m.into_bigint().to_bytes_le();
        m_le.resize(32, 0);
        nonce_input.extend(m_le);
        let r = Scalar::from_le_bytes_mod_order(&blake_hash::Blake512::digest(&nonce_input));
        let r8 = (BabyJub::GENERATOR * r).into_affine();
        let a = self.public().coordinates().expect("generated public key");
        let h = poseidon::hash(&[r8.x, r8.y, a[0], a[1], m]);
        let h = Scalar::from_le_bytes_mod_order(&h.into_bigint().to_bytes_le());
        Signature {
            r8x: decimal(&r8.x),
            r8y: decimal(&r8.y),
            s: decimal(&(r + h * s)),
        }
    }
    pub fn demo_tag_key(&self) -> [u8; 32] {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(b"zktls/tag-key/v3");
        h.update(self.0);
        h.finalize().into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn matches_independent_circomlibjs_vector() {
        let v: serde_json::Value =
            serde_json::from_str(include_str!("../../../tests/fixtures/eddsa_v3.json")).unwrap();
        let k = SigningKey::from_hex(v["seed_hex"].as_str().unwrap()).unwrap();
        let m: [u8; 128] = hex::decode(v["message_hex"].as_str().unwrap())
            .unwrap()
            .try_into()
            .unwrap();
        let a: PublicKey = serde_json::from_value(v["public_key"].clone()).unwrap();
        let sig: Signature = serde_json::from_value(v["signature"].clone()).unwrap();
        assert_eq!(k.public().x, a.x);
        assert_eq!(k.public().y, a.y);
        assert_eq!(k.sign(&m).r8x, sig.r8x);
        assert_eq!(k.sign(&m).r8y, sig.r8y);
        assert_eq!(k.sign(&m).s, sig.s);
        assert_eq!(
            decimal(&message_hash(&m)),
            v["message_hash"].as_str().unwrap()
        );
        assert_eq!(decimal(&a.hash().unwrap()), v["key_hash"].as_str().unwrap());
        a.verify(&m, &sig).unwrap();
    }

    #[test]
    fn signs_all_bytes_and_rejects_bad_encodings() {
        let k = SigningKey::from_seed([7; 32]);
        let m = [0; 128];
        let sig = k.sign(&m);
        let a = k.public();
        a.verify(&m, &sig).unwrap();
        for i in 0..128 {
            let mut bad = m;
            bad[i] ^= 1;
            assert!(a.verify(&bad, &sig).is_err(), "byte {i}");
        }
        let mut bad = sig.clone();
        bad.s = Scalar::MODULUS.to_string();
        assert!(a.verify(&m, &bad).is_err());
        bad.s = format!("0{}", sig.s);
        assert!(a.verify(&m, &bad).is_err());
        for (x, y) in [("0", "1"), ("0", "0"), ("1", "1")] {
            assert!(PublicKey {
                x: x.into(),
                y: y.into()
            }
            .verify(&m, &sig)
            .is_err());
        }
    }
}
