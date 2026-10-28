# AMM me shard-e të pavarura dhe motor likuidimi në Solana

> **Shënim versioni:** kjo dosje ruan kërkimin fillestar. Vendimi aktual i përdoruesit është nisje me **8 shard-e** dhe zgjerim deri në **56** me maskë `u64`. Propozimi fillestar me katër shard-e më poshtë është zëvendësuar nga [overview Rust/Anchor 8→56](../sharded_amm/docs/OVERVIEW_RUST_ANCHOR_8_TO_56.md).

Datë: 28 tetor 2026. Dosje kërkimi dhe projektim paraprak; nuk është kontratë e audituar ose provë performance në blockchain.

## Vendimi i ekipit

Tre agjentë paralelë, me role arkitekt Solana, projektues matematikor AMM dhe inxhinier sigurie/likuidimesh, hulumtuan burimet dhe shkëmbyen kritika. Koordinatori bashkoi gjetjet dhe ekzekutoi një model të vogël të riprodhueshëm.

Rekomandojmë një prototip në Solana me 4 shard-e fikse likuiditeti, secili me gjendje, tick-e, vault-e dhe kontabilitet LP të veçantë. Më pas matim 1/2/4/8 shard-e. Shtojmë likuidimin me rezerva të izoluara dhe vetëm pastaj split/merge dinamik.

Ky është projekt teknikisht i arsyeshëm për kërkim dhe prototip. Nuk kemi provuar që është ekonomikisht më i mirë se një CLMM normal, as që është produkti i parë i këtij lloji. Një çmim global që ndryshohet nga çdo swap mbetet varësi e përbashkët; llogaritë PDA nuk e zhdukin këtë varësi.

## 1. Çfarë verifikuam

| Pretendimi fillestar | Përfundimi |
|---|---|
| Transaction v1 u aktivizua më 15 shtator 2026 | Faqja zyrtare e upgrade-it thotë mainnet epoch 1035, rreth 01:00 UTC. [S1] |
| Madhësia është 4,096 bajt dhe ka 64 adresa inline | Këto janë kufijtë e përshkruar për v1; adresat përfshijnë të gjitha llogaritë dhe programet, jo vetëm shard-et. [S2] |
| V1 heq nevojën për ALT | Formati v1 nuk mbështet ALT. V0 vazhdon t'i mbështesë. [S2] |
| Para v1 nuk mund të kishte sharding | Nuk mbështetet. V0 me ALT arrinte tashmë 64 llogari runtime; v1 thjeshton paketimin. [S1, S2] |
| ALT shkakton gjithmonë një slot pritje gjatë swap-it | Warmup zbatohet për adresat e saposhtuara; nuk është pritje e detyrueshme për çdo përdorim të një ALT ekzistuese. [S4] |
| Çdo protokoll huadhënieje përdor PoolState me sqrtP dhe L | Përgjithësim i pasaktë. Kamino ka obligation të veçantë, por likuidimi shkruan edhe rezerva/vault-e të përbashkëta. [S6] |
| Zero-contention | Vetëm transaksionet me bashkësi të pavarura llogarish shmangin konfliktet përkatëse; brenda të njëjtit shard ka ende serializim. [S3] |
| Është kontrata e parë e këtij lloji | E paverifikuar. Duhet kërkim i veçantë për punët dhe produktet ekzistuese. |

SIMD-0385 në GitHub ende shfaq metadata Review, ndërsa faqja zyrtare e upgrade-it raporton aktivizim. Për datën mbështetemi te faqja e aktivizimit; për formatin te specifikimi. Nuk kemi bërë provë RPC të pavarur në mainnet.

AccountInUse është një gabim i mundshëm në përpunim kur një llogari është e kyçur; nuk do të thotë që çdo swap në një pool të përbashkët dështon. Scheduler-i mund të rendisë transaksionet. [S3]

## 2. Arkitektura e rekomanduar

