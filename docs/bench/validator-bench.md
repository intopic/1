# Benchmark-u në validator (A-10)

Data: 29 tetor 2026. Statusi: **NORMATIVE** për matjen; numrat janë nga një validator lokal me një nyje, jo nga mainnet.

## Çfarë mat

I njëjti numër swap-esh të pavarur (256 tregtarë × 16 = 4 096 për çdo provë) dërgohet në një `solana-test-validator` (Agave 4.3.2) me programin real SBF. Swap-et shpërndahen mbi 1, 2, 4 ose 8 shard-e. Çdo tregtar ka payer-in dhe llogaritë e veta të tokenave, prandaj të vetmet llogari writable që dy swap-e mund të ndajnë janë ato të shard-it: `ShardState`, dy vault-et dhe tick arrays.

Kontrolli negativ: 8 shard-e, por i njëjti fee payer për çdo swap. Payer-i është writable në çdo transaksion, njësoj si një vault i përbashkët, prandaj swap-et konkurrojnë përsëri për një llogari.

Metrika kryesore është **swap/slot maks.**: sa swap-e të suksesshme hynë në slot-in më të ngarkuar. Për atë slot lexohet blloku dhe mblidhen CU-të që swap-et përdorën realisht.

## Rezultati (lokal, 4 CPU, 3 përsëritje, rendi i alternuar 1→8 dhe 8→1)

| konfigurimi | swap/slot maks. (mediana) | herë më shumë se 1 shard |
|---|---:|---:|
| 1 shard | 923 | 1.00 |
| 2 shard-e | 1773 | 1.92 |
| 4 shard-e | 2307 | 2.50 |
| 8 shard-e | 2309 | 2.50 |
| 8 shard-e, payer i përbashkët (kontroll negativ) | 907 | 0.98 |

| konfigurimi | tentativa | të dërguara | të suksesshme | dështuara | pa status | swap/slot maks. | CU në slot-in maks. (M) | vonesa p50 / p95 (slot) | dërgimi (ms) |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 1 shard | 1 | 4096 | 4096 | 0 | 0 | 924 | 36.9 | 3 / 5 | 369 |
| 1 shard | 2 | 4096 | 3743 | 0 | 353 | 921 | 36.9 | 3 / 5 | 244 |
| 1 shard | 3 | 4096 | 4075 | 0 | 21 | 923 | 36.9 | 3 / 5 | 279 |
| 2 shard-e | 1 | 4096 | 4096 | 0 | 0 | 1839 | 73.8 | 2 / 3 | 346 |
| 2 shard-e | 2 | 4096 | 3995 | 0 | 101 | 1773 | 70.9 | 2 / 3 | 705 |
| 2 shard-e | 3 | 4096 | 4096 | 0 | 0 | 1653 | 66.2 | 2 / 3 | 432 |
| 4 shard-e | 1 | 4096 | 4096 | 0 | 0 | 2307 | 92.4 | 2 / 3 | 454 |
| 4 shard-e | 2 | 4096 | 4061 | 0 | 35 | 2306 | 92.4 | 2 / 2 | 441 |
| 4 shard-e | 3 | 4096 | 4096 | 0 | 0 | 2307 | 92.3 | 2 / 3 | 407 |
| 8 shard-e | 1 | 4096 | 4096 | 0 | 0 | 2309 | 92.3 | 2 / 3 | 392 |
| 8 shard-e | 2 | 4096 | 4096 | 0 | 0 | 2311 | 92.3 | 2 / 3 | 419 |
| 8 shard-e | 3 | 4096 | 3833 | 0 | 263 | 2308 | 92.3 | 2 / 3 | 353 |
| 8 shard-e, payer i përbashkët (kontroll negativ) | 1 | 4096 | 4096 | 0 | 0 | 906 | 36.3 | 4 / 6 | 349 |
| 8 shard-e, payer i përbashkët (kontroll negativ) | 2 | 4096 | 4071 | 0 | 25 | 907 | 36.3 | 4 / 6 | 411 |
| 8 shard-e, payer i përbashkët (kontroll negativ) | 3 | 4096 | 4096 | 0 | 0 | 907 | 36.3 | 4 / 6 | 405 |

Asnjë swap nuk dështoi. "Pa status" janë transaksione që validatori i hodhi pa i ekzekutuar (`maxRetries: 0`, pa ridërgim); ato nuk numërohen si sukses. Tarifa e çdo transaksioni është tarifa bazë 5 000 lamports, pa priority fee. Swap-i kërkon 60 000 CU dhe përdor rreth 40 000.

## Si shpjegohet

Agave 4.3.2 kufizon CU-të për bllok dhe CU-të për çdo llogari writable brenda bllokut ([`block_cost_limits.rs`](https://github.com/anza-xyz/agave/blob/v4.3.2/cost-model/src/block_cost_limits.rs), [`slot_params.rs`](https://github.com/anza-xyz/agave/blob/v4.3.2/runtime/src/slot_params.rs)): 60M për bllok dhe 24M për llogari, ose 100M dhe 40M kur është aktiv feature-i `raise_block_limits_to_100m` (SIMD-0286). Test validator-i aktivizon të gjitha feature-t, prandaj këtu vlejnë 100M dhe 40M.

- **1 shard:** rreth 37M CU në slot, pra pishina godet kufirin për llogari. Një pishinë me një gjendje të vetme nuk mund të përdorë më shumë se 40% të bllokut, sado bosh të jetë blloku.
- **2 shard-e:** rreth 1.9 herë më shumë, sepse çdo shard ka kufirin e vet.
- **4 dhe 8 shard-e:** rreth 2.5 herë, 92M CU në slot. Këtu kufiri bëhet blloku, jo pishina. 4 dhe 8 shard-e japin të njëjtin numër, sepse blloku është plot.
- **Payer i përbashkët:** bie përsëri në nivelin e 1 shard-i. Ndarja ndihmon vetëm kur payer-at dhe llogaritë e tokenave të tregtarëve janë të ndara.

Raporti 2.5× nuk varet nga kufiri që është aktiv, sepse kufiri për llogari është 40% e bllokut në të dyja rastet (24/60 dhe 40/100).

## Çfarë nuk provon

- Nuk është mainnet: s'ka gossip, rotacion leader-ash, priority fees, MEV ose trafik nga programe të tjera. Në mainnet blloku ndahet me gjithë rrjetin, prandaj kufiri i bllokut arrihet rrallë nga një pishinë e vetme. Avantazhi i matur këtu është se pishina nuk ndalet te 40% e bllokut.
- Nuk mat kohën e ekzekutimit paralel në thread-e. Mat sa swap-e pranon blloku, që varet nga kufijtë e kostos.
- Nuk mat swap-e të ndara në disa shard-e në një transaksion (shih [tx-budget.md](../tx-budget.md)) ose cilësinë e çmimit (ENG-09).
- Nuk justifikon fjalët "100% paralel" ose "zero bllokim".

## Si riprodhohet

```
bash scripts/build-linux.sh            # ndërton target/deploy/sharded_amm.so
bash scripts/bench-validator.sh 16 3   # shkruan target/bench/validator-bench.md
```

Në GitHub: workflow-i `bench` (`.github/workflows/bench.yml`, `workflow_dispatch` me `rounds` dhe `repeats`). Kodi: `tests/svm/examples/bench_setup.rs` (ndërton tregun në LiteSVM dhe i shkruan llogaritë për `--account-dir`) dhe `tests/svm/examples/bench_run.rs` (dërgon swap-et dhe mat).
