# Specifikimi numerik — Q64.64, ticks, likuiditet dhe tarifa

**Statusi: NORMATIVE.** Ky dokument dhe [account-layout.md](account-layout.md) janë burimi i vetëm normativ për matematikën dhe ABI-në; kur një dokument tjetër (handoff, overview, research) ndryshon prej tyre, vlen ky. Formulat e §2–§7 janë implementuar në `programs/sharded-amm/src/math/` dhe `src/engine/`, thirren nga instruksionet onchain dhe testohen bit-për-bit kundrejt referencës së pavarur [`tools/reference/clmm_vectors.py`](../tools/reference/clmm_vectors.py). Vendimi formal është [ADR-0001](adr/0001-numeric-domain.md).

## 1. Llojet dhe njësitë

| Madhësia | Lloji | Njësia / kuptimi | Domain-i |
|---|---|---|---|
| `tick` | `i32` | `P = 1.0001^tick`, ku `P` = token B për 1 token A, në njësi bazë (raw, pa decimals) | `MIN_TICK = -443636 ≤ tick ≤ MAX_TICK = 443636` |
| `sqrt_price_x64` | `u128` | `√P · 2^64` (Q64.64) | `MIN_SQRT_PRICE_X64 ≤ s ≤ MAX_SQRT_PRICE_X64`, gjithmonë `< 2^96` |
| `liquidity` (L) | `u128` | njësi `√(A·B)` | aktive `≤ u128::MAX`; për tick shih §4 |
| shumat e tokenëve | `u64` | njësi bazë të mint-it | `0 ≤ a ≤ u64::MAX`; çdo rezultat që s'futet në `u64` refuzohet |
| `fee_growth_*_x128` | U256 (`[u64; 4]`) | tarifë LP për njësi L, Q128 | global dhe outside **checked, pa wrap**; inside modulare (§6) |
| `fee_rate_ppm` | `u32` | pjesë e input-it, emërues `1_000_000` | `0 ≤ r ≤ 100_000` (10%) |
| `protocol_share_bps` | `u16` | pjesë e tarifës totale, emërues `10_000` | `0 ≤ p ≤ 5_000` |

Token A dhe B janë mint-et në renditje kanonike (`mint_a < mint_b` sipas bajtëve). Matematika punon vetëm me shuma raw; decimals përdoren vetëm për pragje operative (§9).

**Q96 nuk përdoret.** Asnjë konstante e kopjuar nga implementime Q64.96 (p.sh. Uniswap v3) nuk lejohet. Raydium përdor gjithashtu Q64.64; konstantet tona janë rillogaritur në mënyrë të pavarur (§2) dhe nuk merren nga kodi i tyre.

## 2. Domain-i tick ↔ çmim

`MAX_TICK = 443636 = floor(log_{1.0001}(2^64))`, pra `P ∈ [2^-64, 2^64]` dhe `√P · 2^64 ∈ [2^32, 2^96]`. Vlerat e sakta, nga referenca e pavarur me 120 shifra:

```text
MIN_SQRT_PRICE_X64 (floor, tick = -443636) = 4295048016
MAX_SQRT_PRICE_X64 (floor, tick =  443636) = 79226673515401279992447579061   (96 bit; ceil = ...062)
```

Burimi: [`tools/reference/tick_vectors.py`](../tools/reference/tick_vectors.py) (vetëm stdlib Python `decimal`), dalja në [`docs/vectors/tick_vectors.json`](vectors/tick_vectors.json) me 47 vektorë (kufijtë, ±1, ±60, ±100000 dhe fuqitë e 2). Komanda:

```bash
python3 -I tools/reference/tick_vectors.py > docs/vectors/tick_vectors.json
```

