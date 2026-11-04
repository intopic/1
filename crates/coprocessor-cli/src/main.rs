//! `coprocessor` – CLI dhe daemon i solana-zktls-coprocessor.
//!
//! ```text
//! coprocessor mock-session --key tests/fixtures/mock_notary_key.pem --balance 15200 \
//!     --wallet <PUBKEY> --out session.tls.json
//! coprocessor setup --out-dir artifacts
//! coprocessor prove --input session.tls.json --threshold 10000 --out proof.json
//! coprocessor verify --proof proof.json --vk artifacts/vk.json
//! coprocessor bank-key-hash --input session.tls.json
//! coprocessor export-vk --snarkjs-vk circuits/build/verification_key.json --out artifacts/vk.json
//! coprocessor serve --addr 127.0.0.1:8787
//! ```

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use serde::Deserialize;
use zktls_prover::mock::{load_private_key_pem, sample_bank_response, MockNotary};
use zktls_prover::prover::{fr_to_decimal, DevGroth16Prover, ProofBundle, SnarkjsProver};
use zktls_prover::solana::{verify_reference, ProofJson, SolanaVerifyingKey, VerifyingKeyJson};
use zktls_prover::witness::ZkWebProofWitness;
use zktls_prover::NotarizedSession;

#[derive(Parser)]
#[command(name = "coprocessor", version, about = "Coprocessor zkTLS për Solana")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Clone, Copy, ValueEnum)]
enum Backend {
    /// Qarku arkworks i zhvillimit (RSA verifikohet jashtë provës)
    Dev,
    /// Qarku i plotë Circom përmes snarkjs
    Snarkjs,
}

