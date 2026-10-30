"""Actual local-validator landing experiment, NOT worker-overlap instrumentation.

Three equal-size, three-signature workloads: independent, shared fee payer,
shared source with distinct payers. Fixtures are preloaded at genesis. All
balances are fake local tokens. No external RPC, wallet, or chain is used.
"""
import base64
from concurrent.futures import ThreadPoolExecutor
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import random
import shutil
import statistics
import struct
import subprocess
import tempfile
import time
import urllib.request

from solders.hash import Hash
from solders.keypair import Keypair
from solders.pubkey import Pubkey
from solders.transaction import Transaction

from fetch_token_elf import ELF_SHA, fetch
from model import exact_maximum, graph
from protocol_probe import TOKEN, footprint, transfer

ROOT = Path(__file__).resolve().parent
OUT = ROOT / "results" / "validator"
RPC = "http://127.0.0.1:18899"
SYSTEM = "11111111111111111111111111111111"
N = 8
ROUNDS = 30
WARMUP = 2
OPEN = urllib.request.build_opener(urllib.request.ProxyHandler({}))


def account_bytes(mint, owner, balance):
    """Classic SPL fixture layout; contains no private key or runtime dependency."""
    data = bytearray(165)
    data[:32], data[32:64] = bytes(mint), bytes(owner)
    struct.pack_into("<Q", data, 64, balance)
    data[108] = 1
    return bytes(data)


def rpc(method, params=()):
    request = urllib.request.Request(RPC, json.dumps({"jsonrpc": "2.0", "id": 1,
        "method": method, "params": list(params)}).encode(), {"Content-Type": "application/json"})
    with OPEN.open(request, timeout=10) as response:
        data = json.load(response)
    if "error" in data:
        raise RuntimeError(f"{method}: {data['error']}")
    return data["result"]


def mint_bytes(supply):
    return bytes(36) + struct.pack("<Q", supply) + bytes([6, 1]) + bytes(36)


def fixture(directory):
    mint_x, mint_y = Pubkey.new_unique(), Pubkey.new_unique()
    accounts, token_state, cases, supplies = {}, {}, {}, {str(mint_x): 0, str(mint_y): 0}

    def add(key, owner, data, lamports):
        accounts[str(key)] = {"pubkey": str(key), "account": {"lamports": lamports,
            "data": [base64.b64encode(data).decode(), "base64"], "owner": str(owner),
            "executable": False, "rentEpoch": 0}}

    def wallet():
        kp = Keypair()
        add(kp.pubkey(), SYSTEM, b"", 1_000_000_000)
        return kp

    def token(owner, mint, balance=100_000_000):
        key = Pubkey.new_unique()
        add(key, TOKEN, account_bytes(mint, owner.pubkey(), balance), 10_000_000)
        token_state[str(key)] = {"mint": str(mint), "amount": balance}
        supplies[str(mint)] += balance
        return key

    for name in ("independent", "shared_payer", "shared_source"):
        common_payer, common_a = wallet(), wallet()
        common_source = token(common_a, mint_x) if name == "shared_source" else None
        pairs = []
        for _ in range(N):
            a = common_a if name == "shared_source" else wallet()
            b = wallet()
            payer = common_payer if name == "shared_payer" else wallet()
            ax = common_source if common_source is not None else token(a, mint_x)
            ay, bx, by = token(a, mint_y), token(b, mint_x), token(b, mint_y)
            pairs.append((a, b, payer, ax, ay, bx, by))
        cases[name] = pairs
    # Dedicated negative-control pair, excluded from performance workloads.
    a, b, payer = wallet(), wallet(), wallet()
    negative = (a, b, payer, token(a, mint_x, 1000), token(a, mint_y, 1000),
                token(b, mint_x, 1000), token(b, mint_y, 19))
    for mint in (mint_x, mint_y):
        add(mint, TOKEN, mint_bytes(supplies[str(mint)]), 10_000_000)
    directory.mkdir()
    for address, image in accounts.items():
        (directory / f"{address}.json").write_text(json.dumps(image))
    return (mint_x, mint_y), cases, negative, token_state, supplies


def build(mints, pair, blockhash, sell_b=20):
    a, b, payer, ax, ay, bx, by = pair
    instructions = [transfer(ax, mints[0], bx, a.pubkey(), 10),
                    transfer(by, mints[1], ay, b.pubkey(), sell_b)]
    return Transaction.new_signed_with_payer(instructions, payer.pubkey(), [a, b, payer], blockhash)


def account_views(addresses):
    result = {}
    for start in range(0, len(addresses), 100):
        group = addresses[start:start + 100]
        images = rpc("getMultipleAccounts", [group, {"encoding": "base64", "commitment": "confirmed"}])["value"]
        for key, image in zip(group, images):
            assert image is not None, f"missing account {key}"
            result[key] = image
    return result


def token_balances(token_state):
    views = account_views(list(token_state))
    result = {}
    for key, image in views.items():
        data = base64.b64decode(image["data"][0])
        assert image["owner"] == str(TOKEN) and len(data) == 165
        assert str(Pubkey.from_bytes(data[:32])) == token_state[key]["mint"]
        result[key] = struct.unpack_from("<Q", data, 64)[0]
    return result