`sqrt_price_at_tick` shumëzon mantisa 128-bit të normalizuara të `√(1.0001^(±2^i))`. Ato vijnë nga dy tabela të gjeneruara nga [`tools/reference/tick_table.py`](../tools/reference/tick_table.py), me rrënjë katrore të plotë ekzakte, `M ∈ [2^127, 2^128)` dhe eksponent `E`. Llogaritja bëhet vetëm me `u128`, pa U512 dhe pa pjesëtim, dhe në fund zhvendoset në Q64.64. Kontrata është që rezultati të jetë **floor-i ekzakt** `floor(√(1.0001^t)·2^64)`, e verifikuar për **të gjitha 887 273 ticks** e domain-it. Referenca e pavarur është [`tools/reference/all_tick_prices.py`](../tools/reference/all_tick_prices.py), me fallback me rrënjë ekzakte pranë numrave të plotë. Testi është `full_tick_table` dhe ekzekutohet në CI. `tick_at_sqrt_price` llogarit fillimisht një `log2` me pikë fikse (20 bit thyesë). Ky vlerësim bie brenda ±1 tick, gjë që testohet për çdo të 5-tin tick. Rezultati i saktë vendoset pastaj me funksionin përpara, kështu që përgjigjja është ekzakte pavarësisht gabimit të vlerësimit. Konstantet `MIN/MAX_SQRT_PRICE_X64` janë dalja e funksionit në kufij.

`get_tick_at_sqrt_price(s)` kthen tick-un më të madh `t` me `sqrt_price_at(t) ≤ s`. Kjo është konventa e kufirit në barazi:

- Një pozicion `[lower, upper)` është aktiv kur `lower ≤ current_tick < upper`.
- Kur një swap A→B (çmimi zbret) arrin saktësisht `sqrt_price_at(t)` të një tick-u të inicializuar, tick-u kalohet dhe `current_tick = t − 1`.
- Kur një swap B→A (çmimi ngjitet) arrin saktësisht `sqrt_price_at(t)`, tick-u kalohet dhe `current_tick = t`.

`tick_spacing ∈ [1, 1000]`; ticks e pozicioneve duhet të jenë shumëfisha të spacing-ut dhe brenda domain-it.

## 3. Formulat e shumave dhe rrumbullakimi

Rregulli i përgjithshëm: **çdo rrumbullakim favorizon pool-in.** Input-i që kërkohet nga përdoruesi rrumbullakoset lart; output-i që i paguhet rrumbullakoset poshtë; çmimi pas një hapi rrumbullakoset në drejtimin që jep më pak output.

Me `sa < sb` (sqrt price X64) dhe `L`:

| Formula | Përkufizim real | Rrumbullakimi | Gjerësia max e produktit | Ndërmjetësi |
|---|---|---|---|---|
| `Δa(L, sa, sb)` | `L · (sb − sa) · 2^64 / (sa · sb)` | lart për input, poshtë për output | `L·2^64` = 192 bit, `× (sb−sa)` < 2^96 → **288 bit** | `mul_div` me **U512**, pastaj `/ sa` |
| `Δb(L, sa, sb)` | `L · (sb − sa) / 2^64` | lart për input, poshtë për output | 128 + 96 = **224 bit** | U256 |
| `next_s` nga input A | `L · s · 2^64 / (L·2^64 + Δa·s)` | **lart** (çmimi zbret më pak) | numerues 128+96+64 = **288 bit**; emëruesi < 2^193 | U512 |
| `next_s` nga input B | `s + Δb · 2^64 / L` | **poshtë** (çmimi ngjitet më pak) | `Δb·2^64` < 2^128 | u128, kontroll `next_s ≤ MAX` |

Shënime:

- Shprehja “përdorim U256” nuk mjafton: dy formulat e A kërkojnë 288 bit. Implementimi përdor `mul_div(a, b, c)` me produkt të plotë 512-bit dhe pjesëtim të saktë, ose faktorizim të provuar që nuk humbet saktësi; zgjedhja e bibliotekës fiksohet në ENG-05 me rishikim.
- `Δa` me dy pjesëtime: `ceil(ceil(x / sb) / sa) ≥ ceil(x / (sa·sb))`, pra mbetet në favor të pool-it; për floor, `floor(floor(x / sb) / sa) = floor(x / (sa·sb))` ekzakt.
- Çdo rezultat shume që kalon `u64::MAX` kthen gabim; asnjë cast ngushtues pa kontroll.
- Pjesëtim me zero: `sa, sb ≥ MIN_SQRT_PRICE_X64 > 0`; `L = 0` trajtohet më vete (§5).

