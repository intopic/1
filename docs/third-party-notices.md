# Third-party notices

## Kod i importuar ose i portuar

Asnjë. Kur të portohet një skedar nga një upstream i lejuar (shih [ADR-0003](adr/0003-provenance-and-licensing.md)), ky seksion merr licencën e plotë, NOTICE-in përkatës, burimin (repo/commit/path) dhe përshkrimin e ndryshimeve, dhe hyrja shtohet te [`upstream-lock.json`](upstream-lock.json).

## Varësi të drejtpërdrejta

| Crate | Versioni | Licenca | Përdoret nga |
|---|---|---|---|
| `anchor-lang` | =1.2.1 | Apache-2.0 | `programs/sharded-amm`, `env-anchor` |
| `solana-program` | =4.1.0 | Apache-2.0 | `env-smoke` |

Varësitë tranzitive janë të fiksuara në `Cargo.lock`. Një SBOM i plotë gjenerohet në ENG-10.
