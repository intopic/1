//! `coprocessor` – CLI dhe daemon i solana-zktls-coprocessor.
//!
//! ```text
//! coprocessor mock-session --key tests/fixtures/mock_notary_seed.hex --balance 15200 \
//!     --wallet <PUBKEY> --out session.tls.json
//! coprocessor setup --out-dir artifacts
//! coprocessor prove --input session.tls.json --min 10000 --app airdrop-2026 --out proof.json
//! coprocessor mock-session --kind age --dob 2001-05-17 ... && coprocessor prove --min-age 18 ...
//! coprocessor verify --proof proof.json --vk artifacts/vk.json
//! coprocessor bank-key-hash --input session.tls.json
//! coprocessor export-vk --snarkjs-vk circuits/build/verification_key.json --out artifacts/vk.json
//! coprocessor app-id --app airdrop-2026
//! coprocessor serve --addr 127.0.0.1:8787 --mock-notary-key tests/fixtures/mock_notary_seed.hex
//! ```

mod chain;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use serde::Deserialize;
use zktls_prover::mock::{sample_bank_response, sample_identity_response, MockNotary};
use zktls_prover::prover::{
    fr_to_decimal, DevGroth16Prover, ProofBundle, ProveTimings, SnarkjsProver,
};
use zktls_prover::solana::{verify_reference, ProofJson, SolanaVerifyingKey, VerifyingKeyJson};
use zktls_prover::witness::{
    app_id_from_label, birth_cutoff, ValueRange, ZkWebProofWitness, BIRTH_DATE_KEYS, MAX_AGE_YEARS,
};
use zktls_prover::NotarizedSession;

#[derive(Parser)]
#[command(name = "coprocessor", version, about = "Coprocessor zkTLS për Solana")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Backend {
    /// Qarku arkworks i zhvillimit (EdDSA verifikohet jashtë provës)
    Dev,
    /// Qarku i plotë Circom përmes snarkjs
    Snarkjs,
    /// Qarku i plotë Circom: dëshmitari me snarkjs, prova me rapidsnark (C++)
    Rapidsnark,
}

/// Motori i provës (i përbashkët për `prove` dhe `serve`).
#[derive(clap::Args, Clone)]
struct EngineArgs {
    #[arg(long, value_enum, default_value_t = Backend::Rapidsnark)]
    backend: Backend,
    /// Çelësi i provimit të qarkut dev
    #[arg(long, default_value = "artifacts/dev_pk.bin")]
    pk: PathBuf,
    #[arg(long, default_value = "circuits/build/main_js/main.wasm")]
    wasm: PathBuf,
    #[arg(long, default_value = "circuits/build/main_final.zkey")]
    zkey: PathBuf,
    #[arg(long, default_value = "snarkjs")]
    snarkjs: String,
    /// Binari `prover` i rapidsnark (për `--backend rapidsnark`)
    #[arg(long, default_value = "prover")]
    rapidsnark: String,
    /// Gjeneruesi C++ i dëshmitarit (`circom --c`, p.sh. circuits/build/cpp/main_c_cpp/main_c);
    /// pa të, dëshmitari llogaritet me `snarkjs wtns calculate`
    #[arg(long)]
    witness_bin: Option<String>,
}

enum Engine {
    Dev(Box<DevGroth16Prover>),
    Circom { prover: SnarkjsProver, rapid: bool },
}

impl Engine {
    fn load(a: &EngineArgs, keep_files: Option<PathBuf>) -> Result<Self> {
        Ok(match a.backend {
            Backend::Dev => Engine::Dev(Box::new(DevGroth16Prover::load(&a.pk).with_context(
                || format!("mungon {} – ekzekutoni `coprocessor setup`", a.pk.display()),
            )?)),
            Backend::Snarkjs | Backend::Rapidsnark => {
                for f in [&a.zkey]
                    .into_iter()
                    .chain(a.witness_bin.is_none().then_some(&a.wasm))
                {
                    if !f.exists() {
                        bail!(
                            "mungon {} – kompiloni qarkun dhe ekzekutoni setup_keys.sh",
                            f.display()
                        );
                    }
                }
                let rapid = matches!(a.backend, Backend::Rapidsnark);
                Engine::Circom {
                    prover: SnarkjsProver {
                        wasm: a.wasm.clone(),
                        zkey: a.zkey.clone(),
                        snarkjs_bin: a.snarkjs.clone(),
                        rapidsnark: rapid.then(|| a.rapidsnark.clone()),
                        witness_bin: a.witness_bin.clone(),
                        work_dir: keep_files
                            .clone()
                            .unwrap_or_else(|| std::env::temp_dir().join("zktls-prove")),
                        keep_files: keep_files.is_some(),
                    },
                    rapid,
                }
            }
        })
    }

