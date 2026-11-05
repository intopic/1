// Testet TypeScript të programit (localnet). Ekzekutim: `anchor test` nga rrënja.
//
// Rrjedha: CLI-ja Rust krijon një sesion testues për portofolin e provider-it,
// prodhon provën Groth16 (qarku dev), pastaj programi e verifikon on-chain.
// Ekzekutohet në CI (workflow `zk-e2e`, job-i `program`).
import * as anchor from "@coral-xyz/anchor";
import { ComputeBudgetProgram, Keypair, LAMPORTS_PER_SOL, PublicKey } from "@solana/web3.js";
import { execSync } from "child_process";
import { createHash } from "crypto";
import { readFileSync, mkdirSync } from "fs";
import { expect } from "chai";
// ESM: `anchor.BN` nuk eksportohet si emër nga paketa CJS, ndaj importohet direkt.
import BN from "bn.js";

// `anchor test` ekzekutohet nga rrënja e depos (ESM: __dirname nuk ekziston)
const ROOT = process.cwd();
const TMP = `${ROOT}/target/ts-test`;
const CLI = `cargo run -q --release -p coprocessor-cli --`;
const hex = (s: string) => Array.from(Buffer.from(s, "hex"));
// claim_key_id = SHA-256(çelësi JSON)[0..8] si u64 BE
const claimKeyId = (k: string) => new BN(createHash("sha256").update(k).digest().subarray(0, 8));
const BPF_UPGRADEABLE = new PublicKey("BPFLoaderUpgradeab1e11111111111111111111111");

