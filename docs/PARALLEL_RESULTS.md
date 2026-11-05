# Measured prover parallelism

All **2,304 timed proofs passed** mathematical verification and request-binding
checks. These results use one four-logical-CPU GitHub runner, full v3 Circom proofs,
Rapidsnark v0.0.8 and the C++ witness generator. They are not Fly deployment results.

With mixed traffic, two active workers improved throughput about **19%** over one;
four improved it about **26%**. Four only improved throughput about **6%** over two,
while mixed-case p95 rose from **1.41 s to 2.28 s**. Two workers are a useful starting
point on this measured four-CPU host when tail latency matters. The default Fly VM
has two vCPUs and must be measured separately before changing its worker count.

Latency includes queueing under **16 concurrent clients and 128 requests per trial**.
It is not the latency of one isolated proof. Each table row pools two trials (256
proofs); worker order reverses on the second repetition. Percentiles use all raw
samples; throughput is valid count divided by summed timed batch durations.

The earlier 8.75 proofs/s benchmark used 16 requests per case on another runner.
This run's 9.74 proofs/s one-worker baseline is a new measurement, not an improvement
attributed solely to changing worker count. Hardware and batch size differ.

## Active prover workers on one host

Synthetic sessions; every timed proof is verified afterwards. No live bank, MPC-TLS, Fly fleet or Solana transaction throughput is measured.

Host: {'platform': 'Linux-7.0.0-1019-azure-x86_64-with-glibc2.43', 'logical_cpus': 4, 'affinity_cpus': 4, 'cpu_model': 'AMD EPYC 9V74 80-Core Processor'}

| Workers | Claim | Valid/requests | proofs/s | p50 ms | p95 ms | CPU s/proof | sampled RSS sum MiB |
|---:|---|---:|---:|---:|---:|---:|---:|
| 1 | balance | 256/256 | 9.74 | 1636.59 | 1664.74 | 0.31 | 18.36 |
| 1 | age | 256/256 | 9.72 | 1645.35 | 1687.24 | 0.31 | 18.42 |
| 1 | mixed | 256/256 | 9.74 | 1639.44 | 1670.34 | 0.31 | 18.43 |
| 2 | balance | 256/256 | 11.55 | 1383.86 | 1413.97 | 0.31 | 29.61 |
| 2 | age | 256/256 | 11.59 | 1377.25 | 1405.58 | 0.31 | 29.70 |
| 2 | mixed | 256/256 | 11.58 | 1376.28 | 1409.96 | 0.31 | 29.71 |
| 4 | balance | 256/256 | 12.34 | 1287.03 | 2249.19 | 0.32 | 49.61 |
| 4 | age | 256/256 | 12.35 | 1282.29 | 2276.03 | 0.32 | 51.72 |
| 4 | mixed | 256/256 | 12.24 | 1283.51 | 2278.01 | 0.32 | 51.44 |

Best measured mixed-case throughput on this host: **12.24 proofs/s**, 4 active workers.

### Arithmetic projection, not a measured fleet

Only if each replica sustains this rate independently and upstream services keep up:

| Equivalent machines | projected proofs/s |
|---:|---:|
| 1 | 12.24 |
| 2 | 24.47 |
| 5 | 61.18 |
| 10 | 122.35 |

RSS is sampled every 50 ms and summed across daemon and children; shared pages may be counted twice. It is not an exact peak or unique-memory measurement.

Ten replicas matching this runner and its four-worker configuration would have an
arithmetic target of **122.35 proofs/s**: about **13.6 minutes for 100,000 fresh proofs**.
Using the two-worker configuration gives about **115.76 proofs/s** for ten equivalent
replicas. These are conditional projections, not measured fleet or end-to-end rates.
Do not apply them directly to the two-vCPU Fly VM in `fly.toml`.

See [the source job](https://github.com/intopic/1/actions/runs/42089349039/job/127462977226),
[raw samples](measurements/parallel-linux-ci.json) and [Fly pool setup](SCALING.md).
The Fly image and TLSNotary bridge have separate validation jobs; check the workflow
status for their conclusions. No Fly machines were provisioned by these tests.
