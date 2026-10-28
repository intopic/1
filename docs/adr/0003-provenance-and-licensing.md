# ADR-0003 — Provenance dhe licencat

**Statusi:** ACCEPTED · **Data:** 2026-10-28

## Vendimi

- **Raydium CLMM** (Apache-2.0), snapshot `ed7c84a54ced59c55981780546adb0b4583dcf85`, është e vetmja bazë kodi e lejuar për studim dhe ripërdorim të mundshëm. Çdo skedar ose fragment i portuar regjistrohet në [`upstream-lock.json`](../upstream-lock.json) me repo, commit, path, SHA256 të origjinalit, licencë, ndryshime lokale dhe audit-scope kur dihet, dhe shënohet në [`third-party-notices.md`](../third-party-notices.md). Snapshot-i nuk përfaqëson HEAD-in ose deployment-in aktual të Raydium.
- **Orca Whirlpools** është nën Orca License; **nuk kopjohet asnjë kod**, as si referencë e drejtpërdrejtë implementimi, pa pëlqim të shkruar ose verifikim të veçantë licence për një commit historik.
- Raportet e auditimit të upstream-it janë kontekst, jo certifikatë për këtë program.
- Varësitë nga crates.io (p.sh. `anchor-lang` 1.2.1, Apache-2.0) menaxhohen me `Cargo.lock` dhe listohen në notices; ato nuk konsiderohen kod i importuar.

## Gjendja aktuale

Nuk është importuar asnjë skedar nga Raydium ose Orca. Konstantet e domain-it në [numeric-spec.md](../numeric-spec.md) janë rillogaritur nga përkufizimi matematikor me `tools/reference/tick_vectors.py`; përputhja e `MIN_SQRT_PRICE_X64 = 4295048016` me vlerën e publikuar nga Raydium është kontroll i pavarur, jo kopjim.
