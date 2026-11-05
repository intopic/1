// SDK minimal për integruesit (DEX, wallet, DAO):
//   * `isVerified(program, wallet, kërkesa)` – a e plotëson atestimi kushtin (si zk-gate on-chain)
//   * `verifyAndAttest(program, proof)` – dërgon provën nga daemon-i
import { Buffer } from "buffer";
import * as anchor from "@coral-xyz/anchor";
import { ComputeBudgetProgram, PublicKey } from "@solana/web3.js";

/** Formati i `proof.json` / përgjigjes së `POST /prove`. */
export const U64_MAX = 18446744073709551615n;

export interface ProofBundle {
  backend: string;
  wallet: string;
  min_value: number | string;
  max_value: number | string;
  nullifier: string;
  attested_at: number;
  proof_a: string;
  proof_b: string;
  proof_c: string;
  public_inputs: string[];
}

export const hexBytes = (s: string) => Array.from(Buffer.from(s, "hex"));

// seed-i: claim_key_id si u64 little-endian (si argumentet u64 të Anchor)
const le8 = (v: bigint) => Buffer.from(v.toString(16).padStart(16, "0"), "hex").reverse();

/** Një atestim për çdo (portofol, lloj pretendimi): bilanci dhe mosha janë të pavarura. */
export const attestationPda = (programId: PublicKey, wallet: PublicKey, claim: string = "balance") =>
  PublicKey.findProgramAddressSync(
    [Buffer.from("attestation"), wallet.toBuffer(), le8(CLAIM_KEYS[claim])],
    programId
  )[0];

export const nullifierPda = (programId: PublicKey, nullifierHex: string) =>
  PublicKey.findProgramAddressSync([Buffer.from("nullifier"), Buffer.from(nullifierHex, "hex")], programId)[0];

/** `claim_key_id` = SHA-256(çelësi JSON)[0..8] si u64 BE. */
export const CLAIM_KEYS: Record<string, bigint> = {
  balance: 6292056758205541950n,
  date_of_birth: 10499278090922323829n,
  birth_date: 249661563938424948n,
  birthdate: 14520292931301669765n,
  dob: 4057514819593230876n,
};
const BIRTH_KEYS = new Set(["date_of_birth", "birth_date", "birthdate", "dob"]);

export interface Attestation {
  user: PublicKey;
  /** Hash-i (hex) i çelësit të bankës/urës që e lëshoi. */
  bankKeyHash: string;
  /** Çelësi JSON i provuar (p.sh. "balance", "date_of_birth"), ose id-ja hex nëse nuk njihet. */
  claim: string;
  /** true kur pretendimi është mbi një datë lindjeje (`maxValue` = data kufi YYYYMMDD). */
  isBirthDate: boolean;
  minValue: bigint;
  maxValue: bigint;
  nullifier: string;
  attestedAt: number;
}

/** Lexon atestimin e një portofoli për një lloj pretendimi (null nëse nuk ekziston). */
export async function fetchAttestation(
  program: anchor.Program<any>,
  wallet: PublicKey,
  claimKey: string = "balance"
): Promise<Attestation | null> {
  const rec: any = await (program.account as any).attestationRecord.fetchNullable(
    attestationPda(program.programId, wallet, claimKey)
  );
  if (!rec) return null;
  const id = BigInt(rec.claimKeyId.toString());
  const claim = Object.keys(CLAIM_KEYS).find((k) => CLAIM_KEYS[k] === id) ?? id.toString(16);
  return {
    user: rec.user,
    bankKeyHash: Buffer.from(rec.bankKeyHash).toString("hex"),
    claim,
    isBirthDate: BIRTH_KEYS.has(claim),
    minValue: BigInt(rec.minValue.toString()),
    maxValue: BigInt(rec.maxValue.toString()),
    nullifier: Buffer.from(rec.nullifier).toString("hex"),
    attestedAt: Number(rec.attestedAt.toString()),
  };
}

/** Kushti i aplikacionit, si `gate::Requirement` on-chain. */
export interface Requirement {
  claim?: string;
  /** Prova duhet të ketë vërtetuar `vlera >= minValue`. */
  minValue?: bigint;
  /** Prova duhet të ketë vërtetuar `vlera <= maxValue`. */
  maxValue?: bigint;
  /** Sesioni TLS jo më i vjetër se kaq sekonda (rekomandohet gjithmonë për bilanc). */
  maxAgeSecs?: number;
  /** Pranohet vetëm ky burim (hash hex i çelësit të bankës/urës). */
  bankKeyHash?: string;
}

/** Kontrolli i pastër i `Requirement` mbi një atestim (pa rrjet), si `gate::check`. */
export function meetsRequirement(a: Attestation, req: Requirement, nowSecs = Math.floor(Date.now() / 1000)): boolean {
  if (req.claim && a.claim !== req.claim) return false;
  if (a.minValue < (req.minValue ?? 0n) || a.maxValue > (req.maxValue ?? U64_MAX)) return false;
  if (req.maxAgeSecs && req.maxAgeSecs > 0 && nowSecs - a.attestedAt > req.maxAgeSecs) return false;
  if (req.bankKeyHash && a.bankKeyHash !== req.bankKeyHash.toLowerCase()) return false;
  return true;
}

