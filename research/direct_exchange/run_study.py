"""Reproducible local model experiments; NOT Solana throughput benchmarks."""
import hashlib
import json
import platform
import random
import statistics
import sys
import time
from pathlib import Path

from model import (Candidate, bio_projected, competitive_projected, degree_greedy,
                   exact_maximum, first_fit, fixture, graph, independent,
                   local_inhibition)

ROOT = Path(__file__).resolve().parent
SELECTORS = {"first_fit": first_fit, "degree_greedy": degree_greedy,
             "local_inhibition": local_inhibition,
             "sigmoid_projected": bio_projected,
             "competitive_projected": competitive_projected}


def analyze(candidates):
    adjacency = graph(candidates)
    optimum = exact_maximum(adjacency)
    rows = {}
    for name, select in SELECTORS.items():
        begin = time.perf_counter_ns()
        chosen = select(adjacency)
        elapsed = time.perf_counter_ns() - begin
        assert independent(chosen, adjacency)
        rows[name] = {"selected": len(chosen), "solve_ns": elapsed,
                      "fraction_of_exact": len(chosen) / len(optimum)}
    return {"candidates": len(candidates), "conflict_edges": sum(map(len, adjacency)) // 2,
            "exact_maximum": len(optimum), "selectors": rows}


def synthetic_graph(n, p, seed):
    rng = random.Random(seed)
    rights = [set() for _ in range(n)]
    for i in range(n):
        for j in range(i):
            if rng.random() < p:
                key = f"shared:{i}:{j}"
                rights[i].add(key)
                rights[j].add(key)
    return [Candidate(str(i), frozenset({"policy"}), frozenset(r)) for i, r in enumerate(rights)]


def main():
    _, txs = fixture(8)
    free = [t.footprint() for t in txs]
    _, paid = fixture(8, shared_payer=True)
    cases = {"eight_independent": free,
             "same_fee_payer": [t.footprint() for t in paid],
             "same_treasury": [Candidate(t.name, t.reads, t.writes | {"treasury"}) for t in free],
             "same_source_account": [Candidate(t.name, t.reads, t.writes | {"source"}) for t in free],
             "four_shared_resources_two_candidates_each": [
                 Candidate(t.name, t.reads, t.writes | {f"resource:{i // 2}"})
                 for i, t in enumerate(free)],
             "adversarial_star": [Candidate("center", frozenset(), frozenset(map(str, range(8))))] +
                 [Candidate(f"leaf:{i}", frozenset(), frozenset({str(i)})) for i in range(8)]}
    results = {name: analyze(candidates) for name, candidates in cases.items()}
    samples = []
    for p in (.1, .3, .7):
        for seed in range(32):
            samples.append({"density_parameter": p, "seed": seed,
                            **analyze(synthetic_graph(16, p, seed))})
    aggregates = {}
    for name in SELECTORS:
        fractions = [r["selectors"][name]["fraction_of_exact"] for r in samples]
        times = [r["selectors"][name]["solve_ns"] for r in samples]
        aggregates[name] = {"mean_fraction_of_exact": statistics.mean(fractions),
                            "minimum_fraction_of_exact": min(fractions),
                            "median_solve_us": statistics.median(times) / 1000}
    report = {"experiment": "synthetic account-lock graph + integer settlement model",
              "not_measured": ["Solana scheduler parallelism", "onchain throughput", "MEV resilience",
                               "cryptographic order authorization", "production contract safety"],
              "platform": platform.platform(), "python": sys.version,
              "source_sha256": {p.name: hashlib.sha256(p.read_bytes()).hexdigest()
                                for p in ROOT.glob("*.py")},
              "cases": results, "random_graph_count": len(samples),
              "random_graphs": samples, "aggregate": aggregates}
    output = ROOT / "results"
    output.mkdir(exist_ok=True)
    (output / "model_results.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"cases": {k: v["exact_maximum"] for k, v in results.items()},
                      "aggregate": aggregates}, indent=2))


if __name__ == "__main__":
    main()
