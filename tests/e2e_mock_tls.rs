//! Testi i plotë End-to-End me të dhëna testuese:
//! noter i simuluar → sesion JSON → verifikim RSA (triad-core) → normalizim →
//! dëshmitar Circom → provë Groth16 (qarku dev) → kodim Solana → verifikim
//! me ekuacionin e programit on-chain.
//!
//! `ZKTLS_WRITE_FIXTURES=1 cargo test -p zktls-prover --test e2e_mock_tls`
//! rishkruan `tests/fixtures/dev_proof_fixture.json` (përdoret nga testet e programit Anchor).

use rand::SeedableRng;
use zktls_prover::mock::{
    load_private_key_pem, sample_bank_response, sample_identity_response, MockNotary,
};
use zktls_prover::prover::{verify_ark, DevGroth16Prover};
use zktls_prover::solana::{self, verify_reference, SolanaVerifyingKey};
use zktls_prover::witness::{app_id_from_label, birth_cutoff, ValueRange, ZkWebProofWitness};
use zktls_prover::{public_layout as L, NotarizedSession, ProverError};

const KEY_PEM: &str = include_str!("fixtures/mock_notary_key.pem");
const CERT_PEM: &str = include_str!("fixtures/mock_notary_cert.pem");
const WALLET: [u8; 32] = [
    0x0b, 0x6a, 0x3f, 0x11, 0x92, 0x7e, 0x01, 0xcd, 0x55, 0x10, 0xaa, 0x04, 0x9e, 0x3d, 0x71, 0x28,
    0xf0, 0x0d, 0x6c, 0x45, 0x83, 0x2b, 0x19, 0xe7, 0x60, 0x33, 0x8a, 0xbc, 0x04, 0x5f, 0x9d, 0x21,
];
const TS: u64 = 1_793_800_000;
const IBAN: &str = "AL35202111090000000001234567";
const OTHER_WALLET: [u8; 32] = [0x42; 32];

fn notary() -> MockNotary {
    MockNotary {
        key: load_private_key_pem(KEY_PEM).unwrap(),
        certificate_pem: Some(CERT_PEM.into()),
        tag_secret: None,
    }
}

fn session_for(http: &str, key: &str, account: &str, wallet: [u8; 32]) -> NotarizedSession {
    let s = notary()
        .notarize("api.mockbank.test", http, key, account, wallet, TS)
        .unwrap();
    // kalon përmes JSON si në CLI
    NotarizedSession::from_json(&serde_json::to_string(&s).unwrap()).unwrap()
}

fn session(balance: u64) -> NotarizedSession {
    session_for(&sample_bank_response(balance), "balance", IBAN, WALLET)
}

fn app() -> ark_bn254::Fr {
    app_id_from_label("test-airdrop-2026")
}

