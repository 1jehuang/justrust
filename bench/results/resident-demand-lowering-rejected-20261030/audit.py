#!/usr/bin/env python3
"""Offline audit of rejected demand lowering receipts. Never builds or runs Rust."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import statistics
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parent
CANDIDATE = "7792413378f3508b3d3692ec4ea9e2046ebb8adfd6464756be5c40131eda1855"
BASELINE = "471f0b5c10187ecbea879acfe3e5eae52925161e32bce6feeebe49b67af0368f"


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def inventory(root):
    return {str(p.relative_to(root)): digest(p) for p in sorted(root.rglob("*"))
            if p.is_file() and p.name != "SHA256.json" and "__pycache__" not in p.parts}


def audit(root):
    expected = json.loads((root / "SHA256.json").read_text())
    assert inventory(root) == expected, "bundle inventory/hash mismatch"
    assert digest(root / "source/candidate-main.rs") == CANDIDATE
    assert digest(root / "source/baseline-main.rs") == BASELINE
    # Reconstruct the exact rejected source without the repository or original host paths.
    with tempfile.TemporaryDirectory(prefix="resident-demand-audit-") as tmp:
        work = Path(tmp)
        target = work / "toolchain/resident/crates/ra-jit/src/main.rs"
        target.parent.mkdir(parents=True)
        target.write_bytes((root / "source/baseline-main.rs").read_bytes())
        subprocess.run(["git", "apply", str(root / "source/candidate.patch")],
                       cwd=work, check=True, capture_output=True)
        assert digest(target) == CANDIDATE, "patch does not reconstruct tested candidate"
    prov = root / "provenance"
    assert (prov / "candidate-build-v5/exit").read_text().strip() != "0"
    failed = (prov / "candidate-build-v5/build.log").read_text()
    assert "E0425" in failed and "outline.rs" in failed
    for name in ("candidate-build-v6", "candidate-unit-v6"):
        assert (prov / name / "exit").read_text().strip() == "0"
    assert (prov / "suite-after-v6.exit").read_text().strip() == "0"
    for name in ("resident_graph_reuse", "the_roots_of_a_listed_crate", "telemetry"):
        assert (prov / f"candidate-unit-v6/{name}.exit").read_text().strip() == "0"
    text = (prov / "candidate-unit-v6/resident_graph_reuse.log").read_text()
    counts = [(int(a), int(b)) for a, b in re.findall(
        r"resident_graph_reuse revision (\d+): (\d+) body declaration query executions", text)]
    assert counts == list(enumerate([7, 1, 3, 2, 3, 3, 2, 1])), counts
    assert "1 test passed" in text
    values = {}
    tools = []
    for name in ("rsa01", "rsa02"):
        at = root / name
        suite = json.loads((at / "suite.json").read_text())
        for file, sha in suite["artifacts_sha256"].items():
            assert digest(at / file) == sha, (name, file)
        summary = json.loads((at / "summary.json").read_text())
        assert summary["excluded"] == [] and summary["source_built_parity"]
        records = json.loads((at / "records.json").read_text())
        medians = {}
        for role in ("engine", "reference"):
            gates = {r["phase"]: r["status"] for r in records
                     if r["stack"] == role and r["phase"] != "edit"}
            assert gates == {"version": 0, "prepare": 0, "type-error": 101,
                             "type-recovery": 0, "assert-failure": 101, "assert-recovery": 0}
            edits = [r for r in records if r["stack"] == role and r["phase"] == "edit"]
            assert len(edits) == 5 and all(r["status"] == 0 and not r["timed_out"] for r in edits)
            medians[role] = statistics.median(r["elapsed_seconds"] for r in edits)
            assert abs(medians[role] - summary["medians_seconds"][role]) < 1e-12
            assert list((at / f"{role}-worker-logs").rglob("log")), (name, role)
        values[name] = medians
        tools.append(suite["metadata"]["tools_before"])
    assert tools[0]["reference"]["sha256"] == tools[1]["reference"]["sha256"]
    assert tools[0]["engine"]["sha256"] != tools[1]["engine"]["sha256"]
    before, after = values["rsa01"], values["rsa02"]
    assert round(before["engine"], 9) == 0.109617051
    assert round(after["engine"], 9) == 0.110432806
    assert round(before["reference"], 9) == 0.110923157
    assert round(after["reference"], 9) == 0.110535542
    assert after["engine"] > before["engine"], "negative-result direction changed"
    return values, counts


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--seal", action="store_true", help="create the bundle content manifest")
    parser.add_argument("--self-test", action="store_true", help="also check tamper detection")
    args = parser.parse_args()
    if args.seal:
        (ROOT / "SHA256.json").write_text(json.dumps(inventory(ROOT), indent=2) + "\n")
    values, counts = audit(ROOT)
    if args.self_test:
        manifest = json.loads((ROOT / "SHA256.json").read_text())
        tampered = dict(manifest)
        tampered["source/candidate-main.rs"] = "0" * 64
        assert inventory(ROOT) != tampered, "candidate tamper went undetected"
        missing = dict(manifest)
        del missing["provenance/candidate-build-v5/build.log"]
        assert inventory(ROOT) != missing, "missing rejection receipt went undetected"
    print(json.dumps({"audit": "PASS", "medians_seconds": values,
                      "query_executions_by_revision": counts,
                      "verdict": "REJECTED: no useful latency benefit, descriptive only"}, indent=2))


if __name__ == "__main__":
    main()
