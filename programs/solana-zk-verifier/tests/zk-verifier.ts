// Testet TypeScript të programit (localnet). Ekzekutim: `anchor test` nga rrënja.
//
// Rrjedha: CLI-ja Rust krijon një sesion testues për portofolin e provider-it,
// prodhon provën Groth16 (qarku dev), pastaj programi e verifikon on-chain.
// Ekzekutohet në CI (workflow `zk-e2e`, job-i `program`).
import * as anchor from "@coral-xyz/anchor";
import { ComputeBudgetProgram, Keypair, LAMPORTS_PER_SOL, PublicKey } from "@solana/web3.js";
import { execSync } from "child_process";
import { readFileSync, mkdirSync } from "fs";
import { expect } from "chai";
// ESM: `anchor.BN` nuk eksportohet si emër nga paketa CJS, ndaj importohet direkt.
import BN from "bn.js";

// `anchor test` ekzekutohet nga rrënja e depos (ESM: __dirname nuk ekziston)
const ROOT = process.cwd();
const TMP = `${ROOT}/target/ts-test`;
const CLI = `cargo run -q --release -p coprocessor-cli --`;
const hex = (s: string) => Array.from(Buffer.from(s, "hex"));

describe("solana-zk-verifier", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);
  // IDL-ja e gjeneruar nga `anchor build` mban adresën e saktë pas `anchor keys sync`.
  const idl = JSON.parse(readFileSync(`${ROOT}/target/idl/solana_zk_verifier.json`, "utf8"));
  const program = new anchor.Program(idl, provider) as anchor.Program<any>;
  const user = provider.wallet.publicKey;
  console.log("      program id (IDL):", program.programId.toBase58());
  const [statePda] = PublicKey.findProgramAddressSync([Buffer.from("verifier_state")], program.programId);
  const [recordPda] = PublicKey.findProgramAddressSync(
    [Buffer.from("attestation"), user.toBuffer()],
    program.programId
  );

  // Një portofol i dytë që përpiqet të ripërdorë të njëjtën llogari bankare (Sybil)
  const sybil = Keypair.generate();
  let vk: any, proof: any, sybilProof: any, bankKeyHash: string;

  before(() => {
    mkdirSync(TMP, { recursive: true });
    const run = (args: string) => execSync(`${CLI} ${args}`, { cwd: ROOT, stdio: "pipe" }).toString().trim();
    run(`setup --out-dir ${TMP}`);
    run(
      `mock-session --key tests/fixtures/mock_notary_key.pem --cert tests/fixtures/mock_notary_cert.pem ` +
        `--balance 15200 --wallet ${user.toBase58()} --out ${TMP}/session.tls.json`
    );
    run(`prove --input ${TMP}/session.tls.json --min 10000 --app ci-airdrop --pk ${TMP}/dev_pk.bin --out ${TMP}/proof.json`);
    // e njëjta llogari (account-id i parazgjedhur), portofol tjetër
    run(
      `mock-session --key tests/fixtures/mock_notary_key.pem --cert tests/fixtures/mock_notary_cert.pem ` +
        `--balance 15200 --wallet ${sybil.publicKey.toBase58()} --out ${TMP}/sybil.tls.json`
    );
    run(`prove --input ${TMP}/sybil.tls.json --min 10000 --app ci-airdrop --pk ${TMP}/dev_pk.bin --out ${TMP}/sybil.json`);
    bankKeyHash = run(`bank-key-hash --input ${TMP}/session.tls.json`);
    vk = JSON.parse(readFileSync(`${TMP}/vk.json`, "utf8"));
    proof = JSON.parse(readFileSync(`${TMP}/proof.json`, "utf8"));
    sybilProof = JSON.parse(readFileSync(`${TMP}/sybil.json`, "utf8"));
  });

  const APP_ID = 7;
  // PDA-ja e nullifier-it nuk mund të nxirret nga IDL (seed nga një element i Vec-ut)
  const nullifierPda = (p: any) =>
    PublicKey.findProgramAddressSync(
      [Buffer.from("nullifier"), Buffer.from(p.public_inputs[NULLIFIER], "hex")],
      program.programId
    )[0];
  const NULLIFIER = 3;

  it("inicializon VK dhe whitelist-ën", async () => {
    await program.methods
      .initialize(
        {
          alphaG1: hex(vk.alpha_g1),
          betaG2: hex(vk.beta_g2),
          gammaG2: hex(vk.gamma_g2),
          deltaG2: hex(vk.delta_g2),
          ic: vk.ic.map(hex),
        },
        hex(proof.public_inputs[APP_ID]),
        new BN(0)
      )
      .accounts({ authority: user })
      .rpc();
    await program.methods.addBankKey(hex(bankKeyHash)).accounts({ authority: user }).rpc();
    const st: any = await program.account.verifierState.fetch(statePda);
    expect(st.bankKeyWhitelist.length).to.eq(1);
  });

  it("refuzon hyrje publike të ndryshuara", async () => {
    const inputs = proof.public_inputs.map(hex);
    inputs[5][31] ^= 1; // min_value
    let failed = false;
    try {
      await program.methods
        .verifyAndAttest(hex(proof.proof_a), hex(proof.proof_b), hex(proof.proof_c), inputs)
        .accountsPartial({ user, nullifierRecord: nullifierPda(proof) })
        .preInstructions([ComputeBudgetProgram.setComputeUnitLimit({ units: 400_000 })])
        .rpc();
    } catch (e) {
      failed = true;
    }
    expect(failed).to.eq(true);
  });

  it("verifikon provën dhe ruan atestatin", async () => {
    const sig = await program.methods
      .verifyAndAttest(hex(proof.proof_a), hex(proof.proof_b), hex(proof.proof_c), proof.public_inputs.map(hex))
      .accountsPartial({ user, nullifierRecord: nullifierPda(proof) })
      .preInstructions([ComputeBudgetProgram.setComputeUnitLimit({ units: 400_000 })])
      .rpc({ commitment: "confirmed" });
    const tx = await provider.connection.getTransaction(sig, {
      commitment: "confirmed",
      maxSupportedTransactionVersion: 0,
    });
    console.log("      CU të konsumuara:", tx?.meta?.computeUnitsConsumed);
    const rec: any = await program.account.attestationRecord.fetch(recordPda);
    expect(rec.user.toBase58()).to.eq(user.toBase58());
    expect(rec.thresholdPassed).to.eq(true);
    expect(rec.minValue.toNumber()).to.eq(10000);
    expect(Buffer.from(rec.nullifier).toString("hex")).to.eq(proof.nullifier);
    const nrec: any = await program.account.nullifierRecord.fetch(nullifierPda(proof));
    expect(nrec.owner.toBase58()).to.eq(user.toBase58());
  });

  it("refuzon të njëjtën llogari bankare nga një portofol tjetër (anti-Sybil)", async () => {
    expect(sybilProof.nullifier).to.eq(proof.nullifier);
    const sig = await provider.connection.requestAirdrop(sybil.publicKey, LAMPORTS_PER_SOL);
    await provider.connection.confirmTransaction(sig, "confirmed");
    let err = "";
    try {
      await program.methods
        .verifyAndAttest(
          hex(sybilProof.proof_a),
          hex(sybilProof.proof_b),
          hex(sybilProof.proof_c),
          sybilProof.public_inputs.map(hex)
        )
        .accountsPartial({ user: sybil.publicKey, nullifierRecord: nullifierPda(sybilProof) })
        .signers([sybil])
        .preInstructions([ComputeBudgetProgram.setComputeUnitLimit({ units: 400_000 })])
        .rpc();
    } catch (e: any) {
      err = String(e);
    }
    expect(err).to.contain("NullifierAlreadyUsed");
  });

  it("refuzon provë për një app_id tjetër", async () => {
    const other = Keypair.generate();
    const run = (args: string) => execSync(`${CLI} ${args}`, { cwd: ROOT, stdio: "pipe" }).toString().trim();
    run(
      `mock-session --key tests/fixtures/mock_notary_key.pem --cert tests/fixtures/mock_notary_cert.pem ` +
        `--balance 15200 --account-id AL00OTHER --wallet ${other.publicKey.toBase58()} --out ${TMP}/other.tls.json`
    );
    run(`prove --input ${TMP}/other.tls.json --min 1 --app another-app --pk ${TMP}/dev_pk.bin --out ${TMP}/other.json`);
    const p = JSON.parse(readFileSync(`${TMP}/other.json`, "utf8"));
    const sig = await provider.connection.requestAirdrop(other.publicKey, LAMPORTS_PER_SOL);
    await provider.connection.confirmTransaction(sig, "confirmed");
    let err = "";
    try {
      await program.methods
        .verifyAndAttest(hex(p.proof_a), hex(p.proof_b), hex(p.proof_c), p.public_inputs.map(hex))
        .accountsPartial({ user: other.publicKey, nullifierRecord: nullifierPda(p) })
        .signers([other])
        .preInstructions([ComputeBudgetProgram.setComputeUnitLimit({ units: 400_000 })])
        .rpc();
    } catch (e: any) {
      err = String(e);
    }
    expect(err).to.contain("AppIdMismatch");
  });
});
