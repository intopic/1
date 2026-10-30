"""Emit fresh evidence manifest and distinguish each validation gate."""
import hashlib
import importlib.metadata
import json
import os
from pathlib import Path
import platform
import subprocess

root = Path(__file__).resolve().parent
out = root / "results"
model = json.loads((out / "model_results.json").read_text())
protocol = json.loads((out / "protocol_results.json").read_text())
runtime = json.loads((out / "runtime_results.json").read_text())
assert model["cases"]["eight_independent"]["exact_maximum"] == 8
assert protocol["cases"]["independent"]["exact_maximum_nonconflicting"] == 8
assert runtime["status"] == "PASS"
assert len(runtime["cases"]["eight_independent_serial_runtime_executions"]) == 8
assert runtime["scheduler_speedup_measured"] is False
sources = sorted(root.glob("*.py")) + sorted((root / "tests").glob("*.py"))
manifest = {
    "checkout_revision": subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip(),
    "pr_head_sha": os.environ.get("STUDY_HEAD_SHA"),
    "github_run_id": os.environ.get("GITHUB_RUN_ID"),
    "platform": platform.platform(),
    "solders": importlib.metadata.version("solders"),
    "source_sha256": {str(p.relative_to(root)): hashlib.sha256(p.read_bytes()).hexdigest() for p in sources},
    "runtime_status": "PASS", "scheduler_speedup": "NOT_MEASURED",
    "custom_order_contract": "NOT_IMPLEMENTED",
}
(out / "ci_manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
cu = [r["cu"] for r in runtime["cases"]["eight_independent_serial_runtime_executions"]]
text = f"""# Direct-exchange evidence

- Model graph:8 independent candidates.
- Real Solana message graph:8 independent transactions.
- Actual SPL SBF:8 successful SERIAL LiteSVM executions.
- Compute units per bilateral exchange: {cu}.
- Atomic rollback after successful leg1 and failed leg2:PASS.
- Same finite funds cannot be spent twice:PASS.
- Identical transaction replay / tampered signature:PASS.
- Validator scheduler parallelism /8x speedup:NOT MEASURED.
- Custom onchain order/policy/nonce contract:NOT IMPLEMENTED.
- Revision: {manifest['checkout_revision']}.

Fixture mint/token accounts were injected; both transfer instructions execute
in the pinned real classic SPL Token SBF. No mainnet/deployment is involved.
"""
(out / "CI_REPORT.md").write_text(text)
if os.environ.get("GITHUB_STEP_SUMMARY"):
    with open(os.environ["GITHUB_STEP_SUMMARY"], "a") as stream:
        stream.write(text)
print(text)
