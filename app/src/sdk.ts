// SDK minimal për integruesit (DEX, wallet, DAO):
//   * `isVerified(connection, programId, wallet)` – kontroll i thjeshtë on-chain
//   * `verifyAndAttest(program, proof)` – dërgon provën nga daemon-i
import { Buffer } from "buffer";
import * as anchor from "@coral-xyz/anchor";
import { ComputeBudgetProgram, Connection, PublicKey } from "@solana/web3.js";

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

export const attestationPda = (programId: PublicKey, wallet: PublicKey) =>
  PublicKey.findProgramAddressSync([Buffer.from("attestation"), wallet.toBuffer()], programId)[0];

export const nullifierPda = (programId: PublicKey, nullifierHex: string) =>
  PublicKey.findProgramAddressSync([Buffer.from("nullifier"), Buffer.from(nullifierHex, "hex")], programId)[0];

export interface Attestation {
  user: PublicKey;
  minValue: bigint;
  maxValue: bigint;
  nullifier: string;
  attestedAt: number;
}

/** Lexon atestimin e një portofoli (null nëse nuk ekziston). */
export async function fetchAttestation(program: anchor.Program<any>, wallet: PublicKey): Promise<Attestation | null> {
  const rec: any = await (program.account as any).attestationRecord.fetchNullable(
    attestationPda(program.programId, wallet)
  );
  if (!rec) return null;
  return {
    user: rec.user,
    minValue: BigInt(rec.minValue.toString()),
    maxValue: BigInt(rec.maxValue.toString()),
    nullifier: Buffer.from(rec.nullifier).toString("hex"),
    attestedAt: Number(rec.attestedAt.toString()),
  };
}

/**
 * Kontrolli që një DEX/DAO i bën para se të lejojë një veprim: a ka ky portofol
 * një atestim zkTLS? (Për kushte të sakta, krahasoni `minValue`/`maxValue`.)
 */
export async function isVerified(connection: Connection, programId: PublicKey, wallet: PublicKey): Promise<boolean> {
  const info = await connection.getAccountInfo(attestationPda(programId, wallet));
  return info !== null && info.owner.equals(programId);
}

/** Dërgon provën te programi. Kthen nënshkrimin e transaksionit. */
export async function verifyAndAttest(program: anchor.Program<any>, user: PublicKey, p: ProofBundle): Promise<string> {
  return program.methods
    .verifyAndAttest(
      hexBytes(p.proof_a),
      hexBytes(p.proof_b),
      hexBytes(p.proof_c),
      p.public_inputs.map(hexBytes),
      hexBytes(p.nullifier)
    )
    .accountsPartial({ user, nullifierRecord: nullifierPda(program.programId, p.nullifier) })
    .preInstructions([ComputeBudgetProgram.setComputeUnitLimit({ units: 400_000 })])
    .rpc({ commitment: "confirmed" });
}

/** Mesazhe të kuptueshme për gabimet e programit dhe të daemon-it. */
export function explainError(e: unknown): string {
  const s = String((e as any)?.message ?? e);
  const map: [RegExp, string][] = [
    [/NullifierAlreadyUsed/, "Kjo llogari është përdorur tashmë nga një portofol tjetër."],
    [/kushti nuk plotësohet|ThresholdNotMet/, "Të dhënat nuk e plotësojnë kushtin (bilanci ose mosha)."],
    [/StaleAttestation/, "Atestimi ka skaduar. Lidhu sërish me burimin."],
    [/AppIdMismatch/, "Prova është për një aplikacion tjetër."],
    [/BankKeyNotWhitelisted/, "Noteri/banka nuk është e autorizuar në këtë program."],
    [/WalletMismatch/, "Prova është për një portofol tjetër."],
    [/User rejected|rejected the request/i, "Transaksioni u anulua në portofol."],
    [/Failed to fetch|NetworkError/, "Nuk u arrit daemon-i ose RPC-ja. A janë ndezur?"],
  ];
  for (const [re, msg] of map) if (re.test(s)) return msg;
  return s.length > 240 ? s.slice(0, 240) + "…" : s;
}