## 4. Kufijtë e likuiditetit

- `liquidity_net` i një tick-u: `i128`; `liquidity_gross`: `u128`.
- `MAX_LIQUIDITY_PER_TICK = u128::MAX / N`, ku `N = (MAX_TICK − MIN_TICK) / tick_spacing + 1`. Kjo garanton që shuma e L aktive nga të gjitha ticks nuk mund të kalojë `u128::MAX`; çdo shtim që e kalon refuzohet.
- Add liquidity: shumat e kërkuara rrumbullakosen lart; remove: shumat e paguara poshtë. Një cikël add→remove pa lëvizje çmimi nuk mund të japë më shumë token se sa u depozitua.
- `L` aktiv i shard-it duhet të jetë gjithmonë i barabartë me shumën e `L` të pozicioneve me `lower ≤ current_tick < upper` (invariant i testueshëm në ENG-05).

## 5. Cikli i swap-it exact-in

Hyrje: `amount_in > 0`, `min_out`, `sqrt_price_limit`, `deadline`, `shard_id`, `generation`, drejtimi.

1. Kontrollo limitin: për A→B `MIN_SQRT ≤ limit < s`; për B→A `s < limit ≤ MAX_SQRT`. Përndryshe gabim.
2. Përsërit derisa `remaining == 0`:
   - Gjej tick-un e ardhshëm të inicializuar **vetëm** në tick arrays të deklaruara, në renditjen e kërkuar; një varg që mungon, nuk lidhet me shard-in/gjeneratën ose është jashtë rendit → gabim.
   - `target = clamp(sqrt_price_at(next_tick), limit)`.
   - Hapi: nëse `L > 0`, llogarit input-in pa tarifë deri në target, tarifën dhe output-in. Nëse `L = 0` dhe distanca deri te target-i është pozitive, swap-i **dështon** me `ZeroLiquidityGap` (rishikuar 2026-10-28, ADR-0001 6a).
   - Tarifat akumulohen me **L përpara crossing-ut**; pastaj, nëse u arrit tick-u, kalohet: `fee_growth_outside` përmbyset dhe `L` përditësohet me `±liquidity_net` sipas drejtimit.
   - Çdo iteracion konsumon input > 0, ose lëviz çmimin te një target i ri, ose kalon një tick. Kufiri i fortë `MAX_SWAP_STEPS` (propozim: 64) kthen gabim në vend të loop-it.
3. Nëse mbetet input pas arritjes së limitit ose fundit të vargjeve → **gabim, pa partial fill** (politika v1).
4. Kontrollo `amount_out ≥ min_out` dhe deadline; vetëm pastaj CPI të tokenëve. Dështimi i CPI rrëzon gjithë transaksionin.

## 6. Tarifat

Për çdo hap me `L > 0`:

```text
net_budget    = floor(gross · (D − f) / D)                         D = 1_000_000
fee_nominal   = ceil(input_net · f / (D − f))
rounding_dust = gross − input_net − fee_nominal                    vetëm në hapin final, përndryshe 0
total_fee     = fee_nominal + rounding_dust
protocol_fee  = floor(total_fee · protocol_share_bps / 10_000)
lp_fee        = total_fee − protocol_fee
growth_delta  = floor(lp_fee · 2^128 / L)                          // < 2^192, U256
fee_growth_global = fee_growth_global.checked_add(growth_delta)   // pa wrap
```

Fee growth është **Q128** (jo Q64). Në llogari ruhet si `[u64; 4]`, limb-i 0 më pak i rëndësishëm, dhe Borsh e shkruan si 32 bajt little-endian. Vektorët e artë: `fee_growth` (Q128 delta dhe owed), `u256_limbs` (layout-i), `fee_inside` (1 500 pamje nga ecje të rastit me `inside` negativ dhe diferenca që kalojnë 2^256; referenca kontrollon që diferenca modulare = growth-i real brenda range-it). Testet: `math_vectors::{fee_growth_matches, u256_limbs_match_borsh_layout, fee_inside_modular_matches}`.

