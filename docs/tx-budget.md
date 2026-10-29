# Buxheti i një transaksioni me swap të ndarë nëpër shard-e (auditimi F-04)

**Statusi: NORMATIVE për router-in/SDK.** Tabela më poshtë gjenerohet nga testi `tests/svm/tests/tx_budget.rs` (`TX_BUDGET_OUT=docs/tx-budget.md`), me transaksione reale `swap_exact_in` mbi programin SBF në LiteSVM 0.18.1, me feature gates të mainnet-beta (snapshot-i i LiteSVM, 2026-10-15).

## Dy kufij të ndryshëm

| Kufiri | Vlera | Çfarë e zgjidh |
|---|---|---|
| Madhësia e paketës (`PACKET_DATA_SIZE`) | 1 232 bajt | **ALT** e zgjidh: çdo llogari e ngarkuar nga tabela kushton 1 bajt në vend të 32. |
| Llogaritë e kyçura në një transaksion | **64** (sa kohë `increase_tx_account_lock_limit` nuk është aktiv; në snapshot-in e mainnet të LiteSVM nuk është) | **ALT nuk e zgjidh.** Çdo llogari e ngarkuar nga ALT numërohet njësoj. Vetëm më pak llogari e zgjidhin. |

Versioni i mëparshëm i STATUS.md dhe i analizës ENG-09 thoshte se ALT zgjidh kufirin e 64 llogarive. Kjo ishte **gabim** dhe u korrigjua.

## Kalkulatori

```text
llogari = 7 + Σ_i (4 + K_i) + [1 nëse ka ComputeBudget]
  7  = payer, market, registry, user_token_in, user_token_out, token_program, programi AMM
  4  = shard, vault_a, vault_b, vault_authority  (për çdo shard)
  K_i = tick arrays të shard-it i (1..=3)
```

Funksioni `budget(n, k, alt, compute_ix)` jep edhe bajtet e sakta. Testi kontrollon që parashikimi është **i barabartë** me mesazhin e kompiluar për të 48 kombinimet.

## Përfundimet që testi i provon

- **Pa ALT:** paketa e kufizon swap-in e ndarë në 3 shard-e (K ≤ 2) ose 2 shard-e (K = 3).
- **Me ALT:** të 8 shard-et hyjnë për K = 1, 2, 3, me 855–887 bajt.
- **8 shard-e × 3 tick arrays + ComputeBudget = saktësisht 64 llogari.** Kjo është në kufi. Çdo llogari shtesë refuzohet me `TooManyAccountLocks` edhe me ALT. Testi e provon me System Program (65 llogari), që i duhet p.sh. wrap-it të SOL. Prandaj router-i duhet:
  - të përdorë K = 1 ose 2 për shard-et kur bën wrap/unwrap në të njëjtin transaksion, ose
  - të ndajë urdhrin në më pak shard-e, ose në dy transaksione (jo atomike).
- **CU:** ~33–34k për shard në këtë matje (swap i vogël B→A, pa kalim tick-u), ~9k për çdo kalim ([numeric-spec §8b](numeric-spec.md)). 8 shard-e pa kalime ≈ 270k CU, brenda kufirit 1.4M me `SetComputeUnitLimit`.

`SetComputeUnitPrice` përdor të njëjtin program ComputeBudget, kështu që nuk shton llogari.

## Tabela e matur

