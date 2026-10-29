# Account layout (ABI v1, `SCHEMA_VERSION = 1`)

**Statusi:** implementuar për `MarketConfig`, `ShardRegistry`, `ShardState`, vault-et, `TickArray` dhe `Position`. Madhësitë verifikohen nga testi SVM `account_sizes_match_layout_doc`.

Serializimi është Borsh i Anchor-it me discriminator 8-bajtësh; pa padding, pa `Vec`, pa fusha me gjatësi të ndryshueshme. Enum-et janë `u8` me diskriminantë eksplicitë. Çdo ndryshim i layout-it kërkon rritje të `SCHEMA_VERSION` dhe test migrimi.

## PDA (normative)

```text
market:     ["market", schema_u16_le, mint_a, mint_b, market_id_u64_le]
registry:   ["registry", market]
shard:      ["shard", market, id_u8, generation_u64_le]
vault_auth: ["vault_auth", shard]        (pa data, vetëm signer i vault-eve)
vault:      ["vault", shard, mint]       (SPL Token account, authority = vault_auth)
ticks:      ["ticks", shard, start_tick_i32_le]          (ENG-05)
position:   ["lp_pos", shard, owner, tick_lower_i32_le, tick_upper_i32_le]
```

Adresat e plota për input-e fikse janë në [`docs/vectors/pda_fixtures.json`](vectors/pda_fixtures.json), të gjeneruara nga `tools/reference/pda_fixtures.py` (i pavarur nga kodi Rust). Testi SVM `pda_abi` kontrollon që programi real krijon pikërisht këto adresa dhe bump-e.

### Pse pozicioni identifikohet nga range-i (ABI v1)

Një pronar ka **një pozicion për çdo `(shard, tick_lower, tick_upper)`**. Shtimi i likuiditetit në të njëjtin range e rrit pozicionin ekzistues; tarifat e fituara deri atëherë kalojnë në `tokens_owed` para se të ndryshojë likuiditeti, kështu që bashkimi nuk humb asgjë. Përparësitë: klienti e gjen adresën pa numërues onchain, nuk ka llogari counter që bëhet pikë konflikti write-lock-u, dhe nuk ka rrezik garash për `position_id`. Kufizimi: pozicioni nuk transferohet (nuk ka NFT) dhe i njëjti pronar nuk mban dy pozicione të ndara në të njëjtin range. Nëse duhet transferim, kjo bëhet ABI v2.

**Versionimi:** versioni i ABI-së mbahet në seed-in e market-it (`schema_u16_le`). Shard-i lidhet me market-in, pozicioni me shard-in, prandaj çdo ndryshim i seed-eve të pozicionit kërkon `SCHEMA_VERSION = 2`, që jep market, shard dhe pozicione me adresa të reja. Varianti `position_id_u64_le` në dokumentet e vjetra **nuk është ABI** dhe nuk është implementuar.

## `MarketConfig` — 643 bajt

| Fusha | Lloji | Bajt |
|---|---|---:|
| discriminator | — | 8 |
| schema_version | u16 | 2 |
| market_id | u64 | 8 |
| mint_a, mint_b, token_program | Pubkey ×3 | 96 |
| admin, keeper, revenue_authority, pauser | Pubkey ×4 | 128 |
| params.fee_rate_ppm / protocol_share_bps / tick_spacing | u32 / u16 / u16 | 8 |
| dynamic_fees.max_fee_rate_ppm / vol_coeff_ppm / load_coeff_ppm / max_vol_accumulator / filter_slots / decay_slots / reduction_bps | u32 ×4 / u16 ×3 | 22 |
| status | MarketStatus (u8: 0 Initializing, 1 Open, 2 Paused, 3 Cancelled) | 1 |
| bump | u8 | 1 |
| fees_credited_a/b | u64 ×2 (tarifat e kaluara te `tokens_owed` që nga checkpoint-i) | 16 |

`fees_credited_a/b` (29 tetor 2026, riauditimi P2-2) e rritën pozicionin nga 177 në 193 bajt, para çdo deploy-i.
| readiness.min_liquidity / min_reserve_a / min_reserve_b / max_activation_deviation_bps / depth_probe_a / depth_probe_b / max_depth_impact_bps | u128 / u64 / u64 / u16 / u64 / u64 / u16 | 52 |
| pending_fees.fee_rate_ppm / protocol_share_bps / dynamic / eta / active | u32 / u16 / DynamicFeeParams / i64 / bool | 37 |
| pause_flags | u8 (bit 0 swap-e, 1 depozita, 2 rebalancim; ADR-0006) | 1 |
| rebalancers | Pubkey ×4 (bosh = `Pubkey::default()`; ADR-0006) | 128 |
| pending_rebalancers.keys / eta / active | Pubkey ×4 / i64 / bool | 137 |
| config_version | u32 (rritet nga `apply_fees`; certifikata e thellësisë vlen vetëm për versionin e saj) | 4 |
| reserved | [u8; 10] | 10 |