```text
Klienti / router-i jashtë zinxhirit
        |
        +-- MarketConfig PDA                 read-only gjatë swap
        +-- ShardDirectory PDA               read-only gjatë swap
        |
        +-- Shard 0: state, ticks, vault X/Y, fees, LP positions
        +-- Shard 1: state, ticks, vault X/Y, fees, LP positions
        +-- Shard 2: state, ticks, vault X/Y, fees, LP positions
        +-- Shard 3: state, ticks, vault X/Y, fees, LP positions

Likuiduesi -> obligation + reserve lane + vault-e lokale
           -> shard-et AMM të zgjedhura kur nevojitet swap
```

Kjo është arkitekturë e propozuar nga ekipi. Secili shard ruan `sqrt_price`, `active_liquidity`, `current_tick`, `fee_growth`, tick arrays dhe token vault-et e veta. Një directory ndihmon zbulimin; ndryshohet në operacionet e administrimit të shard-eve, jo për çdo swap. Router-i është i zëvendësueshëm: kontrata verifikon kufijtë dhe llogaritë pa i besuar kuotës së tij.

Shembuj PDA seeds, që duhen specifikuar përfundimisht para implementimit:

```text
market = ["market", mint_x, mint_y, fee_tier]
shard = ["shard", market, shard_id, generation]
vault = ["vault", shard, mint]
position = ["position", shard, owner, position_id]
reserve_lane = ["reserve", market, lane_id, asset]
```

Mint-et duhet të kenë renditje kanonike. Çdo account verifikohet për owner, seeds/bump, mint, token authority, program ID dhe lidhje me market/shard-in e saktë. PDA është adresim/autorizim programor; nuk është thread, CPU ose primitive e re scheduler-i.

Për dy transaksione A/B, paralelizmi kërkon:

```text
W(A) ∩ (R(B) ∪ W(B)) = ∅
W(B) ∩ R(A) = ∅
```

Gjendja, dy vault-et dhe tick-et e ndryshme nuk mjaftojnë nëse të dy transaksionet shkruajnë të njëjtin fee payer, treasury ATA, mint LP, llogari përdoruesi ose vault flashloan-i. Tarifat e protokollit akumulohen lokalisht dhe mblidhen më rrallë.

Një transaksion që prek disa shard-e i kyç të gjitha llogaritë e përfshira përgjatë ekzekutimit. Nuk fiton ekzekutim paralel të instruksioneve brenda vetes. Duhet të deklarojë paraprakisht të gjitha llogaritë; programi nuk mund të zbulojë një PDA të re dhe ta shkruajë pa qenë në account list. [S3, S7]

Me 64 llogari gjithsej, edhe minimumi state + dy vault-e për shard konsumon 3 vende për shard, përpara tick arrays, përdoruesit, programit token, fee payer-it dhe oracle-ve. Numri i shard-eve për rrugë përcaktohet me transaksion të serializuar dhe CU të matur, jo me premtimin “64 shard-e”.

Klienti v1 duhet të vendosë shprehimisht compute-unit limit dhe loaded-account-data limit: fushat e lëna jashtë kanë default zero sipas specifikimit. Duhet kontrolluar mbështetja e wallet-it për versionin 1 dhe përdorur base64 për dërgimin e transaksioneve të mëdha. [S1, S2]

## 3. Çfarë ndryshon në matematikë dhe LP

Një CLMM i vetëm ka çmim dhe likuiditet aktiv të përbashkët. Vetëm copëtimi i tick arrays ruan këtë varësi; kodi aktual i swap-it Raydium është një shembull konkret i pool state writable. [S5]

Me shard-e të pavarura, çmimet mund të largohen nga njëri-tjetri pas swap-eve. Arbitrimi dhe router-i ndihmojnë, por nuk krijojnë një çmim global të saktë dhe të menjëhershëm pa koordinim. Router-i duhet të optimizojë çmimin, tarifat dhe rrezikun e konfliktit; shpërndarja round-robin nuk mjafton.

