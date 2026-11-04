# solana-zktls-coprocessor

**Coprocessor kriptografik me performancë të lartë dhe orakull zkTLS për ekosistemin Solana.**

Ura Zero-Knowledge midis të dhënave Web2 (përgjigje HTTPS të atestuara me RSA-2048) dhe Solana:
prova Groth16 gjenerohet off-chain dhe verifikohet on-chain me syscall-et vendase `alt_bn128`.

> ⚠️ Projekt në fazë zhvillimi. Shih [Gjendja e zbatimit](#gjendja-e-zbatimit) për atë që është
> testuar dhe atë që nuk është. Fixture-t në `tests/fixtures/` (çelësi privat i noterit testues) janë
> **vetëm për teste**.

---

## Arkitektura

```text
 ┌────────────────────────┐   sesion i noterizuar (JSON)
 │  Banka / API HTTPS     │──►  HTTP response + atestim RSA-2048 (128 B)
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
          │ circuits/main.circom (RSA + SHA-256 + kushti)
          ▼
 ┌───────────────────────────────────────────────────────────────┐
 │ programs/solana-zk-verifier (Anchor)                          │
 │  whitelist bankash · lidhje me portofolin · alt_bn128_pairing │
 │  → AttestationRecord (PDA ["attestation", user])              │
 └───────────────────────────────────────────────────────────────┘
```

### Modeli i besimit (e rëndësishme)

TLS nuk nënshkruan të dhënat e aplikacionit, vetëm handshake-un. Prandaj nënshkrimi RSA-2048 që
verifikon qarku është ai i një **noteri** (TLSNotary / MPC-TLS ose një oracle i bankës) mbi një
mesazh kanonik 128-bajtësh:

| Bajtet  | Fusha          | Përshkrimi                                |
|---------|----------------|-------------------------------------------|
| 0..8    | magic          | `ZKTLSv01`                                |
| 8..16   | value          | u64 BE (p.sh. bilanci)                    |
| 16..24  | timestamp      | u64 BE, sekonda Unix                      |
| 24..32  | claim_key_id   | SHA-256(`"balance"`)[0..8]                |
| 32..64  | wallet         | çelësi publik Solana i përdoruesit        |
| 64..96  | payload_digest | SHA-256 i përgjigjes HTTP pas maskimit    |
| 96..128 | nonce          | rastësi e noterit                         |

Qarku llogarit SHA-256 të këtij mesazhi **brenda provës**, kështu bilanci është i lidhur me
nënshkrimin. Protokolli MPC-TLS që e prodhon transkriptin nga një lidhje e gjallë **nuk** është pjesë
e kësaj depoje.

### Hyrjet publike (7)

`[commitment, attested_at, claim_key_id, bank_public_key_hash, min_threshold, user_pubkey_hi, user_pubkey_lo]`

* `commitment = Poseidon(value, salt)` – bilanci mbetet privat.
* `bank_public_key_hash = PoseidonLarge(121,17)(N)` – kontrollohet kundër whitelist-ës on-chain.
* `user_pubkey_hi/lo` – programi kërkon që të përputhen me nënshkruesin e transaksionit (anti-replay).

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
├── circuits/                  main.circom, rsa_verify_2048.circom, claim_extractor.circom, scripts/
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
coprocessor mock-session --key tests/fixtures/mock_notary_key.pem \
    --cert tests/fixtures/mock_notary_cert.pem --balance 15200 \
    --wallet $(solana address) --out session.tls.json
coprocessor setup --out-dir artifacts                 # çelësat e qarkut DEV
coprocessor prove --input session.tls.json --threshold 10000 --out proof.json
coprocessor prove --backend snarkjs ...               # qarku i plotë Circom
coprocessor verify --proof proof.json --vk artifacts/vk.json
coprocessor bank-key-hash --input session.tls.json    # për add_bank_key
coprocessor serve --addr 127.0.0.1:8787               # GET /health, POST /prove
```

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
`remove_bank_key(hash)`, `verify_and_attest(proof_a, proof_b, proof_c, public_inputs)`.

---

## Gjendja e zbatimit

| Komponenti | Gjendja | Testuar |
|---|---|---|
| `triad-core` Montgomery CIOS, exp CT (dritare fikse), exp vartime (dritare rrëshqitëse) | ✅ i plotë | ✅ kundër `num-bigint`, 1–64 fjalë |
| `triad-core` AVX-512 IFMA (radix 2^52) | ✅ i plotë | ✅ kundër CIOS, në CPU me IFMA |
| `triad-core` Neon (aarch64) | ⚠️ delegon te CIOS skalar (shih `simd/neon.rs`) | ❌ pa hardware ARM |
| `triad-core` Euklidi (Stein, inversi CT), normalizer, RSA PKCS#1 | ✅ | ✅ |
| `zktls-prover` parser X.509/sesion, dëshmitari Circom, Poseidon (circomlib) | ✅ | ✅ (vektor referencë circomlib) |
| Prapavija **dev** (arkworks Groth16, 7 hyrje) | ✅ | ✅ e2e + verifikim me ekuacionin on-chain |
| Prapavija **snarkjs** (qarku i plotë) | ✅ | ✅ në CI (`zk-e2e.yml`): provë në ~17 s, ~1.7 GB RAM |
| Qarqet Circom (277 321 kufizime) | ✅ mbi `@zk-email/circuits` + `circomlib` | ✅ në CI: kompilim, ceremoni (PPoT 2^19), provë, `snarkjs verify`, refuzim i mesazhit të ndryshuar |
| Programi Anchor | ✅ | ✅ `build-sbf` + localnet + 3 teste TypeScript në CI; prova e qarkut të plotë verifikohet me kodin e programit |
| M5 UI (Next.js + Phantom) | ❌ jo ende | – |

**Prapavija dev nuk është e sigurt për prodhim**: ajo provon vetëm `value ≥ threshold` dhe zotimin;
nënshkrimi RSA verifikohet natively jashtë provës, pra kushdo me çelësin e provimit mund të deklarojë
çdo `bank_key_hash`. Në prodhim regjistroni me `set_verifying_key` VK-në e qarkut Circom.

### Matjet (benchmark, x86_64 me AVX-512 IFMA, profili `bench`)

| Operacioni | Portabël | AVX-512 IFMA |
|---|---|---|
| RSA-2048 `exp` (eksponent 2048-bit, CT) | ~5.0 ms | ~1.9 ms (2.6×) |
| RSA-4096 `exp` (CT) | ~48 ms | ~9.8 ms (4.9×) |
| RSA-2048 verifikim `s^65537` | ~69 µs | – |
| Provë Groth16 (qarku dev) | ~60 ms | – |

### Devijimet nga specifikimi fillestar (dhe arsyet)

1. **Copat RSA në qark: 17 × 121 bit, jo 32 × 64.** Me 121 bit, produktet dhe akumulimi nxënë në
   fushën BN254 dhe përdoret aritmetika e audituar e `@zk-email/circuits`.
2. **Të dhënat e nënshkruara** janë mesazhi kanonik i noterit (shih më lart), jo përgjigja HTTPS e
   nënshkruar nga certifikata e bankës, sepse TLS nuk ofron nënshkrime mbi të dhënat.
3. **Qarku i specifikimit u ndreqë**: `diff <-- …` nuk ishte i kufizuar, moduli RSA nuk ishte i lidhur
   me `bank_public_key_hash` dhe hash-i i nënshkruar nuk ishte i lidhur me bilancin. Tani të treja janë
   të kufizuara, dhe u shtua lidhja me portofolin.
4. **CU**: ~82k mbulon vetëm pairing-un; me 7 hyrje publike, `verify_and_attest` konsumon
   **118 261 CU** (e matur në localnet në CI).
5. **Madhësia e provës**: 128 B është forma e kompresuar; syscall-i kërkon formën e pakompresuar
   (256 B: A 64 + B 128 + C 64). Programi e mohon vetë `A`.
6. **AVX-512 përdoret brenda `exp`**, jo në `mul`, sepse domeni Montgomery radix-2^52 (`R' = 2^2080`)
   ndryshon nga ai radix-2^64 (`R = 2^2048`).
7. **ID e programit** në specifikim (`ZkTLSCoprocessor1111…111`, 45 shenja) dekodohet në 33 bajte dhe
   nuk është i vlefshëm; u zëvendësua me `ZkTLSCoprocessor111111111111111111111111111` (placeholder,
   ekzekutoni `anchor keys sync`).
8. U shtuan në program: `set_verifying_key`, `add/remove_bank_key`, kufiri i moshës së atestimit dhe
   eventi `AttestationVerified`.

## Rregullat e sigurisë

1. **Kohë konstante**: `mul`, `exp`, zbritja përfundimtare dhe inversi përdorin `subtle`, pa degëzime
   sipas sekreteve. `exp_vartime` dhe `binary_gcd` janë vetëm për vlera publike.
2. **Memoria**: `mul`/`exp` punojnë me bufera në stack (≤ 4096 bit); normalizuesi nuk alokon
   (intervalet e maskimit në bufer fiks, hash-i në rrjedhë).
3. **Privatësia**: vetëm zotimi, pragu, koha, ID e çelësit, hash-i i bankës dhe portofoli janë publikë.
4. **Whitelist**: `verify_and_attest` refuzon çdo provë me `bank_key_hash` jashtë listës së autoritetit.

## Plani (milestones)

| Faza | Gjendja |
|---|---|
| M1 Core Engine | ✅ |
| M2 ZK Circuits | ✅ (ceremonia e fazës 2 është me një kontribut; për mainnet duhet ceremoni me shumë palë) |
| M3 Solana Contract | ⚠️ localnet ✅; mbetet deploy në devnet (kërkon portofol me SOL) |
| M4 End-to-End CLI | ✅ (backend dev); dërgimi në Solana bëhet nga testi TS |
| M5 UI & Demo | ❌ |

Licenca: Apache-2.0
