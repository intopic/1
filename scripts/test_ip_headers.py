"""HTTP regressions for IP-header spoofing; uses a loopback DEV daemon."""
import contextlib
import http.client
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
BODY = json.dumps({"kind": "balance", "account_id": "ip-test", "wallet": "11111111111111111111111111111111", "balance": 10000}).encode()


@contextlib.contextmanager
def daemon(header):
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        port = sock.getsockname()[1]
    env = {k: v for k, v in os.environ.items() if not k.startswith("PROVER_")}
    cmd = [str(Path(sys.argv[1]).resolve()), "serve", "--backend", "dev", "--addr", f"127.0.0.1:{port}",
           "--pk", str(Path(sys.argv[2]).resolve()), "--mock-notary-key", "tests/fixtures/mock_notary_seed.hex",
           "--rate-limit-per-min", "1"]
    if header:
        cmd += ["--client-ip-header", header]
    with tempfile.TemporaryFile() as log:
        process = subprocess.Popen(cmd, cwd=ROOT, env=env, stdout=log, stderr=log)
        try:
            for _ in range(100):
                if process.poll() is not None:
                    raise AssertionError("HTTP test daemon exited")
                try:
                    conn = http.client.HTTPConnection("127.0.0.1", port, timeout=1)
                    conn.request("GET", "/health")
                    response = conn.getresponse()
                    response.read()
                    conn.close()
                    if response.status == 200:
                        break
                except OSError:
                    time.sleep(.1)
            else:
                raise AssertionError("HTTP daemon startup timeout")
            yield port
        finally:
            process.terminate()
            process.wait(timeout=10)


def post(port, headers):
    conn = http.client.HTTPConnection("127.0.0.1", port, timeout=5)
    conn.putrequest("POST", "/mock-session")
    conn.putheader("Content-Length", str(len(BODY)))
    for key, value in headers:
        conn.putheader(key, value)
    conn.endheaders(BODY)
    response = conn.getresponse()
    response.read()
    conn.close()
    return response.status


with daemon(None) as port:
    statuses = [post(port, [("X-Forwarded-For", f"1.1.1.{i}"), ("Fly-Client-IP", f"2.2.2.{i}")]) for i in range(1, 5)]
    assert statuses[0] == 200 and 429 in statuses, statuses

with daemon("fly-client-ip") as port:
    assert post(port, []) == 400
    assert post(port, [("Fly-Client-IP", "1.1.1.1, 2.2.2.2")]) == 400
    assert post(port, [("Fly-Client-IP", "1.1.1.1"), ("Fly-Client-IP", "2.2.2.2")]) == 400
    statuses = [post(port, [("Fly-Client-IP", "9.9.9.9"), ("X-Forwarded-For", f"3.3.3.{i}")]) for i in range(1, 5)]
    assert statuses[0] == 200 and 429 in statuses, statuses
    # Distinct proxy-supplied addresses retain independent buckets.
    assert post(port, [("Fly-Client-IP", "8.8.8.8")]) == 200

print("HTTP IP tests passed: untrusted spoofing cannot change the bucket; trusted headers reject chains, duplicates and omission.")
