# Rishikimi i kërkimit të ekipit (28 tetor 2026)

> **Dokument historik (para implementimit).** Ruhet si kontekst. Kur ndryshon nga kodi ose nga dokumentet normative, vlejnë [numeric-spec.md](../numeric-spec.md), [account-layout.md](../account-layout.md), [tx-budget.md](../tx-budget.md), [ADR-të](../adr/README.md) dhe [STATUS.md](../../STATUS.md). Indeksi: [docs/README.md](../README.md).

Burimi: raporti JSON i grupit kërkues (36 seksione: F01–F20, ADR01–12, N01–N10, K01–K10, T01–T15, portat A–E, L01–L12, P01–P08). Raporti origjinal nuk është ruajtur në repo; këtu është krahasimi me kodin dhe vendimet që rrjedhin prej tij. Statuset e raportit (PROPOZIM, E PROVUAR NË MODEL) ruhen: asnjë pikë nuk trajtohet si e provuar onchain.

## 1. Vendimi kryesor i raportit

Vazhdojmë me **8 CLMM lokale të pavarura** (çmim, ticks, vault-e dhe LP për çdo shard), registry vetëm-lexim gjatë swap-it, aktivizim deri në 56. Matematikë standarde CLMM, jo invariant global i ri. **Pa kapital real** deri sa të mbyllen portat C–E. Kjo përputhet me handoff-in dhe me atë që kemi ndërtuar.

Çfarë raporti thotë se **nuk** është bërë: kontratë e audituar, transaksion Solana v1 i ekzekutuar, matje CU/landing/TPS, provë e plotë solvence me numra të plotë, provë fitimprurjeje.

## 2. Ku raporti konfirmon kodin tonë

| Tema | Raporti | Repo |
|---|---|---|
| MIN/MAX sqrt price, MIN/MAX tick | Konstante ekzakte, floor/ceil të ndara | `constants.rs`, `docs/vectors/tick_vectors.json` (referencë 120 shifra) |
| Gjerësitë e produkteve (288/320 bit) | U256 nuk mjafton; U512 ose zëvendësim i provuar | ADR-0001 pika 4 (`mul_div` 512-bit) |
| Maska | `id < 56`, bitet 56–63 të rezervuara; `hash & 55` arrin vetëm 32 ID | `math/bitmap.rs` (`select_by_hash` me rank) |
| Tarifat e protokollit | Lokale për shard, mbledhje e veçantë | `ShardState.protocol_fees_*`, ADR-0004 |
| Exact-in | Konsum i plotë ose revert | ADR-0001 pika 6 |
| Token program | Vetëm SPL klasik në v1 | `prepare_shard` refuzon Token-2022 (test SVM) |
| Keeper | "Keeper-only rebalance" **nuk** pengon arbitrazhin me dy swap-e publike | Nuk e pretendojmë; rebalance jashtë MVP |
| Lending | Specifikim i veçantë, pas AMM-së | Jashtë fushës deri ENG-10+ |

## 3. Ku raporti ndryshon planin tonë (miratuar në këtë rishikim)

| # | Ndryshimi | Burimi | Ku zbatohet |
|---|---|---|---|
| 1 | Fee growth **u256 Q128, checked, pa wrap** në vend të u128 Q64 modular. `outside ≤ global`, deltat jo-negative; mbetja e ndarjes qëndron në escrow të LP. | N07 | ADR-0001 (rishikuar), `ShardState` në ENG-05 (ende pa pozicione, ndryshimi i layout-it nuk prek askënd) |
| 2 | Gap me **L = 0 dhe distancë pozitive → gabim**, jo lëvizje pa konsum. | N04, kundërshembuj | ADR-0001 |
| 3 | Hapi me tarifë ndan `nominal_min`, `rounding_dust` dhe `total_fee`; quote dhe event-i i tregojnë veçmas; përdoruesi jep `max_rounding_dust`. Tarifa nominale nuk është kufi i sipërm për input shumë të vogël. | N05, N08 | ADR-0001, ENG-05/06 |
| 4 | **CPMM baseline** i matshëm para CLMM-së, si kontroll (jo si produkt). | T04 | Milestone i ri ENG-05b; `crates/amm-model` është pika e nisjes |
| 5 | **Anulim/refund para nisjes**: Prepared/Ready ≠ i financuar; çdo hap i setup-it duhet të ketë rrugë rikthimi. | T07 | ENG-04 |
| 6 | **Kufiri i llogarive**: kufiri aktual i dokumentuar është 64 (96 mbetet draft). 8 shard × (state + 2 vault + tick arrays) e kalon atë. Router/Jupiter nisin me **një shard për swap**. | F, runtime spec | ENG-07/09 |
| 7 | **WSOL**: `SyncNative`, rezerva e rent-it të llogarisë native, wrap/unwrap në SDK. | token matrix | ENG-03 testet e WSOL, SDK |
| 8 | Evidencë për **transaksion v1** të ekzekutuar mungon; deri atëherë deklarojmë vetëm legacy/v0. | L, T09 | STATUS |
| 9 | **SAMM** është prior art i drejtpërdrejtë. Nuk pretendojmë risi për "AMM me shards". | F, executive | README/STATUS (asnjë deklaratë risie) |

**Saktësim i N07 gjatë ENG-05:** `global` dhe `outside` janë checked pa wrap, siç propozon raporti, por `inside = global − below − above` mund të jetë negativ për një range ku çmimi nuk ka qenë ende. Kundërshembulli: tick-u i sipërm inicializohet nën çmim, çmimi e kalon, pastaj tick-u i poshtëm inicializohet mbi çmim. Prandaj `inside` mbahet mod 2^256 dhe përdoren vetëm diferencat. Testi `inside_can_be_negative` e dokumenton rastin.

Pika ku **mbajmë** specifikimin tonë: seed-i i `generation` mbetet `u64` LE siç e kërkon handoff-i (raporti sugjeron `u32`). Politika "pa kopjim nga Orca" mbetet, edhe pse raporti vëren që commit-i historik c55588a6 ishte MIT; Raydium (Apache-2.0) mbetet e vetmja referencë, me provenancë.

## 4. Vendim i hapur për pronarin

**Mint me freeze authority.** Profili strikt i raportit i refuzon, por USDC ka freeze authority. Opsionet: (a) lejohet me politikë publike të rrezikut të kujdestarisë (propozimi ynë në ADR-0004), (b) refuzohet dhe çifti i parë ndryshon. Deri në vendim, testet përdorin mint-e prove pa freeze authority.

## 5. Rendi i punës që rrjedh

1. **ENG-05** Matematika: tick math, `mul_div` 512-bit, swap step me N05, fee growth N07, teste diferenciale kundrejt referencës Python (P01/P05). Pa këtë nuk ka seed, LP ose swap.
2. **ENG-05b** CPMM baseline me ledger para/pas (T04).
3. **ENG-04** Seed/finalize/cancel me refund dhe test ndërprerjeje në çdo hap (T07).
4. **ENG-06** Swap exact-in, një shard, `min_out`/limit/deadline, `max_rounding_dust`.
5. **ENG-07..09** SDK v0 me ALT, pastaj një v1 real; benchmark-et e tarifave 5/10/30 bps × spacing 10/60 (ADR-0004) me kapital dhe ngarkesë të njëjtë.
6. Portat C–E të raportit (siguri, ekonomi, audit) para çdo kapitali real.

Çdo hap ekzekutohet dhe provohet në GitHub Actions.
