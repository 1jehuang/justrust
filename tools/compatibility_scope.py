#!/usr/bin/env python3
"""Layered, executable compatibility scope for tools/conformance.py.

The denominator is declared test obligations, not a percentage of the Rust spec.
Missing tests are obligations too. No hand-maintained support/pass states exist.
"""
from __future__ import annotations

from concurrent.futures import ThreadPoolExecutor
import itertools
import os
from pathlib import Path
import re
import shutil
import time

LAYERS = ("feature", "rejection", "interaction", "end_to_end")
SCOPE = Path("data/compiler-compatibility-scope.json")


def validate(scope, c, root):
    if not isinstance(scope, dict) or scope.get("schema_version") != 1:
        raise c.InvalidEvidence("unsupported compatibility scope schema")
    contract = scope.get("scope", {})
    if (contract.get("edition") != "2024"
            or contract.get("target") != "x86_64-unknown-linux-gnu"
            or contract.get("semantic_inventory") != "incomplete"):
        raise c.InvalidEvidence("scope must retain the explicit, incomplete initial compatibility contract")
    c.unique_ids(scope.get("areas"), "area")
    requirements = []
    suites = set(c.integration_suites(root))
    discovered = {(Path(row["file"]).stem, row["test"])
                  for row in c.discover(root)["source_test_functions"]
                  if Path(row["file"]).parent == Path("tests")}
    # The existing differential suite declares independent cases with probes!.
    # This is lexical discovery only. Actual evidence still requires exactly one executed test.
    generated = root / "tests/conformance_generated.rs"
    if generated.is_file():
        for block in re.findall(r"(?m)^probes!\((?:true|false);(.*?)\);", generated.read_text(), re.S):
            for name in re.findall(r"\b([a-zA-Z_][a-zA-Z0-9_]*)\s*=\s*\d+", block):
                discovered.add(("conformance_generated", name))
    def check_tests(tests):
        if not isinstance(tests, list):
            raise c.InvalidEvidence("tests must be an array, including explicit empty gaps")
        seen = set()
        for case in tests:
            if (not isinstance(case, dict) or set(case) != {"suite", "test"}
                    or case.get("suite") not in suites
                    or not re.fullmatch(r"[a-zA-Z_][a-zA-Z0-9_]*", case.get("test", ""))
                    or (case["suite"], case["test"]) not in discovered):
                raise c.InvalidEvidence(f"missing exact test: {case}")
            key = (case["suite"], case["test"])
            if key in seen:
                raise c.InvalidEvidence("duplicate test in coverage gate")
            seen.add(key)
    for area in scope["areas"]:
        if not area.get("title"):
            raise c.InvalidEvidence("area requires a title")
        c.unique_ids(area.get("requirements"), "requirement")
        for requirement in area["requirements"]:
            requirements.append(requirement)
            if not requirement.get("title"):
                raise c.InvalidEvidence("requirement requires a title")
            gates = requirement.get("gates", [])
            if (len(gates) != len(LAYERS)
                    or {row.get("layer") for row in gates} != set(LAYERS)):
                raise c.InvalidEvidence(f"requirement needs all four coverage layers: {requirement['id']}")
            for gate in gates:
                check_tests(gate.get("tests"))
                for note in gate.get("evidence_notes", []):
                    if not isinstance(note, dict) or not isinstance(note.get("note"), str) or not note["note"].strip():
                        raise c.InvalidEvidence("mapping evidence note needs a nonempty rationale")
                    check_tests(note.get("tests"))
                    if not note["tests"] or any(case not in gate["tests"] for case in note["tests"]):
                        raise c.InvalidEvidence("mapping evidence note must refer to tests in its gate")
    c.unique_ids(requirements, "requirement")
    matrix = scope.get("interaction_matrix")
    if matrix is not None:
        axes = c.unique_ids(matrix.get("axes"), "interaction axis")
        if len(axes) < 2 or any(not axis.get("title") for axis in matrix["axes"]):
            raise c.InvalidEvidence("interaction matrix needs at least two named axes")
        c.unique_ids(matrix.get("scenarios"), "interaction scenario")
        for scenario in matrix["scenarios"]:
            features = scenario.get("features", [])
            if len(set(features)) != len(features) or len(features) < 2 or not set(features) <= axes:
                raise c.InvalidEvidence("scenario must combine distinct declared interaction axes")
            check_tests(scenario.get("tests"))
    return scope