| shard-e | tick arrays/shard | mesazhi | llogari | bajt | rezultati (LiteSVM, mainnet features) | CU |
|---:|---:|---|---:|---:|---|---:|
| 1 | 1 | legacy | 13 | 597 | OK | 33236 |
| 1 | 1 | v0 + ALT | 13 | 323 | OK | 33224 |
| 2 | 1 | legacy | 18 | 828 | OK | 66317 |
| 2 | 1 | v0 + ALT | 18 | 399 | OK | 66303 |
| 3 | 1 | legacy | 23 | 1059 | OK | 99390 |
| 3 | 1 | v0 + ALT | 23 | 475 | OK | 99383 |
| 4 | 1 | legacy | 28 | 1290 | ekzekutohet, por 1290 > 1232 bajt: s'dërgohet | 132466 |
| 4 | 1 | v0 + ALT | 28 | 551 | OK | 132462 |
| 5 | 1 | legacy | 33 | 1521 | ekzekutohet, por 1521 > 1232 bajt: s'dërgohet | 165539 |
| 5 | 1 | v0 + ALT | 33 | 627 | OK | 165525 |
| 6 | 1 | legacy | 38 | 1752 | ekzekutohet, por 1752 > 1232 bajt: s'dërgohet | 198610 |
| 6 | 1 | v0 + ALT | 38 | 703 | OK | 198607 |
| 7 | 1 | legacy | 43 | 1983 | ekzekutohet, por 1983 > 1232 bajt: s'dërgohet | 231677 |
| 7 | 1 | v0 + ALT | 43 | 779 | OK | 231670 |
| 8 | 1 | legacy | 48 | 2214 | ekzekutohet, por 2214 > 1232 bajt: s'dërgohet | 264770 |
| 8 | 1 | v0 + ALT | 48 | 855 | OK | 264762 |
| 1 | 2 | legacy | 14 | 630 | OK | 33790 |
| 1 | 2 | v0 + ALT | 14 | 325 | OK | 33788 |
| 2 | 2 | legacy | 20 | 894 | OK | 67432 |
| 2 | 2 | v0 + ALT | 20 | 403 | OK | 67435 |
| 3 | 2 | legacy | 26 | 1158 | OK | 101070 |
| 3 | 2 | v0 + ALT | 26 | 481 | OK | 101058 |
| 4 | 2 | legacy | 32 | 1422 | ekzekutohet, por 1422 > 1232 bajt: s'dërgohet | 134693 |
| 4 | 2 | v0 + ALT | 32 | 559 | OK | 134709 |
| 5 | 2 | legacy | 38 | 1686 | ekzekutohet, por 1686 > 1232 bajt: s'dërgohet | 168316 |
| 5 | 2 | v0 + ALT | 38 | 637 | OK | 168280 |
| 6 | 2 | legacy | 44 | 1950 | ekzekutohet, por 1950 > 1232 bajt: s'dërgohet | 201956 |
| 6 | 2 | v0 + ALT | 44 | 715 | OK | 201964 |
| 7 | 2 | legacy | 50 | 2214 | ekzekutohet, por 2214 > 1232 bajt: s'dërgohet | 235599 |
| 7 | 2 | v0 + ALT | 50 | 793 | OK | 235571 |
| 8 | 2 | legacy | 56 | 2478 | ekzekutohet, por 2478 > 1232 bajt: s'dërgohet | 269261 |
| 8 | 2 | v0 + ALT | 56 | 871 | OK | 269270 |
| 1 | 3 | legacy | 15 | 663 | OK | 34371 |
| 1 | 3 | v0 + ALT | 15 | 327 | OK | 34378 |
| 2 | 3 | legacy | 22 | 960 | OK | 68574 |
| 2 | 3 | v0 + ALT | 22 | 407 | OK | 68586 |
| 3 | 3 | legacy | 29 | 1257 | ekzekutohet, por 1257 > 1232 bajt: s'dërgohet | 102796 |
| 3 | 3 | v0 + ALT | 29 | 487 | OK | 102781 |
| 4 | 3 | legacy | 36 | 1554 | ekzekutohet, por 1554 > 1232 bajt: s'dërgohet | 136972 |
| 4 | 3 | v0 + ALT | 36 | 567 | OK | 136944 |
| 5 | 3 | legacy | 43 | 1851 | ekzekutohet, por 1851 > 1232 bajt: s'dërgohet | 171145 |
| 5 | 3 | v0 + ALT | 43 | 647 | OK | 171151 |
| 6 | 3 | legacy | 50 | 2148 | ekzekutohet, por 2148 > 1232 bajt: s'dërgohet | 205348 |
| 6 | 3 | v0 + ALT | 50 | 727 | OK | 205333 |
| 7 | 3 | legacy | 57 | 2445 | ekzekutohet, por 2445 > 1232 bajt: s'dërgohet | 239595 |
| 7 | 3 | v0 + ALT | 57 | 807 | OK | 239540 |
| 8 | 3 | legacy | 64 | 2742 | ekzekutohet, por 2742 > 1232 bajt: s'dërgohet | 273821 |
| 8 | 3 | v0 + ALT | 64 | 887 | OK | 273840 |
