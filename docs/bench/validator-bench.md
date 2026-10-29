# Benchmark-u në validator (A-10)

Data: 29 tetor 2026 (përsëritur me kapital total të barabartë dhe me router-in real të njëjtën ditë; versioni i parë i jepte çdo shard-i të njëjtin likuiditet, pra 8 shard-e kishin 8 herë më shumë kapital, dhe dha të njëjtin raport 2,5×). Statusi: **NORMATIVE** për matjen; numrat janë nga një validator lokal me një nyje, jo nga mainnet.

## Çfarë mat

I njëjti numër swap-esh të pavarur (256 tregtarë × 16 = 4 096 për çdo provë) dërgohet në një `solana-test-validator` (Agave 4.3.2) me programin real SBF. Swap-et shpërndahen mbi 1, 2, 4 ose 8 shard-e. **Kapitali total është i njëjtë në çdo konfigurim:** secili ka tregun e vet me të njëjtin likuiditet total (L = 10^15, `TOTAL_LIQUIDITY` në `bench_setup.rs`), të ndarë në mënyrë të barabartë mes shard-eve që përdor. Shard-et e tjera të atij tregu mbajnë vetëm minimumin e gatishmërisë (L = 10^9), që tregu të hapet, dhe nuk marrin swap-e. Çdo tregtar ka payer-in dhe llogaritë e veta të tokenave, prandaj të vetmet llogari writable që dy swap-e mund të ndajnë janë ato të shard-it: `ShardState`, dy vault-et dhe tick arrays.

**Me router-in real** (auditimi i paralelizmit, P-01): në vend të round-robin-it, shard-in e çdo swap-i e zgjedh `amm_router::plan`. Të gjithë klientët e një shpërthimi planifikojnë nga i njëjti snapshot i freskët, i lexuar nga validatori, dhe secili me seed-in e vet. Provohen dy mënyra: router-i determinist i mëparshëm (gjithmonë më i miri global, me id si barazues) dhe router-i me "power of two choices".

Kontrolli negativ: 8 shard-e, por i njëjti fee payer për çdo swap. Payer-i është writable në çdo transaksion, njësoj si një vault i përbashkët, prandaj swap-et konkurrojnë përsëri për një llogari.

Metrika kryesore është **swap/slot maks.**: sa swap-e të suksesshme hynë në slot-in më të ngarkuar. Për atë slot lexohet blloku dhe mblidhen CU-të që swap-et përdorën realisht.

## Rezultati (GitHub Actions, 3 përsëritje, rendi i alternuar 1→8 dhe 8→1)

