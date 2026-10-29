# Simulimi i tarifave: analiza (29 tetor 2026)

Rezultatet: [fee-schedule-results.md](fee-schedule-results.md), nga `cargo run --release -p amm-sim --bin eng09 -- --fees` (8 seed × 1 ditë me hapa 1-minutësh). Në CI: job-i `fees` i workflow-it `stress`. Kodi: [`crates/sim/src/fees.rs`](../../crates/sim/src/fees.rs).

## Çfarë simulohet

- **Tre tregje:** stablecoin (0,2%/ditë luhatje, interval ±1%), SOL/USDC (4%/ditë, ±20%), memecoin (20%/ditë, ±50%). Kapital $1M për treg.
- **Tarifat e propozuara:**
  - stablecoin 0,01% dhe 0,05%;
  - SOL/USDC 0,05%, 0,10%, 0,30% dhe 0,05% me tarifë dinamike deri në 0,30%;
  - memecoin 0,30%, 1% dhe 0,30% me tarifë dinamike deri në 0,80%.
- **Produkti siç është ndërtuar:** 8 shard-e me kapital të barabartë. Çdo urdhër kalon nga router-i real (`amm_router::plan`). Pas çdo minute bot-i bën një raund (`rebalance_round`, 4 çelësa, $0,01 për transaksion). Protokolli merr 16%.
- **Tre cilësime të router-it:**
  - taksë 10 bps për çdo shard shtesë (cilësimi aktual, për paralelizëm);
  - taksë 3 bps;
  - pa taksë.
- **Krahasimi:** çdo rast llogaritet edhe si 1 pool me të njëjtin kapital.

## Gjetjet

1. **Ndarja në 8 shard-e i kushton tregtarit, kur router-i ndan pak.** Me taksë 10 bps urdhri mesatar përdor 1,7 shard-e. Çdo shard ka vetëm 1/8 e thellësisë, prandaj ndikimi në çmim rritet:

   | SOL/USDC 0,05% | kosto e tregtarit | shard-e për urdhër |
   |---|---:|---:|
   | 1 pool | 21,4 bps | 1,0 |
   | 8 shard-e, taksë 10 bps | 32,9 bps (+11,5) | 1,76 |
   | 8 shard-e, taksë 3 bps | 25,5 bps (+4,1) | 2,61 |
   | 8 shard-e, pa taksë | 21,4 bps (njësoj) | 8,00 |

   Pa taksë çmimi është identik me një pool. Por çdo urdhër shkruan të 8 shard-et, kështu që paralelizmi humbet (auditimi P-02, [tx-budget.md](../tx-budget.md)). Kjo është zgjedhja kryesore: çmim më i mirë për tregtarin ose më shumë urdhra paralelë.

2. **Ku shkon kosto shtesë.** Pjesa më e madhe shkon te bot-i, pjesa tjetër te LP-të. Me SOL/USDC 0,05% dhe taksë 10 bps, bot-i fiton neto $6 636 në ditë. Kjo është më shumë se pjesa e protokollit nga tarifat ($1 710). LP-të fitojnë +0,66% kundrejt +0,44% me 1 pool. Ky fitim i bot-it vjen nga tregtarët. Në mainnet kushdo mund ta arbitrojë këtë hendek me kapitalin e vet (ADR-0006), kështu që nuk duhet llogaritur si e ardhur e sigurt.

3. **SOL/USDC.**
   - Në këtë model, LP-të fitojnë më shumë me 0,30%: +3,03% kundrejt +0,44% me 0,05% (1 pool).
   - Por modeli e mban volumin fiks. Në realitet, agregatorët ia dërgojnë urdhrin tregut më të lirë. Me 0,30%, tregtari paguan 46 bps kundrejt 21 bps, dhe volumi do të shkonte te Raydium ose Orca.
   - **0,05% me tarifë dinamike** jep +1,09% për LP-të (2,5 herë më shumë se fikse), me vetëm +3,3 bps për tregtarin (24,8 kundrejt 21,4).

4. **Memecoin.**
   - Me 0,30% fikse LP-të humbin mesatarisht (−0,6% në ditë, me shpërndarje shumë të gjerë).
   - Me 1%, LP-të fitojnë +4,5%.
   - 0,30% me tarifë dinamike del afër zeros (+0,2%), sepse kufiri 0,80% nuk mjafton për 20% luhatje në ditë.
   - Rreth 570 urdhra në ditë refuzohen, sepse çmimi del jashtë intervalit ±50%.

5. **Stablecoin.** 0,01% është e mjaftueshme. Kosto e tregtarit është 3,5 bps me 1 pool (6,8 me taksë 3 bps), dhe LP-të fitojnë.

## Rekomandimi

| Tregu | Tarifa | Pse |
|---|---|---|
| Stablecoin | 0,01% fikse | Konkurruese; LP-të fitojnë me volum të lartë. |
| SOL/USDC | 0,05% + dinamike deri në 0,30% | Çmim konkurrues kur tregu është i qetë; mbron LP-të kur lëviz. |
| Memecoin | 1% fikse | E vetmja nga të provuarat ku LP-të nuk humbin mesatarisht. |

**Router-i:** taksa 3 bps e ul koston shtesë të tregtarit nga +11,5 në +4,1 bps (SOL/USDC 0,05%), por urdhri përdor 2,6 shard-e në vend të 1,8. Kjo duhet matur në validator para se ta ndryshojmë: benchmark-u i sotëm mat vetëm urdhra të vegjël që shkojnë në 1 shard.

## Kufijtë e modelit

- Volumi është fiks, nuk reagon ndaj tarifës. Kjo e favorizon tarifën e lartë.
- Arbitrazhi nuk paguan gas, dhe nuk ka konkurrent për bot-in.
- Ka një pozicion LP për shard dhe asnjë MEV ose JIT.
- Qarkullimi është ~10 herë kapitali në ditë. Prandaj numrat absolutë "%/ditë" janë shumë më të lartë se në tregje reale; vetëm krahasimet mes rreshtave kanë kuptim.
