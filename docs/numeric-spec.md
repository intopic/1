# Specifikimi numerik — Q64.64, ticks, likuiditet dhe tarifa

**Statusi:** ENG-01, i propozuar për rishikim. Ky dokument fikson domain-in dhe rregullat që kodi i ENG-05/06 duhet të zbatojë. Asnjë formulë këtu nuk është ende e implementuar onchain; vetëm konstantet e maskës (ENG-02) dhe vektorët e referencës ekzistojnë si kod. Vendimi formal është [ADR-0001](adr/0001-numeric-domain.md).

## 1. Llojet dhe njësitë

| Madhësia | Lloji | Njësia / kuptimi | Domain-i |
|---|---|---|---|
| `tick` | `i32` | `P = 1.0001^tick`, ku `P` = token B për 1 token A, në njësi bazë (raw, pa decimals) | `MIN_TICK = -443636 ≤ tick ≤ MAX_TICK = 443636` |
| `sqrt_price_x64` | `u128` | `√P · 2^64` (Q64.64) | `MIN_SQRT_PRICE_X64 ≤ s ≤ MAX_SQRT_PRICE_X64`, gjithmonë `< 2^96` |
| `liquidity` (L) | `u128` | njësi `√(A·B)` | aktive `≤ u128::MAX`; për tick shih §4 |
| shumat e tokenëve | `u64` | njësi bazë të mint-it | `0 ≤ a ≤ u64::MAX`; çdo rezultat që s'futet në `u64` refuzohet |
| `fee_growth_*_x64` | `u128` | tarifë LP për njësi L, Q64.64 | **modulare `2^128`** (§6) |
| `fee_rate_ppm` | `u32` | pjesë e input-it, emërues `1_000_000` | `0 ≤ r ≤ 100_000` (10%) |
| `protocol_share_bps` | `u16` | pjesë e tarifës totale, emërues `10_000` | `0 ≤ p ≤ 5_000` |

Token A dhe B janë mint-et në renditje kanonike (`mint_a < mint_b` sipas bajtëve). Matematika punon vetëm me shuma raw; decimals përdoren vetëm për pragje operative (§9).

**Q96 nuk përdoret.** Asnjë konstante e kopjuar nga implementime Q64.96 (p.sh. Uniswap v3) nuk lejohet. Raydium përdor gjithashtu Q64.64; konstantet tona janë rillogaritur në mënyrë të pavarur (§2) dhe nuk merren nga kodi i tyre.

## 2. Domain-i tick ↔ çmim

`MAX_TICK = 443636 = floor(log_{1.0001}(2^64))`, pra `P ∈ [2^-64, 2^64]` dhe `√P · 2^64 ∈ [2^32, 2^96]`. Vlerat e sakta, nga referenca e pavarur me 120 shifra:

```text
MIN_SQRT_PRICE_X64 (floor, tick = -443636) = 4295048016
MAX_SQRT_PRICE_X64 (ceil,  tick =  443636) = 79226673515401279992447579062   (96 bit)
```

Burimi: [`tools/reference/tick_vectors.py`](../tools/reference/tick_vectors.py) (vetëm stdlib Python `decimal`), dalja në [`docs/vectors/tick_vectors.json`](vectors/tick_vectors.json) me 47 vektorë (kufijtë, ±1, ±60, ±100000 dhe fuqitë e 2). Komanda:

```bash
python3 -I tools/reference/tick_vectors.py > docs/vectors/tick_vectors.json
```

`get_sqrt_price_at_tick` i ENG-05 do të llogarisë një përafrim me tabelë fuqish. Kontrata e tij: për çdo tick në domain, rezultati `s` plotëson `floor ≤ s ≤ ceil` i vektorit ose ka gabim relativ të deklaruar e të testuar; konstantet `MIN/MAX_SQRT_PRICE_X64` të kodit janë **dalja e funksionit** në kufij dhe testohen kundrejt këtyre vlerave. Nëse algoritmi jep vlerë tjetër nga ajo e saktë (siç ndodh në implementime ekzistuese), ndryshimi dokumentohet me test, jo me rregullim të heshtur.

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
   - Hapi: nëse `L > 0`, llogarit input-in pa tarifë deri në target, tarifën dhe output-in; nëse `L = 0`, hapi kalon te target pa konsumuar input dhe pa tarifë.
   - Tarifat akumulohen me **L përpara crossing-ut**; pastaj, nëse u arrit tick-u, kalohet: `fee_growth_outside` përmbyset dhe `L` përditësohet me `±liquidity_net` sipas drejtimit.
   - Çdo iteracion konsumon input > 0, ose lëviz çmimin te një target i ri, ose kalon një tick. Kufiri i fortë `MAX_SWAP_STEPS` (propozim: 64) kthen gabim në vend të loop-it.
