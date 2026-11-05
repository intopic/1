# solana-zktls-coprocessor

**Coprocessor kriptografik me performancë të lartë dhe orakull zkTLS për ekosistemin Solana.**

Ura Zero-Knowledge midis të dhënave Web2 (përgjigje HTTPS të atestuara nga ura me EdDSA-Poseidon) dhe Solana:
prova Groth16 gjenerohet off-chain dhe verifikohet on-chain me syscall-et vendase `alt_bn128`.

> ⚠️ Projekt në fazë zhvillimi. Shih [Gjendja e zbatimit](#gjendja-e-zbatimit) për atë që është
> testuar dhe atë që nuk është. Fixture-t në `tests/fixtures/` (çelësi privat i noterit testues) janë
> **vetëm për teste**.

---

## Arkitektura

```text
 ┌────────────────────────┐   sesion i noterizuar (JSON)
 │  Banka / API HTTPS     │──►  HTTPS → TLSNotary → urë → EdDSA-Poseidon
 └────────────────────────┘                │
                                           ▼
 ┌───────────────────────────────────────────────────────────────┐
 │ crates/triad-core  (Rust, no_std opsional)                    │
 │  MED-01 normalizer  maskim PII + nxjerrje zero-copy           │
 │  ANT-05 euclid      Stein GCD, inversi CT, -N^-1 mod 2^64     │
 │  MOD-06 montgomery  CIOS 2048/4096 + AVX-512 IFMA             │
 └───────────────────────────────────────────────────────────────┘
                                           │ VerifiedSession
                                           ▼
 ┌───────────────────────────────────────────────────────────────┐
 │ crates/zktls-prover                                           │
 │  tls_parser → witness (input.json) → prover (Groth16 BN254)   │
 │  solana: kodim EIP-197 (G1 64 B, G2 128 B, Fr 32 B)           │
 └───────────────────────────────────────────────────────────────┘
          │ circuits/main.circom (EdDSA-Poseidon + kushti)
          ▼
 ┌───────────────────────────────────────────────────────────────┐
 │ programs/solana-zk-verifier (Anchor)                          │
 │  whitelist bankash · lidhje me portofolin · alt_bn128_pairing │
 │  → AttestationRecord (PDA ["attestation", user, claim_id])    │
 └───────────────────────────────────────────────────────────────┘
```

### Modeli i besimit (e rëndësishme)

TLS nuk nënshkruan përmbajtjen HTTP. TLSNotary vërteton sesionin; **ura** verifikon
prezantimin dhe nënshkruan mesazhin kanonik 128-bajtësh me **BabyJubjub EdDSA-Poseidon**.
Qarku verifikon këtë nënshkrim dhe lidh çelësin me whitelist-ën on-chain.

| Bajtet  | Fusha          | Përshkrimi                                |
|---------|----------------|-------------------------------------------|
| 0..8    | magic          | `ZKTLSv03`                                |
| 8..16   | value          | u64 BE (p.sh. bilanci)                    |
| 16..24  | timestamp      | u64 BE, sekonda Unix                      |
| 24..32  | claim_key_id   | SHA-256(`"balance"`)[0..8]                |
| 32..64  | wallet         | çelësi publik Solana i përdoruesit        |
| 64..96  | payload_digest | SHA-256 i përgjigjes HTTP pas maskimit    |
| 96..128 | account_tag    | `0x00 ‖ HMAC(tag_key, "zktls/account-tag/v2" ‖ issuer ‖ id)[1..]` |

Qarku hash-on të 128 bajtet me Poseidon (domain + pesë copa BE prej ≤31 bajtesh),
pastaj verifikon EdDSA-në. SHA-256 mbetet digest-i i payload-it që llogarit ura jashtë
qarkut. Asnjë bajt nuk reduktohet modulo fushës. Formati i sesionit është `version: 3`;
sesionet dhe çelësat RSA të vjetër kërkojnë migrim.

**Privatësia:** ura sheh të gjithë përgjigjen e bankës në RAM. Edhe daemon-i i provës
sheh përgjigjen nëse përdorni `/prove` në një server të largët. Kodi i urës nuk e ruan
transkriptin dhe nuk e log-on, por operatori mund të ndryshojë serverin ose të lexojë RAM-in.
Përdoruesi duhet t'u besojë operatorëve; ZK-ja fsheh vlerën nga Solana, jo nga këta serverë.
`session.tls.json` ruan përgjigjen lokalisht dhe duhet trajtuar si të dhënë private.

**v3 i matur:** 6,582 kufizime (42.1× më pak se 277,321 e RSA), PPoT `2^13`.
Në CI Linux, Rapidsnark + C++ mori ~110 ms gjithsej (një mostër); daemon-i arriti
8.75 prova/s. Shih [migrimin dhe matjet](docs/V3.md). Koha 50 ms dhe `zkey <2 MB` nuk janë premtime.

### Hyrjet publike (10)

`[commitment, attested_at, claim_key_id, nullifier, bank_public_key_hash, min_value, max_value, app_id, user_pubkey_hi, user_pubkey_lo]`

* `commitment = Poseidon(value, salt)` – vlera mbetet private.
* `min_value ≤ value ≤ max_value` (64-bit) – një formë e vetme për pragje, kufij dhe data.
* `nullifier = Poseidon(account_tag, app_id)` – i njëjti për të njëjtën llogari brenda një aplikacioni,
  i palidhshëm mes aplikacioneve. Programi e lidh me portofolin e parë (PDA `["nullifier", nullifier]`),
  ndaj një llogari bankare nuk mund të "farmojë" airdrop/votë me shumë portofola.
* `app_id` – vendoset në `initialize`; prova për një aplikacion tjetër refuzohet.
* `claim_key_id` – `initialize` merr listën e llojeve të lejuara (p.sh. vetëm `date_of_birth`), ndaj
  një provë "bilanc ≤ 20081104" nuk mund të kalojë si provë moshe. `initialize` mund ta thërrasë
  vetëm upgrade authority i programit.
* `bank_public_key_hash = Poseidon(domain, Ax, Ay)` – kontrollohet kundër whitelist-ës on-chain.
* `user_pubkey_hi/lo` – programi kërkon që të përputhen me nënshkruesin e transaksionit (anti-replay).

### Mosha ≥ 18 pa zbuluar datëlindjen

Normalizuesi e kthen `"date_of_birth": "2001-05-17"` në `20010517`. Mosha ≥ N provohet si
`value ≤ cutoff`, ku `cutoff = sot − N vjet` (YYYYMMDD): `coprocessor prove --min-age 18`.
On-chain shihet vetëm `max_value = 20081104`, jo data e lindjes.

Asnjë e dhënë personale (emër, IBAN, cookie) nuk del si hyrje publike.

---

## Struktura

```text
├── Cargo.toml                 workspace (triad-core, zktls-prover, coprocessor-cli)
├── Anchor.toml, package.json  testet TypeScript të programit
├── Makefile
├── crates/
│   ├── triad-core/src/        lib, bigint, normalizer, euclid, montgomery, rsa, simd/{avx512,neon}
│   ├── zktls-prover/src/      tls_parser, attestation, witness, poseidon, prover, solana, mock
│   └── coprocessor-cli/src/   main.rs (prove, verify, serve, setup, mock-session, export-vk, bank-key-hash)
├── circuits/                  main.circom, claim_extractor.circom, scripts/
├── programs/solana-zk-verifier/  Anchor: lib, state, error, instructions/{initialize,verify_proof}
└── tests/                     e2e_mock_tls.rs, benchmarks/montgomery_bench.rs, fixtures/
```

---

## Instalimi

Kërkesat: Rust ≥ 1.89 (AVX-512 intrinsics), për qarqet `circom` ≥ 2.1.6 + Node.js ≥ 18,
për programin `solana-cli` (Agave) + `anchor-cli` 0.31.1.

```bash
make test          # të gjitha testet Rust + testet native të programit
make bench         # CIOS portabël kundrejt AVX-512 IFMA
make demo          # mock-session → setup → prove → verify (pa Solana)
make circuits      # circom → build/main.r1cs + wasm
make setup-keys    # Powers of Tau + Groth16 zkey + vk.json për Solana
make program-build # cargo build-sbf
make anchor-test   # localnet + testet TypeScript
```

## Përdorimi i CLI-së

```bash
coprocessor mock-session --key tests/fixtures/mock_notary_seed.hex \
    --balance 15200 \
    --wallet $(solana address) --out session.tls.json
coprocessor setup --out-dir artifacts                 # çelësat e qarkut DEV
coprocessor prove --backend dev --input session.tls.json --min 10000 --app airdrop-2026 --out proof.json
coprocessor mock-session --kind age --dob 2001-05-17 ... --out id.tls.json
coprocessor prove --backend dev --input id.tls.json --min-age 18 --app kyc-lite --out age.json
coprocessor prove --backend snarkjs ...               # qarku i plotë Circom
coprocessor verify --proof proof.json --vk artifacts/vk.json
coprocessor bank-key-hash --input session.tls.json    # për add_bank_key
coprocessor serve --backend dev --addr 127.0.0.1:8787 # vetëm localnet, GET /health, POST /prove
coprocessor serve --backend rapidsnark --rapidsnark /usr/local/bin/prover \
    --witness-bin circuits/build/cpp/main_c_cpp/main_c         # qarku i plotë (CPP_WITNESS=1 make circuits)
```

## Demo web (lidh portofolin → provo → verifiko on-chain)

Faqja `app/` ka 4 hapa: lidh portofolin (Phantom/Solflare/Backpack), zgjidh "bilanc ≥ 10 000 €"
ose "mosha ≥ 18", lidhu me burimin (noter i **simuluar**), gjenero provën dhe dërgoje në Solana.

```bash
solana-test-validator                                  # terminal 1
node scripts/sync-program-ids.mjs                      # nga rrënja
anchor build && anchor deploy
cargo build --release -p coprocessor-cli
cd app && npm install && npm run init-localnet         # initialize + VK + whitelist + IDL
../target/release/coprocessor serve --backend dev --pk ../artifacts/demo/dev_pk.bin \
    --mock-notary-key ../tests/fixtures/mock_notary_seed.hex   # terminal 2
npm run dev                                            # http://localhost:5173
```

Phantom duhet vendosur në "Localnet" (ose ndryshoni `VITE_RPC_URL`). Për integruesit (DEX, wallet,
DAO), `app/src/sdk.ts` ka `isVerified(program, wallet, {claim, minValue, maxAgeSecs})` (kontrollon
pragun dhe freskinë, jo vetëm ekzistencën e llogarisë), `fetchAttestation` dhe `verifyAndAttest`.

## Noteri real (TLSNotary) → ura → qarku

`bridge/` (workspace më vete) lidh TLSNotary `v0.1.0-alpha.15` me qarkun:

* `zktls-notary`: noteri. Merr pjesë në TLS-në e përdoruesit me MPC dhe nënshkruan atestimin
  TLSNotary (secp256k1). Nuk sheh as fjalëkalimin/cookie-n, as të dhënat.
* `zktls-fetch`: ana e përdoruesit (CLI). Bën kërkesën HTTPS te banka përmes noterit dhe ndërton një
  prezantim që fsheh `Authorization`/`Cookie`, e dërgon te ura dhe ruan `session.tls.json`.
* `zktls-bridge`: `POST /attest`. Verifikon noterin (çelës i besuar), certifikatën e bankës,
  domenin + rrugën (politika), freskinë (≤ 10 min), përgjigjen e plotë, dhe nënshkruan mesazhin
  128-bajtësh me EdDSA-Poseidon, të verifikuar nga qarku v3.

```bash
cd bridge
NOTARY_KEY=<32 bajte hex> cargo run --release --bin zktls-notary          # printon çelësin publik
BRIDGE_TAG_KEY=<sekreti-i-qendrueshem-32-bajte-hex> \
  cargo run --release --bin zktls-bridge -- --policy policy.json \
    --signing-key bridge_seed.hex                   # shih bridge-policy.example.json
cargo run --release --bin zktls-fetch -- --url https://api.banka.al/v1/accounts/me \
    --header "Authorization: Bearer …" --wallet $(solana address) --bridge 127.0.0.1:8790
coprocessor prove --input session.tls.json --min 10000 --app demo         # pastaj si më parë
```

Për provë pa bankë reale: `cargo run --example test_bank` (domeni `test-server.io`) me
`--connect 127.0.0.1:4443 --test-root-der <root_ca_cert.der>`. Testi `bridge/tests/e2e_tlsnotary.rs`
e bën të gjithë rrjedhën në proces (MPC-TLS real, ~3 s) dhe refuzon noter të panjohur, domen jashtë
politikës, rrugë jashtë politikës, sesion të vjetër dhe prezantim të ndryshuar. Faqja demo pranon
`session.tls.json` me "ngarko".

**Kufizimet aktuale:** ura dhe prover-i i largët e shohin përgjigjen (shih modelin e
besimit më lart). `zktls-fetch` dërgon gjithmonë `Accept-Encoding: identity` dhe refuzon
mbishkrimin e tij. Serveri mund ta injorojë; ura refuzon kompresimin. `chunked` ende nuk
mbështetet: ky header kontrollon kompresimin, jo transferimin HTTP. Në browser nevojitet
zgjerimi/`tlsn-js` i TLSNotary, ndaj sot rrjedha reale bëhet me CLI.

**Noteri.** Banka nuk nënshkruan të dhëna, ndaj një server noteri (në prodhim `notary-server` i
TLSNotary, i ekzekutuar nga ju) merr pjesë në lidhjen TLS me MPC dhe dëshmon që përgjigjja erdhi nga
domeni i bankës, pa parë fjalëkalimin apo bilancin. Banka nuk duhet të bëjë asgjë. Demoja web
përdor noterin e simuluar (`/mock-session`) ose një `session.tls.json` real nga `zktls-fetch`.

## Integrimi në një DEX / aplikacion (`zk-gate`)

Programi verifikues ruan për çdo portofol **dhe çdo lloj pretendimi** një
`AttestationRecord` (PDA `["attestation", wallet, claim_key_id LE]`): bilanci dhe mosha
janë atestime të pavarura, njëra nuk mbishkruan tjetrën. Një program tjetër kërkon
vetëm ato që i duhen dhe thërret një funksion; nuk sheh asnjë të dhënë bankare.
Shembull i plotë: `programs/gated-pool` (pool vetëm-bilanc, vetëm-moshë ose të dyja).

```toml
solana-zk-verifier = { path = "...", features = ["cpi"] }
```

```rust
use solana_zk_verifier::{gate::{self, Requirement}, AttestationRecord, VerifierState};

pub balance_attestation: Account<'info, AttestationRecord>,
pub age_attestation: Option<Account<'info, AttestationRecord>>,
/// PDA ["verifier_state"] e programit verifikues (whitelist-a aktuale e bankave)
pub verifier_state: Box<Account<'info, VerifierState>>,

// në handler: PDA-ja (portofol + lloj), burimi ende i autorizuar dhe kushti kontrollohen brenda
let state = &ctx.accounts.verifier_state;
gate::require_attested_current(&ctx.accounts.balance_attestation, state, &user,
    &Requirement::at_least("balance", 5_000).fresh_within(86_400))?;
// mosha: kufiri llogaritet on-chain nga ora e Solana-s (UTC)
gate::require_attested_current(age, state, &user, &Requirement::min_age(18, Clock::get()?.unix_timestamp))?;
```

Për bilancet vendosni gjithmonë `fresh_within(...)`: pa afat, një bilanc i vërtetuar një
herë vlen përgjithmonë, edhe pasi paratë janë tërhequr. `.from_source(hash)` pranon vetëm
një bankë/urë të caktuar.

Gabimet: `GateWrongAccount` (atestim i portofolit/llojit tjetër), `GateWrongUser`,
`GateClaimMismatch`, `GateThresholdNotMet`, `GateExpired`, `GateSourceRevoked` (banka u
hoq nga whitelist-a), `GateEpochRevoked` (autoriteti i revokoi të gjitha atestimet e vjetra
me `bump_policy_epoch` ose VK të re), `GateWrongSource`; pa atestim fare, Anchor kthen `AccountNotInitialized`. Në front-end: `depositToPool` te `app/src/sdk.ts` dhe
hapi 5 i demos.

## Deploy (fly.io + devnet)

Shih [docs/DEPLOY.md](docs/DEPLOY.md): një imazh Docker për daemon-in (rapidsnark),
urën dhe noterin në fly.io; `scripts/deploy-devnet.sh` për programet.

## API kryesore (Rust)

```rust
// MED-01
let claim = AlgebraicNormalizer::normalize_and_mask(raw_http, "balance", 10_000)?;
AlgebraicNormalizer::mask_sensitive_in_place(&mut buf)?;   // pa alokim, ruan gjatësinë
// ANT-05
let inv = EuclidCore::modular_inverse(&a, &m);              // kohë konstante, m tek
let mu  = EuclidCore::montgomery_constant(n[0]);            // -N^-1 mod 2^64
// MOD-06
let eng = MontgomeryEngine::new(&modulus);                  // 1..=64 fjalë
eng.mul(&a, &b, &mut out);                                  // a·b·R^-1 mod N
eng.exp(&base, &secret_exp, &mut out);                      // CT, AVX-512 automatikisht
eng.exp_vartime(&sig, &[65537], &mut out);                  // eksponent publik
// RSA
RsaPublicKey::from_be_bytes(&n, 65537)?.verify_pkcs1v15_sha256(msg, sig)?;
```

Programi Anchor: `initialize(vk, max_age)`, `set_verifying_key(vk)`, `add_bank_key(hash)`,
`remove_bank_key(hash)`, `verify_and_attest(proof_a, proof_b, proof_c, public_inputs, nullifier, claim_key_id)`, `set_allowed_claim_keys(ids)`.

---

## Gjendja e zbatimit

| Komponenti | Gjendja | Testuar |
|---|---|---|
| `triad-core` Montgomery CIOS, exp CT (dritare fikse), exp vartime (dritare rrëshqitëse) | ✅ i plotë | ✅ kundër `num-bigint`, 1–64 fjalë |
| `triad-core` AVX-512 IFMA (radix 2^52) | ✅ i plotë | ✅ kundër CIOS, në CPU me IFMA |
| `triad-core` Neon (aarch64) | ⚠️ delegon te CIOS skalar (shih `simd/neon.rs`) | ❌ pa hardware ARM |
| `triad-core` Euklidi (Stein, inversi CT), normalizer, RSA PKCS#1 | ✅ | ✅ |
| `zktls-prover` parser sesioni v3, EdDSA, dëshmitari Circom, Poseidon (circomlib) | ✅ | ✅ (vektor referencë circomlib) |
| Prapavija **dev** (arkworks Groth16, 10 hyrje) | ✅ | ✅ e2e + verifikim me ekuacionin on-chain |
| Prapavija **snarkjs** (qarku i plotë) | ✅ | ✅ qarku v3; matjet në [docs/V3.md](docs/V3.md) |
| Qarqet Circom v3 (6 582 kufizime) | ✅ `circomlib`, EdDSA-Poseidon | ✅ vektor i pavarur circomlibjs + 142 witness-e negative; PPoT 2^13 |
| Programi Anchor | ✅ | ✅ `build-sbf` + localnet + 21 teste TypeScript në CI (përfshirë anti-Sybil dhe `app_id`); prova e qarkut të plotë verifikohet me kodin e programit |
| Demo web `app/` (Vite + React + wallet-adapter) | ✅ me noter të simuluar | ✅ build + rrjedha HTTP e daemon-it në CI; rrjedha në browser e testuar me një portofol testues |

**Prapavija dev nuk është e sigurt për prodhim**: ajo provon vetëm diapazonin, zotimin dhe nullifier-in;
nënshkrimi EdDSA verifikohet natively jashtë provës, pra kushdo me çelësin e provimit mund të deklarojë
çdo `bank_key_hash`. CLI-ja përdor Rapidsnark si parazgjedhje. `--backend dev` është zgjedhje eksplicite dhe daemon-i dev lidhet vetëm në loopback. Devnet kërkon `VK_JSON` të qarkut Circom.

### Matjet (benchmark, x86_64 me AVX-512 IFMA, profili `bench`)

| Operacioni | Portabël | AVX-512 IFMA |
|---|---|---|
| RSA-2048 `exp` (eksponent 2048-bit, CT) | ~5.0 ms | ~1.9 ms (2.6×) |
| RSA-4096 `exp` (CT) | ~48 ms | ~9.8 ms (4.9×) |
| RSA-2048 verifikim `s^65537` | ~69 µs | – |
| Provë Groth16 (qarku dev) | ~60 ms | – |

### Devijimet nga specifikimi fillestar (dhe arsyet)

1. **v3:** RSA-2048 + SHA-256 brenda qarkut u zëvendësuan me EdDSA-Poseidon, sepse
   nënshkruesi është ura jonë. Biblioteka aritmetike `triad-core` ruan RSA/CIOS si API më vete.
2. Banka autentikohet nga TLSNotary; ura nënshkruan atestimin, jo certifikata e bankës.
3. Vlera, timestamp-i, lloji, portofoli, payload digest-i dhe account tag-u lidhen të gjitha
   me nënshkrimin. Vektori Rust/Circom/circomlibjs kontrollon përputhshmërinë.
4. **CU**: `verify_and_attest` konsumoi **145 414 CU** në localnet me 10 hyrje publike
   dhe provën DEV. Prova e plotë Circom u verifikua veçmas nga kodi vendas i programit.
5. **Madhësia e provës**: 128 B është forma e kompresuar; syscall-i kërkon formën e pakompresuar
   (256 B: A 64 + B 128 + C 64). Programi e mohon vetë `A`.
6. **AVX-512 përdoret brenda `exp`**, jo në `mul`, sepse domeni Montgomery radix-2^52 (`R' = 2^2080`)
   ndryshon nga ai radix-2^64 (`R = 2^2048`).
7. **ID-të e programeve** janë çelësa publikë realë, të sinkronizuar në `declare_id!`
   dhe `Anchor.toml`. Keypair-et lokale mbeten te `target/deploy/` (jashtë Git).
   `node scripts/sync-program-ids.mjs` ose `anchor keys sync` sinkronizon instalimin tuaj;
   ID e vlefshme nuk do të thotë që programi është deploy-uar.
8. U shtuan në program: `set_verifying_key`, `add/remove_bank_key`, kufiri i moshës së atestimit dhe
   eventi `AttestationVerified`.

## Rregullat e sigurisë

1. **Kohë konstante**: `mul`, `exp`, zbritja përfundimtare dhe inversi përdorin `subtle`, pa degëzime
   sipas sekreteve. `exp_vartime` dhe `binary_gcd` janë vetëm për vlera publike.
2. **Memoria**: `mul`/`exp` punojnë me bufera në stack (≤ 4096 bit); normalizuesi nuk alokon
   (intervalet e maskimit në bufer fiks, hash-i në rrjedhë).
3. **Privatësia**: vetëm zotimi, pragu, koha, ID e çelësit, hash-i i bankës dhe portofoli janë publikë.
4. **Whitelist**: `verify_and_attest` refuzon çdo provë me `bank_key_hash` jashtë listës së autoritetit.
5. **Portofoli i lidhur me TLS**: prover-i dërgon `x-zktls-wallet` brenda sesionit me bankën
   (pjesë e transkriptit të autentikuar); ura refuzon një portofol tjetër në kërkesën JSON.
6. **Nullifier i fshehtë**: `account_tag = HMAC(BRIDGE_TAG_KEY, issuer ‖ id)`. Pa çelësin
   nuk provohen ID të hamendësueshme, dhe e njëjta ID te dy banka jep nullifier-ë të ndryshëm.
   Sekreti është i ndarë nga çelësi EdDSA dhe `issuer` nga domeni, ndaj rrotullimi i çelësit
   ose ndërrimi i domenit nuk e lejon të njëjtën llogari me portofol të dytë. Operatori i
   urës e ka sekretin: ai mund ta lidhë nullifier-in me llogarinë.
7. **Datëlindja**: prova e moshës vërteton `1900-01-01 ≤ datëlindja ≤ kufiri`; `min_age`
   on-chain e kërkon këtë kufi të poshtëm dhe freskinë e pool-it.
8. **Ura**: rruga pa `..`/`%`/`//`, monedha e detyrueshme për bilancet, JSON Pointer opsional
   për vendin e vlerës, decimalet pa prerje të heshtur (`15200.99` me `decimals: 0` refuzohet),
   datëlindje vetëm data kalendarike reale, transkript ≤ 1 MiB i kontrolluar para alokimit.
   **Daemon-i**: header-at ≤ 16 KiB, trupi ≤ 1 MiB, ≤ 128 lidhje, `x-api-key` dhe kufi për IP
   opsionalë, procesi i provës vritet pas 300 s, skedarët e përkohshëm `0700`/`0600`.
9. **Administrimi**: authority ndërrohet në dy hapa (`propose_authority` → `accept_authority`),
   që një adresë e gabuar të mos e bllokojë programin përgjithmonë. `bump_policy_epoch`
   revokon menjëherë çdo atestim të mëparshëm te `require_attested_current`.

## Plani (milestones)

| Faza | Gjendja |
|---|---|
| M1 Core Engine | ✅ |
| M2 ZK Circuits | ✅ (ceremonia e fazës 2 është me një kontribut; për mainnet duhet ceremoni me shumë palë) |
| M3 Solana Contract | ⚠️ localnet ✅; mbetet deploy në devnet (kërkon portofol me SOL) |
| M4 End-to-End CLI | ✅ (dev dhe qarku i plotë v3); dërgimi në Solana bëhet nga testi TS |
| M5 UI & Demo | ✅ demo; shih kufizimet e TLSNotary në browser |

Licenca: Apache-2.0
