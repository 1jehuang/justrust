#!/usr/bin/env python3
"""Source-bound libtest listings, never executed compatibility evidence.

The existing conformance executor owns process launch, timeout and original logs.
This module only interprets explicitly selected suite listings and reconciles
lexical leads with names actually printed by each compiled test binary.
"""
from __future__ import annotations

import math
import os
from pathlib import Path
import re

import compatibility_scope as scope_tools

PURPOSE = "libtest-selector-catalog-v1"
TARGET = "x86_64-unknown-linux-gnu"


def runner_identity(c):
    return {**scope_tools.runner_identity(c), "catalog_environment": {
        key: os.environ.get(key) for key in (
            "PATH", "CARGO_HOME", "RUSTUP_HOME", "CARGO_ENCODED_RUSTFLAGS",
            "CARGO_BUILD_TARGET", "RUSTC", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER",
            "CARGO_BUILD_RUSTC", "RUSTC_BOOTSTRAP", "JUSTRUST_TOOLCHAIN",
            "JUSTRUST_TOOLCHAIN_REQUIRED")}}


def listing(text, c):
    """Require one complete default-format libtest list, including empty lists."""
    if (c.test_count(text) or c.client_retried_failures(text) or "0 tests matched" in text
            or re.search(r"justrust: ok: [0-9]+ tests? passed", text)
            or re.search(r"(?m)^(?:running [0-9]+ tests?$|test result:)", text)):
        raise c.InvalidEvidence("test execution or client replay is not a test listing")
    footers = re.findall(r"(?m)^([0-9]+) tests?, ([0-9]+) benchmarks?\r?$", text)
    if len(footers) != 1:
        raise c.InvalidEvidence("listing requires exactly one libtest count footer")
    rows = re.findall(r"(?m)^([^\s:]+(?:::[^\s:]+)*): (test|benchmark)\r?$", text)
    if len({name for name, _ in rows}) != len(rows):
        raise c.InvalidEvidence("duplicate names in test listing")
    tests = sorted(name for name, kind in rows if kind == "test")
    benches = sorted(name for name, kind in rows if kind == "benchmark")
    if (len(tests), len(benches)) != tuple(map(int, footers[0])):
        raise c.InvalidEvidence("listing names do not match libtest count footer")
    # A footer must follow every row. This also rejects a truncated/reordered
    # combined transcript rather than treating a preceding footer as completion.
    footer = re.search(r"(?m)^[0-9]+ tests?, [0-9]+ benchmarks?\r?$", text)
    if re.search(r"(?m)^([^\s:]+(?:::[^\s:]+)*): (test|benchmark)\r?$", text[footer.end():]):
        raise c.InvalidEvidence("listing names follow completion footer")
    return {"tests": tests, "benchmarks": benches}


def selection(suites, c, root):
    known = set(c.integration_suites(root))
    if (not isinstance(suites, list) or not suites
            or any(not isinstance(suite, str) or suite not in known for suite in suites)
            or len(set(suites)) != len(suites)):
        raise c.InvalidEvidence("catalog requires unique existing --suite selections")
    return sorted(suites)


def entry(suite, target_dir):
    return {"id": "catalog-" + suite, "kind": "tests", "command": [
        "justrust", "test", "--no-fail-fast", "--target-dir", target_dir,
        "--target", TARGET, "--test", suite, "--", "--list", "--include-ignored"]}


