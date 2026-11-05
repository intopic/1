#!/usr/bin/env python3
"""Test ngarkese + izolimi për `coprocessor serve` (me `--mock-notary-key`).

N klientë me portofola dhe llogari të ndryshme dërgojnë /prove paralelisht.
Kontrollon që çdo provë i përket klientit të vet (portofoli) dhe që nullifier-ët
janë unikë. Përdorimi: load_test.py [URL] [N] [paralelizmat, p.sh. 1,4,16]
"""
import json, os, sys, time, statistics, urllib.request, concurrent.futures as cf, subprocess
URL = sys.argv[1] if len(sys.argv) > 1 else "http://127.0.0.1:8787"
B58 = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz'
def b58(b):
    n = int.from_bytes(b, 'big'); s = ''
    while n: n, r = divmod(n, 58); s = B58[r] + s
    return '1' * (len(b) - len(b.lstrip(b'\0'))) + s
def unb58(s):
    n = 0
    for c in s: n = n * 58 + B58.index(c)
    return n.to_bytes(32, 'big')
def owns(p, w):
    """Hyrjet publike 8/9 (gjysmat e portofolit) duhet të jenë portofoli i klientit."""
    b = unb58(w)
    return (int(p["public_inputs"][8], 16) == int.from_bytes(b[:16], 'big')
            and int(p["public_inputs"][9], 16) == int.from_bytes(b[16:], 'big'))
def post(path, body):
    r = urllib.request.Request(URL + path, json.dumps(body).encode(), {"content-type": "application/json"})
    with urllib.request.urlopen(r, timeout=600) as f: return json.loads(f.read())
try:
    pid = int(os.environ.get("PID") or subprocess.check_output(["pgrep", "-x", "coprocessor"]).split()[0])
except Exception:
    pid = None  # daemon-i në një makinë tjetër: pa matje CPU/RAM
def cpu():
    """Sekonda CPU (user+sys) të daemon-it + proceseve bijë të përfunduara
    (rapidsnark dhe dëshmitari C++ janë procese më vete)."""
    if pid is None: return 0.0
    f = open(f"/proc/{pid}/stat").read().split(")")[-1].split()
    return sum(int(x) for x in f[11:15]) / os.sysconf("SC_CLK_TCK")
def rss():
    if pid is None: return "?"
    for l in open(f"/proc/{pid}/status"):
        if l.startswith("VmHWM"): return int(l.split()[1]) // 1024
N = int(sys.argv[2]) if len(sys.argv) > 2 else 64
LEVELS = [int(x) for x in (sys.argv[3] if len(sys.argv) > 3 else "1,4,16,32,64").split(",")]
clients = []
for i in range(N):
    w = b58(os.urandom(32)); acct = f"AL{i:026d}"
    kind = "age" if i % 2 else "balance"
    s = post("/mock-session", {"kind": kind, "balance": 10000 + i, "dob": "1990-01-01", "account_id": acct, "wallet": w})
    claim = {"min_age": 18} if kind == "age" else {"min": 10000}
    clients.append((w, acct, {"session": s, "app": "load", **claim}))
def one(c):
    w, acct, req = c; t = time.perf_counter(); p = post("/prove", req)
    return time.perf_counter() - t, w, acct, p
if os.environ.get("MD"):
    print("| paralel | kërkesa | p50 ms | p95 ms | prova/s | CPU s/provë | përzierje |\n|---|---|---|---|---|---|---|")
else:
  print(f"{'paralel':>8} {'kërkesa':>8} {'p50 ms':>8} {'p95 ms':>8} {'prova/s':>8} {'CPU s/provë':>11} {'gabime':>7}")
failed = 0
for conc in LEVELS:
    c0, t0 = cpu(), time.perf_counter()
    with cf.ThreadPoolExecutor(conc) as ex: res = list(ex.map(one, clients))
    dt, dc = time.perf_counter() - t0, cpu() - c0
    lat = sorted(r[0] * 1000 for r in res)
    p95 = lat[(95 * N + 99) // 100 - 1]  # nearest-rank percentile, including small batches
    bad = 0
    for _, w, acct, p in res:  # izolimi: çdo provë i përket klientit të vet
        if p.get("wallet") != w or not owns(p, w): bad += 1
    nulls = {p["nullifier"] for *_, p in res}
    if len(nulls) != N: bad += N - len(nulls)
    if os.environ.get("VK"):  # verifikim kriptografik i çdo prove (ekuacioni i programit Solana)
        import tempfile
        for *_, p in res:
            with tempfile.NamedTemporaryFile("w", suffix=".json") as f:
                json.dump(p, f); f.flush()
                if subprocess.run([os.environ.get("BIN", "coprocessor"), "verify", "--proof", f.name, "--vk", os.environ["VK"]],
                                  capture_output=True).returncode: bad += 1
    failed += bad
    print(f"| {conc} | {N} | {statistics.median(lat):.0f} | {p95:.0f} | {N/dt:.2f} | {dc/N:.2f} | {bad} |") if os.environ.get("MD") else \
    print(f"{conc:>8} {N:>8} {statistics.median(lat):>8.0f} {p95:>8.0f} {N/dt:>8.1f} {dc/N:>11.3f} {bad:>7}")
print(f"\nRSS maksimale e daemon-it (pa nënproceset witness/prover): {rss()} MiB")
if failed:
    sys.exit(f"GABIM: {failed} prova të përziera/dublikuara")