def case_id(case):
    return "case-" + case["suite"].replace("_", "-") + "--" + case["test"].replace("_", "-")


def case_manifest(scope, base, c):
    cases = {}
    for area in scope["areas"]:
        for requirement in area["requirements"]:
            for gate in requirement["gates"]:
                for case in gate["tests"]:
                    identifier = case_id(case)
                    gate = {"id": identifier, "kind": "case", "suite": case["suite"],
                            "test": case["test"], "profiles": ["full"]}
                    if identifier in cases and cases[identifier] != gate:
                        raise c.InvalidEvidence("case identifier collision")
                    cases[identifier] = gate
    for scenario in scope.get("interaction_matrix", {}).get("scenarios", []):
        for case in scenario["tests"]:
            identifier = case_id(case)
            cases[identifier] = {"id": identifier, "kind": "case", "suite": case["suite"],
                                 "test": case["test"], "profiles": ["full"]}
    result = {"schema_version": 1, "contract": base["contract"],
              "gates": sorted(cases.values(), key=lambda row: row["id"]),
              "requirements": [], "known_gaps": [], "scope_sha256": c.digest(c.encoded(scope))}
    return result


def runner_identity(c):
    executable = shutil.which("justrust")
    if not executable:
        raise c.InvalidEvidence("installed stable justrust is required for current executable evidence")
    path = Path(executable).resolve()
    return {"executable": str(path), "sha256": c.digest(path.read_bytes()),
            "environment": {key: os.environ.get(key) for key in c.ENV_KEYS
                            if key != "JUSTRUST_TEST_PARALLEL"}}


def evidence(scope, manifest, directory, identity, runner, c, root):
    """Recompute each result from original logs, while retaining partial failures."""
    report = c.load_json(directory / "report.json")
    if (report.get("scope_sha256") != c.digest(c.encoded(scope))
            or report.get("manifest_sha256") != c.digest(c.encoded(manifest))):
        raise c.InvalidEvidence(f"changed scope or invocation manifest: {directory}")
    selected = report.get("selected", [])
    if not selected:
        raise c.InvalidEvidence("tracked report requires explicit case selection")
    expected = c.plan(manifest, "full", selected, report.get("target_dir", ""))
    rows = report.get("results")
    if (not isinstance(rows, list) or not all(isinstance(row, dict) for row in rows)
            or [row.get("id") for row in rows] != [row["id"] for row in expected]):
        raise c.InvalidEvidence(f"missing, extra, reordered or duplicate results: {directory}")
    if report.get("schema_version") != 1 or report.get("profile") != "full":
        raise c.InvalidEvidence("invalid tracked report schema/profile")
    stale = (report.get("source_before") != identity or report.get("source_after") != identity
             or report.get("runner_before") != runner or report.get("runner_after") != runner
             or report.get("integration_suites") != c.integration_suites(root))
    results = {}
    for row, entry in zip(rows, expected):
        if any(row.get(key) != entry[key] for key in ("id", "kind", "command")):
            raise c.InvalidEvidence(f"changed exact test invocation: {entry['id']}")
        log = directory / str(row.get("log", ""))
        if not log.is_file() or log.resolve().parent != directory.resolve():
            raise c.InvalidEvidence(f"missing or unsafe log for {entry['id']}")
        payload = log.read_bytes()
        text = payload.decode(errors="replace")
        measured = c.outcome(row.get("returncode"), text, "case", row.get("timed_out", False))
        if (row.get("log_sha256") != c.digest(payload) or measured != row.get("outcome")
                or row.get("tests_passed") != c.test_count(text)):
            raise c.InvalidEvidence(f"inconsistent evidence for {entry['id']}")
        results[entry["id"]] = {"status": "stale" if stale else measured,
                                  "report": str(directory / "report.json"), "log": str(log)}
    return results


