// Testet TypeScript të programit (localnet). Ekzekutim: `anchor test` nga rrënja.
//
// Rrjedha: CLI-ja Rust krijon një sesion testues për portofolin e provider-it,
// prodhon provën Groth16 (qarku dev), pastaj programi e verifikon on-chain.
// STATUS: i pa-ekzekutuar në CI (kërkon solana-test-validator + anchor-cli).
import * as anchor from "@coral-xyz/anchor";
import { ComputeBudgetProgram, PublicKey } from "@solana/web3.js";
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
  const program = anchor.workspace.SolanaZkVerifier as anchor.Program<any>;
  const user = provider.wallet.publicKey;
  console.log("      program id (IDL):", program.programId.toBase58());
  const [statePda] = PublicKey.findProgramAddressSync([Buffer.from("verifier_state")], program.programId);
  const [recordPda] = PublicKey.findProgramAddressSync(
    [Buffer.from("attestation"), user.toBuffer()],
    program.programId
  );

  let vk: any, proof: any, bankKeyHash: string;

  before(() => {
    mkdirSync(TMP, { recursive: true });
    const run = (args: string) => execSync(`${CLI} ${args}`, { cwd: ROOT, stdio: "pipe" }).toString().trim();
    run(`setup --out-dir ${TMP}`);
    run(
      `mock-session --key tests/fixtures/mock_notary_key.pem --cert tests/fixtures/mock_notary_cert.pem ` +
        `--balance 15200 --wallet ${user.toBase58()} --out ${TMP}/session.tls.json`
    );
    run(`prove --input ${TMP}/session.tls.json --threshold 10000 --pk ${TMP}/dev_pk.bin --out ${TMP}/proof.json`);
    bankKeyHash = run(`bank-key-hash --input ${TMP}/session.tls.json`);
    vk = JSON.parse(readFileSync(`${TMP}/vk.json`, "utf8"));
    proof = JSON.parse(readFileSync(`${TMP}/proof.json`, "utf8"));
  });

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
    inputs[4][31] ^= 1; // min_threshold
    let failed = false;
    try {
      await program.methods
        .verifyAndAttest(hex(proof.proof_a), hex(proof.proof_b), hex(proof.proof_c), inputs)
        .accounts({ user })
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
      .accounts({ user })
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
    expect(rec.minThreshold.toNumber()).to.eq(10000);
  });
});
