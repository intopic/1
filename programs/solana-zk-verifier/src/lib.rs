//! # solana-zk-verifier
//!
//! Program Anchor që pranon një provë Groth16 (BN254) të prodhuar nga
//! `zktls-prover`, e verifikon me syscall-et vendase `alt_bn128_*` dhe ruan
//! një `AttestationRecord` (PDA për përdorues).

// Makrot e Anchor 0.31 gjenerojnë thirrje te `AccountInfo::realloc` (e vjetëruar).
#![allow(deprecated)]

use anchor_lang::prelude::*;

pub mod error;
pub mod gate;
pub mod instructions;
pub mod state;

pub use error::ZkVerifierError;
pub use instructions::*;
pub use state::*;

// ID placeholder: zëvendësojeni me `anchor keys sync` para deploy-it.
declare_id!("ZkTLSCoprocessor111111111111111111111111111");

#[program]
pub mod solana_zk_verifier {
    use super::*;

    /// Inicializon gjendjen e verifikuesit me çelësin verifikues (VK).
    pub fn initialize(
        ctx: Context<Initialize>,
        verifying_key: VerifyingKey,
        app_id: [u8; 32],
        allowed_claim_keys: Vec<u64>,
        max_attestation_age_secs: i64,
    ) -> Result<()> {
        instructions::initialize_handler(
            ctx,
            verifying_key,
            app_id,
            allowed_claim_keys,
            max_attestation_age_secs,
        )
    }

    /// Ndërron VK-në (p.sh. nga qarku dev te qarku Circom i prodhimit).
    pub fn set_verifying_key(ctx: Context<AdminUpdate>, verifying_key: VerifyingKey) -> Result<()> {
        instructions::set_verifying_key_handler(ctx, verifying_key)
    }

    /// Shton pikat e mbetura IC të VK-së (VK-ja e plotë nuk nxë në një transaksion).
    pub fn extend_vk_ic(ctx: Context<AdminUpdate>, points: Vec<[u8; 64]>) -> Result<()> {
        instructions::extend_vk_ic_handler(ctx, points)
    }

    /// Shton hash-in Poseidon të një çelësi RSA banke/noteri në listën e bardhë.
    pub fn add_bank_key(ctx: Context<AdminUpdate>, bank_key_hash: [u8; 32]) -> Result<()> {
        instructions::add_bank_key_handler(ctx, bank_key_hash)
    }

    pub fn remove_bank_key(ctx: Context<AdminUpdate>, bank_key_hash: [u8; 32]) -> Result<()> {
        instructions::remove_bank_key_handler(ctx, bank_key_hash)
    }

    /// Verifikon provën Groth16 për një përdorues dhe ruan atestatin on-chain.
    pub fn verify_and_attest(
        ctx: Context<VerifyAndAttest>,
        proof_a: [u8; 64],
        proof_b: [u8; 128],
        proof_c: [u8; 64],
        public_inputs: Vec<[u8; 32]>,
        nullifier: [u8; 32],
    ) -> Result<()> {
        instructions::verify_and_attest_handler(
            ctx,
            proof_a,
            proof_b,
            proof_c,
            public_inputs,
            nullifier,
        )
    }
}

#[cfg(test)]
mod tests {
    //! Teste native (pa validator): syscall-et `alt_bn128` kanë implementim
    //! vendas në target-et jo-Solana, ndaj verifikuesi mund të testohet direkt
    //! me fixture-n e prodhuar nga `tests/e2e_mock_tls.rs`.
    use super::*;

    const FIXTURE: &str = include_str!("../../../tests/fixtures/dev_proof_fixture.json");

    fn h<const N: usize>(s: &serde_json::Value) -> [u8; N] {
        hex::decode(s.as_str().unwrap())
            .unwrap()
            .try_into()
            .unwrap()
    }

    type Fixture = (
        VerifyingKey,
        [u8; 64],
        [u8; 128],
        [u8; 64],
        Vec<[u8; 32]>,
        Pubkey,
    );

