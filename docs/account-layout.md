# Account layout (ABI v1, `SCHEMA_VERSION = 1`)

**Statusi:** ENG-03, implementuar për `MarketConfig`, `ShardRegistry`, `ShardState` dhe vault-et. `TickArray` dhe `LpPosition` vijnë në ENG-05. Madhësitë verifikohen nga testi SVM `account_sizes_match_layout_doc`.

Serializimi është Borsh i Anchor-it me discriminator 8-bajtësh; pa padding, pa `Vec`, pa fusha me gjatësi të ndryshueshme. Enum-et janë `u8` me diskriminantë eksplicitë. Çdo ndryshim i layout-it kërkon rritje të `SCHEMA_VERSION` dhe test migrimi.

## PDA (normative)

```text
market:     ["market", schema_u16_le, mint_a, mint_b, market_id_u64_le]
registry:   ["registry", market]
shard:      ["shard", market, id_u8, generation_u64_le]
vault_auth: ["vault_auth", shard]        (pa data, vetëm signer i vault-eve)
vault:      ["vault", shard, mint]       (SPL Token account, authority = vault_auth)
ticks:      ["ticks", shard, start_tick_i32_le]          (ENG-05)
position:   ["lp_pos", shard, owner, position_id_u64_le] (ENG-05)
```

## `MarketConfig` — 316 bajt

| Fusha | Lloji | Bajt |
|---|---|---:|
| discriminator | — | 8 |
| schema_version | u16 | 2 |
| market_id | u64 | 8 |
| mint_a, mint_b, token_program | Pubkey ×3 | 96 |
| admin, keeper, revenue_authority, pauser | Pubkey ×4 | 128 |
| params.fee_rate_ppm / protocol_share_bps / tick_spacing | u32 / u16 / u16 | 8 |
| status | MarketStatus (u8: 0 Initializing, 1 Open, 2 Paused) | 1 |
| bump | u8 | 1 |
| reserved | [u8; 64] | 64 |

## `ShardRegistry` — 65 bajt

market (32), prepared_mask u64, active_mask u64, registry_version u64, bump u8.

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
| reserved | [u8; 64] | 64 |

Fee growth u ndryshua nga u128 Q64 në U256 Q128 më 2026-10-28 (ENG-05, kërkimi N07), para çdo deploy-i; prandaj `SCHEMA_VERSION` mbetet 1.

`ShardStatus::Prepared = 0` sepse `init` i Anchor-it deserializon llogarinë e sapokrijuar me data zero; një vlerë 0 e pavlefshme e enum-it e bën `init` të dështojë.
