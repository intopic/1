// Përgatit localnet-in për demon:
//   1. kopjon IDL-në (target/idl) te app/public/idl.json
//   2. krijon çelësat e qarkut DEV (artifacts/demo) nëse mungojnë
//   3. thërret `initialize(vk, app_id, 0)` dhe `add_bank_key(noteri i simuluar)`
//
// Kërkon: `anchor build && anchor deploy` (ose `solana-test-validator` me programin),
// portofolin e autoritetit te ANCHOR_WALLET ose ~/.config/solana/id.json.
// Variabla: RPC_URL (http://127.0.0.1:8899), APP_LABEL (demo).
import * as anchor from "@coral-xyz/anchor";
import { Connection, Keypair, PublicKey } from "@solana/web3.js";
import { execFileSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { homedir, tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const RPC_URL = process.env.RPC_URL ?? "http://127.0.0.1:8899";
const APP_LABEL = process.env.APP_LABEL ?? "demo";
const OUT = join(ROOT, "artifacts/demo");
const IDL = join(ROOT, "target/idl/solana_zk_verifier.json");

const bin = join(ROOT, "target/release/coprocessor");
const cli = (...args) =>
  (existsSync(bin)
    ? execFileSync(bin, args, { cwd: ROOT })
    : execFileSync("cargo", ["run", "-q", "--release", "-p", "coprocessor-cli", "--", ...args], { cwd: ROOT })
  )
    .toString()
    .trim();
const hex = (s) => Array.from(Buffer.from(s, "hex"));
// claim_key_id = SHA-256(çelësi JSON)[0..8] (u64 BE). Demoja pranon bilanc dhe datëlindje.
const claimKeyId = (k) => new anchor.BN(createHash("sha256").update(k).digest().subarray(0, 8));
const CLAIMS = (process.env.CLAIMS ?? "balance,date_of_birth").split(",");

if (!existsSync(IDL)) throw new Error(`mungon ${IDL}: ekzekutoni \`anchor build\` nga rrënja`);
mkdirSync(join(ROOT, "app/public"), { recursive: true });
copyFileSync(IDL, join(ROOT, "app/public/idl.json"));
const idl = JSON.parse(readFileSync(IDL, "utf8"));

const walletPath = process.env.ANCHOR_WALLET ?? join(homedir(), ".config/solana/id.json");
const authority = Keypair.fromSecretKey(Uint8Array.from(JSON.parse(readFileSync(walletPath, "utf8"))));
const provider = new anchor.AnchorProvider(new Connection(RPC_URL, "confirmed"), new anchor.Wallet(authority), {
  commitment: "confirmed",
});
const program = new anchor.Program(idl, provider);
console.log("programi:", program.programId.toBase58(), "· autoriteti:", authority.publicKey.toBase58());

mkdirSync(OUT, { recursive: true });
// VK_JSON: VK-ja e qarkut Circom (prodhim/devnet, `coprocessor export-vk`); përndryshe qarku dev
let vkPath = process.env.VK_JSON;
if (!vkPath) {
  if (!existsSync(join(OUT, "dev_pk.bin"))) cli("setup", "--out-dir", OUT);
  vkPath = join(OUT, "vk.json");
}
const vk = JSON.parse(readFileSync(vkPath, "utf8"));
console.log("VK:", vkPath);
const appId = cli("app-id", "--app", APP_LABEL);

// hash-i i çelësit të noterit të simuluar (nga një sesion i përkohshëm)
const tmp = join(tmpdir(), `zktls-init-${process.pid}.json`);
cli(
  "mock-session",
  "--key", "tests/fixtures/mock_notary_key.pem",
  "--cert", "tests/fixtures/mock_notary_cert.pem",
  "--wallet", authority.publicKey.toBase58(),
  "--out", tmp
);
const bankKeyHash = cli("bank-key-hash", "--input", tmp);

// VK-ja e plotë (11 pika IC, ~1.2 KB) nuk nxë në një transaksion: 5 pikat e para
// shkojnë me initialize/setVerifyingKey, pjesa tjetër me extendVkIc.
const vkHead = { alphaG1: hex(vk.alpha_g1), betaG2: hex(vk.beta_g2), gammaG2: hex(vk.gamma_g2), deltaG2: hex(vk.delta_g2), ic: vk.ic.slice(0, 5).map(hex) };
const extendIc = () => program.methods.extendVkIc(vk.ic.slice(5).map(hex)).accounts({ authority: authority.publicKey }).rpc();

const [statePda] = PublicKey.findProgramAddressSync([Buffer.from("verifier_state")], program.programId);
const existing = await program.account.verifierState.fetchNullable(statePda);
if (!existing) {
  await program.methods
    .initialize(vkHead, hex(appId), CLAIMS.map(claimKeyId), new anchor.BN(0))
    // vetëm upgrade authority i programit mund të inicializojë
    .accountsPartial({
      authority: authority.publicKey,
      programData: PublicKey.findProgramAddressSync(
        [program.programId.toBuffer()],
        new PublicKey("BPFLoaderUpgradeab1e11111111111111111111111")
      )[0],
    })
    .rpc();
  await extendIc();
  console.log("u inicializua me app_id", APP_LABEL, appId, "· pretendimet:", CLAIMS.join(", "));
} else {
  if (Buffer.from(existing.appId).toString("hex") !== appId)
    throw new Error(`programi është inicializuar me një app_id tjetër; rinisni validator-in ose përdorni APP_LABEL të njëjtë`);
  await program.methods
    .setVerifyingKey(vkHead)
    .accounts({ authority: authority.publicKey })
    .rpc();
  await extendIc();
  console.log("gjendja ekzistonte; VK u rifreskua");
}
// noteri i simuluar (demo) dhe, me BRIDGE_SESSION=<session.tls.json nga ura>, çelësi RSA i urës reale
const keys = [[bankKeyHash, "noteri i simuluar"]];
if (process.env.BRIDGE_SESSION) keys.push([cli("bank-key-hash", "--input", process.env.BRIDGE_SESSION), "ura TLSNotary"]);
if (process.env.SKIP_MOCK_KEY === "1") keys.shift();
for (const [h, name] of keys) {
  const st = await program.account.verifierState.fetch(statePda);
  if (!st.bankKeyWhitelist.some((k) => Buffer.from(k).toString("hex") === h)) {
    await program.methods.addBankKey(hex(h)).accounts({ authority: authority.publicKey }).rpc();
    console.log(`${name} u shtua në whitelist`);
  }
}

// Shembulli i DEX-it: pool që pranon vetëm portofole me "bilanc ≥ MIN_BALANCE"
let env = `VITE_RPC_URL=${RPC_URL}\nVITE_APP_LABEL=${APP_LABEL}\n`;
const POOL_IDL = join(ROOT, "target/idl/gated_pool.json");
if (existsSync(POOL_IDL)) {
  copyFileSync(POOL_IDL, join(ROOT, "app/public/pool-idl.json"));
  const pool = new anchor.Program(JSON.parse(readFileSync(POOL_IDL, "utf8")), provider);
  const minBalance = Number(process.env.MIN_BALANCE ?? 10000);
  const [poolPda] = PublicKey.findProgramAddressSync([Buffer.from("pool"), authority.publicKey.toBuffer()], pool.programId);
  if (!(await pool.account.pool.fetchNullable(poolPda))) {
    await pool.methods.createPool(new anchor.BN(minBalance), 0, new anchor.BN(0)).accounts({ authority: authority.publicKey }).rpc();
    console.log(`pool-i i mbrojtur u krijua (bilanc ≥ ${minBalance}):`, poolPda.toBase58());
  }
  env += `VITE_POOL_AUTHORITY=${authority.publicKey.toBase58()}\nVITE_MIN_BALANCE=${minBalance}\n`;
}
if (process.env.PROVER_URL) env += `VITE_PROVER_URL=${process.env.PROVER_URL}\n`;
writeFileSync(join(ROOT, "app/.env.local"), env);
console.log(`
Gati. Tani:
  target/release/coprocessor serve --pk ${OUT}/dev_pk.bin \\
      --mock-notary-key tests/fixtures/mock_notary_key.pem --mock-notary-cert tests/fixtures/mock_notary_cert.pem
  cd app && npm run dev     # http://localhost:5173
`);
