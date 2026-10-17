#!/usr/bin/env python3
"""Stock rejection oracle and explicit post-integration native CLI gate.

No compiler build, install, fallback, or source rewriting. See the adjacent
slice-contract-rejection-README.md for evidence limits and commands.
"""
import argparse
import copy
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "tests/fixtures/slice-contract-rejection"
CONTROL = ROOT / "fixtures/adler2_adapted/cases/four.rs"


def require(condition, message):
    if not condition:
        raise ValueError(message)


def identity(path):
    return {"path": str(path), "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}


def cases_from(directory):
    cases = []
    for path in sorted(directory.glob("*.rs")):
        text = path.read_text()
        stock = re.findall(r"^// stock: (accept|reject)\s*$", text, re.MULTILINE)
        category = re.findall(r"^// category: ([a-z-]+)\s*$", text, re.MULTILINE)
        require(len(stock) == len(category) == 1,
                f"{path}: requires exactly one // stock: accept|reject and // category: NAME")
        require(stock[0] == "reject" or category[0] == "excluded",
                f"{path}: stock-accepted fixtures must identify an excluded boundary")
        cases.append({"source": identity(path), "path": path,
                      "stock": stock[0], "category": category[0]})
    categories = {case["category"] for case in cases}
    require({"lifetime", "unused-invalid-body", "excluded"} <= categories,
            "corpus must include lifetime, unused-invalid-body and excluded categories")
    return cases


def validate_graph(profile, mode, workers, rejected):
    require(profile.get("schema") == 1, "unknown profile schema")
    require(profile.get("mode") == mode and profile.get("workers") == workers,
            "profile mode/worker mismatch")
    require(profile.get("success") is (not rejected), "profile success mismatch")
    graph = profile.get("graph")
    require(isinstance(graph, dict), "missing observed graph")
    require(graph.get("capture_complete") is True and graph.get("events_dropped") == 0,
            "incomplete graph capture")
    require(graph.get("omitted_nodes") == 0, "profile omits nodes: cannot prove no object publication")
    nodes = graph.get("top_nodes")
    phases = graph.get("phase_intervals")
    require(isinstance(nodes, list) and isinstance(phases, list) and nodes,
            "missing graph nodes/phases")
    require(graph.get("nodes") == len(nodes), "graph node accounting mismatch")
    if rejected:
        require(isinstance(profile.get("error"), str) and profile["error"], "missing failure diagnostic")
        require(graph.get("failures", 0) > 0, "missing graph failure")
    else:
        require(profile.get("error") is None and graph.get("failures") == 0,
                "successful control contains failure")
    # Both lists must exclude object work, including zero-duration evaluations.
    if rejected or mode == "check":
        require(not any(row.get("kind") == 103 or row.get("phase") == "object"
                        for row in nodes + phases), "object work present before rejection/in check")
    if not rejected:
        kinds = {row.get("kind") for row in nodes}
        require(109 in kinds or 112 in kinds, "control lacks ownership analysis prerequisite")
        if mode == "build":
            require(103 in kinds, "positive native build lacks object work")


def validate_diagnostic(stderr, category):
    lower = stderr.lower()
    require(bool(stderr.strip()), "rejection has no stderr diagnostic")
    require(not any(word in lower for word in ("panicked at", "parse error", "unexpected token")),
            "parser/crash rejection is not semantic slice acceptance evidence")
    if category == "excluded":
        require("unsupported" in lower, "excluded legal case lacks explicit unsupported diagnostic")
    else:
        require("unsupported" not in lower, "Rust-invalid case hidden behind unsupported boundary")


class Runner:
    def __init__(self, output, timeout):
        self.output = output
        self.timeout = timeout
        self.processes = []

    def run(self, argv, cwd):
        argv = list(map(str, argv))
        row = {"argv": argv, "cwd": str(cwd)}
        number = len(self.processes)
        self.processes.append(row)
        try:
            result = subprocess.run(argv, cwd=cwd, stdin=subprocess.DEVNULL,
                                    capture_output=True, timeout=self.timeout, check=False)
            row.update(exit=result.returncode, timeout=False)
            stdout, stderr = result.stdout, result.stderr
        except subprocess.TimeoutExpired as error:
            row.update(exit=None, timeout=True)
            stdout, stderr = error.stdout or b"", error.stderr or b""
        for name, data in (("stdout", stdout), ("stderr", stderr)):
            path = self.output / f"process-{number:04}.{name}"
            path.write_bytes(data)
            row[name] = data.decode(errors="replace")
            row[name + "_artifact"] = identity(path)
        require(not row["timeout"], f"command timed out: {argv}")
        require(row["exit"] is not None and row["exit"] >= 0, f"command crashed: {argv}")
        return row


def native_case(runner, binary, source, directory, mode, workers, category=None):
    work = directory / f"{source.stem}-{workers}-{mode}"
    work.mkdir()
    local = work / "input.rs"
    shutil.copyfile(source, local)
    executable, profile, trace = work / "program", work / "profile.json", work / "trace.txt"
    command = [binary, local, "--workers", workers, "--profile", profile, "--trace", trace]
    command += ["--check"] if mode == "check" else ["-o", executable]
    row = runner.run(command, work)
    rejected = category is not None
    # Preserve the evidence even when the exit status or graph is wrong.
    for name, path in (("profile", profile), ("trace", trace)):
        if path.is_file():
            saved = runner.output / f"native-{source.stem}-{workers}-{mode}.{name}"
            shutil.copyfile(path, saved)
            row[name + "_artifact"] = identity(saved)
    require((row["exit"] != 0) == rejected, f"unexpected native exit for {source.name}/{mode}/{workers}")
    require(profile.is_file() and trace.is_file(), "native CLI omitted profile or trace")
    data = json.loads(profile.read_text())
    row["profile"] = data
    validate_graph(data, mode, workers, rejected)
    if rejected:
        validate_diagnostic(row["stderr"], category)
        require(not executable.exists(), "failed compile published executable")
        require(trace.read_text().startswith("compilation failed:"), "failure trace lacks failure header")
    elif mode == "build":
        require(executable.is_file() and executable.read_bytes().startswith(b"\x7fELF"),
                "positive native build did not publish ELF")
    else:
        require(not executable.exists(), "check published executable")
    expected = {"input.rs", "profile.json", "trace.txt"}
    if not rejected and mode == "build":
        expected.add("program")
    require({path.name for path in work.iterdir()} == expected,
            "unexpected output/object publication in fresh native working directory")
    return executable if not rejected and mode == "build" else None


def self_test():
    profile = {"schema": 1, "mode": "check", "workers": 1, "success": False,
               "error": "type mismatch", "graph": {"capture_complete": True,
               "events_dropped": 0, "omitted_nodes": 0, "nodes": 1,
               "failures": 1, "top_nodes": [{"kind": 102, "phase": "body"}],
               "phase_intervals": [{"kind": 102, "phase": "body"}]}}
    validate_graph(profile, "check", 1, True)
    mutations = [lambda p: p.update(success=True),
                 lambda p: p["graph"].update(capture_complete=False),
                 lambda p: p["graph"].update(events_dropped=1),
                 lambda p: p["graph"].update(omitted_nodes=1),
                 lambda p: p["graph"].update(failures=0),
                 lambda p: p["graph"]["top_nodes"][0].update(kind=103),
                 lambda p: p["graph"]["phase_intervals"][0].update(kind=103),
                 lambda p: p["graph"].update(nodes=2)]
    for mutate in mutations:
        bad = copy.deepcopy(profile)
        mutate(bad)
        try:
            validate_graph(bad, "check", 1, True)
        except ValueError:
            continue
        raise AssertionError("invalid profile was accepted")
    for text, category in (("unsupported slice", "lifetime"), ("type mismatch", "excluded"),
                           ("parse error", "lifetime"), ("panicked at test", "excluded")):
        try:
            validate_diagnostic(text, category)
        except ValueError:
            continue
        raise AssertionError("invalid diagnostic was accepted")
    print("PASS validator self-test: valid failure + 8 graph mutations + 4 diagnostic mutations")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--fixtures", type=Path, default=FIXTURES)
    parser.add_argument("--rustc", default="rustc")
    parser.add_argument("--output-dir", type=Path)
    parser.add_argument("--native", type=Path, help="explicit post-integration jr-rust executable")
    parser.add_argument("--positive-control", type=Path, default=CONTROL)
    parser.add_argument("--timeout-seconds", type=float, default=30)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return 0
    if args.output_dir is None or args.timeout_seconds <= 0:
        parser.error("--output-dir and positive --timeout-seconds required")
    rustc = shutil.which(args.rustc)
    if rustc is None:
        parser.error("stock rustc executable not found")
    cases = cases_from(args.fixtures.resolve())
    if args.native is not None:
        args.native = args.native.resolve(strict=True)
        require(args.positive_control.is_file(), "positive slice control is required for native mode")
        require("&[u8]" in args.positive_control.read_text().replace(" ", ""),
                "positive control must exercise shared byte slices")
    output = args.output_dir.resolve()
    output.mkdir(parents=True, exist_ok=False)
    runner = Runner(output, args.timeout_seconds)
    sources = output / "sources"
    sources.mkdir()
    for case in cases:
        shutil.copyfile(case["path"], sources / case["path"].name)
    if args.native is not None:
        shutil.copyfile(args.positive_control, sources / "positive-control.rs")
    report = {"schema": 1, "runner": identity(Path(__file__)),
              "claim": "rejection acceptance only, not native integration or ABI correctness",
              "native_status": "not_run" if args.native is None else "requested_not_validated",
              "cases": [{k: v for k, v in c.items() if k != "path"} for c in cases],
              "processes": runner.processes, "passed": False}
    try:
        with tempfile.TemporaryDirectory(prefix="slice-rejection-") as temp:
            directory = Path(temp)
            report["rustc_version"] = runner.run([rustc, "-Vv"], directory)
            require(report["rustc_version"]["exit"] == 0, "rustc identity failed")
            report["rustc"] = identity(Path(rustc).resolve())
            for case in cases:
                work = directory / ("stock-" + case["path"].stem)
                work.mkdir()
                metadata = work / "program.rmeta"
                row = runner.run([rustc, case["path"], "--edition=2021", "--crate-name",
                                  "slice_rejection", "--emit=metadata", "-o", metadata], work)
                require((row["exit"] == 0) == (case["stock"] == "accept"),
                        f"stock expectation mismatch: {case['path'].name}")
                if case["stock"] == "reject":
                    require(not metadata.exists(), "stock rejection published metadata")
            if args.native is not None:
                report["native"] = identity(args.native)
                report["positive_control"] = identity(args.positive_control.resolve())
                # Mandatory positive admission prevents unsupported-parser false passes.
                control = args.positive_control.resolve()
                oracle = directory / "oracle"
                stock = runner.run([rustc, control, "--edition=2021", "--crate-name",
                                    "slice_control", "-o", oracle], directory)
                require(stock["exit"] == 0, "positive stock control rejected")
                expected = runner.run([oracle], directory)
                require(expected["exit"] == 0, "positive stock control crashed")
                for workers in (1, 8):
                    for mode in ("check", "build"):
                        executable = native_case(runner, args.native, control, directory, mode, workers)
                        if executable:
                            observed = runner.run([executable], directory)
                            require(observed["exit"] == expected["exit"] and
                                    observed["stdout"] == expected["stdout"] and
                                    observed["stderr"] == expected["stderr"], "positive control runtime mismatch")
                report["native_status"] = "positive_control_passed_rejections_pending"
                for case in cases:
                    for workers in (1, 8):
                        for mode in ("check", "build"):
                            native_case(runner, args.native, case["path"], directory,
                                        mode, workers, case["category"])
                report["native_status"] = "public_cli_rejection_gate_passed"
            report["passed"] = True
    except (ValueError, OSError, json.JSONDecodeError) as error:
        report["failure"] = str(error)
        print(f"FAIL: {error}", file=sys.stderr)
    finally:
        (output / "evidence.json").write_text(json.dumps(report, indent=2) + "\n")
    print(f"{'PASS' if report['passed'] else 'FAIL'} {len(cases)} cases; "
          f"native={report['native_status']}; evidence={output / 'evidence.json'}")
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
