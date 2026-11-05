# Deploy: fly.io + Solana devnet

Tre pjesë, të gjitha nga i njëjti commit:

| pjesa | ku | çfarë |
|---|---|---|
| programet `solana-zk-verifier` + `gated-pool` | Solana devnet | `scripts/deploy-devnet.sh` |
| daemon-i i provës, ura, noteri | fly.io (një imazh, 3 procese) | `fly.toml`, `deploy/Dockerfile` |
| faqja web (`app/`) | Vercel / Fly / çdo hosting statik | `npm run build` |

**v3:** përdorni çelësa të rinj EdDSA-Poseidon dhe release të qarkut v3. Çelësat RSA,
zkey/VK dhe whitelist-a e vjetër nuk përputhen. Shih [migrimin](V3.md). Fly përdor vetëm
Rapidsnark dhe `/mock-session` është i fikur në konfigurimin standard.

## 1. Çelësat e qarkut (një herë)

GitHub → Actions → **Çelësat e qarkut (release)** → *Run workflow*. Pas përfundimit del
release-i `circuit-keys-<nr>`:

```bash
gh release download circuit-keys-<nr> -D deploy/keys
(cd deploy/keys && sha256sum -c SHA256SUMS)    # pa --ignore-missing: çdo skedar duhet
```

Imazhi Docker e kontrollon sërish `main_final.zkey` kundrejt `SHA256SUMS` gjatë build-it.
Me `--build-arg ZKEY_URL=...` kërkohet edhe `--build-arg ZKEY_SHA256=<hash>`.
`main_final.zkey` (në imazh) dhe `verification_key.json` (on-chain) duhet të jenë nga
**i njëjti** release: përndryshe çdo provë refuzohet me `InvalidProof`. Release-t e vjetra
(para këtij ndryshimi) kanë rrugë të plota në `SHA256SUMS`; për to krahasoni hash-et me dorë.

## 2. fly.io

```bash
fly launch --no-deploy --copy-config --name <emri-juaj>     # ndryshon `app` te fly.toml
# vetëm nëse ndizni urën dhe noterin real:
openssl rand -hex 32 > bridge_seed.hex
fly secrets set NOTARY_KEY="$(openssl rand -hex 32)" BRIDGE_SIGNING_SEED="$(cat bridge_seed.hex)"
# sekreti i etiketës së llogarisë: i pandryshueshëm për gjithë jetën e sistemit
# (ndërrimi i tij = çdo llogari mund të provojë sërish me portofol të ri). Ruajeni offline.
fly secrets set BRIDGE_TAG_KEY="$(openssl rand -hex 32)"
fly secrets set BRIDGE_POLICY_JSON="$(cat my-policy.json)"   # burimet, monedha, çelësi i noterit
# kufiri i prover-it (shih më poshtë)
fly secrets set PROVER_RATE_LIMIT=20 PROVER_CLIENT_IP_HEADER=fly-client-ip
fly deploy
# demo vetëm me noter të simuluar (çelësi publik testues, vetëm devnet):
fly secrets set DEMO_MOCK_NOTARY=1
fly scale count prover=1 bridge=0 notary=0
curl https://<emri-juaj>.fly.dev/health   # {"status":"ok","backend":"circom-rapidsnark",...}
```

- **prover**: `performance-2x` / 4 GB është konfigurim fillestar; për kapacitetin v3
  matni në host-in e deploy-it (`scripts/load_test.py`). Për më shumë: `fly scale count prover=3` ose `fly scale vm performance-4x`.