Pozicioni:

```text
outside(init) = tick ≤ current ? global : 0                   // checked, 0 ≤ outside ≤ global
outside(cross)= global − outside                              // checked
inside(now)   = global − below(lower) − above(upper)          // mod 2^256
total         = floor(old_position_L · (inside(now) − checkpoint mod 2^256) / 2^128)   // 384 bit → U512
owed_delta    = total − credited                              // checked: MathInvariantViolated
owed          = owed.checked_add(owed_delta)  → gabim në overflow
Δliquidity = 0: checkpoint mbetet, credited = total
Δliquidity ≠ 0: checkpoint = inside(now), credited = 0
```

Kur likuiditeti nuk ndryshon, checkpoint-i nuk lëviz, dhe `credited` mban sa është kaluar tashmë te `owed` (29 tetor 2026, riauditimi P2-2). Rrumbullakimi bëhet vetëm një herë për gjithë periudhën, kështu që mbledhja pas çdo swap-i paguan **saktësisht** sa një mbledhje e vetme në fund. Testet: `fee_math::tests::frequent_refreshes_pay_what_one_refresh_pays`, që përfshin rastin e riauditimit (L = 3 dhe L = 1, katër swap-e me nga 1 njësi tarifë) dhe 500 raste të rastit, plus `eng15_fee_checkpoint`.

Versioni i mëparshëm e lëvizte checkpoint-in me `ceil(paid · 2^128 / L)`, gjë që përdorte pak më shumë growth se sa duhej. Rasti i riauditimit paguante 2 në vend të 3. Kur likuiditeti ndryshon, në rritje ose ulje, nis një periudhë e re. Mbetja që humbet atje është më pak se 1 njësi token për çdo ndryshim, dhe i mbetet vault-it. Shuma e paguar nuk e kalon kurrë `floor(L · growth / 2^128)` të çdo periudhe, prandaj aftësia paguese nuk preket.

- `global` është monoton dhe **checked** (`add_growth` refuzon overflow). `outside` **nuk** është monoton: në çdo kalim bëhet `global − outside`, pra mund të ulet; ajo që garantohet dhe kontrollohet është `0 ≤ outside ≤ global` (`cross_outside` dhe `growth_inside` kthejnë `MathInvariantViolated` përndryshe). `inside` nuk mund të jetë checked: me inicializimin standard ai del negativ për një range që çmimi nuk e ka prekur ende (kundërshembulli në `math::fee_math::tests::inside_can_be_negative`). Prandaj `inside` llogaritet mod `2^256` dhe përdoren vetëm diferencat e tij. Diferenca reale (growth brenda range-it që nga checkpoint-i) është jo-negative dhe `< 2^256`, kështu që zbritja modulare është ekzakte. Asnjë shumë, L ose balancë nuk përdor wrapping.
- Tarifat historike llogariten me **L e vjetër** përpara çdo add/remove. Tarifa nuk shtohet kurrë në L aktiv.
- `protocol_fee_a/b` ruhen në `ShardState` me `checked_add`; mblidhen vetëm nga `collect_protocol_fees`.

## 7. Dust dhe pajtimi i vault-it

Mbetjet nga `floor` (growth, output, shumat e remove) mbeten në vault dhe **nuk i përkasin askujt** në v1: nuk mblidhen nga protokolli dhe nuk u jepen LP-ve. Invarianti që testohet pas çdo operacioni (`engine_model`, e korrigjuar pas auditimit F-05):

```text
claims_a = Σ_p principal_a(p)            // sa do të merrte p po ta hiqte tani: floor, me çmimin aktual
         + Σ_p owed_a(p)                 // tarifat e kredituara tashmë
         + Σ_p earned_a(p)               // floor(L_p · (inside_now − checkpoint_p) / 2^128), lexuar pa ndryshuar gjendjen
         + protocol_fee_a
vault_a  = claims_a + dust_a,   dust_a ≥ 0
```