3. Nëse mbetet input pas arritjes së limitit ose fundit të vargjeve → **gabim, pa partial fill** (politika v1).
4. Kontrollo `amount_out ≥ min_out` dhe deadline; vetëm pastaj CPI të tokenëve. Dështimi i CPI rrëzon gjithë transaksionin.

## 6. Tarifat

Për çdo hap me `L > 0`:

```text
fee_step      = ceil(amount_in_step · fee_rate_ppm / (1_000_000 − fee_rate_ppm))   kur arrihet target-i
              = remaining − amount_in_step                                         kur hapi e konsumon gjithë mbetjen
protocol_fee  = floor(fee_step · protocol_share_bps / 10_000)
lp_fee        = fee_step − protocol_fee
growth_delta  = floor(lp_fee · 2^64 / L)            // lp_fee < 2^64, produkti < 2^128
fee_growth_global = fee_growth_global.wrapping_add(growth_delta)
```

Pozicioni:

```text
inside(now)   = global − below(lower) − above(upper)          // të gjitha wrapping mod 2^128
owed_delta    = floor(old_position_L · inside(now).wrapping_sub(checkpoint) / 2^64)   // 256 bit → U256
owed          = owed.checked_add(owed_delta)  → gabim në overflow
checkpoint    = inside(now)
```

- **Politika modulare** vlen vetëm për `fee_growth_*`: diferencat llogariten me `wrapping_sub`, prandaj wrap-i i akumulatorit nuk humbet tarifa për sa kohë një pozicion nuk lë të kalojë më shumë se `2^128` njësi growth pa u përditësuar. Asnjë shumë, L ose balancë nuk përdor wrapping.
- Tarifat historike llogariten me **L e vjetër** përpara çdo add/remove. Tarifa nuk shtohet kurrë në L aktiv.
- `protocol_fee_a/b` ruhen në `ShardState` me `checked_add`; mblidhen vetëm nga `collect_protocol_fees`.

## 7. Dust dhe pajtimi i vault-it

Mbetjet nga `floor` (growth, output, shumat e remove) mbeten në vault dhe **nuk i përkasin askujt** në v1: nuk mblidhen nga protokolli dhe nuk u jepen LP-ve. Invarianti që testohet pas çdo operacioni:

```text
vault_a ≥ Σ principal_a(pozicionet, rrumbullakuar lart) + Σ owed_a + protocol_fee_a
```

(e njëjta për B). Donacionet direkte në vault rrisin vetëm dust-in; nuk krijojnë L ose shares.

## 8. Pse 512 bit dhe jo “U256 kudo” — përmbledhje e provave

| Produkti | Kufiri i sipërm | Bit |
|---|---|---|
| `L · 2^64` | `2^128 · 2^64` | 192 |
| `L · 2^64 · (sb − sa)` | `2^192 · 2^96` | 288 |
| `L · s · 2^64` | `2^128 · 2^96 · 2^64` | 288 |
| `L · (sb − sa)` | `2^128 · 2^96` | 224 |
| `old_L · growth_delta` | `2^128 · 2^128` | 256 |
| `lp_fee · 2^64` | `2^64 · 2^64` | 128 |
| `amount · fee_rate_ppm` | `2^64 · 2^17` | 81 |

Rreshtat 288-bit janë arsyeja pse `mul_div` kërkon ndërmjetës 512-bit. Specifikimi i handoff-it përmend deri në 320 bit për produkte naive me faktorin `2^64` shtesë; formulat e mësipërme e shmangin atë rend duke mos shumëzuar kurrë tre faktorë 128/96/64 pa pjesëtim të ndërmjetëm të saktë.

## 9. Pragjet operative (jo matematikë kontrate)

Pragjet e readiness/aktivizimit shprehen fillimisht në njësi raw të mint-eve të çiftit të zgjedhur; pragje USD kërkojnë oracle të veçantë dhe janë jashtë v1. Vlerat konkrete varen nga çifti dhe janë vendim i hapur në [ADR-0004](adr/0004-open-product-decisions.md).