#[test]
fn full_pipeline_dev_backend() {
    let s = session(15_200);
    assert!(!s.http_response.is_empty());

    // 1-3: RSA + normalizim + integritet
    let vs = s.verify(10_000).unwrap();
    assert_eq!(vs.message.value, 15_200);
    assert!(vs.claim.is_greater_or_equal);
    assert_eq!(vs.notary_key.size_bytes(), 256);

    // dëshmitari Circom
    let mut rng = rand::rngs::StdRng::seed_from_u64(2026);
    let w = ZkWebProofWitness::from_session(
        &vs,
        ValueRange::at_least(10_000),
        app(),
        Some(ark_bn254::Fr::from(424242u64)),
    )
    .unwrap();
    let input = w.to_circom_input();
    assert_eq!(input["attestation_msg"].as_array().unwrap().len(), 128);
    assert_eq!(input["rsa_signature"].as_array().unwrap().len(), 17);
    assert_eq!(input["rsa_modulus"].as_array().unwrap().len(), 17);
    assert_eq!(w.expected_public_json().as_array().unwrap().len(), L::COUNT);

    // provë Groth16
    let prover = DevGroth16Prover::setup(&mut rng).unwrap();
    let (proof, inputs) = prover.prove(&w, &mut rng).unwrap();
    assert_eq!(inputs.len(), L::COUNT);
    assert!(verify_ark(prover.vk(), &proof, &inputs).unwrap());

    // formati Solana (ekuacioni i programit, me −A)
    let svk = SolanaVerifyingKey::from_ark(prover.vk());
    assert_eq!(svk.ic.len(), L::COUNT + 1);
    assert!(verify_reference(&svk, &proof, &inputs).unwrap());
    // roundtrip i kodimit
    assert_eq!(SolanaVerifyingKey::from_json(&svk.to_json()).unwrap(), svk);
    let pj = proof.to_json(&inputs);
    assert_eq!(pj.decode().unwrap(), (proof, inputs.clone()));
    // roundtrip snarkjs
    let sj = serde_json::to_string(&solana::proof_to_snarkjs(&proof.to_ark().unwrap())).unwrap();
    assert_eq!(
        solana::SolanaProof::from_ark(&solana::proof_from_snarkjs(&sj).unwrap()),
        proof
    );

    // ndryshimi i çdo hyrjeje publike e rrëzon provën
    for i in 0..L::COUNT {
        let mut bad = inputs.clone();
        bad[i][31] ^= 1;
        assert!(!verify_reference(&svk, &proof, &bad).unwrap(), "hyrja {i}");
    }

    if std::env::var("ZKTLS_WRITE_FIXTURES").is_ok() {
        let fixture = serde_json::json!({
            "note": "Fixture e gjeneruar nga tests/e2e_mock_tls.rs (qarku DEV). VETËM PËR TESTE.",
            "wallet": bs58::encode(WALLET).into_string(),
            "vk": svk.to_json(),
            "proof": pj,
        });
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/dev_proof_fixture.json"
        );
        std::fs::write(path, serde_json::to_string_pretty(&fixture).unwrap()).unwrap();
    }
}

#[test]
fn rejects_tampering_and_unmet_threshold() {
    // pragu nuk plotësohet
    let vs = session(9_000).verify(10_000).unwrap();
    assert!(!vs.claim.is_greater_or_equal);
    assert!(matches!(
        ZkWebProofWitness::from_session(&vs, ValueRange::at_least(10_000), app(), None),
        Err(ProverError::ThresholdNotMet)
    ));

    // transkripti i ndryshuar (bilanc i fryrë) → digest-i nuk përputhet
    let mut s = session(15_200);
    s.http_response = s.http_response.replace("15200", "95200");
    assert!(matches!(s.verify(0), Err(ProverError::Attestation(_))));

    // nënshkrim i ndryshuar → RSA dështon
    let mut s = session(15_200);
    let mut sig = hex::decode(&s.attestation.signature_hex).unwrap();
    sig[100] ^= 0x01;
    s.attestation.signature_hex = hex::encode(sig);
    assert!(matches!(s.verify(0), Err(ProverError::Rsa(_))));

    // mesazh i ndryshuar (vlera) → RSA dështon
    let mut s = session(15_200);
    let mut msg = hex::decode(&s.attestation.message_hex).unwrap();
    msg[15] ^= 0x01;
    s.attestation.message_hex = hex::encode(msg);
    assert!(matches!(s.verify(0), Err(ProverError::Rsa(_))));

    // çelës tjetër noteri (moduli hex i ndryshuar) → RSA dështon ose çelës i pavlefshëm
    let mut s = session(15_200);
    s.notary.certificate_pem = None;
    let other = load_private_key_pem(KEY_PEM).unwrap();
    use rsa::traits::PublicKeyParts;
    let mut n = other.n().to_bytes_be();
    n[200] ^= 0x10;
    s.notary.modulus_hex = Some(hex::encode(n));
    assert!(s.verify(0).is_err());
}

#[test]
fn certificate_and_modulus_paths_agree() {
    let s = session(15_200);
    let from_cert = s.notary_public_key().unwrap();
    let k = load_private_key_pem(KEY_PEM).unwrap();
    use rsa::traits::PublicKeyParts;
    assert_eq!(from_cert.modulus_be_bytes(), k.n().to_bytes_be());
    assert_eq!(from_cert.exponent(), 65537);
}

