# Bot-i i rebalancimit kundër validatorit lokal

Bot-i jashtë zinxhirit për `rebalance_pair` (ADR-0006): [`tests/svm/examples/rebalancer.rs`](../../tests/svm/examples/rebalancer.rs), planifikimi në `amm_router::rebalance_round`. Programi onchain nuk ndryshon. Ekzekutohet nga [`scripts/rebalancer-validator.sh`](../../scripts/rebalancer-validator.sh) dhe workflow-i `rebalancer` në CI.

## Si punon

Çdo raund:

1. **Snapshot.** Tregu, 8 shard-et dhe tick arrays rreth çmimit të secilit lexohen me një `getMultipleAccounts`, pra nga i njëjti slot.
2. **Plani.** `rebalance_round` llogarit fitimin për çdo çift shard-esh (28 me 8 shard-e) dhe zgjedh saktësisht grupin me deri në 4 çifte pa asnjë shard të përbashkët me fitimin total më të madh (maximum weight matching, me kërkim branch and bound). Propozim i gerfa-s, 29 tetor 2026. Krahasuar me versionin e parë (hendeku më i gjerë, pastaj më i gjeri mes shard-eve që mbetën), në 40 botë të rastit me thellësi të ndryshme: fitim total më i madh se 1% në 25 nga 40 botë, mediana +2,4%, maksimumi +21,9% (testi host `a_round_is_the_best_set_of_disjoint_pairs`, i krahasuar edhe me forcë brutale mbi të gjitha kombinimet). Koha për raund: ~16 ms me 8 shard-e; me më shumë se 16 shard-e llogariten vetëm 8 më të lirët dhe 8 më të shtrenjtët (~64 ms me 56), sepse të 1 540 çiftet marrin ~0,9 s. Çdo shard ka state, vault-e dhe tick arrays të vetat, kështu që transaksionet nuk shkruajnë asnjë llogari të përbashkët dhe mund të zbresin në të njëjtin slot.
3. **Pragu i fitimit.** Një plan dërgohet vetëm nëse fitimi në tokenin e vet është të paktën sa kostoja e një transaksioni në atë token (`--min-a`, `--min-b`). Onchain `min_profit` vendoset 90% e fitimit të parashikuar, por kurrë nën pragun. Nëse çmimet lëvizin ndërkohë, transaksioni dështon në vend që të paguajë më pak se kostoja.
4. **Dërgimi.** Plani `k` nënshkruhet dhe paguhet nga çelësi `k`, një nga 4 rebalancer-at e listuar. Asnjë payer i përbashkët, që do t'i serializonte (shih kontrollin negativ në [validator-bench.md](validator-bench.md)).
5. **Pas dërgimit.** `recovery::after_send`: dështimi dekodohet në gabimin e programit dhe klasifikohet. Transaksioni që nuk shihet ridërgohet me të njëjtat bajte sa kohë blockhash-i është i vlefshëm, dhe braktiset vetëm kur ka skaduar dhe historiku i plotë tregon që nuk ka ekzekutuar.

Ndryshime në router: `find_rebalance_min` ka prag të veçantë për fitimin në A dhe në B. Kur të dy drejtimet e kalojnë pragun, zgjidhet ai me vlerë më të madhe (A vlerësohet në B me mesataren gjeometrike të dy çmimeve). Më parë krahasoheshin numrat bruto të dy tokenëve. `RebalanceUnprofitable` dhe `RebalanceNoPriceGap` tani klasifikohen `Refresh`: për një plan që paguante në snapshot, kjo do të thotë vetëm që çmimet lëvizën.

Pragjet në test janë në njësi të tokenëve të testit. Në një treg real vendosen nga tarifa që paguan bot-i (5 000 lamports për nënshkrim plus priority fee) shumëzuar me çmimin e tokenit në lamports, plus një marzh.

## Testi

Tregu me 8 shard-e nga `bench_setup`, me 4 çelësa rebalancer të listuar (çdo çelës paguan tarifat e veta dhe nuk ka asnjë token), tarifë 0,30%.

