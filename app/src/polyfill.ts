import { Buffer } from "buffer";
// anchor/web3.js përdorin Buffer të Node-it
(globalThis as any).Buffer ??= Buffer;