    fn load_from(json: &str) -> Fixture {
        let f: serde_json::Value = serde_json::from_str(json).unwrap();
        let vk = &f["vk"];
        let vk = VerifyingKey {
            alpha_g1: h(&vk["alpha_g1"]),
            beta_g2: h(&vk["beta_g2"]),
            gamma_g2: h(&vk["gamma_g2"]),
            delta_g2: h(&vk["delta_g2"]),
            ic: vk["ic"].as_array().unwrap().iter().map(h).collect(),
        };
        let p = &f["proof"];
        let inputs = p["public_inputs"]
            .as_array()
            .unwrap()
            .iter()
            .map(h)
            .collect();
        let wallet: [u8; 32] = bs58::decode(f["wallet"].as_str().unwrap())
            .into_vec()
            .unwrap()
            .try_into()
            .unwrap();
        (
            vk,
            h(&p["proof_a"]),
            h(&p["proof_b"]),
            h(&p["proof_c"]),
            inputs,
            Pubkey::new_from_array(wallet),
        )
    }

    fn load() -> Fixture {
        load_from(FIXTURE)
    }

    /// Prova e qarkut të plotë Circom (RSA-2048 + SHA-256), e prodhuar në CI
    /// nga workflow `zk-e2e.yml`. Anashkalohet kur variabli nuk është vendosur.
    #[test]
    fn accepts_circom_fixture_if_present() {
        let Ok(path) = std::env::var("ZKTLS_CIRCOM_FIXTURE") else {
            eprintln!("ZKTLS_CIRCOM_FIXTURE mungon - testi anashkalohet");
            return;
        };
        let (vk, a, b, c, inputs, wallet) = load_from(&std::fs::read_to_string(path).unwrap());
        assert_eq!(inputs.len(), NUM_PUBLIC_INPUTS);
        assert!(verify_groth16_bn254(&a, &b, &c, &inputs, &vk).unwrap());
        let (hi, lo) = wallet_halves(&wallet);
        assert_eq!(inputs[layout::USER_PUBKEY_HI], hi);
        assert_eq!(inputs[layout::USER_PUBKEY_LO], lo);
        let mut bad = inputs.clone();
        bad[layout::MIN_VALUE][31] ^= 1;
        assert!(!verify_groth16_bn254(&a, &b, &c, &bad, &vk).unwrap_or(false));
    }

    #[test]
    fn accepts_valid_fixture_proof() {
        let (vk, a, b, c, inputs, _) = load();
        assert_eq!(inputs.len(), NUM_PUBLIC_INPUTS);
        assert!(verify_groth16_bn254(&a, &b, &c, &inputs, &vk).unwrap());
    }

    #[test]
    fn rejects_modified_inputs_and_proof() {
        let (vk, a, b, c, inputs, _) = load();
        for i in 0..inputs.len() {
            let mut bad = inputs.clone();
            bad[i][31] ^= 1;
            assert!(
                !verify_groth16_bn254(&a, &b, &c, &bad, &vk).unwrap_or(false),
                "hyrja {i}"
            );
        }
        // C i zëvendësuar me A → provë e pavlefshme
        assert!(!verify_groth16_bn254(&a, &b, &a, &inputs, &vk).unwrap_or(false));
        // hyrje >= r refuzohet
        let mut big = inputs.clone();
        big[0] = [0xff; 32];
        assert!(verify_groth16_bn254(&a, &b, &c, &big, &vk).is_err());
        // numër i gabuar hyrjesh
        assert!(verify_groth16_bn254(&a, &b, &c, &inputs[..6], &vk).is_err());
    }

    #[test]
    fn wallet_binding_matches_prover_layout() {
        let (_, _, _, _, inputs, wallet) = load();
        let (hi, lo) = wallet_halves(&wallet);
        assert_eq!(inputs[layout::USER_PUBKEY_HI], hi);
        assert_eq!(inputs[layout::USER_PUBKEY_LO], lo);
    }

    #[test]
    fn negate_is_involution() {
        let (_, a, _, _, _, _) = load();
        let n = negate_g1(&a).unwrap();
        assert_ne!(n, a);
        assert_eq!(negate_g1(&n).unwrap(), a);
        assert_eq!(negate_g1(&[0u8; 64]).unwrap(), [0u8; 64]);
    }

    #[test]
    fn account_sizes_fit() {
        // VerifierState duhet të nxërë në një llogari të krijuar me CPI (≤ 10 KiB).
        const { assert!(8 + VerifierState::INIT_SPACE < 10_240) };
        assert_eq!(
            8 + AttestationRecord::INIT_SPACE,
            8 + 32 + 1 + 8 + 8 + 32 + 32 + 32 + 8 + 8 + 8 + 1
        );
        assert_eq!(8 + NullifierRecord::INIT_SPACE, 8 + 32 + 8 + 1);
    }
}
