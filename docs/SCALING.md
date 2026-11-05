# Prover parallelism and a pool of ten Fly Machines

Measured results: [2,304 verified proofs on one four-CPU runner](PARALLEL_RESULTS.md).

The ZK E2E workflow compares **1, 2 and 4 active proof workers on the same runner**.
For each setting it runs balance, age and mixed traffic: 128 distinct wallets/source
accounts per trial, 16 concurrent clients, two repeats, four excluded warmup requests.
Worker order reverses on the second repeat. Every timed proof is checked against its
request and cryptographically verified outside the timed batch. Any HTTP failure,
swapped request, duplicate nullifier or invalid proof fails the job.

Download `circom-artifacts/parallel/` for raw latency samples, logs and the Markdown
report. Metrics include valid proofs/s, median and nearest-rank p95, CPU seconds/proof,
and sampled RSS summed across daemon and prover/witness children. RSS is approximate:
50 ms sampling can miss peaks and shared pages can be counted twice.

These are synthetic signed sessions: **bank latency, MPC-TLS, Fly Proxy cold starts,
and Solana transaction capacity are not measured**. The earlier 8.75 proofs/s result
used one active proof worker on a four-logical-CPU runner. It is not a per-user limit.
The current Fly `performance-2x` VM has two vCPUs; measure it before extrapolating.
More workers can compete for CPU because Rapidsnark also uses internal threads.

## Existing-machine autostart, one to ten active provers

`fly.toml` keeps one prover warm in the primary region and enables autostart/autostop.
Request concurrency has soft limit 1 (matching the initial one active worker) and
hard limit 8. This signals pressure before the daemon's 32-waiting-request queue fills.
The old soft limit 64 exceeded that queue. Health requests also count as traffic;
this initial policy must be validated under real load. Adjust it with measured worker
capacity, not just the number of open TCP connections.

After deploying an authenticated, configured app, provision an existing pool:

```bash
fly scale count prover=10 --region fra -a <app>
fly scale show -a <app>
fly machine list -a <app>
```

Fly Proxy starts/stops machines from this existing pool as demand changes. This
configuration does **not create machines automatically** and TOML contains no fictional
`max_machines=10` setting. With exactly ten provisioned provers the pool provides up to
ten active provers; manual scaling or other automation can change that count. The
bridge and notary are additional machines, so ten provers is not ten machines total.
For production use separate apps for prover, bridge and notary as described in DEPLOY.md.

Before an announced airdrop, start the stopped prover IDs explicitly with
`fly machine start <id> -a <app>` and check health. Autostart can add seconds of cold
start latency. Idle machines are stopped later; this is not an instantaneous global
queue. Clients must handle 429/503 with bounded retry/backoff and avoid duplicate
claims. Rate limits are local to each replica, not a shared fleet-wide quota.

If one identical deployed VM sustains **R valid proofs/s**, an unconstrained fleet of
N such VMs has an arithmetic target near **N × R**. For illustration only, R=8.75 and
N=10 gives 87.5 proofs/s; 100,000 fresh proofs would take about 19 minutes at sustained
capacity. This is not a measured Fly result or an end-to-end guarantee. Ten current
prover VMs allocate 20 vCPUs and 40 GB RAM when running. CPU/RAM cost grows with active
machines; stopped root filesystems still cost money.

Scaling provers does not scale the default four simultaneous MPC-TLS sessions of one
notary. Bank endpoints, bridge/notary capacity, rate limits, RPC and writable Solana
accounts can become the next bottleneck. Age and balance share the circuit; balance
freshness still depends on application policy. Reusing a valid on-chain attestation
does not require another server proof for every airdrop claim.

No Fly deployment or ten-machine allocation is performed by the benchmark workflow.

References: [Fly autostop/autostart](https://docs.fly.io/launch/autostop-autostart),
[machine counts](https://docs.fly.io/launch/scale-count),
[service configuration](https://docs.fly.io/reference/configuration).