def summarize(scope, manifest, reports, identity, runner, c, root):
    observations = {}
    for directory in reports:
        for key, value in evidence(scope, manifest, directory, identity, runner, c, root).items():
            observations.setdefault(key, []).append(value)
    counts = {status: 0 for status in ("passed", "failed", "timeout", "infrastructure", "unverified",
                                      "untested", "missing", "stale", "conflicting")}
    areas = []
    used_suites = set()
    test_requirements = {}
    def observe(case):
        used_suites.add(case["suite"])
        attempts = observations.get(case_id(case), [])
        current = [row["status"] for row in attempts if row["status"] != "stale"]
        states = set(current)
        status = ("conflicting" if len(states) > 1 else current[0] if current
                  else "stale" if attempts else "untested")
        return {**case, "status": status, "evidence": attempts,
                "command": c.command({"kind": "case", **case}, "target/justrust-slots/0")}
    def gate_status(cases):
        states = {row["status"] for row in cases}
        precedence = ("conflicting", "failed", "timeout", "infrastructure", "unverified",
                      "stale", "untested")
        return ("missing" if not cases else
                next((value for value in precedence if value in states), "passed"))
    for area in scope["areas"]:
        requirements = []
        for requirement in area["requirements"]:
            gates = []
            for gate in requirement["gates"]:
                cases = []
                for case in gate["tests"]:
                    identifier = case_id(case)
                    if gate["layer"] == "interaction":
                        test_requirements.setdefault(identifier, set()).add(requirement["id"])
                    cases.append(observe(case))
                status = gate_status(cases)
                counts[status] += 1
                gates.append({"layer": gate["layer"], "status": status, "tests": cases,
                              "evidence_notes": gate.get("evidence_notes", [])})
            requirements.append({"id": requirement["id"], "title": requirement["title"],
                                 "all_declared_gates_passed": all(row["status"] == "passed" for row in gates),
                                 "gates": gates})
        areas.append({"id": area["id"], "title": area["title"], "requirements": requirements})
    matrix = scope.get("interaction_matrix", {})
    scenarios = [{**scenario, "tests": [observe(case) for case in scenario["tests"]]}
                 for scenario in matrix.get("scenarios", [])]
    for scenario in scenarios:
        scenario["status"] = gate_status(scenario["tests"])
    matrix_pairs = []
    for pair in itertools.combinations(sorted(axis["id"] for axis in matrix.get("axes", [])), 2):
        matching = [row for row in scenarios if set(pair) <= set(row["features"])]
        # Every declared scenario covering this pair must pass, not just a convenient one.
        cases = [case for scenario in matching for case in scenario["tests"]]
        status = "missing" if not matching or any(not row["tests"] for row in matching) else gate_status(cases)
        matrix_pairs.append({"features": list(pair), "status": status,
                             "scenarios": [row["id"] for row in matching]})
    total = sum(counts.values())
    layer_counts = {layer: {status: sum(gate["status"] == status for area in areas
                                      for requirement in area["requirements"] for gate in requirement["gates"]
                                      if gate["layer"] == layer) for status in counts} for layer in LAYERS}
    pairs = sorted({pair for values in test_requirements.values()
                    for pair in itertools.combinations(sorted(values), 2)})
    missing = [{"area": area["id"], "requirement": requirement["id"],
                "title": requirement["title"], "layer": gate["layer"]}
               for area in areas for requirement in area["requirements"] for gate in requirement["gates"]
               if not gate["tests"]]
    mapped = total - len(missing)
    unmapped = sorted(set(c.integration_suites(root)) - used_suites)
    blockers = ["semantic inventory has not been established as complete"]
    if missing:
        blockers.append(f"{len(missing)} declared layer obligations have no mapped tests")
    if unmapped:
        blockers.append(f"{len(unmapped)} existing integration suites need coverage triage")
    return {"schema_version": 1, "scope": scope["scope"], "source": identity,
            "full_rust_support_established": False,
            "full_rust_completion_percent": None,
            "measurement_readiness": {
                "can_estimate_full_rust_completion": False, "blockers": blockers,
                "mapped_gate_count": mapped, "declared_gate_count": total,
                "mapped_declared_gates_percent": round(100 * mapped / total, 2),
                "mapped_exact_test_count": len(manifest["gates"]),
                "mapped_integration_suite_count": len(used_suites),
                "discovered_integration_suite_count": len(c.integration_suites(root)),
            },
            "missing_test_obligations": missing,
            "metric": "passing declared test obligations, not percent of the Rust language",
            "declared_gate_count": total, "gate_counts": counts,
            "passing_declared_gates_percent": round(100 * counts["passed"] / total, 2),
            "missing_test_gates": counts["missing"], "by_layer": layer_counts,
            "requirements_with_all_declared_gates_passed": sum(row["all_declared_gates_passed"]
                                                               for area in areas for row in area["requirements"]),
            "areas": areas,
            "unmapped_integration_suites": unmapped,
            "interaction_matrix": {"axes": matrix.get("axes", []), "scenarios": scenarios,
                                   "pairs": matrix_pairs, "pair_count": len(matrix_pairs),
                                   "passing_pairs": sum(row["status"] == "passed" for row in matrix_pairs),
                                   "missing_pairs": sum(row["status"] == "missing" for row in matrix_pairs)},
            "declared_interaction_pairs": [list(pair) for pair in pairs],
            "interaction_note": "Pairs share an explicitly mapped interaction test; unlisted pairs and higher-order combinations are not established. No pairwise completeness claim.",
            "limitations": ["The inventory is incomplete and versioned; adding missing requirements can lower the percentage.",
                            "Each mapped test is evidence for its exact scenarios, not all semantics suggested by a requirement title.",
                            "An unsupported-valid rejection test must not be mapped as feature acceptance.",
                            "All four layers are required; unimplemented test obligations remain in the denominator.",
                            "Source/client/environment changes invalidate old evidence; conflicting current runs are not green."]}


