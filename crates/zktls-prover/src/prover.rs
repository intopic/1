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

/// Qarku i zhvillimit (arkworks vendas) me të njëjtat 10 hyrje publike si
/// `circuits/main.circom`. Provon:
/// * `min ≤ value ≤ max` (të gjitha 64-bit)
/// * `commitment = Poseidon(value, salt)`
/// * `nullifier = Poseidon(account_tag, app_id)`
///
/// NUK provon nënshkrimin RSA as SHA-256 – këto verifikohen natively nga
/// [`crate::tls_parser`] para provimit. Përdoret për teste, CI dhe demo në
/// localnet; për prodhim regjistroni VK-në e qarkut Circom.
#[derive(Clone, Default)]
pub struct DevClaimCircuit {
    pub value: Option<u64>,
    pub account_tag: Option<Fr>,
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
        let get = |i: usize| self.public.and_then(|p| fr_to_u64(&p[i]));
        let (min_val, max_val) = (get(L::MIN_VALUE), get(L::MAX_VALUE));
        let (_, min) = alloc_u64_bits(&cs, min_val)?;
        let (_, max) = alloc_u64_bits(&cs, max_val)?;
        min.enforce_equal(&pubs[L::MIN_VALUE])?;
        max.enforce_equal(&pubs[L::MAX_VALUE])?;
        // value − min ∈ [0, 2^64) dhe max − value ∈ [0, 2^64)
        let lo_val = match (self.value, min_val) {
            (Some(v), Some(m)) => Some(v.checked_sub(m).ok_or(SynthesisError::Unsatisfiable)?),
            _ => None,
        };
        let hi_val = match (self.value, max_val) {
            (Some(v), Some(m)) => Some(m.checked_sub(v).ok_or(SynthesisError::Unsatisfiable)?),
            _ => None,
        };
        let (_, lo) = alloc_u64_bits(&cs, lo_val)?;
        lo.enforce_equal(&(&value - &min))?;
        let (_, hi) = alloc_u64_bits(&cs, hi_val)?;
        hi.enforce_equal(&(&max - &value))?;

        let salt = FpVar::new_witness(cs.clone(), || {
            self.salt.ok_or(SynthesisError::AssignmentMissing)
        })?;
        poseidon::hash_gadget(&[value, salt])?.enforce_equal(&pubs[L::COMMITMENT])?;

