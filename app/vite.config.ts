import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  // web3.js / anchor presin `global` dhe `Buffer` (polyfill në src/polyfill.ts)
  define: { global: "globalThis" },
  server: { port: 5173 },
});
