#!/usr/bin/env node
// Local environment smoke only: legacy wire format, no wallet/config files.
import { generateKeyPairSync, sign } from 'node:crypto';
import { setTimeout as sleep } from 'node:timers/promises';

const LOCAL_RPC = 'http://127.0.0.1:18899';
const alphabet = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz';
const requestTimeoutMs = 8_000;
const confirmationTimeoutMs = 60_000;
let requestId = 0;

function base58(bytes) {
  if (!Buffer.isBuffer(bytes) || bytes.length > 64) throw new Error('Invalid base58 input');
  let leading = 0;
  while (leading < bytes.length && bytes[leading] === 0) leading++;
  let number = bytes.length ? BigInt(`0x${bytes.toString('hex')}`) : 0n;
  let encoded = '';
  while (number > 0n) {
    encoded = alphabet[Number(number % 58n)] + encoded;
    number /= 58n;
  }
  return '1'.repeat(leading) + encoded;
}

function decode32(value, label) {
  if (typeof value !== 'string' || value.length === 0 || value.length > 44) {
    throw new Error(`${label}: expected a canonical 32-byte base58 value`);
  }
  let number = 0n;
  for (const character of value) {
    const digit = alphabet.indexOf(character);
    if (digit < 0) throw new Error(`${label}: invalid base58 character`);
    number = number * 58n + BigInt(digit);
  }
  const bytes = Buffer.alloc(32);
  for (let i = 31; i >= 0; i--) {
    bytes[i] = Number(number & 255n);
    number >>= 8n;
  }
  if (number !== 0n || base58(bytes) !== value) {
    throw new Error(`${label}: expected exactly 32 bytes and canonical encoding`);
  }
  return bytes;
}

function shortvec(value) {
  if (!Number.isSafeInteger(value) || value < 0 || value > 65_535) {
    throw new Error('Unsupported shortvec length');
  }
  const bytes = [];
  do {
    const chunk = value & 127;
    value >>>= 7;
    bytes.push(chunk | (value ? 128 : 0));
  } while (value);
  return Buffer.from(bytes);
}

function legacyTransaction(payer, program, blockhash, data, privateKey) {
  const message = Buffer.concat([
    Buffer.from([1, 0, 1]), // One signer; only the program account is readonly.
    shortvec(2), payer, program, blockhash,
    shortvec(1), Buffer.from([1]), shortvec(0), shortvec(data.length), data,
  ]);
  const signature = sign(null, message, privateKey);
  if (signature.length !== 64) throw new Error('Unexpected Ed25519 signature length');
  return {
    wire: Buffer.concat([shortvec(1), signature, message]).toString('base64'),
    signature: base58(signature),
  };
}

async function rpc(method, params = [], timeoutMs = requestTimeoutMs) {
  const id = ++requestId;
  const response = await fetch(LOCAL_RPC, {
    method: 'POST',
    redirect: 'error',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ jsonrpc: '2.0', id, method, params }),
    signal: AbortSignal.timeout(Math.max(1, timeoutMs)),
  });
  if (!response.ok) throw new Error(`${method}: HTTP ${response.status}`);
  const body = await response.json();
  if (body.id !== id || body.jsonrpc !== '2.0') throw new Error(`${method}: invalid RPC response`);
  if (body.error) throw new Error(`${method}: ${JSON.stringify(body.error)}`);
  if (!Object.hasOwn(body, 'result')) throw new Error(`${method}: missing result`);
  return body.result;
}

async function confirm(signature) {
  const deadline = Date.now() + confirmationTimeoutMs;
  while (Date.now() < deadline) {
    const result = await rpc('getSignatureStatuses', [[signature], { searchTransactionHistory: false }],
      Math.min(requestTimeoutMs, deadline - Date.now()));
    const status = result?.value?.[0];
    if (status?.err) throw new Error(`Transaction failed: ${JSON.stringify(status.err)}`);
    if (status && ['confirmed', 'finalized'].includes(status.confirmationStatus)) return status;
    const remaining = deadline - Date.now();
    if (remaining > 0) await sleep(Math.min(500, remaining));
  }
  throw new Error(`Local confirmation timed out after ${confirmationTimeoutMs} ms`);
}

function hasPong(logs) {
  return Array.isArray(logs) && logs.includes('Program log: environment-smoke: pong');
}