    fn name(&self) -> &'static str {
        match self {
            Engine::Dev(_) => "dev-arkworks",
            Engine::Circom { rapid: true, .. } => "circom-rapidsnark",
            Engine::Circom { rapid: false, .. } => "circom-snarkjs",
        }
    }

    /// Conservative default; tune --parallel using measurements on the deployment host.
    fn default_parallel(&self) -> usize {
        let cores = std::thread::available_parallelism().map_or(2, |n| n.get());
        match self {
            Engine::Dev(_) => (cores / 2).max(1),
            Engine::Circom { .. } => (cores / 3).max(1),
        }
    }

    fn prove(
        &self,
        session: &NotarizedSession,
        claim: &ClaimArgs,
    ) -> Result<(ProofBundle, ZkWebProofWitness, ProveTimings)> {
        let w = witness_for(session, claim)?;
        let (proof, inputs, t) = match self {
            Engine::Dev(p) => {
                let t0 = std::time::Instant::now();
                let (proof, inputs) = p.prove(&w, &mut rand::rngs::OsRng)?;
                let t = ProveTimings {
                    witness_ms: 0,
                    prove_ms: t0.elapsed().as_millis() as u64,
                };
                (proof, inputs, t)
            }
            Engine::Circom { prover, .. } => prover.prove_timed(&w)?,
        };
        Ok((ProofBundle::new(self.name(), &w, &proof, &inputs), w, t))
    }
}

#[derive(Clone, Copy, ValueEnum, Deserialize)]
#[serde(rename_all = "lowercase")]
enum MockKind {
    Balance,
    Age,
}

/// Kontrolli i hershëm i nullifier-it on-chain (opsional): pa të, një IBAN i
/// përdorur nga portofol tjetër do të provohej kot dhe refuzohej on-chain.
#[derive(clap::Args, Clone, Default)]
struct PrecheckArgs {
    /// RPC i Solana-s, p.sh. https://api.devnet.solana.com
    #[arg(long, requires = "verifier_program")]
    rpc_url: Option<String>,
    /// Adresa e programit `solana-zk-verifier`
    #[arg(long, requires = "rpc_url")]
    verifier_program: Option<String>,
}

impl PrecheckArgs {
    fn load(&self) -> Result<Option<chain::Precheck>> {
        match (&self.rpc_url, &self.verifier_program) {
            (Some(r), Some(p)) => Ok(Some(chain::Precheck::new(r, p)?)),
            _ => Ok(None),
        }
    }
}

/// Mesazhi kur nullifier-i është i zënë (i njëjti emër si gabimi i programit).
const NULLIFIER_USED: &str = "NullifierAlreadyUsed";

/// `Ok(())` nëse prova ia vlen të bëhet. Gabim i RPC-së → vazhdo (programi vendos).
fn precheck(
    p: Option<&chain::Precheck>,
    session: &NotarizedSession,
    claim: &ClaimArgs,
) -> Result<()> {
    let Some(p) = p else { return Ok(()) };
    let w = witness_for(session, claim)?;
    let nullifier = zktls_prover::solana::fr_to_be(&w.nullifier());
    match p.nullifier_state(&nullifier, &w.wallet) {
        Ok(chain::NullifierState::Free) => Ok(()),
        Ok(chain::NullifierState::UsedBy(owner)) => bail!(
            "{NULLIFIER_USED}: kjo llogari burimore është përdorur tashmë për këtë aplikacion nga portofoli {}",
            bs58::encode(owner).into_string()
        ),
        Err(e) => {
            eprintln!("kontrolli i nullifier-it dështoi ({e:#}); vazhdohet me provën");
            Ok(())
        }
    }
}

/// Çfarë pretendohet: `--min/--max` për vlera, ose `--min-age` për moshë.
#[derive(clap::Args, Clone, Deserialize)]
struct ClaimArgs {
    /// Vlera minimale (p.sh. bilanci ≥ 10000). Alias: --threshold
    #[arg(long, alias = "threshold")]
    #[serde(default, alias = "threshold")]
    min: Option<u64>,
    /// Vlera maksimale (përfshirëse)
    #[arg(long)]
    #[serde(default)]
    max: Option<u64>,
    /// Mosha minimale në vite (përdor fushën `date_of_birth`, data e sotme UTC)
    #[arg(long)]
    #[serde(default)]
    min_age: Option<u64>,
    /// Emri i aplikacionit/kampanjës për nullifier-in (p.sh. "airdrop-2026")
    #[arg(long, default_value = "default")]
    #[serde(default = "default_app")]
    app: String,
}

fn default_app() -> String {
    "default".into()
}