- **Faza e qetë.** Tetë trader-a shtyjnë shard-et: çiftet poshtë, tekët lart, me sasi të ndryshme. Pastaj vepron vetëm bot-i. Kërkohet: çdo rebalancim zbret, të paktën një raund përdor dy ose më shumë çelësa, dhe hendeku bie nën brezin e tarifave + 5 bps.
- **Faza me zhurmë.** Trader-at bëjnë swap-e në shard-e të rastit ndërsa bot-i punon, kështu që disa snapshot-e vjetrohen. Kërkohet: çdo dështim klasifikohet `Refresh` (asnjë gabim tjetër), balancat e rebalancer-ave nuk zbresin kurrë, dhe pasi ndalet zhurma hendeku kthehet nën brez.

**Brezi i tarifave.** Një rebalancim paguan vetëm kur hendeku është më i gjerë se dy tarifat (2 × 30 = 60 bps). Brenda brezit asnjë rebalancim nuk ka fitim, prandaj hendeku mbetet rreth 60 bps. Kjo nuk është defekt: është kufiri ekonomik. Me tarifë 0,05% brezi do të ishte 10 bps.

## Rezultati (GitHub Actions)

**Provenance:** commit `cc0cdab` (me matching-un), Agave 4.3.2 `solana-test-validator`, runner `ubuntu-24.04`, 200 swap-e zhurme, [CI run 40948357479](https://github.com/intopic/1/actions/runs/40948357479). Hash-i i `sharded_amm.so` është në rreshtin e provenancës brenda artifact-it `rebalancer` të atij run-i.

Floors: 5000 A, 5000 B per transaction. Four rebalancer keys, each its own fee payer. Noise: 200 swaps.

| faza | raundi | hendeku para (bps) | pas (bps) | dërguar | zbritën | dështuan | ridërguar | skaduan | slot-e | maks. në 1 slot | fitimi A | fitimi B | CU maks./tx | tarifa (lamports) |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| calm | 1 | 179.8 | 62.0 | 4 | 4 | 0 | 0 | 0 | 1 | 4 | 3359226146 | 1025896603 | 74016 | 20000 |
| calm | 2 | 62.0 | 60.3 | 2 | 2 | 0 | 0 | 0 | 1 | 2 | 0 | 540774 | 73072 | 10000 |
| noisy | 3 | 68.1 | 77.1 | 1 | 0 | 1 | 0 | 0 | 1 | 0 | 0 | 0 | 65959 | 5000 |
| noisy | 4 | 77.1 | 77.5 | 1 | 0 | 1 | 0 | 0 | 1 | 0 | 0 | 0 | 64905 | 5000 |
| noisy | 5 | 77.5 | 66.9 | 2 | 2 | 0 | 0 | 0 | 1 | 2 | 5194590 | 48232365 | 73482 | 10000 |
| noisy | 6 | 66.9 | 65.9 | 1 | 1 | 0 | 0 | 0 | 1 | 1 | 0 | 12018193 | 74020 | 5000 |
| noisy | 7 | 65.9 | 82.3 | 2 | 0 | 2 | 0 | 0 | 1 | 0 | 0 | 0 | 65480 | 10000 |
| noisy | 8 | 82.3 | 74.1 | 1 | 1 | 0 | 0 | 0 | 1 | 1 | 71415803 | 0 | 73948 | 5000 |
| noisy | 9 | 74.1 | 79.4 | 3 | 2 | 1 | 0 | 0 | 1 | 2 | 0 | 163778356 | 73995 | 15000 |
| noisy | 10 | 79.4 | 84.3 | 1 | 1 | 0 | 0 | 0 | 1 | 1 | 0 | 59276593 | 73675 | 5000 |
| noisy | 11 | 84.3 | 93.9 | 2 | 2 | 0 | 0 | 0 | 1 | 2 | 0 | 93633931 | 73493 | 10000 |
| noisy | 12 | 93.9 | 71.3 | 3 | 3 | 0 | 0 | 0 | 1 | 3 | 15249510 | 217600873 | 72988 | 15000 |
| noisy | 13 | 71.3 | 66.3 | 2 | 1 | 1 | 0 | 0 | 1 | 1 | 0 | 19867056 | 73522 | 10000 |
| noisy | 14 | 66.3 | 71.7 | 1 | 1 | 0 | 0 | 0 | 1 | 1 | 5395286 | 0 | 72980 | 5000 |
| noisy | 15 | 71.7 | 77.0 | 2 | 1 | 1 | 0 | 0 | 1 | 1 | 869124 | 0 | 72699 | 10000 |
| noisy | 16 | 77.0 | 78.1 | 2 | 1 | 1 | 0 | 0 | 1 | 1 | 1521908 | 0 | 72937 | 10000 |
| noisy | 17 | 78.1 | 105.1 | 1 | 1 | 0 | 0 | 0 | 1 | 1 | 0 | 51191800 | 73589 | 5000 |
| noisy | 18 | 105.1 | 88.9 | 2 | 2 | 0 | 0 | 0 | 1 | 2 | 352696265 | 0 | 73496 | 10000 |
| noisy | 19 | 88.9 | 80.0 | 2 | 2 | 0 | 0 | 0 | 1 | 2 | 120660443 | 3098467 | 73054 | 10000 |
| noisy | 20 | 80.0 | 68.2 | 2 | 2 | 0 | 0 | 0 | 1 | 2 | 57139406 | 14758687 | 73465 | 10000 |
| noisy | 21 | 68.2 | 98.6 | 1 | 1 | 0 | 0 | 0 | 1 | 1 | 0 | 10075420 | 73165 | 5000 |
| noisy | 22 | 98.6 | 73.4 | 1 | 1 | 0 | 0 | 0 | 1 | 1 | 0 | 237345444 | 72447 | 5000 |
| noisy | 23 | 73.4 | 86.2 | 1 | 1 | 0 | 0 | 0 | 1 | 1 | 0 | 91589040 | 73807 | 5000 |
| noisy | 24 | 86.2 | 88.4 | 1 | 1 | 0 | 0 | 0 | 1 | 1 | 0 | 108879958 | 73210 | 5000 |
| noisy | 25 | 88.4 | 69.7 | 1 | 1 | 0 | 0 | 0 | 1 | 1 | 115922502 | 0 | 73426 | 5000 |
| noisy | 26 | 69.7 | 53.3 | 1 | 1 | 0 | 0 | 0 | 1 | 1 | 0 | 14331579 | 73464 | 5000 |

| dështimi → klasa | herë |
|---|---:|
| RebalanceUnprofitable → Refresh | 8 |

Hendeku: 179.8 bps pas shtyrjes, 60.3 bps pas fazës së qetë, 53.3 bps në fund. Brezi i tarifave (dy tarifa) është 60.0 bps: brenda tij asnjë rebalancim nuk paguan.

**Lexo kështu.** Në këtë skenar (gjysma poshtë, gjysma lart) matching-u dhe versioni i parë zgjedhin të njëjtat çifte. Në raundin e parë 4 rebalancime në 4 çifte të ndara zbritën në **të njëjtin slot**. Gjatë zhurmës, dështimet (`RebalanceUnprofitable`) ndodhin kur një swap i trader-it zbret para rebalancimit: transaksioni dështon, bot-i humb vetëm tarifën e transaksionit (5 000 lamports), jo tokena. CU për rebalancim: rreth 73k (me limit 300k); një rebalancim që dështon kushton ~65k CU dhe 5 000 lamports. Gjatë zhurmës, 8 nga 37 transaksione dështuan, të gjitha `RebalanceUnprofitable → Refresh`; asnjë humbje tokenash.

## Kufijtë

Validator me një nyje në një makinë: pa gossip, pa rotacion lideri, pa bot-e konkurrente, pa treg priority fee. Nuk mat fitimin në mainnet. Kushdo mund ta mbyllë ende hendekun me dy swap-e të zakonshëm dhe kapitalin e vet (ADR-0006), kështu që në mainnet bot-i ka konkurrencë.
