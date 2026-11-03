#!/usr/bin/env python3
"""Executable compatibility evidence index. No third-party Python dependencies.

A green run is evidence for its exact selected gates, never full Rust support.
Use plan before run. Runtime reports/logs belong in scratch or ignored target/.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
MANIFEST = Path("data/compiler-conformance.json")
KINDS = {"tests", "lib", "all-tests", "clippy", "fmt", "case"}
STATES = {"open", "partial", "implemented-slice"}
ENV_KEYS = ("JUSTRUST_REMOTE_REQUIRED", "JUSTRUST_REMOTE", "JUSTRUST_TEST_PARALLEL",
            "JR_STOCK_RUSTC", "JUSTRUST_REAL_RUSTC", "CARGO_PROFILE_DEV_DEBUG",
            "CARGO_PROFILE_TEST_DEBUG", "CARGO_INCREMENTAL", "RUSTFLAGS", "RUSTUP_TOOLCHAIN")


class InvalidEvidence(ValueError):
    pass


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def encoded(value) -> bytes:
    return (json.dumps(value, sort_keys=True, indent=2) + "\n").encode()


def load_json(path: Path):
    try:
        return json.loads(path.read_text())
    except (OSError, ValueError) as error:
        raise InvalidEvidence(f"cannot read JSON {path}: {error}") from error


def integration_suites(root: Path = ROOT) -> list[str]:
    return sorted(path.stem for path in (root / "tests").glob("*.rs"))


def unique_ids(rows: list, label: str) -> set[str]:
    if not isinstance(rows, list) or not rows:
        raise InvalidEvidence(f"{label} must be a nonempty array")
    ids = []
    for row in rows:
        name = row.get("id") if isinstance(row, dict) else None
        if not isinstance(name, str) or not re.fullmatch(r"[a-z][a-z0-9-]*", name):
            raise InvalidEvidence(f"invalid {label} identifier: {name!r}")
        ids.append(name)
    if len(ids) != len(set(ids)):
        raise InvalidEvidence(f"duplicate {label} identifier")
    return set(ids)


def validate(data: dict, root: Path = ROOT) -> dict:
    if not isinstance(data, dict) or data.get("schema_version") != 1:
        raise InvalidEvidence("unsupported manifest schema")
    contract = data.get("contract", {})
    if contract.get("semantic_inventory") != "incomplete":
        raise InvalidEvidence("this finite inventory cannot certify specification completeness")
    if contract.get("edition") != "2024" or contract.get("target") != "x86_64-unknown-linux-gnu":
        raise InvalidEvidence("unreviewed edition/target contract")
    oracle = contract.get("oracle", {})
    verifier = root / oracle.get("verifier", "absent")
    text = verifier.read_text() if verifier.is_file() else ""
    if not all(isinstance(oracle.get(key), str) and oracle[key] in text
               for key in ("release", "commit", "llvm")):
        raise InvalidEvidence("oracle contract is not bound to the exact existing verifier")
    gates = unique_ids(data.get("gates"), "gate")
    requirements = unique_ids(data.get("requirements"), "requirement")
    suites = set(integration_suites(root))
    for gate in data["gates"]:
        if gate.get("kind") not in KINDS:
            raise InvalidEvidence(f"unknown gate kind: {gate['id']}")
        if not isinstance(gate.get("profiles"), list) or any(
                name not in {"smoke", "full"} for name in gate["profiles"]):
            raise InvalidEvidence(f"invalid profiles: {gate['id']}")
        if gate["kind"] == "tests":
            selected = gate.get("suites")
            if not selected or len(set(selected)) != len(selected) or not set(selected) <= suites:
                raise InvalidEvidence(f"missing or duplicate integration suites: {gate['id']}")
        elif gate["kind"] == "case":
            if (gate.get("suite") not in suites
                    or not re.fullmatch(r"[a-zA-Z_][a-zA-Z0-9_]*", gate.get("test", ""))):
                raise InvalidEvidence(f"invalid exact test selector: {gate['id']}")
        elif "suites" in gate:
            raise InvalidEvidence(f"unexpected suite filter on {gate['id']}")
    for requirement in data["requirements"]:
        if requirement.get("state") not in STATES or not requirement.get("remaining"):
            raise InvalidEvidence(f"missing honest scope/remainder: {requirement['id']}")
        if not requirement.get("gates") or not set(requirement["gates"]) <= gates:
            raise InvalidEvidence(f"unmapped requirement: {requirement['id']}")
    gap_ids = set()
    for gap in data.get("known_gaps", []):
        if gap.get("id") in gap_ids or gap.get("requirement") not in requirements:
            raise InvalidEvidence("duplicate or unmapped known gap")
        gap_ids.add(gap.get("id"))
        if gap.get("suite") not in suites or gap.get("classification") not in {
                "valid-rejected", "invalid-accepted", "wrong-code", "untested"}:
            raise InvalidEvidence("invalid known gap classification/suite")
        if not gap.get("test") or gap["test"] not in (root / "tests" / (gap["suite"] + ".rs")).read_text():
            raise InvalidEvidence("known gap no longer references an existing test")
    return data


def command(gate: dict, target_dir: str) -> list[str]:
    kind = gate["kind"]
    if kind == "fmt":
        return ["cargo", "fmt", "--all", "--check"]
    if kind == "clippy":
        return ["justrust", "clippy", "--all-targets", "--target-dir", target_dir, "--", "-D", "warnings"]
    args = ["justrust", "test", "--no-fail-fast", "--target-dir", target_dir]
    if kind == "tests":
        for suite in gate["suites"]:
            args.extend(["--test", suite])
    elif kind == "case":
        args.extend(["--test", gate["suite"]])
        return args + ["--", gate["test"], "--exact", "--include-ignored", "--nocapture", "--test-threads=1"]
    elif kind == "lib":
        args.append("--lib")
    else:
        args.append("--all-targets")
    return args + ["--", "--test-threads=4"]


def plan(data: dict, profile: str, selected: list[str], target_dir: str) -> list[dict]:
    if profile not in {"smoke", "full"} or not isinstance(selected, list) or not target_dir:
        raise InvalidEvidence("invalid profile, gate selection or target directory")
    known = {gate["id"] for gate in data["gates"]}
    if set(selected) - known or len(selected) != len(set(selected)):
        raise InvalidEvidence("unknown or duplicate selected gates")
    gates = [gate for gate in data["gates"]
             if gate["id"] in selected or (not selected and profile in gate["profiles"])]
    if not gates:
        raise InvalidEvidence("empty plan")
    return [{"id": gate["id"], "kind": gate["kind"], "command": command(gate, target_dir)}
            for gate in gates]


def source_identity(root: Path = ROOT) -> dict:
    try:
        revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root).decode().strip()
        paths = subprocess.check_output(
            ["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard"], cwd=root)
    except (OSError, subprocess.CalledProcessError) as error:
        raise InvalidEvidence(f"git provenance unavailable: {error}") from error
    # Include all compiler, library, fixture, runner and build configuration inputs.
    # Historical measurement logs and unrelated agent locks are not compiler inputs.
    inputs = sorted({os.fsdecode(path) for path in paths.split(b"\0") if path and (
        os.fsdecode(path).startswith(("src/", "libraries/", "tests/", "tools/", ".cargo/", "docs/compiler/"))
        or os.fsdecode(path) in {"Cargo.toml", "Cargo.lock", str(MANIFEST),
                                "data/compiler-compatibility-scope.json",
                                "data/compiler-compatibility-contract.json",
                                "data/compiler-language-inventory.json",
                                "data/compiler-library-project-inventory.json",
                                ".github/workflows/compiler-conformance.yml"})})
    state = hashlib.sha256()
    for name in inputs:
        path = root / name
        if path.is_symlink():
            value = ["symlink", os.readlink(path), digest(path.read_bytes()) if path.is_file() else None]
        elif path.is_file():
            value = ["file", digest(path.read_bytes())]
        else:
            value = ["deleted"]
        state.update(encoded([name, value]))
    return {"revision": revision, "inputs_sha256": state.hexdigest(), "input_files": len(inputs)}


def discover(root: Path = ROOT) -> dict:
    """Lexical discovery flags needing triage, not semantic classifications."""
    markers = []
    oracle_selectors = []
    test_functions = []
    for path in sorted((root / "tests").rglob("*.rs")):
        lines = path.read_text().splitlines()
        pending_test = False
        for number, line in enumerate(lines, 1):
            if re.match(r"\s*#\[test\]", line):
                pending_test = True
            function = re.match(r"\s*(?:pub\s+)?fn\s+(\w+)\s*\(", line)
            if pending_test and function:
                test_functions.append({"file": str(path.relative_to(root)), "line": number,
                                       "test": function.group(1)})
                pending_test = False
            if re.search(r"(?i)unsupported|#\[ignore|known.?gap|wrong.?code", line):
                markers.append({"file": str(path.relative_to(root)), "line": number,
                                "text": line.strip()[:240]})
            if re.search(r'Command::new\s*\(\s*"rustc"\s*\)|var(?:_os)?\s*\(\s*"(?:RUSTC|JUSTRUST_REAL_RUSTC)"', line):
                oracle_selectors.append({"file": str(path.relative_to(root)), "line": number,
                                         "text": line.strip()[:240]})
    manifest = root / "tests/fixtures/full_programs/manifest.tsv"
    return {"integration_suites": integration_suites(root), "source_test_functions": test_functions,
            "triage_markers": markers,
            "oracle_selector_leads": oracle_selectors,
            "warning": "Lexical markers include rejection assertions, comments and source strings. Oracle selector leads require identity review, not automatic rewriting. Do not count them as features or proven gaps.",
            "full_program_manifest": str(manifest.relative_to(root)) if manifest.exists() else None}


def test_count(text: str) -> int:
    # Stable client's summary or direct libtest output. Never infer support from exit zero alone.
    counts = re.findall(r"justrust: ok: (\d+) tests? passed", text)
    if not counts:
        counts = re.findall(r"test result: ok\. (\d+) passed;", text)
    return sum(map(int, counts))


def client_retried_failures(text: str) -> bool:
    # Match complete compiler-owned diagnostics, not user test names containing
    # "retry", "parallel" or "failures". The installed client can replay failed
    # tests itself even though this driver never retries a command.
    markers = (
        r"justrust: [^:\r\n]+: \S+ failed while other test processes ran in parallel "
        r"and passed when rerun alone by name; not counted as failures",
        r"justrust: [^:\r\n]+: confirming \d+ failures by replaying the original test "
        r"group alone instead of retrying each test serially",
    )
    return any(re.fullmatch(marker, line) for line in text.splitlines() for marker in markers)


def outcome(returncode: int | None, text: str, kind: str, timed_out: bool = False) -> str:
    if timed_out:
        return "timeout"
    if returncode is None:
        return "infrastructure"
    if returncode != 0:
        return "failed"
    if client_retried_failures(text):
        return "unverified"
    if "0 tests matched" in text:
        return "unverified"
    if kind in {"tests", "lib", "all-tests", "case"} and test_count(text) == 0:
        return "unverified"
    if kind == "case" and test_count(text) != 1:
        return "unverified"
    return "passed"


def execute(entry: dict, directory: Path, timeout: int, root: Path = ROOT) -> dict:
    log = directory / (entry["id"] + ".log")
    started = time.monotonic()
    code, timed_out = None, False
    env = dict(os.environ)
    env["JUSTRUST_TEST_PARALLEL"] = "0"
    # Do not inject an oracle executable into the compiler's native path.
    # Individual differential harnesses validate their stock oracle separately.
    with log.open("wb") as output:
        try:
            child = subprocess.Popen(entry["command"], cwd=root, env=env, stdout=output,
                                     stderr=subprocess.STDOUT, start_new_session=True)
            try:
                code = child.wait(timeout=timeout)
            except subprocess.TimeoutExpired:
                timed_out = True
                os.killpg(child.pid, signal.SIGKILL)
                child.wait()
        except OSError as error:
            output.write(str(error).encode())
    payload = log.read_bytes()
    text = payload.decode(errors="replace")
    result = {**entry, "outcome": outcome(code, text, entry["kind"], timed_out),
              "returncode": code, "timed_out": timed_out,
              "seconds": round(time.monotonic() - started, 3),
              "tests_passed": test_count(text), "log": log.name, "log_sha256": digest(payload)}
    # Preserve the full original log, including any client-owned retry. Such a
    # retry cannot turn this gate green even when the client exits successfully.
    print(f"{entry['id']}: {result['outcome']} ({result['seconds']}s), {log}", flush=True)
    if result["outcome"] != "passed":
        print(text, flush=True)
    return result


def assess(data: dict, report: dict, directory: Path, identity: dict, root: Path = ROOT) -> dict:
    problems = []
    if report.get("schema_version") != 1:
        raise InvalidEvidence("unsupported report schema")
    if report.get("manifest_sha256") != digest(encoded(data)):
        problems.append("manifest changed")
    if report.get("source_before") != identity or report.get("source_after") != identity:
        problems.append("source revision or compiler inputs changed")
    expected = plan(data, report.get("profile", ""), report.get("selected", []), report.get("target_dir", ""))
    entries = report.get("results", [])
    if not isinstance(entries, list) or not all(isinstance(row, dict) for row in entries):
        raise InvalidEvidence("malformed gate results")
    if [row.get("id") for row in entries] != [row["id"] for row in expected]:
        problems.append("missing, reordered or duplicate gate results")
    verified = set()
    for entry, required in zip(entries, expected):
        if any(entry.get(key) != required[key] for key in ("id", "kind", "command")):
            problems.append(f"changed invocation: {required['id']}")
            continue
        log = directory / str(entry.get("log", ""))
        if log.resolve().parent != directory.resolve() or not log.is_file():
            problems.append(f"missing or unsafe log: {required['id']}")
            continue
        payload = log.read_bytes()
        text = payload.decode(errors="replace")
        measured = outcome(entry.get("returncode"), text, required["kind"], entry.get("timed_out", False))
        if (digest(payload) != entry.get("log_sha256") or measured != entry.get("outcome")
                or entry.get("tests_passed") != test_count(text)):
            problems.append(f"tampered/inconsistent gate evidence: {required['id']}")
        elif measured == "passed":
            verified.add(required["id"])
        else:
            problems.append(f"{required['id']}: {measured}")
    full_gates = {gate["id"] for gate in data["gates"] if "full" in gate["profiles"]}
    all_suites = integration_suites(root)
    if report.get("integration_suites") != all_suites:
        problems.append("integration suite inventory changed")
    return {"selected_gates_passed": not problems,
            "full_gate_set_passed": not problems and full_gates <= verified,
            "full_rust_support_established": False,
            "reason": "Finite suites do not establish complete language/library/project coverage. The semantic inventory and open requirements remain explicit.",
            "problems": problems,
            "requirements": [{"id": row["id"], "scope_state": row["state"],
                               "mapped_gates_passed": not problems and set(row["gates"]) <= verified,
                               "remaining": row["remaining"]} for row in data["requirements"]],
            "known_gaps": data["known_gaps"]}


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["validate", "plan", "discover", "run", "assess", "coverage", "track"])
    parser.add_argument("--manifest", type=Path, default=ROOT / MANIFEST)
    parser.add_argument("--profile", choices=["smoke", "full"], default="smoke")
    parser.add_argument("--gate", action="append", default=[])
    parser.add_argument("--target-dir", default=str(ROOT / "target/justrust-slots/0"))
    parser.add_argument("--out", type=Path, help="new output directory for run, existing report directory for assess")
    parser.add_argument("--timeout", type=int, default=3600, help="seconds per gate")
    parser.add_argument("--scope", type=Path, default=ROOT / "data/compiler-compatibility-scope.json")
    parser.add_argument("--area", action="append", default=[], help="track only these compatibility areas")
    parser.add_argument("--case", action="append", default=[], help="track exact mapped suite::test")
    parser.add_argument("--evidence", type=Path, action="append", default=[], help="coverage: retained track directory")
    parser.add_argument("--json", action="store_true", help="machine-readable layered coverage")
    parser.add_argument("--dry-run", action="store_true", help="show exact track invocations without executing")
    parser.add_argument("--jobs", type=int, default=1, help="track: independent exact-case processes, 1..8")
    parser.add_argument("--shard", action="append", default=[], metavar="INDEX/COUNT",
                        help="track only: zero-based partition of sorted area/case-selected IDs")
    args = parser.parse_args(argv)
    try:
        if args.shard and args.action != "track":
            raise InvalidEvidence("--shard is only valid with track")
        data = validate(load_json(args.manifest))
        if args.action in {"coverage", "track"}:
            import compatibility_scope
            return compatibility_scope.main(args, data, sys.modules[__name__])
        if args.action == "validate":
            import compatibility_scope
            scope = compatibility_scope.validate(load_json(args.scope), sys.modules[__name__], ROOT)
            print(json.dumps({"manifest": "valid", "requirements": len(data["requirements"]),
                              "scope_requirements": sum(len(area["requirements"]) for area in scope["areas"]),
                              "suites": len(integration_suites()), "full_rust_support_established": False}))
        elif args.action == "plan":
            print(encoded({"contract": data["contract"], "gates": plan(data, args.profile, args.gate, args.target_dir),
                           "known_gaps": data["known_gaps"]}).decode(), end="")
        elif args.action == "discover":
            print(encoded(discover()).decode(), end="")
        elif args.action == "assess":
            if args.out is None:
                raise InvalidEvidence("assess requires --out report directory")
            result = assess(data, load_json(args.out / "report.json"), args.out, source_identity())
            print(encoded(result).decode(), end="")
            return 0 if result["selected_gates_passed"] else 1
        else:
            if args.out is None or args.timeout <= 0:
                raise InvalidEvidence("run requires a new --out directory and positive timeout")
            entries = plan(data, args.profile, args.gate, args.target_dir)
            args.out.mkdir(parents=True, exist_ok=False)
            before = source_identity()
            report = {"schema_version": 1, "manifest_sha256": digest(encoded(data)),
                      "source_before": before, "profile": args.profile, "selected": args.gate,
                      "target_dir": args.target_dir, "integration_suites": integration_suites(),
                      "environment": {**{key: os.environ.get(key) for key in ENV_KEYS},
                                      "JUSTRUST_TEST_PARALLEL": "0"}, "results": []}
            (args.out / "discovery.json").write_bytes(encoded(discover()))
            for entry in entries:
                report["results"].append(execute(entry, args.out, args.timeout))
                (args.out / "report.json").write_bytes(encoded(report))
            report["source_after"] = source_identity()
            (args.out / "report.json").write_bytes(encoded(report))
            result = assess(data, report, args.out, source_identity())
            (args.out / "assessment.json").write_bytes(encoded(result))
            print(encoded(result).decode(), end="")
            return 0 if result["selected_gates_passed"] else 1
        return 0
    except (InvalidEvidence, OSError, TypeError, KeyError) as error:
        print(f"conformance: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