impl ClaimArgs {
    fn range(&self) -> Result<ValueRange> {
        if let Some(years) = self.min_age {
            if self.min.is_some() || self.max.is_some() {
                bail!("--min-age nuk kombinohet me --min/--max");
            }
            let cutoff = birth_cutoff(today_yyyymmdd(), years)
                .ok_or_else(|| anyhow!("min_age duhet të jetë 0..={MAX_AGE_YEARS}"))?;
            return Ok(ValueRange::born_on_or_before(cutoff));
        }
        let r = ValueRange {
            min: self.min.unwrap_or(0),
            max: self.max.unwrap_or(u64::MAX),
        };
        if r.min > r.max {
            bail!("--min > --max");
        }
        Ok(r)
    }
}

/// Data e sotme UTC si YYYYMMDD (algoritmi civil i H. Hinnant).
fn today_yyyymmdd() -> u64 {
    let days = (now() / 86_400) as i64 + 719_468;
    let era = days.div_euclid(146_097);
    let doe = days - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y * 10_000 + m * 100 + d) as u64
}

#[derive(Subcommand)]
enum Cmd {
    /// Krijon një sesion të noterizuar testues (noter i simuluar)
    MockSession {
        #[arg(long)]
        key: PathBuf,
        /// Lloji i të dhënave: bilanc bankar ose datë lindjeje
        #[arg(long, value_enum, default_value_t = MockKind::Balance)]
        kind: MockKind,
        #[arg(long, default_value_t = 15_200)]
        balance: u64,
        /// Data e lindjes për `--kind age` (YYYY-MM-DD)
        #[arg(long, default_value = "2001-05-17")]
        dob: String,
        /// Identifikuesi i llogarisë burimore (IBAN / user-id) – burimi i nullifier-it
        #[arg(long, default_value = "AL35202111090000000001234567")]
        account_id: String,
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
        #[command(flatten)]
        claim: ClaimArgs,
        #[command(flatten)]
        engine: EngineArgs,
        /// VETËM PËR DEBUG/CI: mbaj input.json/proof.json/public.json të snarkjs në këtë drejtori
        /// (përndryshe fshihen pas provës, sepse përmbajnë të dhëna personale)
        #[arg(long)]
        keep_files: Option<PathBuf>,
        /// Ruaj edhe input.json për Circom
        #[arg(long)]
        witness_out: Option<PathBuf>,
        #[command(flatten)]
        precheck: PrecheckArgs,
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
    /// `app_id` (hex BE, 32 bajte) për një emër aplikacioni – argument i `initialize`
    AppId {
        #[arg(long, default_value = "default")]
        app: String,
    },
    /// Daemon HTTP minimal (me CORS): GET /health, POST /prove {session, min, max, min_age, app},
    /// dhe, nëse jepet `--mock-notary-key`, POST /mock-session {kind, balance, dob, account_id, wallet}
    Serve {
        #[arg(long, default_value = "127.0.0.1:8787")]
        addr: String,
        #[command(flatten)]
        engine: EngineArgs,
        /// Provat e njëkohshme (0 = automatikisht sipas bërthamave dhe backend-it)
        #[arg(long, default_value_t = 0)]
        parallel: usize,
        /// VETËM PËR DEMO: aktivizon noterin e simuluar në /mock-session
        #[arg(long)]
        mock_notary_key: Option<PathBuf>,
        #[command(flatten)]
        precheck: PrecheckArgs,
        #[command(flatten)]
        access: AccessArgs,
    },
    /// PDA-ja `["nullifier", nullifier]` (base58), për kontroll me klientët JS
    NullifierPda {
        #[arg(long)]
        program: String,
        /// hex, 32 bajte
        #[arg(long)]
        nullifier: String,
    },
}

/// Kontrolli i aksesit të daemon-it.
#[derive(clap::Args)]
struct AccessArgs {
    /// Çelësat API të lejuar, të ndarë me presje (≥ 16 karaktere secili). Kur jepen,
    /// POST /prove dhe /mock-session kërkojnë header-in `x-api-key`.
    #[arg(
        long,
        env = "PROVER_API_KEYS",
        hide_env_values = true,
        value_delimiter = ','
    )]
    api_keys: Vec<String>,
    /// Kërkesa POST maksimale për IP (IPv6: për /64) në minutë; 0 = pa kufi.
    #[arg(long, env = "PROVER_RATE_LIMIT", default_value_t = 0)]
    rate_limit_per_min: u32,
    /// Header-i me IP-në reale kur serveri është pas një proxy (p.sh. `fly-client-ip`).
    /// Vendoseni VETËM kur proxy-ja e mbishkruan gjithmonë; përndryshe klienti e falsifikon.
    #[arg(long, env = "PROVER_CLIENT_IP_HEADER")]
    client_ip_header: Option<String>,
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