def assess(report, directory, c, root, scope, identity, runner):
    required = {"schema_version", "purpose", "selected_suites", "target_dir",
                "integration_suites", "source_before", "source_after",
                "runner_before", "runner_after", "results"}
    if (not isinstance(report, dict) or set(report) != required
            or type(report.get("schema_version")) is not int or report["schema_version"] != 1
            or report.get("purpose") != PURPOSE
            or any(report.get(key) is None for key in
                   ("source_before", "source_after", "runner_before", "runner_after"))):
        raise c.InvalidEvidence("malformed or unfinalized inventory-only catalog receipt")
    suites = selection(report["selected_suites"], c, root)
    target_dir = report["target_dir"]
    if not isinstance(target_dir, str) or not target_dir:
        raise c.InvalidEvidence("catalog target directory must be nonempty")
    expected = [entry(suite, target_dir) for suite in suites]
    results = report["results"]
    if (not isinstance(results, list) or not all(isinstance(row, dict) for row in results)
            or [row.get("id") for row in results] != [row["id"] for row in expected]):
        raise c.InvalidEvidence("missing, reordered or duplicate catalog results")
    stale = (report["source_before"] != identity or report["source_after"] != identity
             or report["runner_before"] != runner or report["runner_after"] != runner
             or report["integration_suites"] != c.integration_suites(root))
    lexical = scope_tools.selector_candidates(c, root)
    manifest = scope_tools.case_manifest(scope, {"contract": {}}, c)
    mapped = {(gate["suite"], gate["test"]) for gate in manifest["gates"]}
    observations = []
    for suite, row, wanted in zip(suites, results, expected):
        if set(row) != {"id", "kind", "command", "outcome", "returncode", "timed_out",
                        "seconds", "tests_passed", "log", "log_sha256"}:
            raise c.InvalidEvidence("unexpected catalog process result fields")
        if any(row.get(key) != wanted[key] for key in ("id", "kind", "command")):
            raise c.InvalidEvidence("changed catalog invocation")
        if (type(row.get("returncode")) not in (int, type(None))
                or type(row.get("timed_out")) is not bool
                or type(row.get("tests_passed")) is not int
                or type(row.get("seconds")) not in (int, float)
                or (type(row["seconds"]) is float and not math.isfinite(row["seconds"]))
                or row["seconds"] < 0):
            raise c.InvalidEvidence("malformed catalog process result")
        name = row.get("log")
        if not isinstance(name, str) or name != wanted["id"] + ".log":
            raise c.InvalidEvidence("unsafe catalog log name")
        log = directory / name
        if log.is_symlink() or not log.is_file() or log.resolve().parent != directory.resolve():
            raise c.InvalidEvidence("missing or unsafe catalog log")
        payload = log.read_bytes()
        text = payload.decode(errors="replace")
        measured = c.outcome(row["returncode"], text, "tests", row["timed_out"])
        if (row.get("log_sha256") != c.digest(payload) or row.get("outcome") != measured
                or row["tests_passed"] != c.test_count(text)):
            raise c.InvalidEvidence("tampered or inconsistent catalog process result")
        names = {"tests": [], "benchmarks": []}
        if row["timed_out"]:
            status = "timeout"
        elif row["returncode"] is None:
            status = "infrastructure"
        elif row["returncode"] != 0:
            status = "failed"
        else:
            try:
                names = listing(payload.decode("utf-8"), c)
                status = "listed"
            except (c.InvalidEvidence, UnicodeDecodeError):
                status = "unverified"
        if stale:
            status = "stale"
        if status != "listed":
            names = {"tests": [], "benchmarks": []}
        observed = set(names["tests"])
        leads = {test for candidate_suite, test in lexical if candidate_suite == suite}
        required_names = {test for candidate_suite, test in mapped if candidate_suite == suite}
        observations.append({"suite": suite, "status": status, **names, "log": str(log),
            "lexical_only": sorted(leads - observed) if status == "listed" else [],
            "listed_only": sorted(observed - leads) if status == "listed" else [],
            "mapped_not_listed": sorted(required_names - observed) if status == "listed" else []})
    all_listed = bool(observations) and all(row["status"] == "listed" for row in observations)
    return {"schema_version": 1, "purpose": PURPOSE, "source": identity,
        "selected_suite_count": len(suites), "suites": observations,
        "all_selected_suites_listed": all_listed,
        "all_selected_mapped_tests_listed": all_listed and not any(row["mapped_not_listed"] for row in observations),
        "listed_test_count": sum(len(row["tests"]) for row in observations),
        "executed_compatibility_case_count": 0,
        "full_rust_support_established": False, "full_rust_completion_percent": None,
        "limitations": ["Listing is inventory only. No listed test has been executed or proven passing by this receipt.",
                        "Names are specific to this source, client, environment, target and selected suites, not all possible cfg/target/macro configurations.",
                        "Lexical differences require review. Neither listing nor scope membership establishes semantic coverage or reviewed Rust obligations.",
                        "Hashes detect stale or altered receipts, not a malicious operator able to rewrite logs and provenance together."]}