Derivim i ekipit për një interval aktiv CLMM, pa tarifa dhe pa kalim tick-u:

```text
s' = L*s / (L + dx*s)
dy = L*(s - s')
```

Me s=1, L=1000 dhe dx=100, dalja është 90.9091 token1. Në dy shard-e me L=500 secili, dërgimi i 100 në vetëm njërin jep 83.3333, pra 8.33% më pak dalje. Dërgimi i 50 në secilin jep gjithsej 90.9091. Barazia kërkon çmime, tarifa dhe kurba proporcionale; nuk përgjithësohet pas divergjencës ose kalimeve të ndryshme të tick-eve.

LP-të duhet të zotërojnë pozicione për shard. Një LP global me NAV plotësisht të saktë dhe të përditësuar për çdo veprim kërkon kontabilitet të përbashkët ose settlement me epoka. Ky është ndryshim ekonomik që duhet komunikuar qartë.

## 4. Likuidimi

Faza e parë duhet të përdorë rezerva të izoluara: obligation për huamarrës, reserve state/vault lokal, indeks borxhi lokal, kufi huadhënieje dhe bad-debt ledger lokal. Likuidimet në të njëjtën rezervë vazhdojnë të konkurrojnë.

Kufijtë globalë mund të ndahen në quota:

```text
sum(cap_lane) <= cap_global
used_lane <= cap_lane
```

Transferimi i një quota-je debiton burimin dhe krediton destinacionin në mënyrë atomike, ose përdor escrow të versionuar që konsumohet vetëm një herë. Nuk lejohet që dy shard-e të përdorin të njëjtin kapacitet.

Për MVP zgjedhim një çift kolateral/borxh, pa cross-margin ndërmjet lanes dhe me pozicione prove. Likuidimi verifikon freskinë dhe identitetin e oracle-it, intervalin e besimit sipas furnizuesit, borxhin e rifreskuar, gjendjen e kolateralit, close factor dhe bonusin. Ulja e borxhit duhet të lidhet me shlyerjen reale të tokenëve. Nëse ka swap, vendosen slippage/min_out, kufi çmimi dhe objektiva CPI të lejuara. Çmimi i shard-it tonë nuk shërben vetë si oracle për kolateralin e vet.

Përdorimi i reserve lane të ndryshëm ndryshon mënyrën si ndahet kapitali dhe risku. Sharding i obligations pa sharding të rezervave nuk mjafton; kodi Kamino e ilustron këtë. [S6]

## 5. Split/merge dinamik, pas MVP

Kontrolluesi jashtë zinxhirit vëzhgon ngarkesën; programi verifikon një operacion të kufizuar dhe të autorizuar. Nuk krijojmë shard për çdo kërkesë përdoruesi. Kostoja e rent-it, migrimit dhe fragmentimit duhet të justifikojë split-in.

Rruga e propozuar: Active -> Frozen/Migrating -> Retired. Çdo swap kontrollon generation dhe status. Operacionet e vjetra kundër parent-it refuzohen pas kalimit. Fëmijët nuk bëhen aktivë para se kapitali dhe detyrimet e tyre të jenë të mbuluara.

Duhet të ruhen token balances, çdo range position, fee/reward të fituara, detyrime protokolli, quota borxhi dhe dust nga rrumbullakimi. Në migrim lazy, një marker kanonik regjistron konsumimin njëherësh të të drejtës së vjetër. Nëse likuiditeti i agreguar është transferuar paraprakisht, migrimi i pozicionit nuk duhet ta shtojë përsëri.

Për pozicione arbitrare CLMM, split proporcional nuk është operacion i zgjidhur vetëm nga kjo dosje: duhet specifikim i të drejtave dhe prova e migrimit. Merge i shard-eve me çmime të ndryshme kërkon rebalance/settlement dhe vlerësim LP; nuk mjafton mbledhja e L. MVP nuk premton merge të përgjithshëm.

