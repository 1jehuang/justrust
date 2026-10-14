#!/usr/bin/env python3
"""Check preserved measurement and build provenance without running new builds."""
import hashlib
import json
from pathlib import Path
import statistics

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]


def main():
    provenance = json.loads((HERE / "provenance.json").read_text())
    assert provenance["before"]["source"] == "bench/query-read-cap/baseline-query_reads.rs"
    assert provenance["after"]["source"] == "bench/query-read-cap/candidate-query_reads.rs"
    assert provenance["before"]["source_sha256"] == "6c8eebbc2796dff1f6a55a2f3f60867122283c8b43e65ddb62cd937d829efad7"
    assert provenance["after"]["source_sha256"] == "38e52413ab64a050b76830c8d274556d39309e0f9dca84d2ddea6627a1302cde"
    for path, digest in provenance["preserved_evidence_sha256"].items():
        assert hashlib.sha256((ROOT / path).read_bytes()).hexdigest() == digest, path
    harness = (HERE / "harness.rs").read_text()
    assert '#[cfg_attr(feature = "baseline", path = "baseline-query_reads.rs")]' in harness
    assert '#[cfg_attr(not(feature = "baseline"), path = "candidate-query_reads.rs")]' in harness
    assert "../../toolchain/" not in harness
    sweeps = {name: json.loads((HERE / f"{name}.json").read_text()) for name in ("before", "after")}
    paired = json.loads((HERE / "paired.json").read_text())
    runs = {row["id"]: row for row in (json.loads(line) for line in (HERE / "build-runs.jsonl").read_text().splitlines())}
    for name, sweep in sweeps.items():
        declared = provenance[name]
        assert sweep["source"] == declared.get("historical_source", declared["source"])
        compiler = sweep["compiler_version"].splitlines()
        assert compiler[0] == provenance["compiler_version"]
        assert f'commit-hash: {provenance["compiler_commit"]}' in compiler
        assert f'host: {provenance["host"]}' in compiler
        assert f'LLVM version: {provenance["llvm"]}' in compiler
        assert sweep["source_sha256"] == declared["source_sha256"]
        assert hashlib.sha256((ROOT / declared["source"]).read_bytes()).hexdigest() == declared["source_sha256"]
        assert sweep["binary_sha256"] == declared["binary_sha256"] == paired["binary_sha256"][name]
        assert sweep["build_run_id"] == declared["run_id"]
        assert runs[declared["run_id"]]["toolchain"] == provenance["compiler_toolchain"]
        assert runs[declared["run_id"]]["exit"] == 0
        assert runs[declared["run_id"]]["subcommand"] == "build"
        assert len(sweep["samples"]) == 12
    assert len(paired["samples"]) == 48
    assert paired["order_per_round"] == ["before", "after", "after", "before"]
    assert paired["quiet_system_guaranteed"] is False
    for sample in [*sweeps["before"]["samples"], *sweeps["after"]["samples"], *paired["samples"]]:
        assert sample["checked"]
        assert sample["lost_events"] == (0 if sample["case"] == "accepted" else sample["events"])
    for summary in paired["summary"]:
        case_samples = [sample for sample in paired["samples"] if sample["case"] == summary["case"] and sample["threads"] == summary["threads"]]
        assert len(case_samples) == 12
        for name in ("before", "after"):
            seconds = [sample["seconds"] for sample in case_samples if sample["binary"] == name]
            assert len(seconds) == 6
            assert statistics.median(seconds) == summary[f"{name}_median_seconds"]
            assert [min(seconds), max(seconds)] == summary[f"{name}_range_seconds"]
        for round_index in range(1, 4):
            round_samples = [sample for sample in case_samples if sample["round"] == round_index]
            assert [sample["binary"] for sample in round_samples] == paired["order_per_round"]
            assert all(sample["affinity"] == ([0] if summary["threads"] == 1 else list(range(8))) for sample in round_samples)
    for validation in provenance["validation"]:
        run = runs[validation["run_id"]]
        assert run["exit"] == 0
        assert run["toolchain"] == provenance["compiler_toolchain"]
        assert run["route"] == "JUSTRUST_REMOTE=off"
        if "passed" in validation:
            assert run["tests"]["passed"] == validation["passed"]
        assert run["diagnostics"]["warnings"] == 0
    native_dir = ROOT / "data/runs/query-read-cap-20261014"
    native_before = json.loads((native_dir / "native-baseline.json").read_text())
    native_after = json.loads((native_dir / "native-candidate.json").read_text())
    assert native_before["recorder_source_sha256"] == provenance["before"]["source_sha256"]
    assert native_after["recorder_source_sha256"] == provenance["after"]["source_sha256"]
    assert native_before["all_four_tests_passed"] and native_after["all_seven_native_tests_passed"]
    assert native_after["speed_claim_eligible"] is False
    assert native_before["compiler_driver_sha256"] == "b9a17b8f106072eb240189957630c634fa061dae3a6d42e9b26ab2949bc750af"
    assert native_after["compiler_driver_sha256"] == "270af337d3643eda83e3f14660bd7e6ea35fc6bd170c088a70aeb9121758f90e"
    assert native_after["compiler_launcher_sha256"] == "95466164bba3372fee62e475ca1e20164f10d6ac23e3568a371f7e0fc636beba"
    assert native_after["compiler_launcher_sha256"] == native_before["compiler_launcher_sha256"]
    assert native_after["compiler_driver_sha256"] != native_before["compiler_driver_sha256"]
    for before, after in zip(native_before["samples"], native_after["samples"], strict=True):
        assert (before["fixture"], before["threads"], before["source_sha256"]) == (after["fixture"], after["threads"], after["source_sha256"])
        assert after["exit_code"] == 0 and after["wait_structural_complete"]
        assert after["full_pipeline_complete"] is False and after["reads"]["semantic_complete"] is False
        if after["fixture"] == "contention":
            assert after["reads"]["lost_events"] > 30_000_000
            assert after["reads"]["records_complete"] is False
        else:
            assert after["reads"]["lost_events"] == 0 and after["reads"]["records_complete"]
    print("Verified source/binary/build provenance, 72 checked timing samples, ABBA order/affinity, exact loss checks, all cited run records, and source/fixture-matched real native acceptance summaries.")


if __name__ == "__main__":
    main()