fn witness_for(session: &NotarizedSession, claim: &ClaimArgs) -> Result<ZkWebProofWitness> {
    let range = claim.range()?;
    // Një provë moshe ka kuptim vetëm mbi një datë lindjeje: përndryshe p.sh.
    // `balance: 15200` do të kalonte si "i lindur para 2008".
    if claim.min_age.is_some() && !BIRTH_DATE_KEYS.contains(&session.target_key.as_str()) {
        bail!(
            "--min-age kërkon një sesion me datë lindjeje ({}), jo `{}`",
            BIRTH_DATE_KEYS.join("/"),
            session.target_key
        );
    }
    let vs = session.verify(range.min)?;
    Ok(ZkWebProofWitness::from_session(
        &vs,
        range,
        app_id_from_label(&claim.app),
        None,
    )?)
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::MockSession {
            key,
            kind,
            balance,
            dob,
            account_id,
            wallet,
            out,
        } => {
            let notary = load_notary(&key)?;
            let s = mock_session(
                &notary,
                &MockRequest {
                    kind,
                    balance,
                    dob,
                    account_id,
                    wallet,
                },
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
            claim,
            engine,
            keep_files,
            witness_out,
            precheck: pc,
            out,
        } => {
            let session = read_session(&input)?;
            precheck(pc.load()?.as_ref(), &session, &claim)?;
            let t0 = std::time::Instant::now();
            let (bundle, w, t) = Engine::load(&engine, keep_files)?.prove(&session, &claim)?;
            eprintln!(
                "kohët: dëshmitari {} ms, prova {} ms",
                t.witness_ms, t.prove_ms
            );
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
            let h = k.hash()?;
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
        Cmd::AppId { app } => {
            println!(
                "{}",
                hex::encode(zktls_prover::solana::fr_to_be(&app_id_from_label(&app)))
            );
        }
        Cmd::Serve {
            addr,
            engine,
            parallel,
            mock_notary_key,
            precheck: pc,
            access,
        } => {
            if matches!(engine.backend, Backend::Dev) {
                let bind: std::net::SocketAddr = addr.parse().context("invalid listen address")?;
                if !bind.ip().is_loopback() {
                    bail!("dev backend is restricted to loopback; use rapidsnark for deployed services");
                }
            }
            let pc = pc.load()?;
            if let Some(p) = &pc {
                eprintln!("kontrolli i nullifier-it: {} ", p.rpc_url);
            }
            let notary = mock_notary_key.map(|k| load_notary(&k)).transpose()?;
            if notary.is_some() {
                eprintln!("KUJDES: noteri i simuluar është aktiv në /mock-session (vetëm demo)");
            }
            let engine = Engine::load(&engine, None)?;
            let parallel = if parallel == 0 {
                engine.default_parallel()
            } else {
                parallel
            };
            let api_keys: Vec<String> = access
                .api_keys
                .iter()
                .map(|k| k.trim().to_string())
                .filter(|k| !k.is_empty())
                .collect();
            if let Some(k) = api_keys.iter().find(|k| k.len() < 16) {
                bail!("çelës API shumë i shkurtër ({} < 16 karaktere)", k.len());
            }
            if api_keys.is_empty() && access.rate_limit_per_min == 0 {
                eprintln!(
                    "KUJDES: pa PROVER_API_KEYS dhe pa PROVER_RATE_LIMIT, /prove është i hapur"
                );
            }
            let limiter = (access.rate_limit_per_min > 0)
                .then(|| RateLimiter::new(access.rate_limit_per_min));
            serve(
                &addr,
                Ctx {
                    prover: engine,
                    gate: ProveGate::new(parallel),
                    parallel,
                    notary,
                    pc,
                    api_keys,
                    limiter,
                    ip_header: access.client_ip_header.map(|h| h.to_ascii_lowercase()),
                },
            )?
        }
        Cmd::NullifierPda { program, nullifier } => {
            let program: [u8; 32] = bs58::decode(&program)
                .into_vec()?
                .try_into()
                .map_err(|_| anyhow!("program i pavlefshëm"))?;
            let n: [u8; 32] = hex::decode(&nullifier)?
                .try_into()
                .map_err(|_| anyhow!("nullifier duhet 32 bajte"))?;
            println!(
                "{}",
                bs58::encode(chain::nullifier_pda(&program, &n)).into_string()
            );
        }
    }
    Ok(())
}

fn load_notary(key: &Path) -> Result<MockNotary> {
    Ok(MockNotary {
        key: zktls_prover::eddsa::SigningKey::from_hex(&std::fs::read_to_string(key)?)?,
        tag_secret: None,
    })
}

#[derive(Deserialize)]
struct MockRequest {
    kind: MockKind,
    #[serde(default = "default_balance")]
    balance: u64,
    #[serde(default = "default_dob")]
    dob: String,
    account_id: String,
    wallet: String,
}

fn default_balance() -> u64 {
    15_200
}

fn default_dob() -> String {
    "2001-05-17".into()
}

/// Sesion i noterizuar nga noteri i simuluar (bankë ose regjistër identiteti testues).
fn mock_session(notary: &MockNotary, r: &MockRequest) -> Result<NotarizedSession> {
    let (server, http, key_name) = match r.kind {
        MockKind::Balance => (
            "api.mockbank.test",
            sample_bank_response(r.balance),
            "balance",
        ),
        MockKind::Age => (
            "api.mockregistry.test",
            sample_identity_response(&r.dob),
            "date_of_birth",
        ),
    };
    if r.account_id.trim().is_empty() {
        bail!("account_id mungon");
    }
    Ok(notary.notarize(
        server,
        &http,
        key_name,
        r.account_id.trim(),
        parse_wallet(&r.wallet)?,
        now(),
    )?)
}

#[derive(Deserialize)]
struct ProveRequest {
    session: NotarizedSession,
    #[serde(flatten)]
    claim: ClaimArgs,
}

/// Server HTTP/1.1 minimal, një kërkesë për lidhje. Pa TLS: vendoseni pas një
/// reverse proxy (Fly e bën vetë).
/// Lidhje të njëkohshme maksimale; të tjerat marrin 503.
const MAX_CONNECTIONS: usize = 128;

/// Kufizon provat e njëkohshme (çdo provë përdor vetë disa bërthama): kërkesat
/// e tjera presin në radhë në vend që të mbingarkojnë CPU-në dhe ta ngadalësojnë
/// secilën. Çdo kërkesë ka gjendjen e vet (sesion, dëshmitar, provë): nuk ka
/// gjendje të përbashkët mes klientëve përveç çelësit të provimit (vetëm lexim).
struct ProveGate {
    st: std::sync::Mutex<GateState>,
    cv: std::sync::Condvar,
    /// Kërkesat që mund të presin në radhë; përtej kësaj → 503 menjëherë, në vend
    /// që klienti të presë minuta dhe të bjerë në timeout.
    max_waiting: usize,
}

struct GateState {
    free: usize,
    waiting: usize,
}

/// Kërkesa në pritje për çdo vend prove (~2–4 s secila me rapidsnark).
const QUEUE_PER_SLOT: usize = 32;

impl ProveGate {
    fn new(slots: usize) -> Self {
        let slots = slots.max(1);
        ProveGate {
            st: std::sync::Mutex::new(GateState {
                free: slots,
                waiting: 0,
            }),
            cv: std::sync::Condvar::new(),
            max_waiting: slots * QUEUE_PER_SLOT,
        }
    }

    /// `None` kur radha është plot.
    fn run<T>(&self, f: impl FnOnce() -> T) -> Option<T> {
        let mut st = self.st.lock().unwrap();
        if st.free == 0 && st.waiting >= self.max_waiting {
            return None;
        }
        st.waiting += 1;
        let mut st = self.cv.wait_while(st, |s| s.free == 0).unwrap();
        st.waiting -= 1;
        st.free -= 1;
        drop(st);
        struct Release<'a>(&'a ProveGate);
        impl Drop for Release<'_> {
            fn drop(&mut self) {
                self.0.st.lock().unwrap().free += 1;
                self.0.cv.notify_one();
            }
        }
        let _r = Release(self);
        Some(f())
    }
}
/// Afati i leximit/shkrimit: një klient i ngadaltë nuk e bllokon serverin.
const IO_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
/// Afati total për të lexuar kërkesën (rreshti + header-at + trupi).
const REQUEST_DEADLINE: std::time::Duration = std::time::Duration::from_secs(30);