def submit(tx):
    row = {"signature": str(tx.signatures[0]), "submit_start_ns": time.perf_counter_ns(),
           "serialized_bytes": len(bytes(tx))}
    try:
        signature = rpc("sendTransaction", [base64.b64encode(bytes(tx)).decode(),
            {"encoding": "base64", "skipPreflight": True, "maxRetries": 0}])
        assert signature == row["signature"]
        row["rpc_error"] = None
    except Exception as error:
        # A timeout is not evidence that the transaction did not land.
        row["rpc_error"] = str(error)
    row["submit_response_ns"] = time.perf_counter_ns()
    return row


def confirm(rows, timeout=30):
    deadline, pending = time.monotonic() + timeout, {r["signature"]: r for r in rows}
    while pending and time.monotonic() < deadline:
        signatures = list(pending)
        values = rpc("getSignatureStatuses", [signatures, {"searchTransactionHistory": True}])["value"]
        stamp = time.perf_counter_ns()
        for signature, status in zip(signatures, values):
            if status and status.get("confirmationStatus") in ("confirmed", "finalized"):
                row = pending.pop(signature)
                row.update(confirmed_ns=stamp, slot=status["slot"], err=status["err"])
        if pending:
            time.sleep(.02)
    for row in pending.values():
        row.update(confirmed_ns=None, err="UNKNOWN_AFTER_TIMEOUT")
    return rows


def detail(row):
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        data = rpc("getTransaction", [row["signature"], {"encoding": "json", "commitment": "confirmed",
                                                        "maxSupportedTransactionVersion": 0}])
        if data:
            meta = data["meta"]
            row.update(cu=meta.get("computeUnitsConsumed"), fee=meta["fee"], logs=meta.get("logMessages"),
                       transaction_error=meta["err"])
            assert row["transaction_error"] == row["err"]
            return row
        time.sleep(.03)
    raise AssertionError(f"confirmed transaction metadata unavailable: {row['signature']}")


def fresh_hash(previous):
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        value = rpc("getLatestBlockhash", [{"commitment": "confirmed"}])["value"]["blockhash"]
        if value not in previous:
            previous.add(value)
            return Hash.from_string(value)
        time.sleep(.025)
    raise AssertionError("blockhash did not advance; no duplicate-message workaround used")


