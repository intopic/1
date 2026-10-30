# Validator scheduler gate: NOT PASSED; serial SPL SBF gate: PASSED IN CI

Initial local Windows attempts failed before executing transactions. The
subsequent Linux CI run executed real SPL SBF in LiteSVM successfully:
https://github.com/intopic/1/actions/runs/40983318078
Eight bilateral exchanges used12,293 CU each; atomic rollback, insufficient
funds, replay and tampered signatures passed. No validator scheduler overlap,
TPS or8x speedup was measured. The scheduler gate below remains required.

## Historical local Windows attempts

- Native Agave4.3.2 version/help succeed. Two fresh temporary localhost
  validator startups (normal and escalated) both fail unpacking genesis:
  `Access is denied. (os error5)`. Both processes exited; no service left.
- Ports18899/18900,18902/18903 and19000-19100 were isolated from project
  configuration. No existing ledger reset or wallet configuration changed.
- Isolated Python3.12 venv uses solders0.27.1. Wire/signature checks work;
  LiteSVM import fails: native Windows module lacks FeatureSet/LiteSVM.
- No WSL distribution is installed. Cached newer Agave Rust dependencies
  require a compiler newer than installed1.95, so a cached source directory
  does not constitute a working runtime.
- No GitHub Actions, remote RPC, mainnet/devnet funding or deployment used.

## Reproducible experiment required on a working Linux runtime

1. Pin compiler, Agave, feature set, classic SPL Token ELF and hashes. Run
   a temporary local validator with a fresh ledger and isolated ports. Record
   host CPU, banking-thread configuration and competing load. LiteSVM can
   validate SPL atomicity separately, but cannot establish scheduler overlap.
2. Initialize actual token mints and user accounts with System/SPL instructions;
   mint supplies must equal known account balances. Set up8 independent
   pairs of owners, each with two mints and four token accounts. Fund SOL
   before timing. Generate keys for testing only; never use user wallets.
3. Submit transactions containing A_X->B_X and B_Y->A_Y TransferChecked,
   signed by both owners. Verify actual post-balances and successful logs.
4. Require failure and complete SPL rollback when leg2 is underfunded,
   frozen or unauthorized. Separately account for SOL fees on failed tx.
   Verify tampered signatures, wrong mint/decimals/authority and replay.
5. Compare equal workloads: (a) independent pairs/payers, (b) common payer,
   (c) common source with distinct payers. Do not divide one pool's capital
   into8 new reserves to manufacture the independent workload.
6. Use identical payload lengths and budgets; warm up; repeat at least30
   bursts per workload, randomize order, record every signature and timeout.
   Blockhash freshness and duplicate transactions must not confound results.
7. Save monotonic send/response/confirmation timestamps, slots, err, CU,
   pre/post token balances, and unknown outcomes. Report successful orders/s,
   fees/success, p50/p95/p99 latency and failures, not just RPC submissions/s.
8. To claim execution parallelism, instrument actual scheduler/worker
   intervals or equivalent runtime evidence. Concurrent RPC requests and
   same-slot inclusion alone are insufficient proof of simultaneous execution.
9. Compare the incremental value of the selector with Solana's existing
   scheduler and basic greedy ordering. Independent transfers are already
   supported; showing8 independent transactions alone is not an innovation.

Acceptance: no unauthorized transfer, double-spend, negative/overflow balance,
or business-state commit after failure; complete outcome accounting; a measured
benefit over a fair baseline. If economics/fairness or runtime evidence fails,
reject the candidate mechanism rather than claiming stronger safety.
