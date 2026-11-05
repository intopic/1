// Generate persistent local deployment identities and synchronize public IDs.
// Uses only Node's Ed25519 implementation; private keypairs stay in ignored target/.
import { generateKeyPairSync } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { resolve } from "node:path";

const root = fileURLToPath(new URL("../", import.meta.url));
const base58 = bytes => {
  const alphabet = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
  let n = BigInt(`0x${Buffer.from(bytes).toString("hex")}`), out = "";
  while (n > 0n) { out = alphabet[Number(n % 58n)] + out; n /= 58n; }
  for (const b of bytes) { if (b !== 0) break; out = "1" + out; }
  return out;
};
let toml = readFileSync(resolve(root, "Anchor.toml"), "utf8");
mkdirSync(resolve(root, "target/deploy"), { recursive: true });
for (const [name, dir] of [["solana_zk_verifier", "solana-zk-verifier"], ["gated_pool", "gated-pool"]]) {
  const path = resolve(root, `target/deploy/${name}-keypair.json`);
  if (!existsSync(path)) {
    const { privateKey, publicKey } = generateKeyPairSync("ed25519");
    const seed = privateKey.export({type: "pkcs8", format: "der"}).subarray(-32);
    const pub = publicKey.export({type: "spki", format: "der"}).subarray(-32);
    writeFileSync(path, JSON.stringify([...seed, ...pub]), {mode: 0o600, flag: "wx"});
  }
  const key = JSON.parse(readFileSync(path));
  if (key.length !== 64 || key.some(b => !Number.isInteger(b) || b < 0 || b > 255)) throw Error(`Invalid keypair: ${path}`);
  const id = base58(key.slice(32));
  const source = resolve(root, `programs/${dir}/src/lib.rs`);
  writeFileSync(source, readFileSync(source, "utf8").replace(/declare_id!\("[^"]+"\)/, `declare_id!("${id}")`));
  toml = toml.replace(new RegExp(`^${name} = "[^"]+"`, "gm"), `${name} = "${id}"`);
  console.log(`${name}: ${id}`);
}
writeFileSync(resolve(root, "Anchor.toml"), toml);