def percentile(values, p):
    ordered = sorted(values)
    return ordered[max(0, math.ceil(len(ordered) * p) - 1)]


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    binary = shutil.which("solana-test-validator")
    assert binary, "solana-test-validator required; no runtime test is skipped"
    elf = fetch()
    version = subprocess.check_output([binary, "--version"], text=True).strip()
    started, process = time.monotonic(), None
    raw_rows, bursts = [], []
    rng = random.Random(8642)
    with tempfile.TemporaryDirectory(prefix="direct-exchange-validator-") as temporary:
        work = Path(temporary)
        mints, cases, negative, expected, supplies = fixture(work / "accounts")
        args = [binary, "--ledger", str(work / "ledger"), "--bind-address", "127.0.0.1",
                "--rpc-port", "18899", "--faucet-port", "18902", "--gossip-port", "18903",
                "--dynamic-port-range", "19000-19100", "--account-dir", str(work / "accounts"),
                "--bpf-program", str(TOKEN), str(elf), "--log"]
        manifest = {"version": version, "validator_sha256": hashlib.sha256(Path(binary).read_bytes()).hexdigest(),
                    "token_elf_sha256": ELF_SHA, "platform": platform.platform(), "cpu_count": os.cpu_count(),
                    "rounds_per_workload": ROUNDS, "warmup_per_workload": WARMUP, "burst_size": N,
                    "ordering_seed": 8642, "fixture_method": "genesis account preload", "argv": args,
                    "scheduler_worker_overlap_instrumented": False,
                    "scope": "observed localnet burst landing/confirmation, not saturated maximum TPS or mainnet"}
        (OUT / "manifest.json").write_text(json.dumps(manifest, indent=2))
        with (OUT / "validator.log").open("w") as log:
            try:
                process = subprocess.Popen(args, stdout=log, stderr=subprocess.STDOUT)
                for _ in range(180):
                    assert process.poll() is None, "validator exited; inspect validator.log"
                    try:
                        if rpc("getHealth") == "ok":
                            break
                    except Exception:
                        pass
                    time.sleep(.5)
                else:
                    raise AssertionError("validator health timeout")
                program = account_views([str(TOKEN)])[str(TOKEN)]
                assert hashlib.sha256(base64.b64decode(program["data"][0])).hexdigest() == ELF_SHA
                before_mints = account_views([str(m) for m in mints])
                assert token_balances(expected) == {k: v["amount"] for k, v in expected.items()}
                used_hashes, structural = set(), {}
                with ThreadPoolExecutor(max_workers=N) as pool:
                    for round_id in range(-WARMUP, ROUNDS):
                        order = list(cases)
                        rng.shuffle(order)
                        for name in order:
                            blockhash = fresh_hash(used_hashes)
                            txs = [build(mints, pair, blockhash) for pair in cases[name]]
                            if name not in structural:
                                adjacency = graph([footprint(tx, str(i)) for i, tx in enumerate(txs)])
                                maximum = len(exact_maximum(adjacency))
                                assert maximum == (N if name == "independent" else 1)
                                structural[name] = maximum
                            start_ns = time.perf_counter_ns()
                            rows = confirm(list(pool.map(submit, txs)))
                            end_ns = time.perf_counter_ns()
                            for index, row in enumerate(rows):
                                row.update(workload=name, round=round_id, index=index)
                                raw_rows.append(row)
                            assert all(r["err"] is None for r in rows), f"unsettled workload: {rows}"
                            list(pool.map(detail, rows))
                            assert all(r["cu"] and r["cu"] > 0 for r in rows)
                            for pair in cases[name]:
                                _, _, _, ax, ay, bx, by = pair
                                for key, delta in ((ax, -10), (ay, 20), (bx, 10), (by, -20)):
                                    expected[str(key)]["amount"] += delta
                            assert token_balances(expected) == {k: v["amount"] for k, v in expected.items()}
                            bursts.append({"workload": name, "round": round_id,
                                           "confirmation_window_ms": (end_ns-start_ns)/1e6,
                                           "confirmed_exchanges_per_second_in_window": N/((end_ns-start_ns)/1e9)})
                            print(f"validator {name} round={round_id} confirmed={N}", flush=True)
                assert account_views([str(m) for m in mints]) == before_mints
                for mint, supply in supplies.items():
                    assert sum(v["amount"] for v in expected.values() if v["mint"] == mint) == supply
                # Real validator rollback control: second token leg lacks funds.
                snapshot = token_balances(expected)
                payer = str(negative[2].pubkey())
                payer_before = account_views([payer])[payer]["lamports"]
                tx = build(mints, negative, fresh_hash(used_hashes), sell_b=1001)
                failed = detail(confirm([submit(tx)])[0])
                assert failed["err"] == {"InstructionError": [1, {"Custom": 1}]}, failed
                assert failed["logs"].count(f"Program {TOKEN} success") == 1
                assert token_balances(expected) == snapshot
                assert payer_before-account_views([payer])[payer]["lamports"] == failed["fee"] > 0
                raw_rows.append(dict(failed, workload="negative_second_leg", round=0))
                summary = {}
                for name in cases:
                    rows = [r for r in raw_rows if r["workload"] == name and r["round"] >= 0]
                    windows = [b for b in bursts if b["workload"] == name and b["round"] >= 0]
                    latency = [(r["confirmed_ns"]-r["submit_start_ns"])/1e6 for r in rows]
                    rates = [b["confirmed_exchanges_per_second_in_window"] for b in windows]
                    assert len(rows) == N*ROUNDS
                    summary[name] = {"successes": len(rows), "failures": 0, "unknown": 0,
                        "latency_ms_p50": statistics.median(latency), "latency_ms_p95": percentile(latency,.95),
                        "latency_ms_p99": percentile(latency,.99),
                        "median_confirmed_exchanges_per_second_in_burst_window": statistics.median(rates),
                        "cu_values": sorted({r["cu"] for r in rows}), "fee_values": sorted({r["fee"] for r in rows}),
                        "serialized_bytes_values": sorted({r["serialized_bytes"] for r in rows}),
                        "max_nonconflicting_from_real_messages": structural[name]}
                assert len({tuple(v["serialized_bytes_values"]) for v in summary.values()}) == 1
                report = {"status": "PASS", "workloads": summary, "negative_second_leg_rollback": "PASS",
                          "warmup_transactions": WARMUP*N*len(cases), "measured_transactions": ROUNDS*N*len(cases),
                          "scheduler_overlap_proven": False, "eightfold_speedup_proven": False,
                          "interpretation": "low-load burst confirmation includes block phase and RPC polling; not peak TPS"}
                (OUT / "validator_results.json").write_text(json.dumps(report, indent=2))
                print(json.dumps(report, indent=2), flush=True)
            finally:
                (OUT / "transactions.json").write_text(json.dumps(raw_rows, indent=2))
                (OUT / "bursts.json").write_text(json.dumps(bursts, indent=2))
                if process and process.poll() is None:
                    process.terminate()
                    try:
                        process.wait(timeout=15)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait(timeout=10)
                (OUT / "cleanup.json").write_text(json.dumps({"validator_running": bool(process and process.poll() is None),
                    "elapsed_seconds": time.monotonic()-started}, indent=2))
    report_text = "# Actual local validator: observed burst landing\n\n" + json.dumps(report, indent=2)
    (OUT / "VALIDATOR_REPORT.md").write_text(report_text)
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a") as stream:
            stream.write("\n## Local validator\n\nAll720 measured transactions and rollback control passed. "
                         "Scheduler worker overlap and8x speedup remain unproven.\n")


if __name__ == "__main__":
    main()
