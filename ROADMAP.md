# Plan pune

> **Dokument historik (para implementimit).** Ruhet si kontekst. Kur ndryshon nga kodi ose nga dokumentet normative, vlejnë [numeric-spec.md](docs/numeric-spec.md), [account-layout.md](docs/account-layout.md), [tx-budget.md](docs/tx-budget.md), [ADR-të](docs/adr/README.md) dhe [STATUS.md](STATUS.md). Indeksi: [docs/README.md](docs/README.md).

Përditësuar: 28 tetor 2026. Faza mbyllet me prova, jo vetëm me kod të shkruar. Afatet nga dosja kërkimore janë orientuese; këtu përdorim varësi dhe rezultate konkrete.

**Parametrat e miratuar nga përdoruesi:** `INITIAL_SHARDS=8`, `MAX_SHARDS=56`, `INITIAL_MASK=0xff`, `VALID_MASK=(1u64<<56)-1`. Launch nuk bëhet me 4 ose 56. Dokumenti udhëheqës është `docs/OVERVIEW_RUST_ANCHOR_8_TO_56.md`.

## Faza 0 — baza lokale

| ID | Detyra | Roli | Varet nga | Rezultati |
|---|---|---|---|---|
| P0-01 | Krijo workspace dhe kontrollin e mjedisit | Koordinator / DevEx | — | README, Cargo workspace, preflight |
| P0-02 | Specifiko shard-et dhe R/W sets | Arkitekt Solana | Kërkimi | architecture.md, account-model.md |
| P0-03 | Model reference i quote/reserve update | Specialist AMM / Rust | Kërkimi | Crate pa varësi të jashtme me teste |
| P0-04 | Threat model dhe porta testimi | Siguri | P0-02, P0-03 | security.md, acceptance.md |
| P0-05 | Ekzekuto kontrollet dhe regjistro kufizimet | Koordinator | P0-01..04 | STATUS.md me prova reale |

## Faza 1 — specifikimi që paraprin kontratën

| ID | Detyra | Varet nga | Kriteri i përfundimit |
|---|---|---|---|
| P1-01 | Konfirmo Linux/WSL dhe instalo toolchain të fiksuar Solana/Anchor | P0-05 | Versionet, build SBF dhe validator lokal funksional |
| P1-02 | Përcakto fixed-point CLMM, tick spacing, ranges dhe rounding | P0-03 | Specifikim + teste differential me precizion të lartë |
| P1-03 | Specifiko LP pozicione, fees dhe add/remove liquidity | P0-02, P1-02 | Ruajtje e aktiveve/detyrimeve për çdo veprim |
| P1-04 | Verifiko klientin v1 real | P1-01 | Transaksion i serializuar, madhësi/llogari/CU të raportuara dhe simulim me sukses |
| P1-05 | Zgjidh çiftin e testit dhe politikat token/oracle | P0-04 | Tokenë prove; mint/feed/decimals të përcaktuara; asnjë kapital real |
| P1-06 | Implemento bitmap helper-at 8→56 dhe testet | P0-02 | ID të kontrolluara, reserved bits zero, count/rank, maska me boshllëqe; pa `hash & 55` |
| P1-07 | Fikso politikën prepare/seed/activate dhe minimum depth | P1-02, P1-03, P1-05, P1-06 | Pragje në dy drejtime, status/LP/inventar aktual; TVL nuk mjafton |

**Statusi P1-01:** PARTIAL — Agave native dhe build SBF kaluan; Anchor bllokohet nga antivirus Windows dhe validator-i native dështon te genesis unpack. Pas restart-it WSL 3.0.3.0 funksionon, por regjistrimi Ubuntu dështon sepse virtualizimi firmware është False. Kërkohet aktivizimi manual BIOS VTx në HP ProBook 430 G4, pastaj Ubuntu/setup/check dhe projekti `env-anchor`. Provat janë në `docs/environment-report.json`; udhëzimi te `docs/BIOS_VIRTUALIZATION.md`. Boot Linux, Anchor/IDL dhe runtime mbeten të paverifikuara.

## Faza 2 — AMM që hapet me 8 shard-e

