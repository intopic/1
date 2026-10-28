# ADR-0004 — Vendime produkti të hapura (handoff §16)

**Statusi:** OPEN · **Data:** 2026-10-28

Këto vendime duhen nga pronari i produktit përpara handler-ave financiarë (ENG-04..ENG-06). Propozimet më poshtë janë pikënisje inxhinierike, jo vendime; kodi nuk i supozon në heshtje.

| Vendimi | Propozimi | Pse ka rëndësi | Bllokon |
|---|---|---|---|
| Çifti fillestar, decimals | P.sh. wSOL/USDC në devnet me mint-e testi | Pragjet raw, test fixtures | ENG-04 |
| Token program | Vetëm SPL Token klasik; Token-2022 refuzohet në v1 | SEC-11 | ENG-03 |
| Mint me freeze authority | Lejohet me deklarim publik të rrezikut (USDC e ka) | Vault-i mund të ngrihet | ENG-03 |
| `fee_rate_ppm` | 3_000 (0,30%) | Ekonomia LP | ENG-06 |
| `protocol_share_bps` | 2_000 (20%) — parametër, jo ligj | Revenue vs LP | ENG-06 |
| `tick_spacing` | 60 | Madhësia e tick arrays, CU | ENG-03 |
| Çmimi/range fillestar i 8 shard-eve | I njëjti çmim fillestar, range i njëjtë, kapital i barabartë | Krahasueshmëria e benchmark-ut | ENG-04 |
| Çmimi i shard-it të ri | Çmim nga financuesi brenda tolerancës ndaj medianës së shard-eve aktive | Arbitrazh në aktivizim | ENG-08 |
| Pragu i readiness | L aktive ≥ X dhe inventar real ≥ Y në njësi raw, të dy drejtimet | Aktivizim bosh | ENG-04/08 |
| Cooldown aktivizimi | Pa cooldown në v1, vetëm kontrolle aktuale | Spam/kosto | ENG-08 |
| Rolet | `admin` (parametra), `keeper` (prepare/activate brenda politikës), `revenue` (protocol fees), `pauser`; upgrade authority e veçantë, multisig për kapital real | SEC-12 | ENG-03 |
| Pauzë selektive | Vetëm pauzë tregu në v1; `paused_mask` më vonë | Skema | ENG-03 |
| Kufiri i tick arrays për swap | ≤ 3 vargje, `MAX_SWAP_STEPS = 64` | CU/accounts | ENG-06 |
| Rebalance | Jashtë MVP; vetëm `rebalance_pair` i kufizuar pasi të ketë model ekonomik që nuk prek principal LP | LP principal | ENG-09 |
| Upgrade/migration | Upgrade authority multisig + vonesë për çdo deployment me kapital real | Besimi i custody-t | ENG-10 |

Kur një vendim merret, ky rresht zëvendësohet me vendimin, datën dhe kush e mori, dhe krijohet ADR i veçantë nëse ndryshon ABI-në.