/// Lexues me afat total. `set_read_timeout` vlen për çdo `read` veç e veç, ndaj
/// një klient që dërgon 1 bajt çdo 9 s (slowloris) do ta mbante lidhjen me orë
/// dhe, me 128 lidhje të tilla, do t'ua zinte vendin të gjithëve.
struct DeadlineReader {
    s: TcpStream,
    until: std::time::Instant,
}

impl Read for DeadlineReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let left = self
            .until
            .saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "kërkesa nuk mbërriti brenda afatit",
            ));
        }
        self.s.set_read_timeout(Some(left.min(IO_TIMEOUT)))?;
        self.s.read(buf)
    }
}

/// Kufi kërkesash për IP me dritare fikse prej një minute.
struct RateLimiter {
    per_min: u32,
    m: std::sync::Mutex<std::collections::HashMap<std::net::IpAddr, (u64, u32)>>,
}

impl RateLimiter {
    fn new(per_min: u32) -> Self {
        RateLimiter {
            per_min,
            m: Default::default(),
        }
    }

    /// IPv4 ashtu siç është; IPv6 sipas /64 (një klient zakonisht ka gjithë /64-ën,
    /// ndaj kufiri për adresë të vetme do të ishte i pakuptimtë).
    fn bucket(ip: std::net::IpAddr) -> std::net::IpAddr {
        match ip.to_canonical() {
            std::net::IpAddr::V6(v6) => {
                let s = v6.segments();
                std::net::IpAddr::V6(std::net::Ipv6Addr::new(s[0], s[1], s[2], s[3], 0, 0, 0, 0))
            }
            v4 => v4,
        }
    }

