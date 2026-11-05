# Deploy: fly.io + Solana devnet

Tre pjesë, të gjitha nga i njëjti commit:

| pjesa | ku | çfarë |
|---|---|---|
| programet `solana-zk-verifier` + `gated-pool` | Solana devnet | `scripts/deploy-devnet.sh` |
| daemon-i i provës, ura, noteri | fly.io (një imazh, 3 procese) | `fly.toml`, `deploy/Dockerfile` |
| faqja web (`app/`) | Vercel / Fly / çdo hosting statik | `npm run build` |

## 1. Çelësat e qarkut (një herë)

GitHub → Actions → **Çelësat e qarkut (release)** → *Run workflow*. Pas ~20 min del
release-i `circuit-keys-<nr>`:

```bash
gh release download circuit-keys-<nr> -D deploy/keys
sha256sum -c deploy/keys/SHA256SUMS --ignore-missing
```

`main_final.zkey` (në imazh) dhe `verification_key.json` (on-chain) duhet të jenë nga
**i njëjti** release: përndryshe çdo provë refuzohet me `InvalidProof`.

## 2. fly.io

```bash
fly launch --no-deploy --copy-config --name <emri-juaj>     # ndryshon `app` te fly.toml
# vetëm nëse ndizni urën dhe noterin real:
openssl genrsa -out bridge_rsa.pem 2048
fly secrets set NOTARY_KEY="$(openssl rand -hex 32)" BRIDGE_RSA_KEY_PEM="$(cat bridge_rsa.pem)"
fly secrets set BRIDGE_POLICY_JSON="$(cat my-policy.json)"   # burimet, monedha, çelësi i noterit
fly deploy
# demo vetëm me noter të simuluar:
fly scale count prover=1 bridge=0 notary=0
curl https://<emri-juaj>.fly.dev/health   # {"status":"ok","backend":"circom-rapidsnark",...}
```

- **prover**: `performance-2x` / 4 GB, rreth 2.3 s për provë, ~0.4 prova/s për makinë.
  Për më shumë: `fly scale count prover=3` ose `fly scale vm performance-4x`.
- `VERIFIER_PROGRAM` (+ `SOLANA_RPC_URL`): daemon-i llogarit nullifier-in para provës dhe,
  nëse llogaria është përdorur nga një portofol tjetër, kthen **409** në milisekonda, pa
  shpenzuar ~8 s CPU. Pas deploy-it në devnet: `fly secrets set VERIFIER_PROGRAM=<adresa>`
  (ose te `[env]`).
- `DEMO_MOCK_NOTARY=1` aktivizon `/mock-session` me çelësin testues **publik** të depos.
  Vetëm devnet. Në prodhim vendoseni `0` dhe mos e shtoni atë çelës në whitelist.
- Politika e urës: çdo burim bilanci duhet të ketë `"currency"` (p.sh. `"EUR"`), që
  15 200 lekë të mos kalojnë si 15 200 €. Ura refuzon në nisje dy burime të të njëjtit
  `target_key` me monedha të ndryshme (on-chain të gjitha duken si një burim i vetëm).
  `path_prefix` përputhet në kufi segmenti dhe query string (`?...`) refuzohet, përveç
  kur burimi ka `"allow_query": true`.
- Noteri: `NOTARY_MAX_SESSIONS` (parazgjedhur 4) sesione MPC-TLS njëkohësisht, secili me
  afat `NOTARY_SESSION_TIMEOUT_SECS` (180). Lidhjet përtej kufirit mbyllen menjëherë.
- Prover-i: në radhë presin deri në 32 kërkesa për çdo vend prove (~75 s); përtej
  kësaj kthen **503** me `Retry-After: 5`. Kërkesa HTTP duhet të mbërrijë e plotë brenda 30 s.

## 3. Solana devnet

```bash
solana-keygen new                       # nëse s'keni portofol
solana airdrop 2 -u devnet              # ose https://faucet.solana.com (duhen ~6 SOL)
PROVER_URL=https://<emri-juaj>.fly.dev bash scripts/deploy-devnet.sh
```

Skripti: `anchor keys sync` → `anchor build` → `anchor deploy` → VK e Circom on-chain,
`app_id`, llojet e pranuara (bilanc + datëlindje), whitelist-a e noterit të simuluar
(dhe e urës me `BRIDGE_SESSION=<session.tls.json>`), pool-i demo (bilanc ≥ 10 000, sesioni
TLS jo më i vjetër se `POOL_MAX_AGE_SECS`, parazgjedhur 1 ditë). Shkruan `app/.env.local`.
Në mainnet skripti refuzon qarkun dev dhe noterin e simuluar (kërkon `VK_JSON`,
`SKIP_MOCK_KEY=1` dhe `BRIDGE_SESSION`).

## 4. Faqja web

```bash
cd app && npm run build      # dist/ → Vercel/Netlify/Fly static
```

## Kontroll para prezantimit

- [ ] `curl https://<app>.fly.dev/health` kthen `circom-rapidsnark`
- [ ] faqja hapet me Phantom në devnet, hapat 1 deri 5 kalojnë
- [ ] një portofol tjetër me të njëjtin IBAN refuzohet (`NullifierAlreadyUsed`)
