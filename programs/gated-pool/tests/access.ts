// Real localnet transactions: admission evidence and access lifetime are independent.
// Verifier tests run first and initialize the DEV VK; circuit verification has a
// separate full-Circom job. No test bypass or clock-changing instruction is deployed.
import * as anchor from "@coral-xyz/anchor";
import { ComputeBudgetProgram, Keypair, LAMPORTS_PER_SOL, PublicKey, SYSVAR_CLOCK_PUBKEY } from "@solana/web3.js";
import { execFileSync } from "child_process";
import { createHash } from "crypto";
import { readFileSync } from "fs";
import { expect } from "chai";
import BN from "bn.js";

describe("business admission and independent access grants", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);
  const root = process.cwd();
  const load = (name: string) => JSON.parse(readFileSync(`${root}/target/idl/${name}.json`, "utf8"));
  const v: any = new anchor.Program(load("solana_zk_verifier"), provider);
  const p: any = new anchor.Program(load("gated_pool"), provider);
  const user = provider.wallet.publicKey;
  const numberSeed = (n: number) => new BN(n).toArrayLike(Buffer, "le", 8);
  const address = (program: PublicKey, ...seeds: Buffer[]) => PublicKey.findProgramAddressSync(seeds, program)[0];
  const state = address(v.programId, Buffer.from("verifier_state"));
  const claim = (name: string) => Buffer.from(createHash("sha256").update(name).digest().subarray(0, 8)).reverse();
  const attestation = (name: string) => address(v.programId, Buffer.from("attestation"), user.toBuffer(), claim(name));
  const balance = attestation("balance"), age = attestation("date_of_birth");
  const grantFor = (policy: PublicKey, wallet = user) => address(v.programId, Buffer.from("access_grant"), policy.toBuffer(), wallet.toBuffer());
  const positionFor = (pool: PublicKey, wallet = user) => address(p.programId, Buffer.from("access_position"), pool.toBuffer(), wallet.toBuffer());
  let sequence = 100;
  let long: any;
  const config = (overrides: any = {}) => ({ minBalance: new BN(5000), minAge: 18,
    balanceMaxAgeSecs: new BN(86400), ageMaxAgeSecs: new BN(0),
    accessDurationSecs: new BN(30 * 86400), recheckIntervalSecs: new BN(0),
    balanceSource: null, ageSource: null, ...overrides });
  const error = async (f: () => Promise<any>) => {
    try { await f(); } catch (e: any) { return String(e); }
    return "";
  };
  const clock = async () => Number((await provider.connection.getAccountInfo(SYSVAR_CLOCK_PUBKEY, "confirmed"))!.data.readBigInt64LE(32));
  const until = async (timestamp: number) => {
    const end = Date.now() + 45000;
    while (await clock() < timestamp) {
      if (Date.now() > end) throw Error("validator clock did not advance");
      await new Promise(resolve => setTimeout(resolve, 250));
    }
  };
  const newPolicy = async (overrides: any = {}) => {
    const id = sequence++;
    const policy = address(v.programId, Buffer.from("access_policy"), user.toBuffer(), numberSeed(id));
    const pool = address(p.programId, Buffer.from("access_pool"), user.toBuffer(), numberSeed(id));
    await v.methods.createAccessPolicy(new BN(id), config(overrides)).accountsPartial({ authority: user, policy }).rpc();
    await p.methods.createAccessPool(new BN(id)).accountsPartial({ authority: user, policy, pool }).rpc();
    return { policy, pool, grant: grantFor(policy) };
  };
  const qualify = (item: any, refresh = false, evidence: any = { balance, age }) =>
    v.methods[refresh ? "refreshAccess" : "admitAccess"]().accountsPartial({ user, policy: item.policy, grant: item.grant,
      balanceAttestation: evidence.balance ?? null, ageAttestation: evidence.age ?? null, verifierState: state }).rpc();
  const deposit = (item: any, wallet = user, signers: Keypair[] = []) =>
    p.methods.depositAccess(new BN(10000)).accountsPartial({ user: wallet, pool: item.pool, position: positionFor(item.pool, wallet),
      policy: item.policy, grant: item.grant, verifierState: state }).signers(signers).rpc();
  const grant = (item: any) => v.account.accessGrant.fetch(item.grant);

  // Refresh real proofs against the already-installed DEV VK after epoch changes.
  const attest = async (kind: "balance" | "age") => {
    const cli = (...args: string[]) => execFileSync(`${root}/target/release/coprocessor`, args, { cwd: root, stdio: "pipe" });
    const base = `${root}/target/ts-test/access-${kind}`;
    cli("mock-session", "--key", "tests/fixtures/mock_notary_seed.hex", "--wallet", user.toBase58(),
      "--out", `${base}.session.json`, ...(kind === "age" ? ["--kind", "age", "--dob", "2001-05-17", "--account-id", "J0USER"] : ["--balance", "15200"]));
    cli("prove", "--backend", "dev", "--input", `${base}.session.json`, "--app", "ci-airdrop", "--pk", "target/ts-test/dev_pk.bin",
      "--out", `${base}.proof.json`, ...(kind === "age" ? ["--min-age", "18"] : ["--min", "10000"]));
    const proof = JSON.parse(readFileSync(`${base}.proof.json`, "utf8"));
    const hex = (s: string) => Array.from(Buffer.from(s, "hex"));
    const nullifier = address(v.programId, Buffer.from("nullifier"), Buffer.from(proof.nullifier, "hex"));
    await v.methods.verifyAndAttest(hex(proof.proof_a), hex(proof.proof_b), hex(proof.proof_c), proof.public_inputs.map(hex),
      hex(proof.nullifier), new BN(Buffer.from(proof.public_inputs[2], "hex").subarray(24)))
      .accountsPartial({ user, nullifierRecord: nullifier }).preInstructions([ComputeBudgetProgram.setComputeUnitLimit({ units: 400000 })]).rpc();
    return proof;
  };

  before(async () => {
    // Independent of other consumer suites' security-revocation tests.
    await attest("balance");
    await attest("age");
  });

  it("creates immutable business terms: 24h balance freshness and 30-day access", async () => {
    long = await newPolicy();
    const stored: any = await v.account.accessPolicy.fetch(long.policy);
    expect(stored.config.balanceMaxAgeSecs.toNumber()).to.eq(86400);
    expect(stored.config.ageMaxAgeSecs.toNumber()).to.eq(0);
    expect(stored.config.accessDurationSecs.toNumber()).to.eq(30 * 86400);
    expect(await error(() => newPolicy({ balanceMaxAgeSecs: new BN(0) }))).to.contain("BalanceFreshnessRequired");
    expect(await error(() => newPolicy({ accessDurationSecs: new BN(-1) }))).to.contain("InvalidPolicy");
    expect(await error(() => newPolicy({ minAge: 151 }))).to.contain("InvalidPolicy");
  });

  it("requires every configured condition, correct claim, threshold and source", async () => {
    expect(await error(() => qualify(long, false, { balance }))).to.contain("MissingEvidence");
    expect(await error(() => qualify(long, false, { age }))).to.contain("MissingEvidence");
    expect(await error(() => qualify(long, false, { balance: age, age: balance }))).to.contain("GateWrongAccount");
    const strict = await newPolicy({ minBalance: new BN(20000) });
    expect(await error(() => qualify(strict))).to.contain("GateThresholdNotMet");
    const older = await newPolicy({ minAge: 21 });
    expect(await error(() => qualify(older))).to.contain("GateThresholdNotMet");
    const source = await newPolicy({ balanceSource: Array(32).fill(123) });
    expect(await error(() => qualify(source))).to.contain("GateWrongSource");
    await qualify(long);
    const g: any = await grant(long);
    expect(g.expiresAt.sub(g.lastQualifiedAt).toNumber()).to.eq(30 * 86400);
    await deposit(long); // No attestation accounts or proof generation at access time.
    expect(await error(() => p.methods.deposit(new BN(1000)).accountsPartial({ user, pool: long.pool,
      position: positionFor(long.pool), balanceAttestation: balance, ageAttestation: age, verifierState: state }).rpc()))
      .to.match(/AccountDiscriminatorMismatch|3002/);
    expect(await error(() => qualify(long))).to.contain("AlreadyActive");
  });

  it("pins business policy and wallet; a weaker policy cannot bypass the gate", async () => {
    const weak = await newPolicy({ minBalance: new BN(0), minAge: 0 });
    await qualify(weak, false, {});
    expect(await error(() => deposit({ ...weak, pool: long.pool }))).to.contain("WrongPolicy");
    const thief = Keypair.generate();
    const sig = await provider.connection.requestAirdrop(thief.publicKey, LAMPORTS_PER_SOL);
    await provider.connection.confirmTransaction(sig, "confirmed");
    expect(await error(() => deposit(long, thief.publicKey, [thief]))).to.contain("WrongGrant");
  });

  it("only the business authority can pause or revoke; users cannot readmit around revocation", async () => {
    const stranger = Keypair.generate();
    expect(await error(() => v.methods.setAccessPolicyPaused(true).accountsPartial({ authority: stranger.publicKey, policy: long.policy }).signers([stranger]).rpc()))
      .to.match(/ConstraintHasOne|ConstraintSeeds/);
    await v.methods.setAccessPolicyPaused(true).accountsPartial({ authority: user, policy: long.policy }).rpc();
    expect(await error(() => deposit(long))).to.contain("PolicyPaused");
    expect(await error(() => qualify(long))).to.contain("PolicyPaused");
    await v.methods.setAccessPolicyPaused(false).accountsPartial({ authority: user, policy: long.policy }).rpc();
    expect(await error(() => v.methods.setAccessRevoked(true).accountsPartial({ authority: stranger.publicKey, policy: long.policy, grant: long.grant }).signers([stranger]).rpc()))
      .to.match(/ConstraintHasOne|ConstraintSeeds/);
    await v.methods.setAccessRevoked(true).accountsPartial({ authority: user, policy: long.policy, grant: long.grant }).rpc();
    expect(await error(() => deposit(long))).to.contain("GrantRevoked");
    expect(await error(() => qualify(long))).to.contain("GrantRevoked");
    expect(await error(() => qualify(long, true))).to.contain("GrantRevoked");
    await p.methods.withdrawAccess(new BN(1000)).accountsPartial({ user, pool: long.pool, position: positionFor(long.pool) }).rpc();
    await v.methods.setAccessRevoked(false).accountsPartial({ authority: user, policy: long.policy, grant: long.grant }).rpc();
    await deposit(long);
  });

  it("access survives evidence freshness expiry; new admission still needs fresh evidence", async () => {
    const shortEvidence = await newPolicy({ minAge: 0, balanceMaxAgeSecs: new BN(10) });
    await attest("balance");
    await qualify(shortEvidence, false, { balance });
    const old: any = await grant(shortEvidence);
    await until(old.balanceAttestedAt.toNumber() + 11);
    await deposit(shortEvidence);
    const unopened = await newPolicy({ minAge: 0, balanceMaxAgeSecs: new BN(10) });
    expect(await error(() => qualify(unopened, false, { balance }))).to.contain("GateExpired");
  });

  it("supports age-only and indefinite grants without repeatedly checking stable DOB", async () => {
    const indefinite = await newPolicy({ minBalance: new BN(0), accessDurationSecs: new BN(0) });
    await qualify(indefinite, false, { age });
    expect((await grant(indefinite)).expiresAt.toNumber()).to.eq(0);
    await deposit(indefinite);
    expect(await error(() => qualify(indefinite, false, { age }))).to.contain("AlreadyActive");
  });

  it("expiry blocks new actions, not withdrawals; renewal requires a newer balance observation", async () => {
    const finite = await newPolicy({ minAge: 0, accessDurationSecs: new BN(3) });
    await qualify(finite, false, { balance });
    await deposit(finite);
    const old: any = await grant(finite);
    await until(old.expiresAt.toNumber());
    expect(await error(() => deposit(finite))).to.contain("GrantExpired");
    expect(await error(() => qualify(finite, true, { balance }))).to.contain("GrantExpired");
    expect(await error(() => qualify(finite, false, { balance }))).to.contain("NewEvidenceRequired");
    await p.methods.withdrawAccess(new BN(1000)).accountsPartial({ user, pool: finite.pool, position: positionFor(finite.pool) }).rpc();
    const nullifierBefore: any = await v.account.attestationRecord.fetch(balance);
    const renewedProof = await attest("balance");
    expect(renewedProof.nullifier).to.eq(Buffer.from(nullifierBefore.nullifier).toString("hex"));
    await qualify(finite, false, { balance });
    const next: any = await grant(finite);
    expect(next.firstAdmittedAt.toString()).to.eq(old.firstAdmittedAt.toString());
    expect(next.balanceAttestedAt.gt(old.balanceAttestedAt)).to.eq(true);
    expect(next.expiresAt.gt(old.expiresAt)).to.eq(true);
  });

  it("periodic refresh requires new evidence and preserves the original access expiry", async () => {
    const periodic = await newPolicy({ minAge: 0, recheckIntervalSecs: new BN(3) });
    await qualify(periodic, false, { balance });
    const old: any = await grant(periodic);
    await until(old.nextCheckAt.toNumber());
    expect(await error(() => deposit(periodic))).to.contain("RecheckRequired");
    expect(await error(() => qualify(periodic, true, { balance }))).to.contain("NewEvidenceRequired");
    await attest("balance");
    await qualify(periodic, true, { balance });
    const next: any = await grant(periodic);
    expect(next.expiresAt.toString()).to.eq(old.expiresAt.toString());
    expect(next.nextCheckAt.gt(old.nextCheckAt)).to.eq(true);
    await deposit(periodic);
  });

  it("issuer and verifier security revocations also stop independent access grants", async () => {
    const rec: any = await v.account.attestationRecord.fetch(balance);
    const hash = Array.from(rec.bankKeyHash as number[]);
    await v.methods.removeBankKey(hash).accounts({ authority: user }).rpc();
    try { expect(await error(() => deposit(long))).to.contain("SourceRevoked"); }
    finally { await v.methods.addBankKey(hash).accounts({ authority: user }).rpc(); }
    await v.methods.bumpPolicyEpoch().accounts({ authority: user }).rpc();
    expect(await error(() => deposit(long))).to.contain("VerifierRevoked");
    // Restore fresh evidence for subsequent legacy consumer tests; nullifier ownership remains unchanged.
    await attest("balance");
    await attest("age");
  });
});