`readiness` dhe `pending_fees` (timelock 24 orë për tarifat) zunë 49 bajt nga `reserved`; madhësia mbeti 316. Tarifa dinamike ([ADR-0005](adr/0005-dynamic-fee-per-shard.md), 2026-10-29) shtoi `dynamic_fees` dhe `pending_fees.dynamic` (44 bajt) dhe e çoi tregun në 360 bajt. Lista e rebalancer-ave dhe ndryshimi i saj me timelock (ADR-0006, 2026-10-29) shtuan 265 bajt, deri në 625. Prova e thellësisë (rregulli 4) shtoi 18 bajt në `readiness`, deri në 643. Të dyja ndodhën para çdo deploy-i; prandaj `SCHEMA_VERSION` mbetet 1. Gjatë swap-it tregu mbetet vetëm për lexim.

## `ShardRegistry` — 73 bajt

market (32), prepared_mask u64, active_mask u64, registry_version u64, bump u8, swap_paused_mask u64. `swap_paused_mask` (riauditimi P2-1) pasqyron se cilat shard-e kanë `PAUSE_SWAPS`. E shkruajnë `set_shard_pause_flags` dhe `reopen_shard`, swap-et nuk e lexojnë, dhe nuk e rrit `registry_version`. Regjistri u rrit nga 65 në 73 bajt, para çdo deploy-i.

## `ShardState` — 296 bajt

| Fusha | Lloji | Bajt |
|---|---|---:|
| discriminator | — | 8 |
| market | Pubkey | 32 |
| id | u8 | 1 |
| generation | u64 (gjithmonë 0 në v1) | 8 |
| status | ShardStatus (u8: 0 Prepared, 1 Active) | 1 |
| vault_a, vault_b | Pubkey ×2 | 64 |
| vault_authority_bump | u8 | 1 |
| sqrt_price_x64 | u128 (0 = ende pa çmim) | 16 |
| tick_current | i32 | 4 |
| liquidity | u128 | 16 |
| fee_growth_global_a/b_x128 | [u64; 4] ×2 (U256 little-endian, Q128, checked, pa wrap) | 64 |
| protocol_fees_a/b | u64 ×2 | 16 |
| bump | u8 | 1 |
| dynamic_fee.vol_accumulator / vol_reference / tick_reference / reference_slot / last_update_slot / slot_swaps | u32 / u32 / i32 / u64 / u64 / u16 | 30 |
| depth_verified | bool (vendoset nga `verify_depth`, fshihet nga çdo ndryshim likuiditeti) | 1 |
| depth_config_version | u32 (`config_version` i tregut kur u bë prova) | 4 |
| pause_flags | u8 (`PAUSE_*` vetëm për këtë shard, `set_shard_pause_flags`) | 1 |
| reserved | [u8; 28] | 28 |

Gjendja e tarifës dinamike (ADR-0005) zuri 30 bajt nga `reserved`, `depth_verified` 1, `pause_flags` 1 dhe `depth_config_version` 4; madhësia mbeti 296. Shkruhet vetëm nga swap-et e këtij shard-i.

Fee growth u ndryshua nga u128 Q64 në U256 Q128 më 2026-10-28 (ENG-05, kërkimi N07), para çdo deploy-i; prandaj `SCHEMA_VERSION` mbetet 1.

`ShardStatus::Prepared = 0` sepse `init` i Anchor-it deserializon llogarinë e sapokrijuar me data zero; një vlerë 0 e pavlefshme e enum-it e bën `init` të dështojë.

## `TickArray` — 3 128 bajt (zero-copy)

PDA `["ticks", shard, start_tick_index_i32_le]`. 32 slote; `start_tick_index` është shumëfish i `32 · tick_spacing`.

| Fusha | Lloji | Bajt |
|---|---|---:|
| discriminator | — | 8 |
| shard | Pubkey | 32 |
| start_tick_index | i32 | 4 |
| initialized_count | u32 | 4 |
| ticks | `Tick` ×32: liquidity_net i128, liquidity_gross u128, fee_growth_outside_a/b [u64; 4] ×2 (si limb-e u64) | 3 072 |
| bump + padding | u8 + [u8; 7] | 8 |

## `Position` — 193 bajt

PDA `["lp_pos", shard, owner, tick_lower_i32_le, tick_upper_i32_le]`.

| Fusha | Lloji | Bajt |
|---|---|---:|
| discriminator | — | 8 |
| shard, owner | Pubkey ×2 | 64 |
| tick_lower, tick_upper | i32 ×2 | 8 |
| liquidity | u128 | 16 |
| fee_growth_inside_last_a/b | [u64; 4] ×2 | 64 |
| tokens_owed_a/b | u64 ×2 | 16 |
| bump | u8 | 1 |
