# Benchmark-u në validator (A-10)

Data: 29 tetor 2026 (përsëritur me kapital total të barabartë dhe me router-in real të njëjtën ditë; versioni i parë i jepte çdo shard-i të njëjtin likuiditet, pra 8 shard-e kishin 8 herë më shumë kapital, dhe dha të njëjtin raport 2,5×). Statusi: **NORMATIVE** për matjen; numrat janë nga një validator lokal me një nyje, jo nga mainnet.

## Çfarë mat

I njëjti numër swap-esh të pavarur (256 tregtarë × 16 = 4 096 për çdo provë) dërgohet në një `solana-test-validator` (Agave 4.3.2) me programin real SBF. Swap-et shpërndahen mbi 1, 2, 4 ose 8 shard-e. **Kapitali total është i njëjtë në çdo konfigurim:** secili ka tregun e vet me të njëjtin likuiditet total (L = 10^18), të ndarë në mënyrë të barabartë mes shard-eve që përdor. Shard-et e tjera të atij tregu mbajnë vetëm minimumin e gatishmërisë (L = 10^9), që tregu të hapet, dhe nuk marrin swap-e. Çdo tregtar ka payer-in dhe llogaritë e veta të tokenave, prandaj të vetmet llogari writable që dy swap-e mund të ndajnë janë ato të shard-it: `ShardState`, dy vault-et dhe tick arrays.

**Me router-in real** (auditimi i paralelizmit, P-01): në vend të round-robin-it, shard-in e çdo swap-i e zgjedh `amm_router::plan`. Të gjithë klientët e një shpërthimi planifikojnë nga i njëjti snapshot i freskët, i lexuar nga validatori, dhe secili me seed-in e vet. Provohen dy mënyra: router-i determinist i mëparshëm (gjithmonë më i miri global, me id si barazues) dhe router-i me "power of two choices".

Kontrolli negativ: 8 shard-e, por i njëjti fee payer për çdo swap. Payer-i është writable në çdo transaksion, njësoj si një vault i përbashkët, prandaj swap-et konkurrojnë përsëri për një llogari.

Metrika kryesore është **swap/slot maks.**: sa swap-e të suksesshme hynë në slot-in më të ngarkuar. Për atë slot lexohet blloku dhe mblidhen CU-të që swap-et përdorën realisht.

## Rezultati (lokal, 4 CPU, 3 përsëritje, rendi i alternuar 1→8 dhe 8→1)

| konfigurimi | swap/slot maks. (mediana) | herë më shumë se 1 shard |
|---|---:|---:|
| 1 shard | 894 | 1.00 |
| 2 shard-e | 1774 | 1.98 |
| 4 shard-e | 2217 | 2.48 |
| 8 shard-e | 2238 | 2.50 |
| 8 shard-e, router (power of two choices) | 2175 | 2.43 |
| 8 shard-e, router determinist (para ndreqjes) | 896 | 1.00 |
| 8 shard-e, payer i përbashkët (kontroll negativ) | 880 | 0.98 |