def coverage(directories, c, root, scope, identity, runner):
    """Combine original inventories, never reinterpret them as test execution.

    A successful current retry can resolve a failed listing attempt. Conflicting
    current successful listings cannot be resolved by path order or recency.
    Every attempt remains visible even when another receipt supplies the names.
    """
    if not isinstance(directories, (list, tuple)) or not directories:
        raise c.InvalidEvidence("catalog-coverage requires retained catalog directories")
    canonical = set()
    for directory in directories:
        if not isinstance(directory, (str, Path)):
            raise c.InvalidEvidence("catalog evidence must name a directory")
        directory = Path(directory).resolve()
        receipt = directory / "catalog.json"
        if not directory.is_dir() or receipt.is_symlink() or not receipt.is_file():
            raise c.InvalidEvidence("missing or unsafe catalog receipt")
        canonical.add(directory)
    expected = sorted(c.integration_suites(root))
    by_suite = {suite: [] for suite in expected}
    for directory in sorted(canonical):
        summary = assess(c.load_json(directory / "catalog.json"), directory,
                         c, root, scope, identity, runner)
        for row in summary["suites"]:
            by_suite[row["suite"]].append({**row,
                "receipt": str(directory / "catalog.json")})
    observations = []
    for suite in expected:
        attempts = by_suite[suite]
        current = [row for row in attempts if row["status"] == "listed"]
        signatures = {(tuple(row["tests"]), tuple(row["benchmarks"])) for row in current}
        if len(signatures) > 1:
            status = "conflicting"
        elif current:
            status = "listed"
        else:
            statuses = {row["status"] for row in attempts}
            status = (next(iter(statuses)) if len(statuses) == 1
                      else "unresolved" if statuses else "missing")
        names = {key: current[0][key] if status == "listed" else [] for key in
                 ("tests", "benchmarks", "lexical_only", "listed_only", "mapped_not_listed")}
        observations.append({"suite": suite, "status": status, **names,
            "attempts": [{key: row[key] for key in ("status", "receipt", "log")}
                         for row in attempts]})
    complete = bool(observations) and all(row["status"] == "listed" for row in observations)
    return {"schema_version": 1, "purpose": "libtest-selector-catalog-coverage-v1",
        "source": identity, "evidence_directory_count": len(canonical),
        "expected_suite_count": len(expected),
        "listed_suite_count": sum(row["status"] == "listed" for row in observations),
        "complete_current_inventory": complete,
        "all_mapped_tests_listed": complete and not any(row["mapped_not_listed"] for row in observations),
        "listed_test_count": sum(len(row["tests"]) for row in observations),
        "suites": observations,
        "executed_compatibility_case_count": 0,
        "full_rust_support_established": False, "full_rust_completion_percent": None,
        "limitations": ["Complete means every current integration suite has an authenticated listing, not that any test was executed or passed.",
                        "A current successful retry resolves listing failures only. Earlier failed and stale attempts remain diagnostic observations.",
                        "Conflicting current listings are unresolved, never selected by path order. Names remain target/configuration-specific.",
                        "Neither listed names nor mapped membership establish reviewed semantic coverage or a full Rust denominator."]}


def main(args, base, c):
    root = c.ROOT
    if args.action == "catalog-coverage":
        scope = scope_tools.validate(c.load_json(args.scope), c, root)
        summary = coverage(args.evidence, c, root, scope,
                           c.source_identity(root), runner_identity(c))
        print(c.encoded(summary).decode(), end="")
        return 0 if summary["all_mapped_tests_listed"] else 1
    if args.out is None:
        raise c.InvalidEvidence("catalog and assess-catalog require --out")
    scope = scope_tools.validate(c.load_json(args.scope), c, root)
    if args.action == "catalog":
        suites = selection(args.suite, c, root)
        if args.timeout <= 0:
            raise c.InvalidEvidence("catalog requires a positive --timeout")
        entries = [entry(suite, args.target_dir) for suite in suites]
        if args.dry_run:
            print(c.encoded({"purpose": PURPOSE, "invocations": entries,
                            "executed_compatibility_case_count": 0}).decode(), end="")
            return 0
        runner = runner_identity(c)
        before = c.source_identity(root)
        args.out.mkdir(parents=True, exist_ok=False)
        report = {"schema_version": 1, "purpose": PURPOSE, "selected_suites": suites,
                  "target_dir": args.target_dir, "integration_suites": c.integration_suites(root),
                  "source_before": before, "source_after": None,
                  "runner_before": runner, "runner_after": None, "results": []}
        receipt = args.out / "catalog.json"
        receipt.write_bytes(c.encoded(report))
        for invocation in entries:
            report["results"].append(c.execute(invocation, args.out, args.timeout, root))
            receipt.write_bytes(c.encoded(report))
        report["source_after"] = c.source_identity(root)
        report["runner_after"] = runner_identity(c)
        receipt.write_bytes(c.encoded(report))
    else:
        report = c.load_json(args.out / "catalog.json")
    summary = assess(report, args.out, c, root, scope, c.source_identity(root), runner_identity(c))
    print(c.encoded(summary).decode(), end="")
    return 0 if summary["all_selected_mapped_tests_listed"] else 1