def render(summary):
    counts = summary["gate_counts"]
    readiness = summary["measurement_readiness"]
    lines = ["Rust compatibility test coverage (edition 2024, Linux x86-64)",
             "Full Rust completion percentage: unavailable (semantic inventory incomplete).",
             f"Test-mapping coverage: {readiness['mapped_gate_count']}/{readiness['declared_gate_count']} declared layer obligations ({readiness['mapped_declared_gates_percent']}%); {readiness['mapped_exact_test_count']} exact tests.",
             f"{counts['passed']}/{summary['declared_gate_count']} declared gates verified ({summary['passing_declared_gates_percent']}%). NOT percentage of all Rust.",
             f"Missing tests: {counts['missing']}; untested: {counts['untested']}; stale: {counts['stale']}; failed: {counts['failed']}; conflicts: {counts['conflicting']}.",
             "AREA / feature / rejection / interaction / end-to-end (passed/declared)"]
    for area in summary["areas"]:
        cells = []
        for layer in LAYERS:
            gates = [gate for requirement in area["requirements"] for gate in requirement["gates"]
                     if gate["layer"] == layer]
            cells.append(f"{sum(gate['status'] == 'passed' for gate in gates)}/{len(gates)}")
        lines.append(area["id"] + " / " + " / ".join(cells))
    matrix = summary["interaction_matrix"]
    lines.append(f"Systematic bounded feature pairs: {matrix['passing_pairs']}/{matrix['pair_count']} verified; {matrix['missing_pairs']} pairs have no tests. Higher-order scenarios run too, but are not exhaustive.")
    lines.extend([f"{len(summary['unmapped_integration_suites'])} integration suites still need explicit coverage mapping.",
                  "Scope is incomplete. Use --json for exact missing requirements, case commands and evidence paths."])
    return "\n".join(lines)


