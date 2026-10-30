#!/usr/bin/env python3
"""Offline integrity and observable acceptance checks, never a speedup test."""
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent


def validate():
    manifest = json.loads((ROOT / "sha256.json").read_text())
    for name, expected in manifest.items():
        assert hashlib.sha256((ROOT / name).read_bytes()).hexdigest() == expected, name
    state = json.loads((ROOT / "state.json").read_text())
    assert state["status"] == "completed"
    assert state["source_before"] == state["source_after"]
    assert state["identities_before"] == state["identities_after"]
    assert len(state["source_before"]) == 63
    assert len(state["rows"]) == 6
    expected_tests = None
    edits = {}
    for row in state["rows"]:
        assert row["status"] == 0 and not row["timed_out"] and row["error"] is None
        parsed = row["parsed"]
        assert (parsed["passed"], parsed["failed"], parsed["ignored"], parsed["filtered"]) == (4, 0, 0, 144)
        if expected_tests is None:
            expected_tests = parsed["tests"]
        assert parsed["tests"] == expected_tests
        receipt = row["raw_output"]
        assert hashlib.sha256((ROOT / receipt["artifact"]).read_bytes()).hexdigest() == receipt["sha256"]
        if row["phase"] == "edit":
            edits[row["mode"]] = row
        units = [json.loads(line) for line in (ROOT / receipt["run_id"] / "units.jsonl").read_text().splitlines()]
        assert all(unit["exit"] == 0 for unit in units)
    assert set(edits) == {"off", "on"}
    assert edits["off"]["source_sha256"] == edits["on"]["source_sha256"]
    for mode in ("off", "on"):
        assert {row["phase"] for row in state["rows"] if row["mode"] == mode} == {"prepare", "edit", "restore"}
    return {"verdict": "passed", "successful_workflows": 6, "restored_files": 63,
            "scope": "instrumented diagnostic integrity and correctness only, not a speedup or cache-input safety proof"}


if __name__ == "__main__":
    print(json.dumps(validate(), indent=2))
