// Testet e shembullit "pool i mbrojtur" (localnet). Ekzekutohen PAS testeve të
// verifikuesit (shih `[scripts] test` në Anchor.toml): aty portofoli i provider-it
// fiton dy atestime të pavarura, "bilanc ≥ 10 000" dhe "mosha ≥ 18".
import * as anchor from "@coral-xyz/anchor";
import { Keypair, LAMPORTS_PER_SOL, PublicKey } from "@solana/web3.js";
import { readFileSync } from "fs";
import { createHash } from "crypto";
import { expect } from "chai";
import BN from "bn.js";

const ROOT = process.cwd();
const claimKeyId = (k: string) => createHash("sha256").update(k).digest().subarray(0, 8);

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
  const attestation = (u: PublicKey, claim: string) =>
    PublicKey.findProgramAddressSync([Buffer.from("attestation"), u.toBuffer(), claimKeyId(claim)], verifier)[0];

  const funded = async () => {
    const k = Keypair.generate();
    const sig = await provider.connection.requestAirdrop(k.publicKey, 2 * LAMPORTS_PER_SOL);
    await provider.connection.confirmTransaction(sig, "confirmed");
    return k;
  };
  const deposit = (
    p: PublicKey,
    u: PublicKey,
    atts: { balance?: PublicKey | null; age?: PublicKey | null },
    signers: Keypair[] = []
  ) =>
    pool.methods
      .deposit(new BN(10_000))
      .accountsPartial({
        user: u,
        pool: p,
        position: positionPda(p, u),
        balanceAttestation: atts.balance ?? null,
        ageAttestation: atts.age ?? null,
      })
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
  const newPool = async (minBalance: number, minAge: number) => {
    const a = await funded();
    await pool.methods
      .createPool(new BN(minBalance), minAge, new BN(0))
      .accounts({ authority: a.publicKey })
      .signers([a])
      .rpc();
    return poolPda(a.publicKey);
  };

  let balancePool: PublicKey; // bilanc ≥ 5 000
  let strictPool: PublicKey; // bilanc ≥ 20 000
  let agePool: PublicKey; // vetëm mosha ≥ 18
  let bothPool: PublicKey; // bilanc ≥ 5 000 DHE mosha ≥ 18

  before(async () => {
    await pool.methods.createPool(new BN(5_000), 0, new BN(0)).accounts({ authority: user }).rpc();
    balancePool = poolPda(user);
    strictPool = await newPool(20_000, 0);
    agePool = await newPool(0, 18);
    bothPool = await newPool(5_000, 18);
  });

  const BAL = () => attestation(user, "balance");
  const AGE = () => attestation(user, "date_of_birth");

  it("pool vetëm-bilanc: mjafton atestimi i bilancit (mosha nuk kërkohet)", async () => {
    await deposit(balancePool, user, { balance: BAL() });
    const pos: any = await pool.account.position.fetch(positionPda(balancePool, user));
    expect(pos.amount.toNumber()).to.eq(10_000);
  });

  it("pool vetëm-moshë: mjafton atestimi i moshës (bilanci nuk kërkohet)", async () => {
    await deposit(agePool, user, { age: AGE() });
    const pos: any = await pool.account.position.fetch(positionPda(agePool, user));
    expect(pos.amount.toNumber()).to.eq(10_000);
  });

  it("pool me të dyja: kërkon të dyja atestimet", async () => {
    expect(await error(() => deposit(bothPool, user, { balance: BAL() }))).to.contain("MissingAttestation");
    expect(await error(() => deposit(bothPool, user, { age: AGE() }))).to.contain("MissingAttestation");
    await deposit(bothPool, user, { balance: BAL(), age: AGE() });
  });

  it("refuzon atestimin e moshës në vend të bilancit (dhe anasjelltas)", async () => {
    expect(await error(() => deposit(balancePool, user, { balance: AGE() }))).to.contain("GateWrongAccount");
    expect(await error(() => deposit(agePool, user, { age: BAL() }))).to.contain("GateWrongAccount");
  });

  it("refuzon portofolin pa atestim", async () => {
    const stranger = await funded();
    const err = await error(() =>
      deposit(balancePool, stranger.publicKey, { balance: attestation(stranger.publicKey, "balance") }, [stranger])
    );
    expect(err).to.match(/AccountNotInitialized|3012/);
  });

  it("refuzon kur prova vërtetoi një prag më të ulët (10 000 < 20 000)", async () => {
    expect(await error(() => deposit(strictPool, user, { balance: BAL() }))).to.contain("GateThresholdNotMet");
  });

  it("refuzon atestimin e një portofoli tjetër", async () => {
    const thief = await funded();
    const err = await error(() => deposit(balancePool, thief.publicKey, { balance: BAL() }, [thief]));
    expect(err).to.match(/GateWrongAccount|GateWrongUser/);
  });

  it("lejon tërheqjen e parave të veta", async () => {
    const before = await provider.connection.getBalance(balancePool);
    await pool.methods
      .withdraw(new BN(4_000))
      .accountsPartial({ user, pool: balancePool, position: positionPda(balancePool, user) })
      .rpc();
    const pos: any = await pool.account.position.fetch(positionPda(balancePool, user));
    expect(pos.amount.toNumber()).to.eq(6_000);
    expect(before - (await provider.connection.getBalance(balancePool))).to.eq(4_000);
  });
});