        let tag = FpVar::new_witness(cs.clone(), || {
            self.account_tag.ok_or(SynthesisError::AssignmentMissing)
        })?;
        poseidon::hash_gadget(&[tag, pubs[L::APP_ID].clone()])?
            .enforce_equal(&pubs[L::NULLIFIER])?;

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
    pub min_value: u64,
    pub max_value: u64,
    /// Nullifier-i (hex BE) – i njëjtë për të njëjtën llogari brenda një app_id.
    pub nullifier: String,
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
            min_value: w.min_value,
            max_value: w.max_value,
            nullifier: hex::encode(solana::fr_to_be(&w.nullifier())),
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
        if w.value < w.min_value || w.value > w.max_value {
            return Err(ProverError::ThresholdNotMet);
        }
        let public = w.public_inputs();
        let circuit = DevClaimCircuit {
            value: Some(w.value),
            account_tag: Some(w.account_tag_fr()),
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

/// Prapavija e prodhimit mbi qarkun Circom:
/// * `rapidsnark: None` – `snarkjs groth16 fullprove` (JavaScript, i ngadaltë)
/// * `rapidsnark: Some(bin)` – dëshmitari me `snarkjs wtns calculate`, prova me
///   `rapidsnark` (C++ me asm për BN254, shumë më i shpejtë te MSM/FFT)
pub struct SnarkjsProver {
    pub wasm: PathBuf,
    pub zkey: PathBuf,
    pub snarkjs_bin: String,
    pub rapidsnark: Option<String>,
    /// Gjeneruesi C++ i dëshmitarit (`circom --c`): `<bin> input.json witness.wtns`.
    /// Pa të: `snarkjs wtns calculate`.
    pub witness_bin: Option<String>,
    /// Drejtoria bazë; çdo provë përdor një nëndrejtori unike që fshihet në fund
    /// (input.json dhe dëshmitari përmbajnë të dhënat personale të përdoruesit).
    pub work_dir: PathBuf,
    /// VETËM PËR DEBUG/CI: shkruaj direkt në `work_dir` dhe mos i fshi skedarët.
    pub keep_files: bool,
}

/// Fshin drejtorinë e përkohshme kur del nga fusha (edhe në gabim).
struct TempDir(PathBuf, bool);
impl Drop for TempDir {
    fn drop(&mut self) {
        if !self.1 {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

/// Kohët e matura të një prove Circom (ms).
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct ProveTimings {
    pub witness_ms: u64,
    pub prove_ms: u64,
}

/// Koha maksimale për një proces të jashtëm (dëshmitar ose provë). Një provë normale
/// zgjat 1–3 s me rapidsnark dhe < 60 s me snarkjs; një proces i ngecur vritet që të
/// mos e mbajë vendin në radhë përgjithmonë.
pub const CMD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

/// Ndryshoret e mjedisit që i kalohen procesit të jashtëm; çdo gjë tjetër (p.sh.
/// sekretet e roleve të tjera) hiqet.
const ENV_KEEP: &[&str] = &[
    "PATH",
    "HOME",
    "LANG",
    "TMPDIR",
    "NODE_OPTIONS",
    "LD_LIBRARY_PATH",
    "OMP_NUM_THREADS",
];

/// Krijon një skedar vetëm për pronarin (0600): input.json mban të dhëna personale.
fn write_private(path: &Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    o.open(path)?.write_all(data)
}

/// Krijon drejtorinë e provës vetëm për pronarin (0700). Nëse ekziston (vetëm me
/// `keep_files`), mbetet ashtu siç është.
fn create_private_dir(dir: &Path, may_exist: bool) -> std::io::Result<()> {
    let mut b = std::fs::DirBuilder::new();
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
    match b.create(dir) {
        Err(e) if may_exist && e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        r => r,
    }
}

/// Nis `bin` me afat; dalja shkon te `log` (jo në pipe, që një proces llafazan të mos
/// bllokohet) dhe lexohet vetëm në gabim.
fn run_cmd(bin: &str, args: &[&std::ffi::OsStr], log: &Path) -> Result<(), ProverError> {
    run_cmd_within(bin, args, log, CMD_TIMEOUT)
}

fn run_cmd_within(
    bin: &str,
    args: &[&std::ffi::OsStr],
    log: &Path,
    timeout: std::time::Duration,
) -> Result<(), ProverError> {
    let err = |m: String| ProverError::Snarkjs(m);
    let out = std::fs::File::create(log)?;
    let mut cmd = Command::new(bin);
    cmd.args(args)
        .env_clear()
        .envs(
            ENV_KEEP
                .iter()
                .filter_map(|k| std::env::var_os(k).map(|v| (k, v))),
        )
        .stdin(std::process::Stdio::null())
        .stdout(out.try_clone()?)
        .stderr(out);
    let mut child = cmd
        .spawn()
        .map_err(|e| err(format!("nuk u nis `{bin}`: {e}")))?;
    let deadline = std::time::Instant::now() + timeout;
    let status = loop {
        if let Some(s) = child.try_wait()? {
            break s;
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(err(format!(
                "`{bin}` nuk mbaroi brenda {} ms",
                timeout.as_millis()
            )));
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    if !status.success() {
        let mut tail = std::fs::read(log).unwrap_or_default();
        let cut = tail.len().saturating_sub(4096);
        tail.drain(..cut);
        return Err(err(format!(
            "`{bin}` dështoi ({status}): {}",
            String::from_utf8_lossy(&tail)
        )));
    }
    Ok(())
}

impl SnarkjsProver {
    pub fn prove(
        &self,
        w: &ZkWebProofWitness,
    ) -> Result<(SolanaProof, Vec<[u8; 32]>), ProverError> {
        self.prove_timed(w).map(|(p, i, _)| (p, i))
    }

    pub fn prove_timed(
        &self,
        w: &ZkWebProofWitness,
    ) -> Result<(SolanaProof, Vec<[u8; 32]>, ProveTimings), ProverError> {
        // drejtori unike për provë: provat paralele nuk ndajnë skedarë
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let dir = if self.keep_files {
            self.work_dir.clone()
        } else {
            self.work_dir.join(format!(
                "p{}-{}-{:016x}",
                std::process::id(),
                SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
                rand::random::<u64>()
            ))
        };
        std::fs::create_dir_all(&self.work_dir)?;
        create_private_dir(&dir, self.keep_files)?;
        let _guard = TempDir(dir.clone(), self.keep_files);
        if self.keep_files {
            eprintln!("skedarët e provës mbahen në {}", dir.display());
        }
        let input = dir.join("input.json");
        let wtns = dir.join("witness.wtns");
        let proof_p = dir.join("proof.json");
        let public_p = dir.join("public.json");
        let log = dir.join("cmd.log");
        if self.keep_files {
            let _ = std::fs::remove_file(&input);
        }
        write_private(&input, &serde_json::to_vec_pretty(&w.to_circom_input())?)?;
        let mut t = ProveTimings::default();
        match &self.rapidsnark {
            None => {
                let t0 = std::time::Instant::now();
                run_cmd(
                    &self.snarkjs_bin,
                    &[
                        "groth16".as_ref(),
                        "fullprove".as_ref(),
                        input.as_os_str(),
                        self.wasm.as_os_str(),
                        self.zkey.as_os_str(),
                        proof_p.as_os_str(),
                        public_p.as_os_str(),
                    ],
                    &log,
                )?;
                t.prove_ms = t0.elapsed().as_millis() as u64;
            }
            Some(rs) => {
                let t0 = std::time::Instant::now();
                match &self.witness_bin {
                    Some(bin) => run_cmd(bin, &[input.as_os_str(), wtns.as_os_str()], &log)?,
                    None => run_cmd(
                        &self.snarkjs_bin,
                        &[
                            "wtns".as_ref(),
                            "calculate".as_ref(),
                            self.wasm.as_os_str(),
                            input.as_os_str(),
                            wtns.as_os_str(),
                        ],
                        &log,
                    )?,
                }
                t.witness_ms = t0.elapsed().as_millis() as u64;
                let t1 = std::time::Instant::now();
                run_cmd(
                    rs,
                    &[
                        self.zkey.as_os_str(),
                        wtns.as_os_str(),
                        proof_p.as_os_str(),
                        public_p.as_os_str(),
                    ],
                    &log,
                )?;
                t.prove_ms = t1.elapsed().as_millis() as u64;
            }
        }
        let proof = solana::proof_from_snarkjs(&std::fs::read_to_string(&proof_p)?)?;
        let public = solana::public_from_snarkjs(&std::fs::read_to_string(&public_p)?)?;
        if public != w.public_inputs_be() {
            return Err(ProverError::Snarkjs(
                "public.json nuk përputhet me hyrjet e pritura".into(),
            ));
        }
        Ok((SolanaProof::from_ark(&proof), public, t))
    }
}

/// Konverton `Fr` në dhjetor (si në JSON-in e snarkjs).
pub fn fr_to_decimal(x: &Fr) -> String {
    num_bigint::BigUint::from_bytes_le(&x.into_bigint().to_bytes_le()).to_string()
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("zkp-test-{name}-{}", rand::random::<u64>()));
        create_private_dir(&d, false).unwrap();
        d
    }

    #[test]
    fn hung_process_is_killed() {
        let d = tmp("hang");
        let t0 = Instant::now();
        let e = run_cmd_within(
            "sleep",
            &["30".as_ref()],
            &d.join("l"),
            Duration::from_millis(200),
        );
        assert!(format!("{}", e.unwrap_err()).contains("nuk mbaroi"));
        assert!(t0.elapsed() < Duration::from_secs(5));
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn failure_reports_output_and_env_is_cleared() {
        let d = tmp("fail");
        let log = d.join("l");
        // CARGO_PKG_NAME ekziston në mjedisin e testit, por nuk i kalohet fëmijës
        let e = run_cmd_within(
            "sh",
            &[
                "-c".as_ref(),
                "echo \"x${CARGO_PKG_NAME}y\"; exit 3".as_ref(),
            ],
            &log,
            Duration::from_secs(10),
        );
        let m = format!("{}", e.unwrap_err());
        assert!(m.contains("xy"), "{m}");
        assert!(run_cmd_within("true", &[], &log, Duration::from_secs(10)).is_ok());
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn private_files() {
        use std::os::unix::fs::PermissionsExt;
        let d = tmp("perm");
        assert_eq!(
            std::fs::metadata(&d).unwrap().permissions().mode() & 0o777,
            0o700
        );
        let f = d.join("input.json");
        write_private(&f, b"{}").unwrap();
        assert_eq!(
            std::fs::metadata(&f).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(write_private(&f, b"{}").is_err()); // create_new
        std::fs::remove_dir_all(d).unwrap();
    }
}
