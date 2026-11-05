#!/usr/bin/env python3
"""Compare active prover workers on ONE host, with full proof verification.

Uses synthetic signed sessions, not live bank/TLSNotary traffic. Each proof is
verified after the timed HTTP batch so verifier CPU does not contaminate proving
throughput. A replica estimate is arithmetic, not a measured Fly fleet.
"""
import argparse
import calendar
import concurrent.futures as cf
import datetime as dt
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import secrets
import signal
import socket
import statistics
import subprocess
import tempfile
import threading
import time
import urllib.error
import urllib.request

B58 = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
ROOT = Path(__file__).resolve().parents[1]


def b58(raw):
    value, out = int.from_bytes(raw, "big"), ""
    while value:
        value, remainder = divmod(value, 58)
        out = B58[remainder] + out
    return "1" * (len(raw) - len(raw.lstrip(b"\0"))) + out


def percentile(values, fraction):
    if not values:
        return None
    return sorted(values)[max(0, math.ceil(fraction * len(values)) - 1)]


def age_cutoff(day):
    year = day.year - 18
    date = day.replace(year=year, day=min(day.day, calendar.monthrange(year, day.month)[1]))
    return date.year * 10000 + date.month * 100 + date.day


def validate_binding(proof, client, app_id, bank_hash, finished_day):
    """Detect swapped clients/claims even when every proof is mathematically valid."""
    inputs = [int(value, 16) for value in proof["public_inputs"]]
    if len(inputs) != 10 or proof["backend"] != "circom-rapidsnark":
        raise ValueError("wrong backend or public input count")
    raw = client["wallet_bytes"]
    claim = "date_of_birth" if client["kind"] == "age" else "balance"
    expected_claim = int.from_bytes(hashlib.sha256(claim.encode()).digest()[:8], "big")
    if proof["wallet"] != client["wallet"] or inputs[8:] != [int.from_bytes(raw[:16], "big"), int.from_bytes(raw[16:], "big")]:
        raise ValueError("wallet mismatch")
    if inputs[1] != client["attested_at"] or inputs[2] != expected_claim:
        raise ValueError("timestamp or claim mismatch")
    if inputs[4] != bank_hash or inputs[7] != app_id:
        raise ValueError("source or application mismatch")
    expected_min = 19000101 if client["kind"] == "age" else 10000
    expected_max = ({age_cutoff(client["started_day"]), age_cutoff(finished_day)}
                    if client["kind"] == "age" else {(1 << 64) - 1})
    if inputs[5] != expected_min or inputs[6] not in expected_max:
        raise ValueError("range mismatch")
    if int(proof["nullifier"], 16) != inputs[3]:
        raise ValueError("nullifier metadata mismatch")


def post(url, path, body, api_key):
    request = urllib.request.Request(url + path, json.dumps(body).encode(),
                                     {"content-type": "application/json", "x-api-key": api_key})
    with urllib.request.urlopen(request, timeout=120) as response:
        return json.load(response)


def cpu_seconds(pid):
    try:
        fields = Path(f"/proc/{pid}/stat").read_text().rpartition(")")[2].split()
        return sum(int(value) for value in fields[11:15]) / os.sysconf("SC_CLK_TCK")
    except (OSError, ValueError, AttributeError):
        return None


def process_tree(pid):
    found, pending = set(), [pid]
    while pending:
        current = pending.pop()
        if current in found:
            continue
        found.add(current)
        try:
            # Command::spawn runs on request threads, not necessarily the main TID.
            for thread in Path(f"/proc/{current}/task").iterdir():
                try:
                    pending.extend(map(int, (thread / "children").read_text().split()))
                except OSError:
                    pass
        except OSError:
            pass
    return found