**Provenance:** commit `c2cb221` (pemë e pastër në CI), Agave 4.3.2 `solana-test-validator`, runner `ubuntu-24.04`, `rounds=16`, `repeats=3`, likuiditet total për çdo treg 10^15, [CI run 40945958551](https://github.com/intopic/1/actions/runs/40945958551). Hash-i i `sharded_amm.so` dhe numri i CPU-ve janë në rreshtin e provenancës brenda artifact-it `validator-bench` të atij run-i. Programi onchain është i njëjtë me `2580214`. Tabela e mëparshme (lokale, commit `fe9f7fe`, pa hash) jepte: router 2,43×, 8 shard-e 2,50×.

| konfigurimi | swap/slot maks. (mediana) | herë më shumë se 1 shard |
|---|---:|---:|
| 1 shard | 883 | 1.00 |
| 2 shard-e | 1763 | 2.00 |
| 4 shard-e | 2124 | 2.41 |
| 8 shard-e | 2210 | 2.50 |
| 8 shard-e, router (power of two choices) | 2029 | 2.30 |
| 8 shard-e, router determinist (para ndreqjes) | 883 | 1.00 |
| 8 shard-e, payer i përbashkët (kontroll negativ) | 868 | 0.98 |

| konfigurimi | tentativa | të dërguara | të suksesshme | dështuara | pa status | swap/slot maks. | CU në slot-in maks. (M) | vonesa p50 / p95 (slot) | dërgimi (ms) |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 1 shard | 1 | 4096 | 4096 | 0 | 0 | 884 | 37.0 | 3 / 5 | 516 |
| 1 shard | 2 | 4096 | 4096 | 0 | 0 | 883 | 37.0 | 3 / 5 | 544 |
| 1 shard | 3 | 4096 | 4096 | 0 | 0 | 882 | 37.0 | 3 / 5 | 515 |
| 2 shard-e | 1 | 4096 | 4073 | 0 | 23 | 1739 | 74.1 | 2 / 3 | 415 |
| 2 shard-e | 2 | 4096 | 4096 | 0 | 0 | 1769 | 74.0 | 2 / 3 | 406 |
| 2 shard-e | 3 | 4096 | 4017 | 0 | 79 | 1763 | 74.0 | 2 / 3 | 413 |
| 4 shard-e | 1 | 4096 | 4096 | 0 | 0 | 1944 | 82.8 | 2 / 3 | 790 |
| 4 shard-e | 2 | 4096 | 4096 | 0 | 0 | 2206 | 92.6 | 2 / 3 | 563 |
| 4 shard-e | 3 | 4096 | 4096 | 0 | 0 | 2124 | 89.7 | 2 / 3 | 677 |
| 8 shard-e | 1 | 4096 | 4096 | 0 | 0 | 2191 | 92.7 | 2 / 3 | 547 |
| 8 shard-e | 2 | 4096 | 4038 | 0 | 58 | 2210 | 92.6 | 2 / 3 | 462 |
| 8 shard-e | 3 | 4096 | 4096 | 0 | 0 | 2211 | 92.6 | 2 / 3 | 762 |
| 8 shard-e, router (power of two choices) | 1 | 4096 | 4096 | 0 | 0 | 2053 | 86.0 | 3 / 4 | 713 |
| 8 shard-e, router (power of two choices) | 2 | 4096 | 4096 | 0 | 0 | 2029 | 85.2 | 2 / 3 | 549 |
| 8 shard-e, router (power of two choices) | 3 | 4096 | 4040 | 0 | 56 | 1944 | 81.7 | 2 / 4 | 445 |
| 8 shard-e, router determinist (para ndreqjes) | 1 | 4096 | 4096 | 0 | 0 | 883 | 37.0 | 4 / 6 | 493 |
| 8 shard-e, router determinist (para ndreqjes) | 2 | 4096 | 4089 | 0 | 7 | 884 | 37.0 | 4 / 6 | 421 |
| 8 shard-e, router determinist (para ndreqjes) | 3 | 4096 | 4096 | 0 | 0 | 883 | 37.0 | 4 / 6 | 459 |
| 8 shard-e, payer i përbashkët (kontroll negativ) | 1 | 4096 | 4096 | 0 | 0 | 868 | 36.4 | 3 / 6 | 443 |
| 8 shard-e, payer i përbashkët (kontroll negativ) | 2 | 4096 | 4096 | 0 | 0 | 868 | 36.4 | 4 / 6 | 494 |
| 8 shard-e, payer i përbashkët (kontroll negativ) | 3 | 4096 | 4096 | 0 | 0 | 870 | 36.4 | 4 / 6 | 442 |

Matja është **për slot**. Në mainnet slot-et janë tani 200 ms (SIMD-0525, që nga epoka 1053); validatori lokal i testit ka ritmin e vet. Numrat këtu janë swap-e për slot në kufijtë e kostos së Agave 4.3.2, jo swap-e për sekondë në mainnet.

Asnjë swap nuk dështoi. "Pa status" janë transaksione që nuk morën status brenda 40 sekondave të pritjes (`maxRetries: 0`, pa ridërgim). Rezultati i tyre është i **panjohur**: matja nuk provon që nuk u ekzekutuan, vetëm që nuk u panë. Ato nuk numërohen si sukses, kështu që swap/slot mund të jetë pak më i ulët se në realitet, por kurrë më i lartë. Tarifa e çdo transaksioni është tarifa bazë 5 000 lamports, pa priority fee. Swap-i kërkon 60 000 CU dhe përdor rreth 40 000.

## Si shpjegohet

Agave 4.3.2 kufizon CU-të për bllok dhe CU-të për çdo llogari writable brenda bllokut ([`block_cost_limits.rs`](https://github.com/anza-xyz/agave/blob/v4.3.2/cost-model/src/block_cost_limits.rs), [`slot_params.rs`](https://github.com/anza-xyz/agave/blob/v4.3.2/runtime/src/slot_params.rs)): 60M për bllok dhe 24M për llogari, ose 100M dhe 40M kur është aktiv feature-i `raise_block_limits_to_100m` (SIMD-0286). Test validator-i aktivizon të gjitha feature-t, prandaj këtu vlejnë 100M dhe 40M.

- **1 shard:** rreth 37M CU në slot, pra pishina godet kufirin për llogari. Një pishinë me një gjendje të vetme nuk mund të përdorë më shumë se 40% të bllokut, sado bosh të jetë blloku.
- **2 shard-e:** rreth 2 herë më shumë, sepse çdo shard ka kufirin e vet.
- **4 dhe 8 shard-e:** rreth 2.5 herë, 92M CU në slot. Këtu kufiri bëhet blloku, jo pishina. 4 dhe 8 shard-e japin të njëjtin numër, sepse blloku është plot.
- **Router-i determinist:** 1,00×. Me të njëjtin snapshot, të 4 096 swap-et zgjodhën shard-in 0, kështu që 8 shard-et u sollën si 1. Ky ishte gabimi P-01.
- **Router-i me power of two choices:** 2,30× (tri provat: 2053, 2029, 1944). Swap-et u shpërndanë 485–544 për shard, por jo aq njëtrajtshëm sa round-robin-i ideal (2,50×): disa shard-e marrin pak më shumë dhe mbushin kufirin e tyre më herët. Në matjen e parë lokale ishte 2,43×; diferenca tregon që rezultati lëviz rreth ±5% nga një ekzekutim te tjetri.
- **Payer i përbashkët:** bie përsëri në nivelin e 1 shard-i. Ndarja ndihmon vetëm kur payer-at dhe llogaritë e tokenave të tregtarëve janë të ndara.

Raporti 2.5× nuk varet nga kufiri që është aktiv, sepse kufiri për llogari është 40% e bllokut në të dyja rastet (24/60 dhe 40/100).

## Çfarë nuk provon

- Nuk është mainnet: s'ka gossip, rotacion leader-ash, priority fees, MEV ose trafik nga programe të tjera. Në mainnet blloku ndahet me gjithë rrjetin, prandaj kufiri i bllokut arrihet rrallë nga një pishinë e vetme. Avantazhi i matur këtu është se pishina nuk ndalet te 40% e bllokut.
- Nuk mat kohën e ekzekutimit paralel në thread-e. Mat sa swap-e pranon blloku, që varet nga kufijtë e kostos.
- Nuk mat swap-e të ndara në disa shard-e në një transaksion (shih [tx-budget.md](../tx-budget.md)) ose cilësinë e çmimit (ENG-09).
- Me kapital të njëjtë, çdo shard ka 1/n e thellësisë. Swap-et e vegjël këtu nuk e matin slippage-in, që rritet në një shard të vetëm (shih ENG-09 dhe router-in, ADR-0007).
- Nuk justifikon fjalët "100% paralel" ose "zero bllokim".

## Si riprodhohet

```
bash scripts/build-linux.sh            # ndërton target/deploy/sharded_amm.so
bash scripts/bench-validator.sh 16 3   # shkruan target/bench/validator-bench.md
```

Në GitHub: workflow-i `bench` (`.github/workflows/bench.yml`, `workflow_dispatch` me `rounds` dhe `repeats`). Kodi: `tests/svm/examples/bench_setup.rs` (ndërton tregun në LiteSVM dhe i shkruan llogaritë për `--account-dir`) dhe `tests/svm/examples/bench_run.rs` (dërgon swap-et dhe mat).
