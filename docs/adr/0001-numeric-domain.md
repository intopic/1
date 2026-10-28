# ADR-0001 — Domain numerik dhe rrumbullakimi

**Statusi:** PROPOSED · **Data:** 2026-10-28 · **Specifikimi:** [numeric-spec.md](../numeric-spec.md)

## Konteksti

Handoff-i (§9, §16) kërkon një specifikim të vetëm Q64.64 me input bounds, bit-width, drejtim rrumbullakimi dhe politikë overflow përpara çdo handler-i financiar.

## Vendimi

1. Sqrt price Q64.64 në `u128`, tick `i32` në `[-443636, 443636]`, L `u128`, shuma `u64`. Asnjë konstante Q96.
2. Konstantet e domain-it verifikohen kundrejt referencës së pavarur `tools/reference/tick_vectors.py` (Python `decimal`, 120 shifra).
3. Çdo rrumbullakim favorizon pool-in (input lart, output poshtë, çmimi pas hapit drejt më pak output-i).
4. `mul_div` me produkt 512-bit për formulat e token A (288 bit); U256 për ato të token B dhe fee owed.
5. `fee_growth_*` **u256 Q128, checked, pa wrap** (rishikuar 2026-10-28 sipas kërkimit N07): `ΔG = floor(LP_fee·2^128 / L_active)`, `0 ≤ outside ≤ global`, deltat inside jo-negative; mbetja e ndarjes mbetet në escrow të LP. Përjashtim i vetëm: `inside` llogaritet mod 2^256, sepse me inicializimin standard mund të jetë negativ (kundërshembull i testuar në `fee_math`); diferencat e tij janë ekzakte. Çdo shumë, L dhe balancë tjetër është `checked_*` dhe refuzon overflow.
6. Exact-in v1: i gjithë input-i konsumohet ose transaksioni dështon. Pa partial fills.
6a. Një gap me `L = 0` dhe distancë pozitive drejt target-it **dështon** me gabim; çmimi nuk lëviz pa konsum (rishikuar 2026-10-28).
6b. Hapi me tarifë (N05): `net_budget = floor(gross·(D−f)/D)`; në hapin final `nominal_min = ceil(input_net·f/(D−f))`, `rounding_dust = gross − input_net − nominal_min ≥ 0`, `total_fee = nominal_min + rounding_dust`. Quote dhe event-i i raportojnë veçmas; swap-i refuzohet kur output-i është 0 ose `rounding_dust > max_rounding_dust` i përdoruesit. Në crossing pa konsum tarifa është 0.
7. `MAX_LIQUIDITY_PER_TICK = u128::MAX / numri_i_ticks` për spacing-un e tregut.
8. Dust-i i ndarjes globale mbetet në vault pa pronar në v1 (nuk është e ardhur e lirë); invarianti i vault-it testohet pas çdo operacioni.

## Alternativat e konsideruara

- **Q64.96** (Uniswap v3): më shumë saktësi për çmime të vogla, por kërkon 160-bit sqrt price dhe konstante që nuk duam t'i trashëgojmë; Q64.64 mjafton për `P ∈ [2^-64, 2^64]`.
- **U256 kudo**: më e thjeshtë, por e pasaktë për formulat 288-bit ose kërkon kufizim të L në ~2^96; refuzuar sepse kufizon pool-et me çmim të ulët.
- **Fee growth modular u128 Q64** (vendimi i parë): standard te Uniswap/Raydium, por argumenti `outside ≤ global` nuk vlen pa ndryshim për aritmetikë modulare. Me u256 Q128 dhe shuma u64, overflow-i kërkon më shumë se 2^128 njësi tarifash për njësi likuiditeti, prandaj checked është i sigurt dhe më i lehtë për t'u provuar.
- **Partial fills**: kërkon kontratë SDK/API për input-in e konsumuar; shtyhet.

## Rreziqet

- Biblioteka 512-bit (zgjedhja finale në ENG-05: `uint` crate ose implementim i brendshëm) kërkon rishikim dhe teste diferenciale.
- Përafrimi i `sqrt_price_at_tick` mund të devijojë nga vlera e saktë; devijimi duhet matur për çdo vektor.
- CU: aritmetika 512-bit në SBF është e kushtueshme; matet në ENG-06.

## Testet që e mbyllin

Diferencial kundrejt referencës racionale për çdo formulë dhe drejtim, ticks ekzaktë, MIN/MAX, L = 0 dhe gap, input 1 njësi, produkte pranë kufijve, wrap i fee growth, cikle add→remove pa fitim nga rrumbullakimi. Seed-et e dështimeve ruhen.
