# ADR-0004 — Konfigurimi eksperimental i produktit (handoff §16)

**Statusi:** ACCEPTED si konfigurim **eksperimental** (jo parametra mainnet) · **Data:** 2026-10-28 · **Vendosi:** gerfa

## Vendimet e pranuara për prototipin

| Tema | Vendimi | Kushti / çfarë mbetet për t'u provuar |
|---|---|---|
| Shard-et | 8 aktive në nisje, kapacitet 56 | Pyetja vendimtare: sa likuiditet ka secili shard. Çifti popullor nuk kompenson shard-e të cekëta. |
| Çifti | Kandidat **WSOL/USDC**. SOL trajtohet vetëm si Wrapped SOL (SPL standard). | Testet nisin me **dy mint-e prove me decimals të ndryshme** (p.sh. 9 dhe 6), pastaj testohet sjellja specifike e WSOL (wrap/unwrap, ATA native). |
| Token program | Vetëm SPL Token klasik; Token-2022 refuzohet në v1 | — |
| Tarifa | Parametër për treg, **jo e fiksuar**. Kandidatët **0,05% / 0,10% / 0,30%** (500 / 1 000 / 3 000 ppm) | Zgjidhet pas krahasimit me kapital dhe ngarkesë të njëjtë, sipas daljes neto për tregtarin dhe rezultatit për LP-të, jo sipas të ardhurave të protokollit. Fragmentimi në 8 shard-e mund ta rëndojë kuotën. |
| Ndarja e tarifës | **84% LP / 16% protokoll** (`protocol_share_bps = 1600`), parametër. Vendim i gerfa-s, 29 tetor 2026 (ishte 80/20), që LP-të të mos marrin më pak se te Raydium CLMM (~16% protokolli) ose Orca (~13%); shifrat e tyre verifikohen para çdo krahasimi publik. Vendoset në krijimin e tregut; në një treg ekzistues ndryshohet me `update_fees` (timelock 24 orë). Testet përdorin ende 2000 si parametër testi. | Shembull: 1 000 USDC → tarifë 3, LP 2,52, protokolli 0,48 (me 0,30%). Me 0,05%: 1 000 USDC → tarifë 0,50, LP 0,42, protokolli 0,08. |
| Tarifat e protokollit | Akumulohen lokalisht në çdo `ShardState`; treasury mblidhet me `collect_protocol_fees` të veçantë | Treasury nuk është kurrë llogari writable në swap. |
| `tick_spacing` | Kandidatët **10 dhe 60**, krahasim | `1.0001^60 − 1 ≈ 0,6018%` është largësia ndërmjet kufijve të lejuar të pozicioneve LP, jo hapi i çmimit gjatë swap-it (çmimi lëviz vazhdimisht brenda tyre). CU varet nga ticks e inicializuara, crossing-et dhe layout-i, jo vetëm nga spacing-u. **I pandryshueshëm** pasi tregu ka pozicione LP, përveç një migrimi të projektuar posaçërisht. |
| Rolet | Katër role të ndara, secili me kufij në handler: | |
| · `admin` | Parametra vetëm brenda bounds; çdo ndryshim lëshon event; vonesë për ndryshime të ndjeshme | Vonesa (timelock) specifikohet në ENG-03/10. |
| · `keeper` | Prepare/activate vetëm kur kontrata provon financimin dhe readiness | Pa transferim principal-i. |
| · `revenue` | Tërheq vetëm protocol fees të llogaritura, te marrësit e autorizuar | — |
| · `pauser` | Ndal operacionet e rrezikshme (swap, add liquidity, prepare/activate) | Daljet e vlefshme LP ruhen (ADR-0002). |
| Upgrade authority | E veçantë nga katër rolet; multisig + vonesë para kapitalit real | **Kufizim i deklaruar:** handler-at i ndalojnë rolet të marrin principal LP, por upgrade authority mund ta zëvendësojë kodin dhe t'i ndryshojë këto garanci. Multisig dhe vonesa e kontrollojnë këtë pushtet, nuk e eliminojnë. |

## Matrica e krahasimit (ENG-09, në GitHub Actions)

Tarifat {0,05%, 0,10%, 0,30%} × spacing {10, 60}, me kapital total, shpërndarje range-sh dhe ngarkesë të njëjtë. Matet: dalja neto e tregtarit për madhësi standarde, rezultati LP (tarifa minus humbja nga lëvizja e çmimit), CU dhe tick crossings. Raporti përfshin edhe rastet ku një konfigurim del më keq.

## Ende të hapura

| Vendimi | Propozimi | Bllokon |
|---|---|---|
| Mint me freeze authority | Lejohet me deklarim publik të rrezikut (USDC e ka). Kërkimi i ekipit propozon profil strikt që i refuzon; pret vendimin e pronarit. | ENG-03 |
| WSOL | `SyncNative`, rezerva e rent-it native, wrap/unwrap në SDK; teste pas mint-eve prove | ENG-03/07 |
| Kufiri i llogarive | 64 llogari të kyçura për transaksion (ALT nuk e rrit; zgjidh vetëm madhësinë e paketës). 8 shard-e × 3 tick arrays + ComputeBudget = 64. Matjet: [tx-budget.md](../tx-budget.md) | ENG-07/09 |
| Çmimi/range fillestar i 8 shard-eve | I njëjti çmim fillestar, range i njëjtë, kapital i barabartë | ENG-04 |
| Çmimi i shard-it të ri | Çmim nga financuesi brenda tolerancës ndaj medianës së shard-eve aktive | ENG-08 |
| Pragu i readiness | L aktive ≥ X dhe inventar real ≥ Y në njësi raw, të dy drejtimet | ENG-04/08 |
| Cooldown aktivizimi | Pa cooldown në v1, vetëm kontrolle aktuale | ENG-08 |
| Pauzë selektive | Vetëm pauzë tregu në v1; `paused_mask` më vonë | ENG-03 |
| Kufiri i tick arrays për swap | ≤ 3 vargje (`MAX_SWAP_TICK_ARRAYS`), `MAX_SWAP_STEPS = 128`. Propozimi fillestar 64 u ngrit në 128: 3 vargje kanë deri në 96 ticks të inicializuar, kështu që një swap i ligjshëm mund të kërkojë ~100 hapa (96 kalime + kufijtë e vargjeve + hapi final); me 64 ai do të dështonte me `TooManySwapSteps` para se të mbaronin vargjet. CU: ~9k për kalim, pra 96 kalime ≈ 0.9M, brenda 1.4M. Vlera normative është `constants.rs` | ENG-06 |
| Rebalance | Jashtë MVP; vetëm `rebalance_pair` i kufizuar pasi të ketë model ekonomik që nuk prek principal LP | ENG-09 |

Kur një vendim merret, rreshti kalon te tabela e parë me datën dhe kush e mori.
