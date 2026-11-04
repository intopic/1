// Testet e shembullit "pool i mbrojtur" (localnet). Ekzekutohen PAS testeve të
// verifikuesit (shih `[scripts] test` në Anchor.toml): aty portofoli i provider-it
// fiton një atestim "bilanc ≥ 10 000"; këtu e përdorim për të hyrë në pool.
import * as anchor from "@coral-xyz/anchor";
import { Keypair, LAMPORTS_PER_SOL, PublicKey } from "@solana/web3.js";
import { readFileSync } from "fs";
import { expect } from "chai";
import BN from "bn.js";

const ROOT = process.cwd();

describe("gated-pool (integrimi i një DEX-i)", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);
  const load = (name: string) => JSON.parse(readFileSync(`${ROOT}/target/idl/${name}.json`, "utf8"));
  const pool: any = new anchor.Program(load("gated_pool"), provider);
  const verifier = new PublicKey(load("solana_zk_verifier").address);
  const user = provider.wallet.publicKey;

  const poolPda = (authority: PublicKey) =>
    PublicKey.findProgramAddressSync([Buffer.from("pool"), authority.toBuffer()], pool.programId)[0];
  const positionPda = (p: PublicKey, u: PublicKey) =>
    PublicKey.findProgramAddressSync([Buffer.from("position"), p.toBuffer(), u.toBuffer()], pool.programId)[0];
  const attestationPda = (u: PublicKey) =>
    PublicKey.findProgramAddressSync([Buffer.from("attestation"), u.toBuffer()], verifier)[0];

  const funded = async () => {
    const k = Keypair.generate();
    const sig = await provider.connection.requestAirdrop(k.publicKey, 2 * LAMPORTS_PER_SOL);
    await provider.connection.confirmTransaction(sig, "confirmed");
    return k;
  };
  const deposit = (p: PublicKey, u: PublicKey, lamports: number, signers: Keypair[] = []) =>
    pool.methods
      .deposit(new BN(lamports))
      .accountsPartial({ user: u, pool: p, position: positionPda(p, u), attestation: attestationPda(u) })
      .signers(signers)
      .rpc();
  const error = async (f: () => Promise<any>) => {
    try {
      await f();
    } catch (e: any) {
      return String(e);
    }
    return "";
  };

  let open: PublicKey; // kërkon bilanc ≥ 5 000
  let strict: PublicKey; // kërkon bilanc ≥ 20 000

  before(async () => {
    await pool.methods.createPool(new BN(5_000), new BN(0)).accounts({ authority: user }).rpc();
    open = poolPda(user);
    const other = await funded();
    await pool.methods
      .createPool(new BN(20_000), new BN(0))
      .accounts({ authority: other.publicKey })
      .signers([other])
      .rpc();
    strict = poolPda(other.publicKey);
  });

  it("pranon depozitë nga portofoli me atestim (≥ 10 000 ≥ 5 000)", async () => {
    await deposit(open, user, 0.1 * LAMPORTS_PER_SOL);
    const pos: any = await pool.account.position.fetch(positionPda(open, user));
    expect(pos.amount.toNumber()).to.eq(0.1 * LAMPORTS_PER_SOL);
    const st: any = await pool.account.pool.fetch(open);
    expect(st.totalDeposits.toNumber()).to.eq(0.1 * LAMPORTS_PER_SOL);
  });

  it("refuzon portofolin pa atestim", async () => {
    const stranger = await funded();
    const err = await error(() => deposit(open, stranger.publicKey, 1000, [stranger]));
    // llogaria e atestimit nuk ekziston
    expect(err).to.match(/AccountNotInitialized|3012/);
  });

  it("refuzon kur prova vërtetoi një prag më të ulët (10 000 < 20 000)", async () => {
    const err = await error(() => deposit(strict, user, 1000));
    expect(err).to.contain("GateThresholdNotMet");
  });

  it("refuzon atestimin e një portofoli tjetër", async () => {
    const thief = await funded();
    const err = await error(() =>
      pool.methods
        .deposit(new BN(1000))
        .accountsPartial({
          user: thief.publicKey,
          pool: open,
          position: positionPda(open, thief.publicKey),
          attestation: attestationPda(user), // atestimi i dikujt tjetër
        })
        .signers([thief])
        .rpc()
    );
    expect(err).to.match(/ConstraintSeeds|2006/);
  });

  it("lejon tërheqjen e parave të veta", async () => {
    const before = await provider.connection.getBalance(open);
    await pool.methods
      .withdraw(new BN(0.05 * LAMPORTS_PER_SOL))
      .accountsPartial({ user, pool: open, position: positionPda(open, user) })
      .rpc();
    const pos: any = await pool.account.position.fetch(positionPda(open, user));
    expect(pos.amount.toNumber()).to.eq(0.05 * LAMPORTS_PER_SOL);
    expect(before - (await provider.connection.getBalance(open))).to.eq(0.05 * LAMPORTS_PER_SOL);
  });
});