## 6. Ku e realizojmë

| Shtresa | Zgjedhja e propozuar |
|---|---|
| Programi | Rust + Anchor, me versione të fiksuara pas një prove kompatibiliteti; optimizim native vetëm nëse matjet e kërkojnë |
| Matematika | Crate Rust i veçantë me aritmetikë fixed-point, kontrolle overflow dhe drejtim të dokumentuar të rrumbullakimit |
| Teste instruksionesh | Mollusk/LiteSVM për correctness dhe CU; këto nuk provojnë throughput të rrjetit [S8] |
| Teste konkurrimi | Validator lokal Agave me dërgues të shumtë; konfigurim CPU dhe version runtime të regjistruar |
| Klienti/router-i | TypeScript me mbështetje të verifikuar v1; kontroll i wallet/RPC dhe simulim para dërgimit |
| Demonstrimi publik | Solana Devnet; më vonë testim infrastrukture dhe mainnet pilot vetëm pas portave të sigurisë |
| Sistemi i zhvillimit | Linux/Ubuntu ose WSL në Windows, sipas instalimit zyrtar [S9] |

Në këtë kompjuter u gjetën `rustc`, `cargo`, Node, Python dhe Git në PATH. `solana` dhe `anchor` nuk u gjetën. `wsl.exe` ekziston, por thirrja e listimit ktheu help; një distro funksionale Linux nuk u konfirmua. Nuk instaluam paketa ose shërbime dhe nuk krijuam wallet/deployment.

Sui është krahasim konceptual i dobishëm për gjendje të ndarë; vetë objektet shared që shkruhen nga shumë transaksione kanë renditje dhe contention. Nuk importojmë semantikën e pronësisë ose konsensusin e Sui-t përmes PDA-ve. [S10]

## 7. Rruga me rezultate të matshme

Këto janë vlerësime pune për një ekip me përvojë; nuk janë afat i garantuar ose ofertë financiare. Fazat mund të dështojnë dhe të ndryshojnë projektin.

1. **Specifikim dhe prova, 1–2 javë:** write sets të plota; model reference; politika LP; zgjedhja e oracle/token; përzgjedhja e versioneve; kriteri ekonomik i suksesit. Prova e parë onchain duhet të ndërtojë e simulojë një v1 real.
2. **AMM me shard-e fikse, 3–5 javë shtesë:** initialize, swap exact-in, depozitë/tërheqje LP, fees dhe router me rrugë të kufizuara. Fillojmë me një model të thjeshtë për izolimin e llogarive; implementimi CLMM kërkon tick crossing dhe accounting të plotë.
3. **Matje, 1–2 javë shtesë:** 1/2/4/8 shard-e, trafik uniform dhe trafik 80% në një shard, route me 1/2/4 shard-e, fee payer dhe vault-e të përbashkëta si kontrolle negative. Të gjitha krahasimet kanë kapital, intervale, tarifa dhe ngarkesë të njëjta.
4. **Likuidim i izoluar, 3–5 javë shtesë:** borxh/kolateral, oracle adversarial, dy likuidues në një pozicion, quota dhe bad debt.
5. **Split i kontrolluar, 3–6 javë shtesë:** generation, migrim dhe rikuperim pas ndërprerjes. Merge i përgjithshëm është kërkim më vete.
6. **Para kapitalit real:** auditim i pavarur, korrigjimet e auditimit, verifikim build-i, kufij ekspozimi dhe procedurë incidenti. Koha varet nga rezultatet dhe ekipi i jashtëm.

Ekip njerëzor i propozuar: dy inxhinierë Solana/Rust, një specialist AMM/risku, një inxhinier router/performance dhe auditues të pavarur. Rolet mund të kombinohen në kërkim; auditimi duhet të ketë pavarësi. Nuk kemi kontaktuar ekipe ose verifikuar oferta çmimesh.