async function main() {
  const [programId, rpcArgument = LOCAL_RPC, ...extra] = process.argv.slice(2);
  if (!programId || extra.length) {
    throw new Error('Usage: node smoke-client.mjs PROGRAM_ID [http://127.0.0.1:18899]');
  }
  // Never follow user/env RPC configuration; this client has one allowed endpoint.
  if (rpcArgument !== LOCAL_RPC) throw new Error(`Only ${LOCAL_RPC} is permitted`);
  const program = decode32(programId, 'program ID');
  const account = await rpc('getAccountInfo', [programId, { encoding: 'base64', commitment: 'confirmed' }]);
  if (account?.value?.executable !== true) throw new Error('Program account is absent or not executable');

  const { privateKey, publicKey } = generateKeyPairSync('ed25519');
  const exported = publicKey.export({ format: 'der', type: 'spki' });
  const prefix = Buffer.from('302a300506032b6570032100', 'hex');
  if (exported.length !== 44 || !exported.subarray(0, 12).equals(prefix)) {
    throw new Error('Unexpected Ed25519 SPKI encoding');
  }
  const payer = exported.subarray(12);
  const payerId = base58(payer);
  const airdropSignature = await rpc('requestAirdrop', [payerId, 1_000_000_000, { commitment: 'confirmed' }]);
  await confirm(airdropSignature);
  const latest = await rpc('getLatestBlockhash', [{ commitment: 'confirmed' }]);
  const blockhash = decode32(latest?.value?.blockhash, 'blockhash');
  const positive = legacyTransaction(payer, program, blockhash, Buffer.from('ping'), privateKey);
  const simulation = await rpc('simulateTransaction', [positive.wire, {
    encoding: 'base64', sigVerify: true, commitment: 'confirmed',
  }]);
  if (simulation?.value?.err || !hasPong(simulation?.value?.logs)) {
    throw new Error(`Positive simulation did not succeed with pong: ${JSON.stringify(simulation?.value)}`);
  }

  const negative = legacyTransaction(payer, program, blockhash, Buffer.from('wrong'), privateKey);
  const negativeSimulation = await rpc('simulateTransaction', [negative.wire, {
    encoding: 'base64', sigVerify: true, commitment: 'confirmed',
  }]);
  const expectedError = { InstructionError: [0, 'InvalidInstructionData'] };
  if (JSON.stringify(negativeSimulation?.value?.err) !== JSON.stringify(expectedError)) {
    throw new Error(`Negative simulation returned unexpected error: ${JSON.stringify(negativeSimulation?.value?.err)}`);
  }

  const signature = await rpc('sendTransaction', [positive.wire, {
    encoding: 'base64', skipPreflight: false, preflightCommitment: 'confirmed', maxRetries: 3,
  }]);
  if (signature !== positive.signature) throw new Error('RPC returned an unexpected transaction signature');
  const status = await confirm(signature);
  // Some RPCs expose status just before getTransaction; allow a short bounded lag.
  const deadline = Date.now() + 10_000;
  let transaction;
  do {
    transaction = await rpc('getTransaction', [signature, {
      encoding: 'json', commitment: 'confirmed', maxSupportedTransactionVersion: 0,
    }], Math.min(requestTimeoutMs, Math.max(1, deadline - Date.now())));
    if (transaction) break;
    if (Date.now() < deadline) await sleep(Math.min(500, deadline - Date.now()));
  } while (Date.now() < deadline);
  if (!transaction?.meta || transaction.meta.err || !hasPong(transaction.meta.logMessages)) {
    throw new Error('Confirmed transaction metadata did not contain successful pong');
  }
  console.log(JSON.stringify({
    ok: true, rpc: LOCAL_RPC, version: 'legacy', scope: 'local-environment-smoke-only',
    programId, programExecutable: true, ephemeralPayer: payerId,
    airdropSignature, signature, confirmationStatus: status.confirmationStatus,
    simulationPong: true, negativeSimulation: expectedError,
    confirmedPong: true, slot: transaction.slot, computeUnitsConsumed: transaction.meta.computeUnitsConsumed ?? null,
    transactionV1Verified: false,
  }, null, 2));
}

main().catch((error) => {
  console.error(JSON.stringify({ ok: false, version: 'legacy', error: error.message }));
  process.exitCode = 1;
});