1. Krijo programin Anchor vetëm pasi P1-01 provon toolchain-in.
2. Implemento initialize market/registry, prepare dhe seed për ID 0..7, me state/vault-e lokale dhe validim PDA/token. Finalizo hapjen me maskë `0xff` vetëm pasi tetë shard-et kalojnë kontrollet.
3. Implemento swap exact-in dhe teste që e krahasojnë me modelin reference.
4. Implemento pozicionet LP, fees dhe tick crossing sipas specifikimit.
5. Krijo router që zgjedh pak shard-e; kontrata kontrollon min_out dhe kufijtë e çmimit.
6. Kalo testet e sigurisë për AMM me tetë shard-e nga acceptance.md; swap lexon registry dhe nuk e ndryshon maskën.

Fillimi mund të përdorë model me një interval për të provuar izolimin e llogarive; kjo nuk quhet CLMM i plotë. Faza nuk mbyllet pa mbulimin e funksioneve që pretendojmë se mbështeten.

## Faza 3 — matja e hipotezës

Launch i produktit është me 8 shard-e. Për baseline krahasojmë 1/2/4/8 në fixtures testimi, me kapital total, intervale, tarifa dhe ngarkesë të njëjta. Pastaj provojmë aktivizimin 8→9 dhe skenarë 16/32/56 pasi kalojnë testet e zgjerimit. Testojmë shpërndarje uniforme, hotspot dhe rrugë me disa shard-e. Përdorim fee payer dhe llogari përdoruesish të ndryshme; rastet me vault/payer të përbashkët janë kontrolle negative.

Rezultatet ruhen me version runtime, hardware, seed ngarkese dhe skript riprodhimi. Metrikat: success rate, retry rate, p50/p95/p99, CU, bajtë, tarifa, dalje neto, divergjencë inventari dhe rezultat LP. Modeli Python ose Rust nuk jep matje TPS.

**Vendimi vazhdo/ndrysho:** duhet përfitim i matur që justifikon fragmentimin. Pragjet numerike vendosen përpara benchmark-ut, sipas përdorimit dhe madhësive të tregtimit, jo pas shikimit të rezultateve.

## Faza 4 — zgjerimi dinamik me kapital të ri deri në 56

Implemento prepare/seed/activate për ID 8..55. `prepared_mask` dëshmon përgatitjen e llogarive, jo automatikisht financimin. Aktivizimi kontrollon gjendjen aktuale dhe vendos statusin + bitin në mënyrë atomike; kapaciteti mbetet 56. Router-i zgjedh bit aktiv/rank ose kuotën më të mirë. Keeper-i propozon zgjerim sipas minimum depth/cooldown; kontrata verifikon kushtet. Provo retry, ndërprerje, aktivizim të dyfishtë dhe tërheqje para aktivizimit. Nuk transferohen LP claims të vjetra thjesht me bit flip.

## Faza 5 — likuidimi

Specifiko fillimisht modelin e borxhit, interesit, kolateralit dhe oracle-it. Më pas ndërto obligation individuale, rezerva/vault-e për lane, quota dhe likuidim atomik. Shto teste për dy likuidues, oracle të vjetër dhe hendek çmimi. Kjo fazë nuk reduktohet në shtimin e një funksioni `liquidate` te AMM-ja.

## Faza 6 — migrimi LP, split/merge dhe kapitali real

Shtimi i shard-it të financuar nga Faza 4 nuk është split i kapitalit ekzistues. Split/migration kërkon specifikim të veçantë të pretendimeve LP, generation dhe rikuperimit. Merge me çmime të ndryshme mbetet punë kërkimore. Pas provave lokale dhe devnet vijnë auditimi i pavarur, korrigjimet, kufijtë e ekspozimit dhe procedurat e incidenteve.

## Ndarja e punës paralelisht

- Arkitekti mban modelin e llogarive; ndryshimet e tij rishikohen nga siguria.
- Specialisti AMM mban matematikën; ndryshimet ekonomike rishikohen nga arkitekti dhe siguria.
- Inxhinieri router/performance vjen pas stabilizimit të API-ve dhe prodhon benchmark-un.
- Koordinatori integron ndryshimet, mban STATUS.md dhe pengon deklarimin si “kryer” pa prova.
- Për çdo fazë, ekipet shmangin ndryshimet paralele në të njëjtët skedarë; ndërfaqet diskutohen përpara integrimit.
