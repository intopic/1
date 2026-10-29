# ADR-0005 — Tarifa dinamike për shard

**Statusi:** PROPOSED · **Data:** 2026-10-29

## Konteksti

Përdoruesi vendosi (29 tetor 2026) që tarifa të rritet kur çmimi lëviz shpejt, por pa prishur pavarësinë e shard-eve. Benchmark-u A-10 ([validator-bench.md](../bench/validator-bench.md)) tregoi se pavarësia vlen vetëm kur shard-et nuk ndajnë asnjë llogari writable. Prandaj gjendja e tarifës nuk mund të jetë në `MarketConfig`, që lexohet nga të gjitha shard-et.

Ideja e një "akumulatori volatiliteti" që mat largësinë e çmimit nga një pikë reference dhe zbehet me kohën është e njohur publikisht (p.sh. dokumentimi i Meteora DLMM). Këtu merret vetëm ideja e përgjithshme. Formula, njësitë dhe kodi janë tonat; asnjë rresht nuk është kopjuar nga Orca, Meteora ose Raydium (ADR-0003).

## Vendimi

### Gjendja (në `ShardState`, nga bajtet e rezervuara)

| Fusha | Tipi | Kuptimi |
|---|---|---|
| `vol_accumulator` | `u32` | Volatiliteti pas swap-it të fundit, në `VOL_UNIT` për çdo `tick_spacing` |
| `vol_reference` | `u32` | Pjesa e trashëguar nga episodi i mëparshëm |
| `tick_reference` | `i32` | Tick-u ku nisi episodi aktual |
| `last_update_slot` | `u64` | Slot-i i swap-it të fundit |
| `slot_swaps` | `u16` | Swap-e në `last_update_slot` (për termin e ngarkesës) |

`reserved` zvogëlohet nga 64 në 42 bajt. Madhësia e llogarisë nuk ndryshon. Asnjë llogari e re nuk i shtohet swap-it.

### Parametrat (në `MarketConfig`, vetëm lexim gjatë swap-it)

`DynamicFeeParams { max_fee_rate_ppm: u32, vol_coeff_ppm: u32, load_coeff_ppm: u32, max_vol_accumulator: u32, filter_slots: u16, decay_slots: u16, reduction_bps: u16 }`.

- `fee_rate_ppm` ekzistues mbetet tarifa bazë dhe minimumi.
- `max_fee_rate_ppm == fee_rate_ppm` e fik tarifën dinamike. Tregjet e reja nisin të fikur.
- Kufijtë: `fee_rate_ppm ≤ max_fee_rate_ppm ≤ MAX_DYNAMIC_FEE_RATE_PPM` (8 000 = 0,80%), `0 < filter_slots ≤ decay_slots`, `reduction_bps ≤ 10 000`, `max_vol_accumulator ≤ MAX_VOL_ACCUMULATOR`.
- Ndryshohen vetëm bashkë me tarifat, me të njëjtin timelock 24-orësh (`update_fees` → `apply_fees`).

### Algoritmi (një herë për swap, në `engine::dynamic_fee`)

Para swap-it, në slot-in `s`, me `e = s − last_update_slot`:

1. Nëse `e ≥ filter_slots`, nis episod i ri: `tick_reference = tick_current` dhe
   `vol_reference = vol_accumulator × reduction_bps / 10 000` nëse `e < decay_slots`, përndryshe `0`.
2. Nëse `s ≠ last_update_slot`, `slot_swaps = 0`.
3. `v = min(vol_reference + |tick_current − tick_reference| × VOL_UNIT / tick_spacing, max_vol_accumulator)`.
4. `fee = min(fee_rate_ppm + vol_coeff_ppm × v / VOL_UNIT + load_coeff_ppm × slot_swaps, max_fee_rate_ppm)`.

Swap-i ekzekutohet i tëri me `fee`. Pas swap-it:

5. `vol_accumulator = min(vol_reference + |tick_after − tick_reference| × VOL_UNIT / tick_spacing, max_vol_accumulator)`, `last_update_slot = s`, `slot_swaps += 1` (saturating).

Të gjitha veprimet janë me numra të plotë, `u64`/`u128` të kontrolluar, pa `f64`. `VOL_UNIT = 1 000`.

### Pse kështu

- **Një herë për swap, jo për çdo hap.** Kosto e vogël dhe e parashikueshme CU. Një swap i madh paguan tarifën që kishte shard-i kur nisi. Swap-i pas tij, zakonisht arbitrazhi ose back-run-i i një sandwich-i, paguan tarifën e rritur. Kjo e bën sulmin sandwich më të shtrenjtë. Viktima mbrohet si gjithmonë me `min_amount_out`.
- **Reference në tick, jo në kohë reale.** Slot-i vjen nga `Clock`, që lexohet pa llogari writable.
- **Termi i ngarkesës (`load_coeff_ppm`) nis 0.** Ai e bën tarifën të varet nga renditja brenda slot-it, të cilën e vendos leader-i. Ndizet vetëm nëse simulimi tregon se nuk rrit koston e tregtarëve të zakonshëm.
- **Nuk prek principalin e LP-ve.** Tarifa shtesë ndahet si çdo tarifë: pjesa e protokollit me `protocol_share_bps`, pjesa tjetër te LP-të me fee growth. Invariantet e aftësisë paguese (numeric-spec §7) mbeten të njëjtat.

### Kuotimi

Router-i ose SDK-ja llogarit të njëjtën tarifë nga `ShardState`, `MarketConfig` dhe slot-i aktual me funksionin publik `engine::dynamic_fee::quote_fee`. `SwapExecuted` raporton `fee_rate_ppm` e përdorur dhe likuiditetin aktiv pas swap-it.

## Alternativat e refuzuara

- **Gjendja në `MarketConfig`:** e bën tregun writable në çdo swap dhe i bashkon të gjitha shard-et në një llogari (shih kontrollin negativ në A-10).
- **Tarifë nga oracle:** shton varësi të jashtme dhe një llogari më shumë në çdo swap.
- **Tarifë për çdo hap swap-i:** më e saktë për swap-e shumë të mëdhenj, por kushton më shumë CU dhe e bën kuotën më të vështirë. Mund të rishikohet pas matjeve.
- **Fusha `f64` ose formula kuadratike:** e para është e ndaluar (konsensusi), e dyta nuk ka arsye pa simulim.

## Evidenca e kërkuar para `ACCEPTED`

1. Teste njësie dhe vetish për `engine::dynamic_fee`: tarifa mbetet brenda `[bazë, maks.]`, monotone në lëvizje, e njëjtë me tarifën bazë kur është fikur, zbehje e saktë.
2. Testet ekzistuese të motorit (aftësia paguese, ruajtja e token-ave) me tarifë dinamike të ndezur.
3. Teste onchain: shard-i rrit tarifën pas lëvizjes, shard-et e tjera mbeten byte-për-byte të pandryshuara, kuota e host-it përputhet me rezultatin onchain.
4. Simulim ENG-09 me tarifë dinamike kundrejt tarifës fikse: PnL i LP-ve dhe kosto e tregtarëve, me disa seed.
