import { useCallback, useEffect, useMemo, useState } from "react";
import * as anchor from "@coral-xyz/anchor";
import { useAnchorWallet, useConnection, useWallet } from "@solana/wallet-adapter-react";
import { WalletMultiButton } from "@solana/wallet-adapter-react-ui";
import { PublicKey, LAMPORTS_PER_SOL } from "@solana/web3.js";
import { APP_LABEL, MIN_AGE, MIN_BALANCE, POOL_AUTHORITY, PROVER_URL, RPC_URL } from "./config";
import {
  Attestation,
  ProofBundle,
  U64_MAX,
  depositToPool,
  explainError,
  fetchAttestation,
  poolPda,
  positionPda,
  verifyAndAttest,
} from "./sdk";

type Kind = "balance" | "age";

async function post<T>(path: string, body: unknown): Promise<T> {
  const r = await fetch(`${PROVER_URL}${path}`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
  const j = await r.json();
  if (!r.ok) throw new Error(j.error ?? `HTTP ${r.status}`);
  return j as T;
}

// "10.000" (stili shqip/evropian; Chromium s'ka të dhëna për "sq-AL")
const fmtNum = (n: number | bigint) => n.toLocaleString("de-DE");
const short = (s: string, n = 6) => (s.length > 2 * n + 1 ? `${s.slice(0, n)}…${s.slice(-n)}` : s);
const fmtDate = (v: bigint) => {
  const s = v.toString();
  return `${s.slice(6, 8)}.${s.slice(4, 6)}.${s.slice(0, 4)}`;
};
const explorer = (sig: string) =>
  `https://explorer.solana.com/tx/${sig}?cluster=custom&customUrl=${encodeURIComponent(RPC_URL)}`;

function Step(props: { n: number; title: string; done?: boolean; active?: boolean; children?: React.ReactNode }) {
  const cls = props.done ? "step done" : props.active ? "step active" : "step";
  return (
    <section className={cls}>
      <header>
        <span className="num">{props.done ? "✓" : props.n}</span>
        <h2>{props.title}</h2>
      </header>
      {props.active || props.done ? <div className="body">{props.children}</div> : null}
    </section>
  );
}

export function App() {
  const { connection } = useConnection();
  const { publicKey } = useWallet();
  const wallet = useAnchorWallet();

  const [program, setProgram] = useState<anchor.Program<any> | null>(null);
  const [setupError, setSetupError] = useState<string>("");
  const [kind, setKind] = useState<Kind>("balance");
  const [balance, setBalance] = useState("15200");
  const [dob, setDob] = useState("2001-05-17");
  const [accountId, setAccountId] = useState("AL35202111090000000001234567");
  const [session, setSession] = useState<any>(null);
  const [proof, setProof] = useState<ProofBundle | null>(null);
  const [proofMs, setProofMs] = useState(0);
  const [sig, setSig] = useState("");
  const [att, setAtt] = useState<Attestation | null>(null);
  const [busy, setBusy] = useState("");
  const [error, setError] = useState("");

  // Programi nga IDL-ja (`npm run init-localnet` e kopjon te public/idl.json)
  useEffect(() => {
    if (!wallet) return setProgram(null);
    fetch("/idl.json")
      .then((r) =>
        r.ok && (r.headers.get("content-type") ?? "").includes("json")
          ? r.json()
          : Promise.reject(new Error("Mungon IDL-ja e programit (public/idl.json): ekzekutoni `npm run init-localnet`."))
      )
      .then((idl) => {
        const provider = new anchor.AnchorProvider(connection, wallet, { commitment: "confirmed" });
        setProgram(new anchor.Program(idl, provider));
      })
      .catch((e) => setSetupError(String(e.message ?? e)));
  }, [connection, wallet]);

  // Shembulli i DEX-it (opsional): `npm run init-localnet` kopjon public/pool-idl.json
  const [poolProgram, setPoolProgram] = useState<anchor.Program<any> | null>(null);
  const [deposited, setDeposited] = useState<bigint | null>(null);
  const [poolSig, setPoolSig] = useState("");
  useEffect(() => {
    if (!wallet || !POOL_AUTHORITY) return setPoolProgram(null);
    fetch("/pool-idl.json")
      .then((r) => (r.ok && (r.headers.get("content-type") ?? "").includes("json") ? r.json() : null))
      .then((idl) => {
        if (!idl) return;
        const provider = new anchor.AnchorProvider(connection, wallet, { commitment: "confirmed" });
        setPoolProgram(new anchor.Program(idl, provider));
      })
      .catch(() => setPoolProgram(null));
  }, [connection, wallet]);
  const refreshPosition = useCallback(async () => {
    if (!poolProgram || !publicKey) return setDeposited(null);
    const p = poolPda(poolProgram.programId, new PublicKey(POOL_AUTHORITY));
    const pos = await (poolProgram.account as any).position.fetchNullable(positionPda(poolProgram.programId, p, publicKey));
    setDeposited(pos ? BigInt(pos.amount.toString()) : 0n);
  }, [poolProgram, publicKey]);
  useEffect(() => {
    refreshPosition().catch(() => setDeposited(null));
  }, [refreshPosition]);
  const deposit = () =>
    run("Pool-i po kontrollon atestimin…", async () => {
      const s = await depositToPool(
        poolProgram!,
        program!.programId,
        new PublicKey(POOL_AUTHORITY),
        publicKey!,
        0.1 * LAMPORTS_PER_SOL
      );
      setPoolSig(s);
      await refreshPosition();
    });

  const refresh = useCallback(async () => {
    if (program && publicKey) setAtt(await fetchAttestation(program, publicKey).catch(() => null));
  }, [program, publicKey]);
  useEffect(() => {
    refresh();
  }, [refresh]);

  // ndryshimi i zgjedhjes fshin hapat pasardhës
  const reset = () => {
    setSession(null);
    setProof(null);
    setSig("");
    setError("");
  };
  useEffect(reset, [publicKey]);
  const choose = (k: Kind) => {
    setKind(k);
    reset();
  };
  useEffect(() => setAccountId(kind === "balance" ? "AL35202111090000000001234567" : "J01234567A"), [kind]);

  const run = async (label: string, f: () => Promise<void>) => {
    setBusy(label);
    setError("");
    try {
      await f();
    } catch (e) {
      setError(explainError(e));
    } finally {
      setBusy("");
    }
  };

  const connectSource = () =>
    run("Noteri po e dëshmon lidhjen…", async () => {
      const s = await post("/mock-session", {
        kind,
        balance: Number(balance),
        dob,
        account_id: accountId,
        wallet: publicKey!.toBase58(),
      });
      setSession(s);
      setProof(null);
      setSig("");
    });

  // Sesion real nga noteri TLSNotary + ura (`zktls-fetch --bridge ... --out session.tls.json`)
  const uploadSession = (f: File | undefined) =>
    f &&
    run("Po lexohet sesioni…", async () => {
      const s = JSON.parse(await f.text());
      if (!s.attestation || !s.http_response) throw new Error("Skedari nuk është session.tls.json");
      setKind(["date_of_birth", "birth_date", "birthdate", "dob"].includes(s.target_key) ? "age" : "balance");
      setProof(null);
      setSig("");
      setSession(s);
    });

  const prove = () =>
    run("Po gjenerohet prova ZK…", async () => {
      const t0 = performance.now();
      const claim = kind === "balance" ? { min: MIN_BALANCE } : { min_age: MIN_AGE };
      const p = await post<ProofBundle>("/prove", { session, app: APP_LABEL, ...claim });
      if (p.wallet !== publicKey!.toBase58())
        throw new Error(`Sesioni është lidhur me portofolin ${short(p.wallet)}; lidhni atë portofol ose krijoni sesion të ri.`);
      setProofMs(Math.round(performance.now() - t0));
      setProof(p);
    });

  const submit = () =>
    run("Po pritet nënshkrimi dhe konfirmimi…", async () => {
      if (!program) throw new Error(setupError || "programi nuk u ngarkua");
      setSig(await verifyAndAttest(program, publicKey!, proof!));
      await refresh();
    });

  const claimText = useMemo(
    () => (kind === "balance" ? `bilanci ≥ ${fmtNum(MIN_BALANCE)} €` : `mosha ≥ ${MIN_AGE} vjeç`),
    [kind]
  );

  const connected = !!publicKey;
  const step = !connected ? 1 : !session ? 2 : !proof ? 3 : !sig ? 4 : 5;

  return (
    <main>
      <div className="top">
        <div>
          <h1>Provo pa zbuluar</h1>
          <p className="lede">
            Provo te një aplikacion Solana që <b>{claimText}</b>, pa treguar bilancin, IBAN-in apo datëlindjen.
          </p>
        </div>
        <WalletMultiButton />
      </div>

      <div className="banner">
        Demo: noter i simuluar, ose sesion real TLSNotary nga <code>zktls-fetch</code>. Aplikacioni:{" "}
        <code>{APP_LABEL}</code>
      </div>

      {att && (
        <div className="verified">
          <strong>✓ Ky portofol është i verifikuar</strong>
          <span>
            {att.isBirthDate
              ? `i lindur më ose para ${fmtDate(att.maxValue)}`
              : att.maxValue === U64_MAX
                ? `${att.claim} ≥ ${fmtNum(att.minValue)}`
                : `${fmtNum(att.minValue)} ≤ ${att.claim} ≤ ${fmtNum(att.maxValue)}`}
            {" · "}nullifier <code>{short(att.nullifier)}</code>
          </span>
        </div>
      )}

      <Step n={1} title="Lidh portofolin" done={connected} active={step === 1}>
        {connected ? (
          <p className="muted">
            <code>{short(publicKey!.toBase58(), 8)}</code>, prova do të lidhet vetëm me këtë portofol.
          </p>
        ) : (
          <p className="muted">Phantom, Solflare ose Backpack.</p>
        )}
      </Step>

      <Step n={2} title="Zgjidh çfarë provon" done={step > 2} active={step === 2}>
        <div className="choices">
          <button className={kind === "balance" ? "choice on" : "choice"} onClick={() => choose("balance")}>
            <b>Bilanc bankar</b>
            <span>≥ {fmtNum(MIN_BALANCE)} €</span>
          </button>
          <button className={kind === "age" ? "choice on" : "choice"} onClick={() => choose("age")}>
            <b>Mosha</b>
            <span>≥ {MIN_AGE} vjeç</span>
          </button>
        </div>
        {step === 2 && (
          <div className="form">
            <p className="muted small">
              Simulim: këto vlera i "kthen" banka/regjistri testues. Në versionin real përdoruesi hyn në faqen e
              bankës dhe asgjë nga këto nuk shkruhet këtu.
            </p>
            {kind === "balance" ? (
              <label>
                Bilanci në bankë (€)
                <input inputMode="numeric" value={balance} onChange={(e) => setBalance(e.target.value.replace(/\D/g, ""))} />
              </label>
            ) : (
              <label>
                Data e lindjes
                <input type="date" value={dob} onChange={(e) => setDob(e.target.value)} />
              </label>
            )}
            <label>
              {kind === "balance" ? "IBAN" : "Nr. personal"}
              <input value={accountId} onChange={(e) => setAccountId(e.target.value)} />
            </label>
            <button className="primary" disabled={!!busy} onClick={connectSource}>
              {kind === "balance" ? "Lidhu me bankën (simulim)" : "Lidhu me regjistrin (simulim)"}
            </button>
            <label className="upload">
              ose ngarko <code>session.tls.json</code> nga noteri real (<code>zktls-fetch</code>)
              <input type="file" accept="application/json,.json" onChange={(e) => uploadSession(e.target.files?.[0])} />
            </label>
          </div>
        )}
      </Step>

      <Step n={3} title="Gjenero provën" done={step > 3} active={step === 3}>
        {session && (
          <p className="muted">
            Noteri dëshmoi përgjigjen nga <code>{session.server_name}</code>. Të dhënat mbeten te ti dhe nuk dalin
            on-chain.
          </p>
        )}
        {step === 3 ? (
          <button className="primary" disabled={!!busy} onClick={prove}>
            Gjenero provën ZK
          </button>
        ) : (
          proof && (
            <p className="muted">
              Prova u krijua për {proofMs < 1000 ? `${proofMs} ms` : `${(proofMs / 1000).toFixed(1)} s`} · nullifier <code>{short(proof.nullifier)}</code>
            </p>
          )
        )}
      </Step>

      <Step n={4} title="Nënshkruaj dhe dërgo në Solana" done={step > 4} active={step === 4}>
        {step === 4 ? (
          <>
            <p className="muted">
              Programi verifikon provën on-chain (~120k CU) dhe ruan vetëm: kushtin, nullifier-in dhe kohën.
            </p>
            <button className="primary" disabled={!!busy || !program} onClick={submit}>
              Dërgo provën
            </button>
            {setupError && <p className="err small">{setupError}</p>}
          </>
        ) : (
          sig && (
            <p className="muted">
              U konfirmua.{" "}
              <a href={explorer(sig)} target="_blank" rel="noreferrer">
                Shiko transaksionin ↗
              </a>
            </p>
          )
        )}
      </Step>

      {poolProgram && connected && (
        <Step n={5} title="Përdore te një DEX: pool i mbrojtur" done={!!poolSig} active>
          <p className="muted">
            Një program tjetër (<code>gated-pool</code>) pranon depozita vetëm nga portofole që kanë vërtetuar{" "}
            <b>bilanc ≥ {fmtNum(MIN_BALANCE)}</b>. Ai nuk sheh asnjë të dhënë bankare: lexon vetëm atestimin on-chain.
          </p>
          <button className="primary" disabled={!!busy || !program} onClick={deposit}>
            Depozito 0.1 SOL
          </button>
          {deposited !== null && deposited > 0n && (
            <p className="muted">
              Pozicioni yt: {Number(deposited) / LAMPORTS_PER_SOL} SOL
              {poolSig && (
                <>
                  {" · "}
                  <a href={explorer(poolSig)} target="_blank" rel="noreferrer">
                    transaksioni ↗
                  </a>
                </>
              )}
            </p>
          )}
        </Step>
      )}

      {busy && <div className="status">{busy}</div>}
      {error && <div className="error">{error}</div>}

      <footer>
        RPC <code>{RPC_URL}</code> · prover <code>{PROVER_URL}</code>
      </footer>
    </main>
  );
}