    /// `Err(sekonda deri në dritaren tjetër)` kur kufiri është kaluar.
    fn check(&self, ip: std::net::IpAddr, now_secs: u64) -> std::result::Result<(), u64> {
        let win = now_secs / 60;
        let mut m = self.m.lock().unwrap();
        if m.len() > 100_000 {
            m.retain(|_, (w, _)| *w == win);
        }
        let e = m.entry(Self::bucket(ip)).or_insert((win, 0));
        if e.0 != win {
            *e = (win, 0);
        }
        if e.1 >= self.per_min {
            return Err(60 - now_secs % 60);
        }
        e.1 += 1;
        Ok(())
    }
}

/// Krahasim në kohë konstante (për çelësat API).
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Gjithçka që i duhet daemon-it për një kërkesë.
struct Ctx {
    prover: Engine,
    gate: ProveGate,
    parallel: usize,
    notary: Option<MockNotary>,
    pc: Option<chain::Precheck>,
    api_keys: Vec<String>,
    limiter: Option<RateLimiter>,
    ip_header: Option<String>,
}

/// Klienti e ka mbyllur lidhjen (p.sh. mbylli faqen) gjatë kohës në radhë: prova
/// nuk do t'i arrinte askujt, ndaj nuk ia vlen ta zemë CPU-në 2 s.
fn client_gone(s: &TcpStream) -> bool {
    if s.set_nonblocking(true).is_err() {
        return false;
    }
    let mut b = [0u8; 1];
    let gone = match s.peek(&mut b) {
        Ok(0) => true,
        Ok(_) => false,
        Err(e) => e.kind() != std::io::ErrorKind::WouldBlock,
    };
    let _ = s.set_nonblocking(false);
    gone
}

fn serve(addr: &str, ctx: Ctx) -> Result<()> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    println!(
        "backend {}, {} prova njëkohësisht{}{}",
        ctx.prover.name(),
        ctx.parallel,
        if ctx.api_keys.is_empty() {
            ""
        } else {
            ", me çelës API"
        },
        match &ctx.limiter {
            Some(l) => format!(", ≤ {}/min për IP", l.per_min),
            None => String::new(),
        }
    );
    let ctx = Arc::new(ctx);
    let active = Arc::new(AtomicUsize::new(0));
    let listener = TcpListener::bind(addr)?;
    println!("coprocessor po dëgjon në http://{addr}");
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };
        let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
        let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
        if active.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
            active.fetch_sub(1, Ordering::SeqCst);
            let _ = respond(&mut stream, 503, r#"{"error":"server i zënë"}"#);
            continue;
        }
        let (ctx, active) = (ctx.clone(), active.clone());
        // një thread për lidhje: një kërkesë e ngadaltë ose e gabuar nuk ndal të tjerat
        std::thread::spawn(move || {
            // zbret numëruesin edhe nëse trajtimi bën panik
            struct Slot(Arc<AtomicUsize>);
            impl Drop for Slot {
                fn drop(&mut self) {
                    self.0.fetch_sub(1, Ordering::SeqCst);
                }
            }
            let _slot = Slot(active);
            if let Err(e) = handle(&ctx, &mut stream) {
                let _ = respond(
                    &mut stream,
                    400,
                    &serde_json::json!({ "error": e.to_string() }).to_string(),
                );
            }
        });
    }
    Ok(())
}