class MemorySampler:
    def __init__(self, pid):
        self.pid = pid
        self.peak_kib = None
        self.stop_event = threading.Event()
        self.thread = threading.Thread(target=self.sample, daemon=True)

    def sample(self):
        while not self.stop_event.is_set():
            total, observed = 0, False
            for pid in process_tree(self.pid):
                try:
                    for line in Path(f"/proc/{pid}/status").read_text().splitlines():
                        if line.startswith("VmRSS:"):
                            total += int(line.split()[1])
                            observed = True
                except OSError:
                    pass
            if observed:
                self.peak_kib = max(self.peak_kib or 0, total)
            self.stop_event.wait(0.05)


def cli(binary, *args):
    return subprocess.check_output([str(binary), *map(str, args)], cwd=ROOT, text=True).strip()


def wait_ready(process, url):
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError("prover exited during startup; inspect its log")
        try:
            with urllib.request.urlopen(url + "/health", timeout=1) as response:
                health = json.load(response)
            if health.get("backend") != "circom-rapidsnark":
                raise RuntimeError(f"unexpected health response: {health}")
            return
        except (OSError, urllib.error.URLError):
            time.sleep(0.1)
    raise RuntimeError("prover health timeout")


def prepare_clients(url, api_key, kind, count, prefix):
    clients = []
    for index in range(count):
        selected = ("age" if index % 2 else "balance") if kind == "mixed" else kind
        raw = secrets.token_bytes(32)
        wallet = b58(raw)
        session = post(url, "/mock-session", {"kind": selected, "balance": 10000 + index,
                       "dob": "1990-01-01", "account_id": f"{prefix}-{index}", "wallet": wallet}, api_key)
        message = bytes.fromhex(session["attestation"]["message_hex"])
        clients.append({"wallet": wallet, "wallet_bytes": raw, "kind": selected,
                        "attested_at": int.from_bytes(message[16:24], "big"),
                        "started_day": dt.datetime.now(dt.timezone.utc).date(),
                        "session": session})
    return clients


def request_proof(url, api_key, client):
    request = {"session": client["session"], "app": "parallel-v3"}
    request.update({"min_age": 18} if client["kind"] == "age" else {"min": 10000})
    start = time.perf_counter()
    try:
        proof = post(url, "/prove", request, api_key)
        return {"latency_ms": (time.perf_counter() - start) * 1000, "proof": proof}
    except (OSError, ValueError, urllib.error.URLError) as error:
        return {"error": str(error), "latency_ms": (time.perf_counter() - start) * 1000}


def trial(args, process, url, api_key, clients, app_id, bank_hash, workers, kind, repeat):
    for client in clients[:args.warmup]:
        result = request_proof(url, api_key, client)
        if "error" in result:
            raise RuntimeError(f"warmup failed: {result['error']}")
    sampler = MemorySampler(process.pid)
    sampler.thread.start()
    before = cpu_seconds(process.pid)
    start = time.perf_counter()
    try:
        with cf.ThreadPoolExecutor(max_workers=args.clients) as pool:
            results = list(pool.map(lambda client: request_proof(url, api_key, client), clients))
        elapsed = time.perf_counter() - start
        after = cpu_seconds(process.pid)
    finally:
        sampler.stop_event.set()
        sampler.thread.join()
    errors, nullifiers, latencies = [], set(), []
    with tempfile.TemporaryDirectory(prefix="parallel-verify-") as temp:
        for index, (client, result) in enumerate(zip(clients, results)):
            try:
                if "error" in result:
                    raise ValueError(result["error"])
                proof = result["proof"]
                validate_binding(proof, client, app_id, bank_hash, dt.datetime.now(dt.timezone.utc).date())
                if proof["nullifier"] in nullifiers:
                    raise ValueError("duplicate nullifier across distinct source accounts")
                nullifiers.add(proof["nullifier"])
                path = Path(temp) / f"{index}.json"
                path.write_text(json.dumps(proof), encoding="utf-8")
                verification = subprocess.run([str(args.bin), "verify", "--proof", str(path), "--vk", str(args.vk)],
                                              capture_output=True, text=True, timeout=30)
                if verification.returncode:
                    raise ValueError("cryptographic verification failed")
                latencies.append(result["latency_ms"])
            except (KeyError, ValueError, TypeError, subprocess.SubprocessError) as error:
                errors.append({"index": index, "error": str(error)[:240]})
    cpu = after - before if before is not None and after is not None else None
    return {"workers": workers, "kind": kind, "repeat": repeat, "requests": len(clients),
            "clients": args.clients, "valid_proofs": len(latencies), "errors": errors,
            "wall_seconds": elapsed, "valid_proofs_per_second": len(latencies) / elapsed,
            "latency_ms": latencies, "p50_ms": statistics.median(latencies) if latencies else None,
            "p95_ms": percentile(latencies, 0.95), "cpu_seconds": cpu,
            "cpu_seconds_per_valid_proof": cpu / len(latencies) if cpu is not None and latencies else None,
            "peak_sampled_sum_rss_kib_including_children": sampler.peak_kib}