## 8. Portat e pranimit

- Asnjë shkelje e ruajtjes së aktiveve/detyrimeve në model dhe fuzzing; rounding i kufizuar dhe i dokumentuar.
- Asnjë migrim/detyrim LP ose quota që mund të konsumohet dy herë.
- Differential tests kundër modelit referencë për tick crossing, fees, skaje numerike dhe likuidime.
- Teste për account substitution, aliasing/duplicate accounts, mints/programs të gabuara dhe extensions Token-2022. MVP mund të lejojë vetëm SPL Token klasik derisa extensions të mbulohen shprehimisht.
- Oracle i vjetër, devijim ekstrem, çmim zero/negativ sipas tipit të feed-it dhe burim i gabuar refuzohen.
- Regjistrohen success rate, p50/p95/p99 latency, CU, bajtë, fees, retry rate, inventory divergence, LP PnL dhe rent/migration cost.
- Vendimi ekonomik përdor daljen neto dhe riskun LP për ngarkesë të barabartë. Rritja e transaksioneve pa përmirësim ekonomik nuk mjafton.
- Nuk shpallim “zero contention”, TPS ose fitim deri në matje të riprodhueshme. Edhe throughput më i mirë duhet krahasuar me rritjen e slippage dhe fragmentimin e kapitalit.

## 9. Prova lokale e ekzekutuar

`feasibility_probe.py` përdor vetëm bibliotekën standarde Python dhe Fraction. Kontrollon pesë skenarë konflikti llogarish dhe tre pohime matematike. Ekzekutimi përfundoi me sukses dhe ruajti `probe_results.json`.

```powershell
python .\sharded_amm_research\feasibility_probe.py
```

Rezultatet: shard-e me llogari të pavarura nuk kanë konflikt në model; shard i njëjtë, vault i përbashkët, fee payer i përbashkët dhe llogari token të të njëjtit përdorues kanë konflikt. Shembulli matematikor riprodhon daljet 90.9091 dhe 83.3333.

Kjo provë nuk ekzekuton SVM, nuk mat scheduler-in, TPS, CU, vonesë ose rent dhe nuk verifikon një kontratë Rust. Është kontroll i supozimeve të projektimit.

## Burime parësore

- [S1 — Solana, aktivizimi dhe migrimi Larger Transaction Sizes](https://solana.com/upgrades/larger-transaction-sizes)
- [S2 — SIMD-0385, formati v1](https://github.com/solana-foundation/solana-improvement-documents/blob/main/proposals/0385-transaction-v1.md)
- [S3 — Solana, transaction pipeline dhe account locks](https://solana.com/docs/core/transactions/transaction-pipeline)
- [S4 — Solana Foundation, ALT dhe warmup](https://github.com/solana-foundation/developer-content/blob/main/content/courses/program-optimization/lookup-tables.md)
- [S5 — Raydium CLMM, implementimi swap](https://github.com/raydium-io/raydium-clmm/blob/master/programs/amm/src/instructions/swap.rs)
- [S6 — Kamino, liquidation account constraints](https://github.com/Kamino-Finance/klend/blob/master/programs/klend/src/handlers/handler_liquidate_obligation_and_redeem_reserve_collateral.rs)
- [S7 — Solana CPI, privilegjet e llogarive](https://solana.com/docs/core/cpi)
- [S8 — Anchor, testimi](https://www.anchor-lang.com/docs/testing)
- [S9 — Solana, instalimi lokal dhe Windows/WSL](https://solana.com/docs/intro/installation)
- [S10 — Sui, shared objects dhe kufizimet e konkurrimit](https://www.docs.sui.io/develop/objects/object-ownership/shared)

Burimet u konsultuan më 28 tetor 2026. Lidhjet në degët main/master mund të ndryshojnë; për implementim dhe auditim duhen fiksuar commit-et dhe versionet.