fn handle(ctx: &Ctx, stream: &mut TcpStream) -> Result<()> {
    let (prover, gate, notary, pc) = (&ctx.prover, &ctx.gate, ctx.notary.as_ref(), ctx.pc.as_ref());
    // Rreshti i kërkesës + header-at: gjithsej ≤ 16 KiB (pa kufi, një klient mund të
    // dërgonte header-a me madhësi arbitrare dhe të mbushte memorien).
    const MAX_HEAD: u64 = 16 * 1024;
    let mut reader = BufReader::new(
        DeadlineReader {
            s: stream.try_clone()?,
            until: std::time::Instant::now() + REQUEST_DEADLINE,
        }
        .take(MAX_HEAD),
    );
    let mut line = String::new();
    reader.read_line(&mut line)?;
    if !line.ends_with('\n') {
        bail!("rreshti i kërkesës është i paplotë ose > 16 KiB");
    }
    let mut parts = line.split_whitespace();
    let (method, path) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    let mut len: Option<usize> = None;
    let mut api_key: Option<String> = None;
    let mut fwd_ip: Option<std::net::IpAddr> = None;
    loop {
        let mut h = String::new();
        reader.read_line(&mut h)?;
        if !h.ends_with('\n') {
            bail!("header-at e kërkesës janë të paplota ose > 16 KiB");
        }
        if h == "\r\n" || h == "\n" {
            break;
        }
        let lower = h.to_ascii_lowercase();
        if let Some(v) = lower.strip_prefix("content-length:") {
            // dy Content-Length (ose chunked) janë baza e "request smuggling" pas një proxy-je
            if len.replace(v.trim().parse()?).is_some() {
                bail!("kërkesa ka disa Content-Length");
            }
        }
        if lower.starts_with("transfer-encoding:") {
            bail!("Transfer-Encoding nuk mbështetet; përdorni Content-Length");
        }
        if let Some((name, value)) = h.split_once(':') {
            let name = name.trim().to_ascii_lowercase();
            if name == "x-api-key" {
                api_key = Some(value.trim().to_string());
            } else if ctx.ip_header.as_deref() == Some(name.as_str()) {
                // proxy-t e shtojnë në fund; i pari është klienti
                fwd_ip = value.split(',').next().and_then(|v| v.trim().parse().ok());
            }
        }
    }
    let len = len.unwrap_or(0);
    if len > 1 << 20 {
        bail!("trupi i kërkesës > 1 MiB");
    }
    // trupi: saktësisht `len` bajte (bajtet e buferuara + `len` të tjera mjaftojnë)
    reader.get_mut().set_limit(len as u64);
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body)?;
    if method == "POST" && matches!(path, "/prove" | "/mock-session") {
        if !ctx.api_keys.is_empty() {
            let ok = api_key.as_deref().is_some_and(|k| {
                ctx.api_keys
                    .iter()
                    .fold(false, |hit, a| hit | ct_eq(a.as_bytes(), k.as_bytes()))
            });
            if !ok {
                return respond(
                    stream,
                    401,
                    r#"{"error":"mungon ose është i gabuar x-api-key"}"#,
                );
            }
        }
        if let Some(l) = &ctx.limiter {
            let ip = match fwd_ip {
                Some(ip) => ip,
                None => stream.peer_addr()?.ip(),
            };
            if let Err(wait) = l.check(ip, now()) {
                return respond_with(
                    stream,
                    429,
                    &serde_json::json!({
                        "error": format!("shumë kërkesa; provo pas {wait} s")
                    })
                    .to_string(),
                    Some(wait),
                );
            }
        }
    }
    match (method, path) {
        ("OPTIONS", _) => respond(stream, 204, ""),
        ("GET", "/health") => respond(
            stream,
            200,
            &serde_json::json!({
                "status": "ok",
                "backend": prover.name(),
                "mock_notary": notary.is_some()
            })
            .to_string(),
        ),
        ("POST", "/mock-session") => {
            let n = notary.ok_or_else(|| anyhow!("noteri i simuluar nuk është aktiv"))?;
            let req: MockRequest = serde_json::from_slice(&body)?;
            respond(
                stream,
                200,
                &serde_json::to_string(&mock_session(n, &req)?)?,
            )
        }
        ("POST", "/prove") => {
            let req: ProveRequest = serde_json::from_slice(&body)?;
            // para radhës: një nullifier i zënë refuzohet në milisekonda, pa zënë CPU
            if let Err(e) = precheck(pc, &req.session, &req.claim) {
                let code = if e.to_string().starts_with(NULLIFIER_USED) {
                    409
                } else {
                    400
                };
                return respond(
                    stream,
                    code,
                    &serde_json::json!({ "error": e.to_string() }).to_string(),
                );
            }
            let run = || (!client_gone(stream)).then(|| prover.prove(&req.session, &req.claim));
            let Some(res) = gate.run(run) else {
                return respond(
                    stream,
                    503,
                    r#"{"error":"radha e provave është plot, provo pas pak sekondash"}"#,
                );
            };
            // klienti u largua ndërkohë: s'ka kujt t'i përgjigjemi
            let Some(res) = res else { return Ok(()) };
            let (bundle, _, _) = res?;
            respond(stream, 200, &serde_json::to_string(&bundle)?)
        }
        _ => respond(stream, 404, r#"{"error":"not found"}"#),
    }
}

fn respond(stream: &mut TcpStream, code: u16, body: &str) -> Result<()> {
    // 503 = i zënë: klienti mund të provojë sërish pas pak
    respond_with(stream, code, body, (code == 503).then_some(5))
}