#[derive(Subcommand)]
enum Cmd {
    /// Krijon një sesion të noterizuar testues (noter i simuluar)
    MockSession {
        #[arg(long)]
        key: PathBuf,
        #[arg(long)]
        cert: Option<PathBuf>,
        #[arg(long, default_value_t = 15_200)]
        balance: u64,
        /// Çelësi publik Solana i përdoruesit (base58)
        #[arg(long)]
        wallet: String,
        #[arg(long, default_value = "session.tls.json")]
        out: PathBuf,
    },
    /// Gjeneron çelësat Groth16 të qarkut dev (pk.bin + vk.json në formatin Solana)
    Setup {
        #[arg(long, default_value = "artifacts")]
        out_dir: PathBuf,
    },
    /// Verifikon sesionin dhe prodhon provën
    Prove {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        threshold: u64,
        #[arg(long, value_enum, default_value_t = Backend::Dev)]
        backend: Backend,
        #[arg(long, default_value = "artifacts/dev_pk.bin")]
        pk: PathBuf,
        #[arg(long, default_value = "circuits/build/main_js/main.wasm")]
        wasm: PathBuf,
        #[arg(long, default_value = "circuits/build/main_final.zkey")]
        zkey: PathBuf,
        #[arg(long, default_value = "snarkjs")]
        snarkjs: String,
        /// Ruaj edhe input.json për Circom
        #[arg(long)]
        witness_out: Option<PathBuf>,
        #[arg(long, default_value = "proof.json")]
        out: PathBuf,
    },
    /// Verifikon një provë off-chain me ekuacionin e programit Solana
    Verify {
        #[arg(long)]
        proof: PathBuf,
        #[arg(long, default_value = "artifacts/vk.json")]
        vk: PathBuf,
    },
    /// Shtyp hash-in Poseidon të çelësit të noterit/bankës (për whitelist-ën on-chain)
    BankKeyHash {
        #[arg(long)]
        input: PathBuf,
    },
    /// Konverton verification_key.json të snarkjs në formatin Solana (vk.json)
    ExportVk {
        #[arg(long, default_value = "circuits/build/verification_key.json")]
        snarkjs_vk: PathBuf,
        #[arg(long, default_value = "artifacts/vk.json")]
        out: PathBuf,
    },
    /// Daemon HTTP minimal: GET /health, POST /prove {session, threshold}
    Serve {
        #[arg(long, default_value = "127.0.0.1:8787")]
        addr: String,
        #[arg(long, default_value = "artifacts/dev_pk.bin")]
        pk: PathBuf,
    },
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn parse_wallet(s: &str) -> Result<[u8; 32]> {
    let v = bs58::decode(s)
        .into_vec()
        .context("wallet nuk është base58")?;
    v.try_into()
        .map_err(|_| anyhow!("wallet duhet të ketë 32 bajte"))
}

fn read_session(p: &Path) -> Result<NotarizedSession> {
    let s = std::fs::read_to_string(p).with_context(|| format!("lexim {}", p.display()))?;
    Ok(NotarizedSession::from_json(&s)?)
}

fn prove_dev(
    prover: &DevGroth16Prover,
    session: &NotarizedSession,
    threshold: u64,
) -> Result<(ProofBundle, ZkWebProofWitness)> {
    let vs = session.verify(threshold)?;
    let w = ZkWebProofWitness::from_session(&vs, threshold, None)?;
    let (proof, inputs) = prover.prove(&w, &mut rand::rngs::OsRng)?;
    Ok((ProofBundle::new("dev-arkworks", &w, &proof, &inputs), w))
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::MockSession {
            key,
            cert,
            balance,
            wallet,
            out,
        } => {
            let notary = MockNotary {
                key: load_private_key_pem(&std::fs::read_to_string(&key)?)?,
                certificate_pem: cert.map(std::fs::read_to_string).transpose()?,
            };
            let s = notary.notarize(
                "api.mockbank.test",
                &sample_bank_response(balance),
                "balance",
                parse_wallet(&wallet)?,
                now(),
            )?;
            std::fs::write(&out, serde_json::to_string_pretty(&s)?)?;
            println!("sesioni u shkrua në {}", out.display());
        }
        Cmd::Setup { out_dir } => {
            std::fs::create_dir_all(&out_dir)?;
            eprintln!("KUJDES: setup lokal i qarkut DEV – toksina nuk është e shpërndarë. Mos e përdorni në mainnet.");
            let p = DevGroth16Prover::setup(&mut rand::rngs::OsRng)?;
            p.save(&out_dir.join("dev_pk.bin"))?;
            std::fs::write(
                out_dir.join("vk.json"),
                serde_json::to_string_pretty(&p.solana_vk().to_json())?,
            )?;
            println!(
                "u shkruan {0}/dev_pk.bin dhe {0}/vk.json",
                out_dir.display()
            );
        }
        Cmd::Prove {
            input,
            threshold,
            backend,
            pk,
            wasm,
            zkey,
            snarkjs,
            witness_out,
            out,
        } => {
            let session = read_session(&input)?;
            let t0 = std::time::Instant::now();
            let (bundle, w) = match backend {
                Backend::Dev => {
                    let prover = DevGroth16Prover::load(&pk).with_context(|| {
                        format!("mungon {} – ekzekutoni `coprocessor setup`", pk.display())
                    })?;
                    prove_dev(&prover, &session, threshold)?
                }
                Backend::Snarkjs => {
                    let vs = session.verify(threshold)?;
                    let w = ZkWebProofWitness::from_session(&vs, threshold, None)?;
                    let sp = SnarkjsProver {
                        wasm,
                        zkey,
                        snarkjs_bin: snarkjs,
                        work_dir: PathBuf::from("target/snarkjs"),
                    };
                    let (proof, inputs) = sp.prove(&w)?;
                    (ProofBundle::new("circom-snarkjs", &w, &proof, &inputs), w)
                }
            };
            if let Some(p) = witness_out {
                std::fs::write(&p, serde_json::to_string_pretty(&w.to_circom_input())?)?;
            }
            std::fs::write(&out, serde_json::to_string_pretty(&bundle)?)?;
            println!(
                "prova u shkrua në {} ({} ms, backend {}, commitment {})",
                out.display(),
                t0.elapsed().as_millis(),
                bundle.backend,
                fr_to_decimal(&w.commitment())
            );
        }
        Cmd::Verify { proof, vk } => {
            let pj: ProofJson = serde_json::from_str(&std::fs::read_to_string(&proof)?)?;
            let vkj: VerifyingKeyJson = serde_json::from_str(&std::fs::read_to_string(&vk)?)?;
            let (p, inputs) = pj.decode()?;
            let ok = verify_reference(&SolanaVerifyingKey::from_json(&vkj)?, &p, &inputs)?;
            if !ok {
                bail!("prova NUK është e vlefshme");
            }
            println!("prova është e vlefshme ✔");
        }
        Cmd::BankKeyHash { input } => {
            let s = read_session(&input)?;
            let k = s.notary_public_key()?;
            let h = zktls_prover::poseidon::rsa_modulus_hash(&k.modulus_be_bytes());
            println!("{}", hex::encode(zktls_prover::solana::fr_to_be(&h)));
        }
        Cmd::ExportVk { snarkjs_vk, out } => {
            let vk = zktls_prover::solana::vk_from_snarkjs(&std::fs::read_to_string(&snarkjs_vk)?)?;
            let svk = SolanaVerifyingKey::from_ark(&vk);
            if svk.ic.len() != zktls_prover::public_layout::COUNT + 1 {
                bail!(
                    "VK ka {} pika IC; pritej {}",
                    svk.ic.len(),
                    zktls_prover::public_layout::COUNT + 1
                );
            }
            if let Some(dir) = out.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(&out, serde_json::to_string_pretty(&svk.to_json())?)?;
            println!("u shkrua {}", out.display());
        }
        Cmd::Serve { addr, pk } => serve(&addr, &pk)?,
    }
    Ok(())
}