(e njëjta për B). Versioni i mëparshëm i këtij seksioni shkruante "principal rrumbullakuar lart" dhe nuk numëronte `earned` që nga checkpoint-i i fundit; testi nuk e kontrollonte atë që thoshte dokumenti. Tani testi kontrollon më shumë se pabarazinë: në 1/8 e operacioneve dhe në fund, një kopje e botës del plotësisht (të gjithë LP-të dhe protokolli) në rend të rastit, të kundërt ose të drejtë, me tarifat para ose pas principal-it dhe protokollin para ose pas LP-ve. Çdo dalje duhet të paguajë **saktësisht** `claims` dhe të lërë **saktësisht** `dust`. Një mutacion që heq `earned` nga `claims` e rrëzon testin. Donacionet direkte në vault rrisin vetëm dust-in; nuk krijojnë L ose shares.

## 8. Pse 512 bit dhe jo “U256 kudo” — përmbledhje e provave

| Produkti | Kufiri i sipërm | Bit |
|---|---|---|
| `L · 2^64` | `2^128 · 2^64` | 192 |
| `L · 2^64 · (sb − sa)` | `2^192 · 2^96` | 288 |
| `L · s · 2^64` | `2^128 · 2^96 · 2^64` | 288 |
| `L · (sb − sa)` | `2^128 · 2^96` | 224 |
| `old_L · Δinside` (Q128, mod 2^256) | `2^128 · 2^256` | 384 → U512 në `mul_div` |
| `lp_fee · 2^128` (growth delta) | `2^64 · 2^128` | 192 |
| `amount · fee_rate_ppm` | `2^64 · 2^17` | 81 |

Rreshtat 288-bit dhe 384-bit janë arsyeja pse `mul_div` kërkon ndërmjetës 512-bit. Specifikimi i handoff-it përmend deri në 320 bit për produkte naive me faktorin `2^64` shtesë; formulat e mësipërme e shmangin atë rend duke mos shumëzuar kurrë tre faktorë 128/96/64 pa pjesëtim të ndërmjetëm të saktë.

`mul_div` jep gjithmonë rezultatin ekzakt, por zgjedh rrugën më të ngushtë që mjafton:

- produkt ≤ 256 bit me pjesëtues fuqi të 2-shit: zhvendosje;
- produkt dhe pjesëtues ≤ 128 bit: pjesëtim `u128`;
- produkt ≤ 256 bit: pjesëtim U256;
- përndryshe: U512.

Një test diferencial e krahason çdo rrugë me referencën U512 për 200 000 raste me gjerësi të rastësishme bitësh. `amount_b_delta` (`L·Δ/2^64`) llogaritet me një produkt 128×128 dhe zhvendosje, pa pjesëtim.

## 8b. Kostoja në SBF (e matur në LiteSVM, Agave 4.3.2)

| Operacioni | CU |
|---|---:|
| `initialize_price` | ~9 300 |
| `increase_liquidity` (pozicion i ri) | ~38 000 |
| `swap_exact_in`, një hap brenda intervalit (2 transferta CPI) | ~42 500 |
| çdo tick i inicializuar i kaluar | ~9 000 |
| swap me 10 kalime | ~131 600 (brenda 200k default) |

Një swap që kalon më shumë se ~15 ticks të inicializuar kërkon `SetComputeUnitLimit`. Testi `swap_compute_units_per_step_and_crossing` (eng06) mbron këto kufij.

## 9. Pragjet operative (jo matematikë kontrate)

Pragjet e readiness/aktivizimit shprehen fillimisht në njësi raw të mint-eve të çiftit të zgjedhur; pragje USD kërkojnë oracle të veçantë dhe janë jashtë v1. Vlerat konkrete varen nga çifti dhe janë vendim i hapur në [ADR-0004](adr/0004-open-product-decisions.md).