#[test]
fn age_over_18_proof() {
    let mut rng = rand::rngs::StdRng::seed_from_u64(18);
    let prover = DevGroth16Prover::setup(&mut rng).unwrap();
    let svk = SolanaVerifyingKey::from_ark(prover.vk());
    let cutoff = birth_cutoff(20261104, 18).unwrap();
    assert_eq!(cutoff, 20081104);

    // i lindur më 2001-05-17 -> mbi 18 vjeç
    let s = session_for(
        &sample_identity_response("2001-05-17"),
        "date_of_birth",
        "J01234567A",
        WALLET,
    );
    let vs = s.verify(0).unwrap();
    assert_eq!(vs.message.value, 20010517);
    // data e lindjes nuk shfaqet në transkriptin e maskuar që hash-ohet
    let w =
        ZkWebProofWitness::from_session(&vs, ValueRange::born_on_or_before(cutoff), app(), None)
            .unwrap();
    let (proof, inputs) = prover.prove(&w, &mut rng).unwrap();
    assert!(verify_reference(&svk, &proof, &inputs).unwrap());

    // i lindur më 2010-01-01 -> nën 18, nuk mund të provohet
    let s = session_for(
        &sample_identity_response("2010-01-01"),
        "date_of_birth",
        "J0999",
        WALLET,
    );
    let vs = s.verify(0).unwrap();
    assert!(matches!(
        ZkWebProofWitness::from_session(&vs, ValueRange::born_on_or_before(cutoff), app(), None),
        Err(ProverError::ThresholdNotMet)
    ));
    // edhe nëse dikush anashkalon kontrollin në Rust, qarku nuk ka zgjidhje
    let mut w =
        ZkWebProofWitness::from_session(&vs, ValueRange::at_most(u64::MAX), app(), None).unwrap();
    w.max_value = cutoff;
    assert!(prover.prove(&w, &mut rng).is_err());
}

#[test]
fn nullifier_is_per_account_and_app() {
    let w = |account: &str, wallet: [u8; 32], app_label: &str| {
        let vs = session_for(&sample_bank_response(15_200), "balance", account, wallet)
            .verify(0)
            .unwrap();
        ZkWebProofWitness::from_session(
            &vs,
            ValueRange::at_least(1),
            app_id_from_label(app_label),
            None,
        )
        .unwrap()
        .nullifier()
    };
    // e njëjta llogari, portofol tjetër -> i njëjti nullifier (Sybil zbulohet)
    assert_eq!(w(IBAN, WALLET, "a"), w(IBAN, OTHER_WALLET, "a"));
    // llogari tjetër -> nullifier tjetër
    assert_ne!(w(IBAN, WALLET, "a"), w("AL00OTHER", WALLET, "a"));
    // aplikacion tjetër -> nullifier tjetër (nuk lidhen përdoruesit mes aplikacioneve)
    assert_ne!(w(IBAN, WALLET, "a"), w(IBAN, WALLET, "b"));

    // prova e mban nullifier-in si hyrje publike të kontrolluar
    let mut rng = rand::rngs::StdRng::seed_from_u64(7);
    let prover = DevGroth16Prover::setup(&mut rng).unwrap();
    let svk = SolanaVerifyingKey::from_ark(prover.vk());
    let vs = session(15_200).verify(0).unwrap();
    let wit =
        ZkWebProofWitness::from_session(&vs, ValueRange::at_least(10_000), app(), None).unwrap();
    let (proof, mut inputs) = prover.prove(&wit, &mut rng).unwrap();
    assert_eq!(inputs[L::NULLIFIER], solana::fr_to_be(&wit.nullifier()));
    assert!(verify_reference(&svk, &proof, &inputs).unwrap());
    inputs[L::NULLIFIER] = solana::fr_to_be(&w("AL00OTHER", WALLET, "test-airdrop-2026"));
    assert!(!verify_reference(&svk, &proof, &inputs).unwrap());
}