- `VERIFIER_PROGRAM` (+ `SOLANA_RPC_URL`): daemon-i llogarit nullifier-in para provës dhe,
  nëse llogaria është përdorur nga një portofol tjetër, kthen **409** në milisekonda, pa
  shpenzuar CPU për provën. Pas deploy-it në devnet: `fly secrets set VERIFIER_PROGRAM=<adresa>`
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
- Prover-i: në radhë presin deri në 32 kërkesa për çdo vend prove (koha varet nga backend-i dhe makina); përtej
  kësaj kthen **503** me `Retry-After: 5`. Kërkesa HTTP duhet të mbërrijë e plotë brenda 30 s.
  Nëse klienti e mbyll lidhjen ndërsa pret në radhë, prova nuk nis fare. Çdo proces
  (dëshmitari, rapidsnark) vritet pas 300 s; skedarët e përkohshëm janë `0700`/`0600`.
- Aksesi te prover-i (POST `/prove` dhe `/mock-session`):
  - `PROVER_RATE_LIMIT=<n>`: n kërkesa në minutë për IP (IPv6: për /64), pastaj **429**
    me `Retry-After`. Pas Fly-t vendosni `PROVER_CLIENT_IP_HEADER=fly-client-ip`
    (Fly e mbishkruan gjithmonë); pa proxy mos e vendosni, se klienti do ta falsifikonte.
  - `PROVER_API_KEYS=<k1>,<k2>`: çdo kërkesë kërkon `x-api-key` (përndryshe **401**). Për
    integrues server-me-server. Një çelës i futur në faqen web (`VITE_PROVER_API_KEY`)
    është publik: ndal vetëm përdorimin e rastësishëm, kufiri për IP mbetet mbrojtja reale.
- Ura: `"target_pointer": "/account/balance"` (dhe `"account_pointer"`) e lidh vlerën me
  vendin e saktë në JSON, që një `loan.balance` të mos lexohet si bilanc. `"decimals": 2`
  i kthen shumat në centë (`15200.99` → `1520099`); me `0`, një shumë me presje refuzohet
  në vend që të pritet në heshtje. `"issuer": "banka-x"` e mban të njëjtin nullifier edhe
  kur banka ndërron domen ose ka disa domene.
- **Prodhim: tre aplikacione Fly të ndara** (prover, ura, noteri), secili me sekretet e
  veta. Në një aplikacion të vetëm, të gjitha proceset marrin të gjitha sekretet nga Fly;
  `entrypoint.sh` i heq ato të roleve të tjera para nisjes, por izolimi i plotë kërkon
  aplikacione të ndara.

## 3. Solana devnet

Për testet me 1/2/4 prova aktive dhe një pool deri në 10 prover-a Fly, shih
[paralelizmin dhe autoscale](SCALING.md). Autostart kërkon makina të krijuara më parë.

```bash
solana-keygen new                       # nëse s'keni portofol
solana airdrop 2 -u devnet              # ose https://faucet.solana.com (duhen ~6 SOL)
PROVER_URL=https://<emri-juaj>.fly.dev bash scripts/deploy-devnet.sh
```

Skripti: `anchor keys sync` → `anchor build` → `anchor deploy` → VK e Circom on-chain,
`app_id`, llojet e pranuara (bilanc + datëlindje), whitelist-a e noterit të simuluar
(dhe e urës me `BRIDGE_SESSION=<session.tls.json>`), pool-i demo (bilanc ≥ 10 000, sesioni
TLS jo më i vjetër se `POOL_MAX_AGE_SECS`, parazgjedhur 1 ditë) dhe afatin global
`MAX_ATTESTATION_AGE_SECS` (devnet: 3 600 s; asnjë provë me sesion më të vjetër se 1 orë).
Shkruan `app/.env.local`.

Administrimi on-chain (vetëm `authority`):

| veprimi | instruksioni |
|---|---|
| ndërro afatin global | `set_max_attestation_age(secs)` |
| revoko një çelës noteri | `remove_bank_key(hash)`: atestimet e tij bien menjëherë te `require_attested_current` |
| revoko gjithçka (VK e re, incident) | `bump_policy_epoch`; `set_verifying_key` e bën vetë |
| ndërro authority-n | `propose_authority(new)` → `accept_authority` (nënshkruar nga i riu) |
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
