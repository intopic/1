import assert from "node:assert/strict";
import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";
import { resolve } from "node:path";
import { buildEddsa } from "circomlibjs";

const root = fileURLToPath(new URL("../../", import.meta.url));
const eddsa = await buildEddsa();
const F = eddsa.F;
const num = b => BigInt(`0x${Buffer.from(b).toString("hex") || "0"}`);
const dec = n => F.toObject(n).toString();
const poseidon = xs => eddsa.poseidon(xs.map(x => F.e(x)));
const domain = s => num(Buffer.from(s));
const messageHash = bytes => poseidon([
  domain("zktls/attestation/v3"),
  ...Array.from({length: 5}, (_, i) => num(bytes.subarray(i * 31, Math.min(i * 31 + 31, 128))))
]);

// Independent circomlibjs vector, consumed by the Rust tests too.
const vectorPath = resolve(root, "tests/fixtures/eddsa_v3.json");
if (process.argv.includes("--write-vector")) {
  const seed = Buffer.from(Array.from({length: 32}, (_, i) => i));
  const msg = Buffer.from(Array.from({length: 128}, (_, i) => i));
  msg.write("ZKTLSv03");
  msg.writeBigUInt64BE(15200n, 8);
  msg.writeBigUInt64BE(1793800000n, 16);
  msg[96] = 0;
  const key = eddsa.prv2pub(seed);
  const sig = eddsa.signPoseidon(seed, messageHash(msg));
  writeFileSync(vectorPath, JSON.stringify({
    source: "circomlibjs 0.1.7, scripts/test.mjs --write-vector (public test seed)",
    seed_hex: seed.toString("hex"), message_hex: msg.toString("hex"),
    public_key: {x: dec(key[0]), y: dec(key[1])},
    signature: {r8x: dec(sig.R8[0]), r8y: dec(sig.R8[1]), s: sig.S.toString()},
    message_hash: dec(messageHash(msg)),
    key_hash: dec(poseidon([domain("zktls/notary-key/v3"), ...key.map(x => F.toObject(x))]))
  }, null, 2) + "\n");
  console.log("Wrote independent circomlibjs vector");
  process.exit(0);
}
const v = JSON.parse(readFileSync(vectorPath));
const bytes = Buffer.from(v.message_hex, "hex");
assert.equal(dec(messageHash(bytes)), v.message_hash);
assert(eddsa.verifyPoseidon(messageHash(bytes), {
  R8: [F.e(v.signature.r8x), F.e(v.signature.r8y)], S: BigInt(v.signature.s)
}, [F.e(v.public_key.x), F.e(v.public_key.y)]));

const wasm = readFileSync(resolve(root, "circuits/build/main_js/main.wasm"));
const require = createRequire(import.meta.url);
const calculator = await require(resolve(root, "circuits/build/main_js/witness_calculator.js"))(wasm);
const value = num(bytes.subarray(8, 16));
const tag = num(bytes.subarray(97));
const salt = 424242n, app = 123n;
const input = {
  attestation_msg: [...bytes].map(String), user_secret_salt: String(salt),
  notary_pubkey: [v.public_key.x, v.public_key.y],
  signature_r8: [v.signature.r8x, v.signature.r8y], signature_s: v.signature.s,
  bank_public_key_hash: v.key_hash, min_value: "10000", max_value: "18446744073709551615",
  app_id: String(app), user_pubkey_hi: String(num(bytes.subarray(32, 48))),
  user_pubkey_lo: String(num(bytes.subarray(48, 64)))
};
const expected = [
  dec(poseidon([value, salt])), String(num(bytes.subarray(16,24))),
  String(num(bytes.subarray(24,32))), dec(poseidon([tag, app])),
  input.bank_public_key_hash, input.min_value, input.max_value, input.app_id,
  input.user_pubkey_hi, input.user_pubkey_lo
];
const w = await calculator.calculateWitness(input, true);
assert.deepEqual(w.slice(1,11).map(String), expected);
let negatives = 0;
const reject = async (label, mutate) => {
  const bad = structuredClone(input); mutate(bad);
  await assert.rejects(() => calculator.calculateWitness(bad, true), label);
  negatives++;
};
// Each byte is bound by the signature, including both payload-digest halves.
for (let i = 0; i < 128; i++) {
  await reject(`message byte ${i}`, b => b.attestation_msg[i] = String(Number(b.attestation_msg[i]) ^ 1));
}
for (const name of ["signature_s", "bank_public_key_hash", "user_pubkey_hi", "user_pubkey_lo"]) {
  await reject(name, b => b[name] = String(BigInt(b[name]) + 1n));
}
for (const name of ["signature_r8", "notary_pubkey"]) {
  for (const i of [0,1]) await reject(`${name}[${i}]`, b => b[name][i] = String(BigInt(b[name][i]) + 1n));
}
await reject("identity signer", b => b.notary_pubkey = ["0", "1"]);
await reject("S >= subgroup order", b => b.signature_s = "2736030358979909402780800718157159386076813972158567259200215660948447373041");
await reject("byte >= 256", b => b.attestation_msg[64] = "256");
await reject("minimum above value", b => b.min_value = String(value + 1n));
await reject("maximum below value", b => b.max_value = String(value - 1n));
await reject("range outside u64", b => b.max_value = String(1n << 64n));

// Optional Rust-generated witness: validates native signing and every public input.
if (process.env.NATIVE_INPUT) {
  const native = JSON.parse(readFileSync(resolve(process.env.NATIVE_INPUT)));
  const nw = await calculator.calculateWitness(native, true);
  const bundle = JSON.parse(readFileSync(resolve(process.env.NATIVE_PROOF)));
  assert.deepEqual(nw.slice(1,11).map(String), bundle.public_inputs.map(x => String(BigInt(`0x${x}`))));
  console.log("Rust witness/public inputs match Circom");
}
mkdirSync(resolve(root, "circuits/build"), { recursive: true });
writeFileSync(resolve(root, "circuits/build/test-input.json"), JSON.stringify(input));
writeFileSync(resolve(root, "circuits/build/test-public.json"), JSON.stringify(expected));
console.log(`PASS: circomlibjs vector, 10 public inputs, ${negatives} negative witnesses`);
process.exit(0);