def execute_cases(entries, directory, timeout, jobs, c, root):
    if not isinstance(jobs, int) or isinstance(jobs, bool) or not 1 <= jobs <= 8:
        raise c.InvalidEvidence("track --jobs must be between 1 and 8")
    if jobs == 1:
        for entry in entries:
            yield c.execute(entry, directory, timeout, root)
        return
    # Commands/logs remain independent and no failed case is retried. Ordered
    # consumption preserves deterministic report identity despite completion order.
    with ThreadPoolExecutor(max_workers=jobs) as pool:
        futures = [pool.submit(c.execute, entry, directory, timeout, root) for entry in entries]
        for future in futures:
            yield future.result()


def main(args, base, c):
    root = c.ROOT
    scope = validate(c.load_json(args.scope), c, root)
    manifest = case_manifest(scope, base, c)
    if args.action == "coverage":
        # Inventory is useful in source-only hosted snapshots too. Only retained
        # executable evidence requires Git and client provenance; never invent it.
        identity = c.source_identity(root) if args.evidence else None
        runner = runner_identity(c) if args.evidence else None
        summary = summarize(scope, manifest, args.evidence, identity, runner, c, root)
        print(c.encoded(summary).decode() if args.json else render(summary))
        return 0
    jobs = getattr(args, "jobs", 1)
    if not isinstance(jobs, int) or isinstance(jobs, bool) or not 1 <= jobs <= 8:
        raise c.InvalidEvidence("track --jobs must be between 1 and 8")
    known = {area["id"] for area in scope["areas"]} | {"interactions"}
    if set(args.area) - known or len(args.area) != len(set(args.area)):
        raise c.InvalidEvidence("unknown or duplicate selected areas")
    selected = set()
    for area in scope["areas"]:
        if args.area and area["id"] not in args.area:
            continue
        for requirement in area["requirements"]:
            for gate in requirement["gates"]:
                selected.update(case_id(case) for case in gate["tests"])
    if not args.area or "interactions" in args.area:
        for scenario in scope.get("interaction_matrix", {}).get("scenarios", []):
            selected.update(case_id(case) for case in scenario["tests"])
    if args.case:
        requested = {case_id({"suite": value.split("::")[0], "test": value.split("::")[-1]})
                     for value in args.case if value.count("::") == 1}
        if len(requested) != len(args.case) or not requested <= selected:
            raise c.InvalidEvidence("--case must select unique mapped suite::test cases within selected areas")
        selected = requested
    entries = c.plan(manifest, "full", sorted(selected), args.target_dir)
    if not selected:
        raise c.InvalidEvidence("selection contains only missing tests, no executable cases")
    if args.dry_run:
        print(c.encoded({"scope": scope["scope"], "gates": entries}).decode(), end="")
        return 0
    if args.out is None or args.timeout <= 0:
        raise c.InvalidEvidence("track requires new --out directory and positive per-case --timeout")
    args.out.mkdir(parents=True, exist_ok=False)
    before = c.source_identity(root)
    before_runner = runner_identity(c)
    report = {"schema_version": 1, "scope_sha256": c.digest(c.encoded(scope)),
              "manifest_sha256": c.digest(c.encoded(manifest)), "profile": "full",
              "selected": sorted(selected), "target_dir": args.target_dir,
              "source_before": before, "runner_before": before_runner,
              "integration_suites": c.integration_suites(root), "results": [],
              "started_unix": time.time()}
    for result in execute_cases(entries, args.out, args.timeout, jobs, c, root):
        report["results"].append(result)
        (args.out / "report.json").write_bytes(c.encoded(report))
    report["source_after"] = c.source_identity(root)
    report["runner_after"] = runner_identity(c)
    (args.out / "report.json").write_bytes(c.encoded(report))
    summary = summarize(scope, manifest, [args.out], report["source_after"], report["runner_after"], c, root)
    (args.out / "coverage.json").write_bytes(c.encoded(summary))
    print(render(summary))
    current = evidence(scope, manifest, args.out, report["source_after"], report["runner_after"], c, root)
    return 0 if all(row["status"] == "passed" for row in current.values()) else 1