| konfigurimi | tentativa | të dërguara | të suksesshme | dështuara | pa status | swap/slot maks. | CU në slot-in maks. (M) | vonesa p50 / p95 (slot) | dërgimi (ms) |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 1 shard | 1 | 4096 | 4096 | 0 | 0 | 896 | 37.0 | 3 / 5 | 460 |
| 1 shard | 2 | 4096 | 4096 | 0 | 0 | 894 | 37.0 | 3 / 6 | 283 |
| 1 shard | 3 | 4096 | 4096 | 0 | 0 | 887 | 36.7 | 4 / 6 | 273 |
| 2 shard-e | 1 | 4096 | 4096 | 0 | 0 | 1781 | 74.0 | 2 / 3 | 327 |
| 2 shard-e | 2 | 4096 | 4096 | 0 | 0 | 1755 | 72.4 | 2 / 3 | 421 |
| 2 shard-e | 3 | 4096 | 4096 | 0 | 0 | 1774 | 73.4 | 2 / 3 | 386 |
| 4 shard-e | 1 | 4096 | 4005 | 0 | 91 | 2222 | 92.6 | 2 / 3 | 397 |
| 4 shard-e | 2 | 4096 | 4065 | 0 | 31 | 2058 | 85.0 | 2 / 3 | 353 |
| 4 shard-e | 3 | 4096 | 3932 | 0 | 164 | 2217 | 91.8 | 2 / 3 | 405 |
| 8 shard-e | 1 | 4096 | 4096 | 0 | 0 | 2096 | 87.2 | 2 / 3 | 472 |
| 8 shard-e | 2 | 4096 | 4017 | 0 | 79 | 2240 | 92.5 | 2 / 3 | 342 |
| 8 shard-e | 3 | 4096 | 4096 | 0 | 0 | 2238 | 92.5 | 2 / 3 | 564 |
| 8 shard-e, router (power of two choices) | 1 | 4096 | 4096 | 0 | 0 | 2175 | 89.9 | 2 / 3 | 421 |
| 8 shard-e, router (power of two choices) | 2 | 4096 | 3719 | 0 | 377 | 2236 | 92.5 | 2 / 3 | 276 |
| 8 shard-e, router (power of two choices) | 3 | 4096 | 4096 | 0 | 0 | 2009 | 83.1 | 2 / 3 | 620 |
| 8 shard-e, router determinist (para ndreqjes) | 1 | 4096 | 4096 | 0 | 0 | 897 | 37.0 | 3 / 6 | 297 |
| 8 shard-e, router determinist (para ndreqjes) | 2 | 4096 | 4096 | 0 | 0 | 896 | 37.0 | 3 / 5 | 362 |
| 8 shard-e, router determinist (para ndreqjes) | 3 | 4096 | 3998 | 0 | 98 | 895 | 37.0 | 4 / 6 | 345 |
| 8 shard-e, payer i përbashkët (kontroll negativ) | 1 | 4096 | 3646 | 0 | 450 | 879 | 36.4 | 3 / 5 | 339 |
| 8 shard-e, payer i përbashkët (kontroll negativ) | 2 | 4096 | 4096 | 0 | 0 | 880 | 36.4 | 4 / 6 | 614 |
| 8 shard-e, payer i përbashkët (kontroll negativ) | 3 | 4096 | 4096 | 0 | 0 | 881 | 36.4 | 4 / 6 | 468 |

Asnjë swap nuk dështoi. "Pa status" janë transaksione që validatori i hodhi pa i ekzekutuar (`maxRetries: 0`, pa ridërgim); ato nuk numërohen si sukses. Tarifa e çdo transaksioni është tarifa bazë 5 000 lamports, pa priority fee. Swap-i kërkon 60 000 CU dhe përdor rreth 40 000.

## Si shpjegohet

Agave 4.3.2 kufizon CU-të për bllok dhe CU-të për çdo llogari writable brenda bllokut ([`block_cost_limits.rs`](https://github.com/anza-xyz/agave/blob/v4.3.2/cost-model/src/block_cost_limits.rs), [`slot_params.rs`](https://github.com/anza-xyz/agave/blob/v4.3.2/runtime/src/slot_params.rs)): 60M për bllok dhe 24M për llogari, ose 100M dhe 40M kur është aktiv feature-i `raise_block_limits_to_100m` (SIMD-0286). Test validator-i aktivizon të gjitha feature-t, prandaj këtu vlejnë 100M dhe 40M.

- **1 shard:** rreth 37M CU në slot, pra pishina godet kufirin për llogari. Një pishinë me një gjendje të vetme nuk mund të përdorë më shumë se 40% të bllokut, sado bosh të jetë blloku.
- **2 shard-e:** rreth 2 herë më shumë, sepse çdo shard ka kufirin e vet.
- **4 dhe 8 shard-e:** rreth 2.5 herë, 92M CU në slot. Këtu kufiri bëhet blloku, jo pishina. 4 dhe 8 shard-e japin të njëjtin numër, sepse blloku është plot.
- **Router-i determinist:** 1,00×. Me të njëjtin snapshot, të 4 096 swap-et zgjodhën shard-in 0, kështu që 8 shard-et u sollën si 1. Ky ishte gabimi P-01.
- **Router-i me power of two choices:** 2,43×. Swap-et u shpërndanë 480–550 për shard, pothuajse si round-robin-i ideal (2,50×).
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
