//! Poseidon (parametrat e circomlib, BN254, x^5) – vendas dhe si gadget R1CS.
//!
//! Gadget-i ndjek saktësisht `light-poseidon` (i përputhshëm me `circomlib`
//! `Poseidon(n)`), prandaj zotimet e qarkut dev dhe të qarkut Circom janë identike.

use ark_bn254::Fr;
use ark_ff::Zero;
use ark_r1cs_std::fields::fp::FpVar;
use ark_r1cs_std::prelude::*;
use ark_relations::r1cs::SynthesisError;
use light_poseidon::parameters::bn254_x5::get_poseidon_parameters;
use light_poseidon::{Poseidon, PoseidonHasher};

/// `Poseidon(inputs)` si në circomlib (1..=12 hyrje).
pub fn hash(inputs: &[Fr]) -> Fr {
    Poseidon::<Fr>::new_circom(inputs.len())
        .expect("1..=12 hyrje")
        .hash(inputs)
        .expect("numri i hyrjeve përputhet")
}

/// Gadget R1CS i Poseidon-it, ~ (8·t + 57..64)·3 kufizime.
pub fn hash_gadget(inputs: &[FpVar<Fr>]) -> Result<FpVar<Fr>, SynthesisError> {
    let width = inputs.len() + 1;
    let p =
        get_poseidon_parameters::<Fr>(width as u8).map_err(|_| SynthesisError::Unsatisfiable)?;
    let mut state: Vec<FpVar<Fr>> = Vec::with_capacity(width);
    state.push(FpVar::Constant(Fr::zero()));
    state.extend(inputs.iter().cloned());
    let half = p.full_rounds / 2;
    let total = p.full_rounds + p.partial_rounds;
    for round in 0..total {
        for (i, s) in state.iter_mut().enumerate() {
            *s += p.ark[round * width + i];
        }
        let full = round < half || round >= half + p.partial_rounds;
        let n_sbox = if full { width } else { 1 };
        for s in state.iter_mut().take(n_sbox) {
            let x2 = s.square()?;
            let x4 = x2.square()?;
            *s = &x4 * &*s;
        }
        let old = state.clone();
        for (i, s) in state.iter_mut().enumerate() {
            let mut acc = FpVar::Constant(Fr::zero());
            for (j, o) in old.iter().enumerate() {
                acc += o * p.mds[i][j];
            }
            *s = acc;
        }
    }
    Ok(state.swap_remove(0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ark_relations::r1cs::ConstraintSystem;
    use std::str::FromStr;

    #[test]
    fn circomlib_vector() {
        // Vektor referencë i circomlib/light-poseidon: Poseidon([1, 2])
        let h = hash(&[Fr::from(1u64), Fr::from(2u64)]);
        let expected = Fr::from_str(
            "7853200120776062878684798364095072458815029376092732009249414926327459813530",
        )
        .unwrap();
        assert_eq!(h, expected);
    }

    #[test]
    fn gadget_matches_native() {
        for n in [1usize, 2, 9] {
            let cs = ConstraintSystem::<Fr>::new_ref();
            let vals: Vec<Fr> = (0..n).map(|i| Fr::from(1000 + i as u64)).collect();
            let vars: Vec<FpVar<Fr>> = vals
                .iter()
                .map(|v| FpVar::new_witness(cs.clone(), || Ok(*v)).unwrap())
                .collect();
            let out = hash_gadget(&vars).unwrap();
            assert_eq!(out.value().unwrap(), hash(&vals));
            assert!(cs.is_satisfied().unwrap());
        }
    }
}
