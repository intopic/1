// Run test.mjs first and setup_keys.sh once. Reports measured API times;
// process startup, setup and transcript fetch are outside this measurement.
import { readFileSync, writeFileSync, statSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { resolve } from "node:path";
import { cpus, platform, arch, availableParallelism } from "node:os";
import { performance } from "node:perf_hooks";
import { createHash } from "node:crypto";
import { groth16, wtns } from "snarkjs";

const root = fileURLToPath(new URL("../build/", import.meta.url));
const input = JSON.parse(readFileSync(resolve(root, "test-input.json")));
const expected = JSON.parse(readFileSync(resolve(root, "test-public.json")));
const vk = JSON.parse(readFileSync(resolve(root, "verification_key.json")));
// Read the R1CS header without constructing a second curve/worker pool.
const r1csBytes = readFileSync(resolve(root, "main.r1cs"));
let constraints;
for (let offset = 12; offset < r1csBytes.length;) {
  const type = r1csBytes.readUInt32LE(offset);
  const length = Number(r1csBytes.readBigUInt64LE(offset + 4));
  const data = offset + 12;
  if (type === 1) constraints = r1csBytes.readUInt32LE(data + 4 + r1csBytes.readUInt32LE(data) + 24);
  offset = data + length;
}
if (!Number.isInteger(constraints)) throw Error("missing R1CS header");
const samples = [];
for (let i = 0; i < 6; i++) {
  const t0 = performance.now();
  await wtns.calculate(input, resolve(root, "main_js/main.wasm"), resolve(root, "bench.wtns"));
  const t1 = performance.now();
  const { proof, publicSignals } = await groth16.prove(resolve(root, "main_final.zkey"), resolve(root, "bench.wtns"), undefined, { singleThread: true });
  const t2 = performance.now();
  if (JSON.stringify(publicSignals) !== JSON.stringify(expected) || !await groth16.verify(vk, publicSignals, proof)) throw Error("invalid proof");
  if (i === 0) {
    writeFileSync(resolve(root, "benchmark-proof.json"), JSON.stringify(proof));
    writeFileSync(resolve(root, "benchmark-public.json"), JSON.stringify(publicSignals));
  }
  samples.push({witness_ms: t1-t0, prove_ms: t2-t1, total_ms: t2-t0});
  console.log(`sample ${i} (${i === 0 ? "cold" : "warm"}): ${JSON.stringify(samples.at(-1))}`);
}
const median = key => samples.slice(1).map(x => x[key]).sort((a,b) => a-b)[2];
const sha256 = name => createHash("sha256").update(readFileSync(resolve(root, name))).digest("hex");
const report = {
  measured_at: new Date().toISOString(), cpu: cpus()[0].model,
  platform: platform(), arch: arch(), logical_cpus: availableParallelism(), node: process.version,
  backend: "snarkjs 0.7.6 API singleThread", circuit: "v3 BabyJubjub EdDSA-Poseidon",
  constraints, ptau_power: Math.ceil(Math.log2(constraints + 64)), zkey_bytes: statSync(resolve(root, "main_final.zkey")).size,
  r1cs_sha256: sha256("main.r1cs"), zkey_sha256: sha256("main_final.zkey"),
  samples, warm_median: {witness_ms: median("witness_ms"), prove_ms: median("prove_ms"), total_ms: median("total_ms")},
  max_rss_kib: process.resourceUsage().maxRSS,
  exclusions: "setup, process startup, TLSNotary, HTTP, Solana; shared development machine"
};
writeFileSync(resolve(root, "benchmark.json"), JSON.stringify(report, null, 2) + "\n");
console.log(JSON.stringify(report, null, 2));
process.exit(0);