/**
 * A e plotëson atestimi on-chain i portofolit kushtin e aplikacionit? Ekzistenca e
 * llogarisë nuk mjafton: dikush mund të ketë vërtetuar vetëm "bilanc ≥ 0". Kontrolli
 * off-chain është vetëm për UI; vendimi që lëviz para bëhet on-chain me zk-gate.
 */
export async function isVerified(
  program: anchor.Program<any>,
  wallet: PublicKey,
  req: Requirement & { claim: string; minValue: bigint }
): Promise<boolean> {
  const a = await fetchAttestation(program, wallet, req.claim);
  return a !== null && meetsRequirement(a, req);
}

/** Dërgon provën te programi. Kthen nënshkrimin e transaksionit. */
export async function verifyAndAttest(program: anchor.Program<any>, user: PublicKey, p: ProofBundle): Promise<string> {
  return program.methods
    .verifyAndAttest(
      hexBytes(p.proof_a),
      hexBytes(p.proof_b),
      hexBytes(p.proof_c),
      p.public_inputs.map(hexBytes),
      hexBytes(p.nullifier),
      // claim_key_id = 8 bajtët e fundit të hyrjes publike 2
      new anchor.BN(Buffer.from(p.public_inputs[2], "hex").subarray(24))
    )
    .accountsPartial({ user, nullifierRecord: nullifierPda(program.programId, p.nullifier) })
    .preInstructions([ComputeBudgetProgram.setComputeUnitLimit({ units: 400_000 })])
    .rpc({ commitment: "confirmed" });
}

/** Mesazhe të kuptueshme për gabimet e programit dhe të daemon-it. */
export function explainError(e: unknown): string {
  const s = String((e as any)?.message ?? e);
  const map: [RegExp, string][] = [
    [/GateThresholdNotMet/, "Prova jote vërteton një prag më të ulët se ç'kërkon ky pool."],
    [/GateClaimMismatch/, "Ky pool kërkon një provë bilanci, jo moshe."],
    [/GateWrongAccount/, "Atestimi i dhënë nuk është i këtij portofoli ose i këtij lloji."],
    [/GateExpired/, "Atestimi është më i vjetër se ç'pranon ky pool. Lidhu sërish me bankën dhe provo përsëri."],
    [/GateSourceRevoked/, "Banka/ura që lëshoi atestimin nuk është më e autorizuar. Verifikohu përsëri."],
    [/GateWrongSource/, "Ky aplikacion pranon vetëm një burim tjetër (bankë/urë)."],
    [/MissingAttestation/, "Ky pool kërkon edhe një atestim tjetër (bilanc ose moshë)."],
    [/AccountNotInitialized|3012/, "Ky portofol nuk ka ende atestim on-chain: kryej hapat 2–4."],
    [/NullifierAlreadyUsed/, "Kjo llogari është përdorur tashmë nga një portofol tjetër."],
    [/kushti nuk plotësohet|ThresholdNotMet/, "Të dhënat nuk e plotësojnë kushtin (bilanci ose mosha)."],
    [/StaleAttestation/, "Atestimi ka skaduar. Lidhu sërish me burimin."],
    [/AppIdMismatch/, "Prova është për një aplikacion tjetër."],
    [/ClaimKeyNotAllowed/, "Ky aplikacion nuk pranon këtë lloj prove."],
    [/kërkon një sesion me datë lindjeje/, "Prova e moshës kërkon një datë lindjeje, jo bilanc."],
    [/BankKeyNotWhitelisted/, "Noteri/banka nuk është e autorizuar në këtë program."],
    [/WalletMismatch/, "Prova është për një portofol tjetër."],
    [/User rejected|rejected the request/i, "Transaksioni u anulua në portofol."],
    [/Failed to fetch|NetworkError/, "Nuk u arrit daemon-i ose RPC-ja. A janë ndezur?"],
  ];
  for (const [re, msg] of map) if (re.test(s)) return msg;
  return s.length > 240 ? s.slice(0, 240) + "…" : s;
}

// ---- Shembulli i një DEX-i: pool që kërkon atestimin (programs/gated-pool) ----
export const poolPda = (poolProgramId: PublicKey, authority: PublicKey) =>
  PublicKey.findProgramAddressSync([Buffer.from("pool"), authority.toBuffer()], poolProgramId)[0];
export const positionPda = (poolProgramId: PublicKey, pool: PublicKey, wallet: PublicKey) =>
  PublicKey.findProgramAddressSync([Buffer.from("position"), pool.toBuffer(), wallet.toBuffer()], poolProgramId)[0];

/** Depozitë në pool: programi i pool-it lexon atestimin zkTLS të portofolit. */
export async function depositToPool(
  pool: anchor.Program<any>,
  verifierId: PublicKey,
  authority: PublicKey,
  user: PublicKey,
  lamports: number
): Promise<string> {
  const p = poolPda(pool.programId, authority);
  return pool.methods
    .deposit(new anchor.BN(lamports))
    .accountsPartial({
      user,
      pool: p,
      position: positionPda(pool.programId, p, user),
      // pool-i i demos kërkon vetëm bilancin; mosha nuk kërkohet
      balanceAttestation: attestationPda(verifierId, user, "balance"),
      ageAttestation: null,
      // whitelist-a aktuale: atestimet nga një bankë e revokuar refuzohen
      verifierState: PublicKey.findProgramAddressSync([Buffer.from("verifier_state")], verifierId)[0],
    })
    .rpc();
}
