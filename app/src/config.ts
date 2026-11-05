// Konfigurimi i demos (mund të ndryshohet me variabla VITE_* në `.env.local`)
export const RPC_URL = import.meta.env.VITE_RPC_URL ?? "http://127.0.0.1:8899";
/** Daemon-i `coprocessor serve --mock-notary-key ...` */
export const PROVER_URL = import.meta.env.VITE_PROVER_URL ?? "http://127.0.0.1:8787";
/** Çelësi API i prover-it (nëse serveri e kërkon). I futur në faqe, është publik. */
export const PROVER_API_KEY: string = import.meta.env.VITE_PROVER_API_KEY ?? "";
/** Emri i aplikacionit: duhet të jetë i njëjti me atë të `npm run init-localnet` */
export const APP_LABEL = import.meta.env.VITE_APP_LABEL ?? "demo";
/** Pragu i bilancit që kërkon "aplikacioni" (p.sh. një pool DeFi) */
export const MIN_BALANCE = Number(import.meta.env.VITE_MIN_BALANCE ?? 10000);
export const MIN_AGE = 18;
/** Autoriteti i pool-it të mbrojtur (shembulli `gated-pool`); bosh = hapi 5 fshihet */
export const POOL_AUTHORITY: string = import.meta.env.VITE_POOL_AUTHORITY ?? "";