fn respond_with(
    stream: &mut TcpStream,
    code: u16,
    body: &str,
    retry_after: Option<u64>,
) -> Result<()> {
    let reason = match code {
        200 => "OK",
        204 => "No Content",
        401 => "Unauthorized",
        409 => "Conflict",
        429 => "Too Many Requests",
        503 => "Service Unavailable",
        404 => "Not Found",
        _ => "Bad Request",
    };
    let retry = retry_after
        .map(|s| format!("Retry-After: {s}\r\n"))
        .unwrap_or_default();
    write!(
        stream,
        "HTTP/1.1 {code} {reason}\r\nContent-Type: application/json\r\n{retry}\
         Access-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\n\
         Access-Control-Allow-Headers: content-type, x-api-key\r\n\
         Access-Control-Expose-Headers: retry-after\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{mpsc, Arc};

    fn wait_until(g: &ProveGate, f: impl Fn(&GateState) -> bool) {
        while !f(&g.st.lock().unwrap()) {
            std::thread::yield_now();
        }
    }

    #[test]
    fn full_circuit_is_the_default() {
        let cli = Cli::try_parse_from(["coprocessor", "serve"]).unwrap();
        let Cmd::Serve { engine, .. } = cli.cmd else {
            panic!("serve")
        };
        assert!(matches!(engine.backend, Backend::Rapidsnark));
        let cli = Cli::try_parse_from(["coprocessor", "serve", "--backend", "dev"]).unwrap();
        let Cmd::Serve { engine, .. } = cli.cmd else {
            panic!("serve")
        };
        assert!(matches!(engine.backend, Backend::Dev));
    }

    #[test]
    fn gate_queue_is_bounded() {
        let g = Arc::new(ProveGate {
            st: std::sync::Mutex::new(GateState {
                free: 1,
                waiting: 0,
            }),
            cv: std::sync::Condvar::new(),
            max_waiting: 1,
        });
        // A zë vendin, B pret në radhë
        let (ta, ra) = mpsc::channel::<()>();
        let ga = g.clone();
        let a = std::thread::spawn(move || ga.run(|| ra.recv().unwrap()));
        wait_until(&g, |s| s.free == 0);
        let (tb, rb) = mpsc::channel::<()>();
        let gb = g.clone();
        let b = std::thread::spawn(move || gb.run(|| rb.recv().unwrap()));
        wait_until(&g, |s| s.waiting == 1);
        // C: radha plot → refuzohet menjëherë
        assert!(g.run(|| ()).is_none());
        ta.send(()).unwrap();
        assert_eq!(a.join().unwrap(), Some(()));
        tb.send(()).unwrap();
        assert_eq!(b.join().unwrap(), Some(()));
        assert_eq!(g.run(|| 5), Some(5));
        let st = g.st.lock().unwrap();
        assert_eq!((st.free, st.waiting), (1, 0));
    }

    #[test]
    fn rate_limit_per_ip_and_v6_prefix() {
        let l = RateLimiter::new(2);
        let a: std::net::IpAddr = "1.2.3.4".parse().unwrap();
        let t = 6_000_030; // 30 s brenda dritares
        assert!(l.check(a, t).is_ok());
        assert!(l.check(a, t).is_ok());
        assert_eq!(l.check(a, t), Err(30));
        assert!(l.check("1.2.3.5".parse().unwrap(), t).is_ok());
        assert!(l.check(a, t + 30).is_ok()); // dritare e re
                                             // e njëjta /64 = i njëjti klient; IPv4 i mapuar = IPv4
        let v6 = |s: &str| s.parse::<std::net::IpAddr>().unwrap();
        assert!(l.check(v6("2001:db8::1"), t).is_ok());
        assert!(l.check(v6("2001:db8::ffff:1"), t).is_ok());
        assert!(l.check(v6("2001:db8::2"), t).is_err());
        assert!(l.check(v6("::ffff:1.2.3.5"), t).is_ok());
        assert!(l.check("1.2.3.5".parse().unwrap(), t).is_err());
    }

    #[test]
    fn api_key_compare() {
        assert!(ct_eq(b"abcdefghijklmnop", b"abcdefghijklmnop"));
        assert!(!ct_eq(b"abcdefghijklmnop", b"abcdefghijklmnoq"));
        assert!(!ct_eq(b"abc", b"abcd"));
    }

    #[test]
    fn detects_closed_client() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let c = TcpStream::connect(l.local_addr().unwrap()).unwrap();
        let (s, _) = l.accept().unwrap();
        assert!(!client_gone(&s));
        drop(c);
        // mbyllja mbërrin pas pak
        let t0 = std::time::Instant::now();
        while !client_gone(&s) {
            assert!(t0.elapsed() < std::time::Duration::from_secs(5));
            std::thread::yield_now();
        }
    }
}