#[derive(Deserialize)]
struct ProveRequest {
    session: NotarizedSession,
    threshold: u64,
}

/// Server HTTP/1.1 minimal, një kërkesë njëherësh. Pa TLS dhe pa autentikim –
/// vendoseni pas një reverse proxy.
fn serve(addr: &str, pk: &Path) -> Result<()> {
    let prover = DevGroth16Prover::load(pk).with_context(|| format!("mungon {}", pk.display()))?;
    let listener = TcpListener::bind(addr)?;
    println!("coprocessor po dëgjon në http://{addr}");
    for stream in listener.incoming() {
        let mut stream = match stream {
            Ok(s) => s,
            Err(_) => continue,
        };
        if let Err(e) = handle(&prover, &mut stream) {
            let _ = respond(
                &mut stream,
                400,
                &serde_json::json!({ "error": e.to_string() }).to_string(),
            );
        }
    }
    Ok(())
}

fn handle(prover: &DevGroth16Prover, stream: &mut TcpStream) -> Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let (method, path) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    let mut len = 0usize;
    loop {
        let mut h = String::new();
        reader.read_line(&mut h)?;
        if h == "\r\n" || h.is_empty() {
            break;
        }
        if let Some(v) = h.to_ascii_lowercase().strip_prefix("content-length:") {
            len = v.trim().parse()?;
        }
    }
    if len > 1 << 20 {
        bail!("trupi i kërkesës > 1 MiB");
    }
    match (method, path) {
        ("GET", "/health") => respond(stream, 200, r#"{"status":"ok"}"#),
        ("POST", "/prove") => {
            let mut body = vec![0u8; len];
            reader.read_exact(&mut body)?;
            let req: ProveRequest = serde_json::from_slice(&body)?;
            let (bundle, _) = prove_dev(prover, &req.session, req.threshold)?;
            respond(stream, 200, &serde_json::to_string(&bundle)?)
        }
        _ => respond(stream, 404, r#"{"error":"not found"}"#),
    }
}

fn respond(stream: &mut TcpStream, code: u16, body: &str) -> Result<()> {
    let reason = match code {
        200 => "OK",
        404 => "Not Found",
        _ => "Bad Request",
    };
    write!(
        stream,
        "HTTP/1.1 {code} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )?;
    Ok(())
}
