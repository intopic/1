# ADR-0001 — Domain numerik dhe rrumbullakimi

**Statusi:** PROPOSED · **Data:** 2026-10-28 · **Specifikimi:** [numeric-spec.md](../numeric-spec.md)

## Konteksti

Handoff-i (§9, §16) kërkon një specifikim të vetëm Q64.64 me input bounds, bit-width, drejtim rrumbullakimi dhe politikë overflow përpara çdo handler-i financiar.

## Vendimi

1. Sqrt price Q64.64 në `u128`, tick `i32` në `[-443636, 443636]`, L `u128`, shuma `u64`. Asnjë konstante Q96.
2. Konstantet e domain-it verifikohen kundrejt referencës së pavarur `tools/reference/tick_vectors.py` (Python `decimal`, 120 shifra).
3. Çdo rrumbullakim favorizon pool-in (input lart, output poshtë, çmimi pas hapit drejt më pak output-i).
4. `mul_div` me produkt 512-bit për formulat e token A (288 bit); U256 për ato të token B dhe fee owed.
5. `fee_growth_*` modulare `2^128` me `wrapping_sub` për diferencat; çdo shumë, L dhe balancë tjetër është `checked_*` dhe refuzon overflow.
6. Exact-in v1: i gjithë input-i konsumohet ose transaksioni dështon. Pa partial fills.
7. `MAX_LIQUIDITY_PER_TICK = u128::MAX / numri_i_ticks` për spacing-un e tregut.
8. Dust mbetet në vault pa pronar në v1; invarianti i vault-it testohet pas çdo operacioni.

## Alternativat e konsideruara

- **Q64.96** (Uniswap v3): më shumë saktësi për çmime të vogla, por kërkon 160-bit sqrt price dhe konstante që nuk duam t'i trashëgojmë; Q64.64 mjafton për `P ∈ [2^-64, 2^64]`.
- **U256 kudo**: më e thjeshtë, por e pasaktë për formulat 288-bit ose kërkon kufizim të L në ~2^96; refuzuar sepse kufizon pool-et me çmim të ulët.
- **Refuzim në overflow të fee growth**: do të bllokonte përfundimisht pool-in pas akumulimit; modular me diferenca është standard dhe i testueshëm.
- **Partial fills**: kërkon kontratë SDK/API për input-in e konsumuar; shtyhet.

## Rreziqet

- Biblioteka 512-bit (zgjedhja finale në ENG-05: `uint` crate ose implementim i brendshëm) kërkon rishikim dhe teste diferenciale.
- Përafrimi i `sqrt_price_at_tick` mund të devijojë nga vlera e saktë; devijimi duhet matur për çdo vektor.
- CU: aritmetika 512-bit në SBF është e kushtueshme; matet në ENG-06.

## Testet që e mbyllin

Diferencial kundrejt referencës racionale për çdo formulë dhe drejtim, ticks ekzaktë, MIN/MAX, L = 0 dhe gap, input 1 njësi, produkte pranë kufijve, wrap i fee growth, cikle add→remove pa fitim nga rrumbullakimi. Seed-et e dështimeve ruhen.