describe("solana-zk-verifier", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);
  // IDL-ja e gjeneruar nga `anchor build` mban adresën e saktë pas `anchor keys sync`.
  const idl = JSON.parse(readFileSync(`${ROOT}/target/idl/solana_zk_verifier.json`, "utf8"));
  // `any`: IDL-ja lexohet në runtime, ndaj tipet e gjeneruara të Anchor nuk vlejnë këtu
  // (Program<any> shkakton TS2589 dhe nuk njeh `program.account.*`).
  const program: any = new anchor.Program(idl, provider);
  const user = provider.wallet.publicKey;
  console.log("      program id (IDL):", program.programId.toBase58());
  const [statePda] = PublicKey.findProgramAddressSync([Buffer.from("verifier_state")], program.programId);
  // claim_key_id i provës (hyrja publike 2: 8 bajtët e fundit, BE)
  const claimOf = (p: any) => new BN(Buffer.from(p.public_inputs[2], "hex").subarray(24));
  // një atestim për çdo (portofol, lloj pretendimi)
  const recordFor = (wallet: PublicKey, claim: BN) =>
    PublicKey.findProgramAddressSync(
      [Buffer.from("attestation"), wallet.toBuffer(), claim.toArrayLike(Buffer, "le", 8)],
      program.programId
    )[0];
  const recordPda = recordFor(user, claimKeyId("balance"));

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
  // PDA-ja e nullifier-it (seeds: ["nullifier", nullifier])
  const nullifierPda = (p: any) =>
    PublicKey.findProgramAddressSync(
      [Buffer.from("nullifier"), Buffer.from(p.public_inputs[NULLIFIER], "hex")],
      program.programId
    )[0];
  const NULLIFIER = 3;

  const programData = () => PublicKey.findProgramAddressSync([program.programId.toBuffer()], BPF_UPGRADEABLE)[0];
  const vkHead = () => ({
    alphaG1: hex(vk.alpha_g1),
    betaG2: hex(vk.beta_g2),
    gammaG2: hex(vk.gamma_g2),
    deltaG2: hex(vk.delta_g2),
    ic: vk.ic.slice(0, 5).map(hex),
  });

  it("refuzon inicializimin nga dikush që s'është upgrade authority", async () => {
    const intruder = Keypair.generate();
    const sig = await provider.connection.requestAirdrop(intruder.publicKey, LAMPORTS_PER_SOL);
    await provider.connection.confirmTransaction(sig, "confirmed");
    let err = "";
    try {
      await program.methods
        .initialize(vkHead(), hex(proof.public_inputs[APP_ID]), [claimKeyId("balance")], new BN(0))
        .accountsPartial({ authority: intruder.publicKey, programData: programData() })
        .signers([intruder])
        .rpc();
    } catch (e: any) {
      err = String(e);
    }
    expect(err).to.contain("Unauthorized");
  });

  it("inicializon VK dhe whitelist-ën", async () => {
    await program.methods
      .initialize(
        {
          alphaG1: hex(vk.alpha_g1),
          betaG2: hex(vk.beta_g2),
          gammaG2: hex(vk.gamma_g2),
          deltaG2: hex(vk.delta_g2),
          // VK-ja e plotë (11 pika IC) nuk nxë në një transaksion: pjesa tjetër me extendVkIc
          ic: vk.ic.slice(0, 5).map(hex),
        },
        hex(proof.public_inputs[APP_ID]),
        // ky aplikacion pranon vetëm prova bilanci
        [claimKeyId("balance")],
        new BN(0)
      )
      .accountsPartial({ authority: user, programData: programData() })
      .rpc();
    await program.methods.extendVkIc(vk.ic.slice(5).map(hex)).accounts({ authority: user }).rpc();
    await program.methods.addBankKey(hex(bankKeyHash)).accounts({ authority: user }).rpc();
    const st: any = await program.account.verifierState.fetch(statePda);
    expect(st.bankKeyWhitelist.length).to.eq(1);
    expect(st.verifyingKey.ic.length).to.eq(11);
    expect(st.authority.toBase58()).to.eq(user.toBase58());
  });

  it("refuzon hyrje publike të ndryshuara", async () => {
    const inputs = proof.public_inputs.map(hex);
    inputs[5][31] ^= 1; // min_value
    let failed = false;
    try {
      await program.methods
        .verifyAndAttest(hex(proof.proof_a), hex(proof.proof_b), hex(proof.proof_c), inputs, hex(proof.nullifier), claimOf(proof))
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
      .verifyAndAttest(
        hex(proof.proof_a),
        hex(proof.proof_b),
        hex(proof.proof_c),
        proof.public_inputs.map(hex),
        hex(proof.nullifier),
        claimOf(proof)
      )
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
          sybilProof.public_inputs.map(hex),
          hex(sybilProof.nullifier),
          claimOf(sybilProof)
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

  it("refuzon herët (pa provë) një nullifier të zënë nga portofol tjetër", async () => {
    const run = (args: string) => execSync(`${CLI} ${args}`, { cwd: ROOT, stdio: "pipe" }).toString().trim();
    const pid = program.programId.toBase58();
    // PDA-ja e llogaritur në Rust (daemon-i) = ajo e web3.js
    expect(run(`nullifier-pda --program ${pid} --nullifier ${proof.nullifier}`)).to.eq(nullifierPda(proof).toBase58());
    const pc = `--rpc-url ${provider.connection.rpcEndpoint} --verifier-program ${pid}`;
    let err = "";
    const t0 = Date.now();
    try {
      run(`prove --input ${TMP}/sybil.tls.json --min 10000 --app ci-airdrop --pk ${TMP}/dev_pk.bin --out ${TMP}/sybil2.json ${pc}`);
    } catch (e: any) {
      err = String(e.stderr ?? e);
    }
    console.log("      refuzimi i hershëm:", Date.now() - t0, "ms");
    expect(err).to.contain("NullifierAlreadyUsed");
    // pronari i vërtetë (rifreskim i atestimit) nuk bllokohet
    run(`prove --input ${TMP}/session.tls.json --min 10000 --app ci-airdrop --pk ${TMP}/dev_pk.bin --out ${TMP}/again.json ${pc}`);
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
        .verifyAndAttest(hex(p.proof_a), hex(p.proof_b), hex(p.proof_c), p.public_inputs.map(hex), hex(p.nullifier), claimOf(p))
        .accountsPartial({ user: other.publicKey, nullifierRecord: nullifierPda(p) })
        .signers([other])
        .preInstructions([ComputeBudgetProgram.setComputeUnitLimit({ units: 400_000 })])
        .rpc();
    } catch (e: any) {
      err = String(e);
    }
    expect(err).to.contain("AppIdMismatch");
  });

  it("refuzon një provë moshe në një aplikacion që pranon vetëm bilanc", async () => {
    const other = Keypair.generate();
    const run = (args: string) => execSync(`${CLI} ${args}`, { cwd: ROOT, stdio: "pipe" }).toString().trim();
    run(
      `mock-session --kind age --dob 2001-05-17 --account-id J0AGE --key tests/fixtures/mock_notary_key.pem ` +
        `--cert tests/fixtures/mock_notary_cert.pem --wallet ${other.publicKey.toBase58()} --out ${TMP}/age.tls.json`
    );
    run(`prove --input ${TMP}/age.tls.json --min-age 18 --app ci-airdrop --pk ${TMP}/dev_pk.bin --out ${TMP}/age.json`);
    const p = JSON.parse(readFileSync(`${TMP}/age.json`, "utf8"));
    const sig = await provider.connection.requestAirdrop(other.publicKey, LAMPORTS_PER_SOL);
    await provider.connection.confirmTransaction(sig, "confirmed");
    let err = "";
    try {
      await program.methods
        .verifyAndAttest(hex(p.proof_a), hex(p.proof_b), hex(p.proof_c), p.public_inputs.map(hex), hex(p.nullifier), claimOf(p))
        .accountsPartial({ user: other.publicKey, nullifierRecord: nullifierPda(p) })
        .signers([other])
        .preInstructions([ComputeBudgetProgram.setComputeUnitLimit({ units: 400_000 })])
        .rpc();
    } catch (e: any) {
      err = String(e);
    }
    expect(err).to.contain("ClaimKeyNotAllowed");
  });

  it("bilanci dhe mosha janë atestime të pavarura (njëra nuk fshin tjetrën)", async () => {
    // aplikacioni tani pranon edhe datëlindjen
    await program.methods
      .setAllowedClaimKeys([claimKeyId("balance"), claimKeyId("date_of_birth")])
      .accounts({ authority: user })
      .rpc();
    const run = (args: string) => execSync(`${CLI} ${args}`, { cwd: ROOT, stdio: "pipe" }).toString().trim();
    run(
      `mock-session --kind age --dob 2001-05-17 --account-id J0USER --key tests/fixtures/mock_notary_key.pem ` +
        `--cert tests/fixtures/mock_notary_cert.pem --wallet ${user.toBase58()} --out ${TMP}/user-age.tls.json`
    );
    run(`prove --input ${TMP}/user-age.tls.json --min-age 18 --app ci-airdrop --pk ${TMP}/dev_pk.bin --out ${TMP}/user-age.json`);
    const p = JSON.parse(readFileSync(`${TMP}/user-age.json`, "utf8"));
    // argumenti claim_key_id duhet të jetë ai i provës
    let err = "";
    try {
      await program.methods
        .verifyAndAttest(hex(p.proof_a), hex(p.proof_b), hex(p.proof_c), p.public_inputs.map(hex), hex(p.nullifier), claimKeyId("balance"))
        .accountsPartial({ user, nullifierRecord: nullifierPda(p), attestationRecord: recordPda })
        .preInstructions([ComputeBudgetProgram.setComputeUnitLimit({ units: 400_000 })])
        .rpc();
    } catch (e: any) {
      err = String(e);
    }
    expect(err).to.contain("ClaimKeyArgMismatch");

    await program.methods
      .verifyAndAttest(hex(p.proof_a), hex(p.proof_b), hex(p.proof_c), p.public_inputs.map(hex), hex(p.nullifier), claimOf(p))
      .accountsPartial({ user, nullifierRecord: nullifierPda(p) })
      .preInstructions([ComputeBudgetProgram.setComputeUnitLimit({ units: 400_000 })])
      .rpc();
    const age: any = await program.account.attestationRecord.fetch(recordFor(user, claimKeyId("date_of_birth")));
    const bal: any = await program.account.attestationRecord.fetch(recordPda);
    expect(age.claimKeyId.toString()).to.eq(claimKeyId("date_of_birth").toString());
    // kufiri i poshtëm 1900-01-01: "datëlindja" 0 nuk kalon
    expect(age.minValue.toNumber()).to.eq(19000101);
    // atestimi i bilancit mbetet i paprekur
    expect(bal.claimKeyId.toString()).to.eq(claimKeyId("balance").toString());
    expect(bal.minValue.toNumber()).to.eq(10000);
  });
});
