# Indeksi i dokumenteve

Përditësuar: 29 tetor 2026. Kur dy dokumente nuk përputhen, vlen dokumenti **normativ** dhe kodi; dokumentet **historike** ruhen vetëm si kontekst i vendimeve.

## Normative

| Dokumenti | Përmbajtja |
|---|---|
| [numeric-spec.md](numeric-spec.md) | Q64.64, ticks, formulat, rrumbullakimi, fee growth Q128, invarianti i aftësisë paguese |
| [account-layout.md](account-layout.md) | ABI v1: llogaritë, madhësitë, PDA-të (me fixture në [vectors/pda_fixtures.json](vectors/pda_fixtures.json)) |
| [tx-budget.md](tx-budget.md) | Llogaritë, bajtet dhe CU për swap të ndarë nëpër shard-e; ALT dhe kufiri 64 |
| [bench/validator-bench.md](bench/validator-bench.md) | Benchmark-u në validator (A-10): swap/slot për 1/2/4/8 shard-e dhe kontrolli negativ |
| [adr/](adr/README.md) | Vendimet e arkitekturës dhe produktit |
| [acceptance.md](acceptance.md) | Kriteret e pranimit dhe evidenca për secilin |
| [security.md](security.md) | Modeli i kërcënimeve |
| [vectors/](vectors/) | Vektorët e artë nga referencat e pavarura në `tools/reference/` |
| [../STATUS.md](../STATUS.md) | Gjendja aktuale dhe rezultatet e CI/stresit |
| [sim/](sim/eng09-analysis.md) | Rezultatet dhe analiza e simulimit ENG-09 |

## Historike (para implementimit)

[DEVELOPER_HANDOFF_8_SHARDS.md](DEVELOPER_HANDOFF_8_SHARDS.md), [OVERVIEW_RUST_ANCHOR_8_TO_56.md](OVERVIEW_RUST_ANCHOR_8_TO_56.md), [architecture.md](architecture.md), [PRODUCT_SCOPE_56_SHARDS.md](PRODUCT_SCOPE_56_SHARDS.md), [account-model.md](account-model.md), [toolchain.md](toolchain.md), [BIOS_VIRTUALIZATION.md](BIOS_VIRTUALIZATION.md), [research/](research/2026-10-28-team-research-review.md), [../ROADMAP.md](../ROADMAP.md), [../research/](../research/VLERESIMI_DHE_PLANI.md).

Çdo dokument historik ka në krye një shënim që e thotë këtë. Ndryshimet e njohura ndaj kodit janë shënuar në vend (p.sh. seed-et e pozicionit, F-02; formula Q64 e tarifave, F-03).