def summary(report):
    lines = ["## Active prover workers on one host", "",
             "Synthetic sessions; every timed proof is verified afterwards. No live bank, MPC-TLS, Fly fleet or Solana transaction throughput is measured.", "",
             f"Host: {report['host']}", "",
             "| Workers | Claim | Valid/requests | proofs/s | p50 ms | p95 ms | CPU s/proof | sampled RSS sum MiB |",
             "|---:|---|---:|---:|---:|---:|---:|---:|"]
    rates = {}
    for workers in report["workers"]:
        for kind in report["kinds"]:
            rows = [row for row in report["trials"] if row["workers"] == workers and row["kind"] == kind]
            if not rows:
                continue
            valid = sum(row["valid_proofs"] for row in rows)
            count = sum(row["requests"] for row in rows)
            wall = sum(row["wall_seconds"] for row in rows)
            latencies = [value for row in rows for value in row["latency_ms"]]
            cpu = sum(row["cpu_seconds"] for row in rows) if all(row["cpu_seconds"] is not None for row in rows) else None
            memory = max((row["peak_sampled_sum_rss_kib_including_children"] for row in rows if row["peak_sampled_sum_rss_kib_including_children"] is not None), default=None)
            fmt = lambda value: f"{value:.2f}" if value is not None else "n/a"
            cpu_per_proof = cpu / valid if cpu is not None and valid else None
            lines.append(f"| {workers} | {kind} | {valid}/{count} | {valid / wall:.2f} | {fmt(statistics.median(latencies) if latencies else None)} | {fmt(percentile(latencies, .95))} | {fmt(cpu_per_proof)} | {fmt(memory / 1024 if memory is not None else None)} |")
            if kind == "mixed" and valid == count:
                rates[workers] = valid / wall
    if rates:
        best = max(rates, key=rates.get)
        rate = rates[best]
        lines += ["", f"Best measured mixed-case throughput on this host: **{rate:.2f} proofs/s**, {best} active workers.",
                  "", "### Arithmetic projection, not a measured fleet", "",
                  "Only if each replica sustains this rate independently and upstream services keep up:", "",
                  "| Equivalent machines | projected proofs/s |", "|---:|---:|"]
        lines.extend(f"| {replicas} | {replicas * rate:.2f} |" for replicas in (1, 2, 5, 10))
    lines += ["", "RSS is sampled every 50 ms and summed across daemon and children; shared pages may be counted twice. It is not an exact peak or unique-memory measurement."]
    if "fatal_error" in report:
        lines += ["", "ERROR: " + report["fatal_error"]]
    return "\n".join(lines) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin", type=Path, required=True)
    parser.add_argument("--prover", type=Path, required=True)
    parser.add_argument("--witness-bin", type=Path, required=True)
    parser.add_argument("--zkey", type=Path, required=True)
    parser.add_argument("--vk", type=Path, required=True)
    parser.add_argument("--workers", type=int, nargs="+", default=[1, 2, 4])
    parser.add_argument("--kinds", nargs="+", choices=["balance", "age", "mixed"], default=["balance", "age", "mixed"])
    parser.add_argument("--clients", type=int, default=16)
    parser.add_argument("--requests", type=int, default=128)
    parser.add_argument("--repeats", type=int, default=2)
    parser.add_argument("--warmup", type=int, default=4)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    if min(args.clients, args.requests, args.repeats, *args.workers) <= 0 or args.warmup < 0:
        parser.error("counts must be positive, warmup nonnegative")
    for name in ("bin", "prover", "witness_bin", "zkey", "vk"):
        setattr(args, name, getattr(args, name).expanduser().resolve(strict=True))
    args.out.parent.mkdir(parents=True, exist_ok=True)
    host = {"platform": platform.platform(), "logical_cpus": os.cpu_count(),
            "affinity_cpus": len(os.sched_getaffinity(0)) if hasattr(os, "sched_getaffinity") else None}
    if Path("/proc/cpuinfo").exists():
        host["cpu_model"] = next((line.split(":", 1)[1].strip() for line in Path("/proc/cpuinfo").read_text().splitlines() if line.startswith("model name")), "unknown")
    report = {"measured_at_utc": dt.datetime.now(dt.timezone.utc).isoformat(), "host": host,
              "workers": args.workers, "kinds": args.kinds, "trials": [],
              "method": "one host; sequential worker configurations; reversed order on alternate repeats; synthetic sessions; warmup excluded; all proofs verified outside timed HTTP batches"}
    try:
        app_id = int(cli(args.bin, "app-id", "--app", "parallel-v3"), 16)
        for repeat in range(args.repeats):
            order = args.workers if repeat % 2 == 0 else list(reversed(args.workers))
            for workers in order:
                with socket.socket() as reservation:
                    reservation.bind(("127.0.0.1", 0))
                    port = reservation.getsockname()[1]
                url = f"http://127.0.0.1:{port}"
                api_key = secrets.token_hex(24)
                env = dict(os.environ, PROVER_API_KEYS=api_key, PROVER_RATE_LIMIT="0")
                log_path = args.out.parent / f"workers-{workers}-repeat-{repeat}.log"
                command = [str(args.bin), "serve", "--addr", f"127.0.0.1:{port}", "--backend", "rapidsnark",
                           "--parallel", str(workers), "--rapidsnark", str(args.prover), "--witness-bin", str(args.witness_bin),
                           "--zkey", str(args.zkey), "--mock-notary-key", str(ROOT / "tests/fixtures/mock_notary_seed.hex")]
                with log_path.open("w", encoding="utf-8") as log:
                    process = subprocess.Popen(command, cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT,
                                               start_new_session=os.name == "posix")
                    try:
                        wait_ready(process, url)
                        for kind in args.kinds:
                            clients = prepare_clients(url, api_key, kind, args.requests, f"W{workers}-R{repeat}-{kind}")
                            with tempfile.TemporaryDirectory(prefix="parallel-key-") as temp:
                                session_path = Path(temp) / "session.json"
                                session_path.write_text(json.dumps(clients[0]["session"]), encoding="utf-8")
                                bank_hash = int(cli(args.bin, "bank-key-hash", "--input", session_path), 16)
                            result = trial(args, process, url, api_key, clients, app_id, bank_hash, workers, kind, repeat)
                            report["trials"].append(result)
                            print(f"workers={workers} kind={kind} repeat={repeat}: {result['valid_proofs_per_second']:.2f} valid proofs/s, errors={len(result['errors'])}", flush=True)
                    finally:
                        if os.name == "posix":
                            try:
                                os.killpg(process.pid, signal.SIGTERM)
                            except ProcessLookupError:
                                pass
                        elif process.poll() is None:
                            process.terminate()
                        try:
                            process.wait(timeout=10)
                        except subprocess.TimeoutExpired:
                            process.kill()
                            process.wait()
    except Exception as error:
        report["fatal_error"] = str(error)
        raise
    finally:
        args.out.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
        args.out.with_suffix(".md").write_text(summary(report), encoding="utf-8")
    if any(row["errors"] for row in report["trials"]):
        raise SystemExit("benchmark failed: rejected, mixed, duplicate or invalid proofs")


if __name__ == "__main__":
    main()
