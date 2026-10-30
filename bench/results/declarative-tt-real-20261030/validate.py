#!/usr/bin/env python3
"""Portable offline dp41 replay. Counts and nested intervals are not time saved."""
from collections import defaultdict
import gzip
import hashlib
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parents[2]
sys.path.insert(0, str(REPO / "bench"))
import blaze_runtime_compare as h
import query_profile as q
from reuse_ceiling import interval_union_ns

PREFIX = "jr_declarative_macro_"
EXPECTED = {
    "prepare": (633, 1329, 633),
    "edit": (634, 1330, 1),
    "restore": (633, 1329, 0),
}
DRIVER = "ac116b9398e1f5c677e73952b8719be9d9e47190fe61fbaeba9b7cf270739595"
LAUNCHER = "570faeb27338cea8625c4a4a7eaeef71fc75470508657ff489c3da96ce5f9144"


def digest(data):
    return hashlib.sha256(data).hexdigest()


def expected_markers(phase):
    eligible, rejected, providers = EXPECTED[phase]
    return {PREFIX + suffix: value for suffix, value in (
        ("eligible", eligible), ("ineligible", rejected),
        ("cache_lookup", eligible), ("cache_provider", providers),
        ("cache_success", eligible),
    )}


def check_state(state):
    assert state["status"] == "completed"
    assert state["scope"] == "real-tt-cache-eligibility-not-speedup"
    assert state["source_before"] == state["source_after"]
    assert len(state["source_before"]) == 63
    identities = state["identities_before"]
    assert identities == state["identities_after"]
    assert identities["rustc_driver_0"]["sha256"] == DRIVER
    assert identities["rustc"]["sha256"] == LAUNCHER
    assert [r["phase"] for r in state["rows"]] == list(EXPECTED)
    before, edit, restore = state["rows"]
    assert before["source_sha256"] == restore["source_sha256"]
    assert edit["source_sha256"] != before["source_sha256"]
    assert before["source_sha256"] == state["source_before"]["src/summary.rs"]
    for row in state["rows"]:
        assert row["status"] == 0 and not row["timed_out"] and row["error"] is None
        assert len(row["profiles"]) == 1
        assert row["profiles"][0]["markers"] == expected_markers(row["phase"])
        env = row["environment"]
        assert env["JUSTRUST_CACHE_PROC_MACROS"] == "0"
        for flag in ("jr-empty-lint-level-fast-path", "jr-cache-small-query-values", "jr-cache-associated-items"):
            assert "-Z" + flag + "=no" in env["RUSTFLAGS"]
        assert "-Zjr-cache-declarative-macros=yes" in env["RUSTFLAGS"]


def profile_replay(data, profile, phase):
    assert digest(data) == profile["sha256"]
    events, strings = q.prepare(data, allow_unmapped_event_labels=True)
    metadata = q.metadata_from_strings(strings)
    assert metadata == profile["metadata"]
    assert "--crate-name justrust " in metadata["cmd"]
    assert "-Zjr-cache-declarative-macros=yes" in metadata["cmd"]
    counts = dict.fromkeys(expected_markers(phase), 0)
    persistence, encoding = defaultdict(list), defaultdict(list)
    total = unresolved = 0
    for record in q.iter_prepared_records(events, strings, allow_unmapped_event_labels=True):
        total += 1
        if not record["label_resolved"]:
            unresolved += 1
            continue
        label = record["label"]
        if label in counts:
            assert record["kind"] == "GenericActivity" and record["record_type"] == "interval"
            counts[label] += 1
        if record["record_type"] != "interval":
            continue
        span = (record["start_ns"], record["end_ns"])
        if label.startswith("incr_comp_") or label in ("encode_query_values", "encode_query_results_for"):
            persistence[(label, record["thread"])].append(span)
        if phase == "edit" and record["kind"] == "GenericActivity" and label in ("encode_query_results_for", "jr_incremental_green_recompute"):
            text = strings.resolve(record["event_id"]).split("\x1e")
            name = text[1] if len(text) == 2 else "<unknown>"
            encoding[(label, name, record["thread"])].append(span)
    assert counts == profile["markers"] == expected_markers(phase)
    assert total == profile["records"] and unresolved == profile["unresolved_records"] == 0
    intervals = [dict(label=k[0], thread=k[1], records=len(v), elapsed_union_ns=interval_union_ns(v)) for k, v in sorted(persistence.items())]
    queries = [dict(label=k[0], query=k[1], thread=k[2], records=len(v), union_ns=interval_union_ns(v)) for k, v in encoding.items()]
    queries.sort(key=lambda r: r["union_ns"], reverse=True)
    return intervals, queries


def validate(root=ROOT, export=None):
    manifest = json.loads((root / "sha256.json").read_text())
    for name, expected in manifest.items():
        assert Path(name).name == name, name
        assert digest((root / name).read_bytes()) == expected, name
    state = json.loads((root / "state.json").read_text())
    check_state(state)
    persistence = json.loads((root / "persistence-intervals.json").read_text())
    encoding = json.loads((root / "encoding-query-intervals.json").read_text())
    expected_tests = None
    recomputed = []
    for row in state["rows"]:
        phase = row["phase"]
        receipt = row["raw_output"]
        raw = (root / receipt["artifact"]).read_bytes()
        assert digest(raw) == receipt["sha256"]
        output = "\n".join(json.loads(line)["l"] for line in raw.decode().splitlines())
        assert h.accepted(row, output, expected_tests, "JR_RUNTIME_EXECUTED_39" if phase == "edit" else None)
        expected_tests = h.test_results(output)
        assert json.loads(json.dumps(expected_tests)) == row["parsed"]
        assert (expected_tests["passed"], expected_tests["failed"], expected_tests["ignored"]) == (4, 0, 0)
        profile = row["profiles"][0]
        data = gzip.decompress((root / (profile["path"] + ".gz")).read_bytes())
        intervals, queries = profile_replay(data, profile, phase)
        recomputed.append(dict(phase=phase, intervals=intervals))
        if phase == "edit":
            assert queries == encoding
    assert recomputed == persistence
    export = export or REPO / "data/runs/xps14-declarative-tt-real-20261030/2026-10.jsonl"
    runs = [json.loads(line) for line in export.read_text().splitlines()]
    assert [r["id"] for r in runs] == [r["raw_output"]["run_id"] for r in state["rows"]]
    for run in runs:
        assert run["exit"] == 0
        assert run["tests"]["passed"] == 4 and run["tests"]["failed"] == 0
    return dict(verdict="passed", workflows=3, selected_tests_per_workflow=4,
                counts={phase: expected_markers(phase) for phase in EXPECTED},
                intervals="recomputed nested descriptive intervals, not additive time saved",
                scope="real TT cache mechanism reuse only, no latency claim")


if __name__ == "__main__":
    if sys.argv[1:] == ["--seal"]:
        originals = [p for p in ROOT.iterdir() if p.is_file() and (p.name in ("state.json", "runner.py", "persistence-intervals.py", "encoding-query-intervals.py") or p.suffix in (".gz", ".jsonl", ".stdout", ".stderr") or p.name.endswith("-intervals.json"))]
        (ROOT / "sha256.json").write_text(json.dumps({p.name: digest(p.read_bytes()) for p in sorted(originals)}, indent=2) + "\n")
    else:
        assert not sys.argv[1:], "usage: validate.py [--seal]"
        print(json.dumps(validate(), indent=2))
