#!/usr/bin/env python3
"""Bounded exact-source slice runtime contract, not full Rust acceptance."""

import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time
from datetime import datetime, timezone


CASES = ("positive.rs", "bounds-empty.rs", "bounds-equal.rs", "bounds-max.rs")
ROOT = Path(__file__).resolve().parents[1]


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def text(data):
    return data.decode("utf-8", errors="replace")


def command(argv, cwd):
    start = time.monotonic_ns()
    result = {"argv": [str(arg) for arg in argv], "cwd": str(cwd)}
    try:
        completed = subprocess.run(result["argv"], cwd=cwd, capture_output=True,
                                   timeout=60, check=False)
        stdout, stderr = completed.stdout, completed.stderr
        result.update(returncode=completed.returncode,
                      status="ok" if completed.returncode == 0 else "nonzero")
    except subprocess.TimeoutExpired as error:
        stdout, stderr = error.stdout or b"", error.stderr or b""
        result.update(returncode=None, status="timeout")
    except OSError as error:
        stdout, stderr = b"", str(error).encode()
        result.update(returncode=None, status="launch_error")
    result.update(elapsed_ns=time.monotonic_ns() - start,
                  stdout=text(stdout), stderr=text(stderr),
                  stdout_base64=base64.b64encode(stdout).decode("ascii"),
                  stderr_base64=base64.b64encode(stderr).decode("ascii"))
    return result


def identity(executable):
    resolved = shutil.which(executable)
    if resolved is None:
        raise RuntimeError("executable not found: " + executable)
    path = Path(resolved).absolute()
    return {"requested": executable, "path": str(path),
            "resolved_path": str(path.resolve()), "sha256": digest(path)}


def save(path, evidence):
    # Publish a current incomplete/failing report before any compiler is invoked.
    # Atomic replacement prevents an interrupted rewrite preserving old success.
    fd, temporary = tempfile.mkstemp(prefix=".evidence-", dir=path.parent)
    try:
        with os.fdopen(fd, "w") as stream:
            json.dump(evidence, stream, indent=2)
            stream.write("\n")
        os.replace(temporary, path)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


def exercise(tool, source, directory, workers=None, trace=False):
    directory.mkdir(mode=0o700)
    executable = directory / "program"
    expected = (b"slice-contract-runtime:ok\n" if source.name == "positive.rs"
                else b"slice-contract-runtime:before\n")
    row = {"source": source.name, "workers": workers, "status": "running",
           "expected_stdout": text(expected),
           "expected_exit": "zero" if source.name == "positive.rs" else "nonzero_or_signal"}
    if workers is None:
        common = [tool, str(source), "--crate-name", source.stem.replace("-", "_")]
        check_args = common + ["--emit=metadata", "-o", str(directory / "check.rmeta")]
        build_args = common + ["-o", str(executable)]
    else:
        common = [tool, str(source), "--workers", str(workers)]
        check_args = common + ["--check"]
        build_args = common + ["-o", str(executable)]
        if trace:
            check_args += ["--trace", str(directory / "check.trace")]
            build_args += ["--trace", str(directory / "build.trace")]
    row["check"] = command(check_args, directory)
    row["build"] = command(build_args, directory)
    if workers is not None:
        row["traces"] = {"status": "captured_raw" if trace else "not_supported"}
        for phase in ("check", "build"):
            path = directory / (phase + ".trace")
            if path.is_file():
                row["traces"][phase] = {"path": str(path), "sha256": digest(path),
                                           "content": text(path.read_bytes())}
            elif trace:
                row["traces"][phase] = {"status": "missing"}
    compiled = (row["check"]["status"] == "ok" and
                row["build"]["status"] == "ok" and executable.is_file())
    if not compiled:
        row.update(status="compilation_failed", runtime={"status": "not_run"})
        return row
    row["executable_sha256"] = digest(executable)
    runtime = command([str(executable)], directory)
    row["runtime"] = runtime
    actual = base64.b64decode(runtime["stdout_base64"])
    code = runtime["returncode"]
    exit_matches = (code == 0 if source.name == "positive.rs"
                    else code is not None and code != 0)
    passed = (runtime["status"] in ("ok", "nonzero") and exit_matches and
              actual == expected and b"slice-contract-runtime:after" not in actual)
    row.update(status="passed" if passed else "runtime_failed",
               observed_stdout=runtime["stdout"], stdout_matches=actual == expected,
               exit_matches=exit_matches)
    return row


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rustc", default="rustc")
    parser.add_argument("--native", help="explicit path to integrated jr-rust")
    parser.add_argument("--output-dir", required=True, type=Path)
    args = parser.parse_args()
    output = args.output_dir.resolve()
    output.mkdir(parents=True, exist_ok=True)
    evidence_path = output / "evidence.json"
    evidence = {"schema_version": 1, "status": "incomplete",
                "started_utc": datetime.now(timezone.utc).isoformat(),
                "scope": "bounded shared-byte-slice runtime prerequisite only",
                "stock": {"status": "not_run", "cases": []},
                "native": {"status": "not_run", "requested": args.native is not None,
                           "cases": []}, "sources": []}
    save(evidence_path, evidence)
    try:
        # A unique private directory retains immutable source snapshots and raw traces.
        scratch = Path(tempfile.mkdtemp(prefix="run-", dir=output))
        evidence["scratch_directory"] = str(scratch)
        for name in CASES:
            original = ROOT / "fixtures" / "slice-contract-runtime" / name
            snapshot = scratch / name
            snapshot.write_bytes(original.read_bytes())
            evidence["sources"].append({"name": name, "original": str(original),
                                        "snapshot": str(snapshot), "sha256": digest(snapshot)})
        evidence["checkout_revision_not_binary_revision"] = command(
            ["git", "-C", str(ROOT), "rev-parse", "HEAD"], scratch)
        stock = evidence["stock"]
        stock["identity"] = identity(args.rustc)
        stock["version"] = command([stock["identity"]["path"], "-Vv"], scratch)
        if stock["version"]["status"] != "ok":
            raise RuntimeError("rustc -Vv failed")
        stock["status"] = "running"
        save(evidence_path, evidence)
        for name in CASES:
            stock["cases"].append(exercise(stock["identity"]["path"], scratch / name,
                                           scratch / ("stock-" + Path(name).stem)))
            save(evidence_path, evidence)
        stock["status"] = ("passed" if all(row["status"] == "passed" for row in stock["cases"])
                           else "failed")
        if args.native is not None:
            native = evidence["native"]
            native["status"] = "running"
            native["identity"] = identity(args.native)
            native["help"] = command([native["identity"]["path"], "--help"], scratch)
            trace = (native["help"]["status"] == "ok" and
                     "--trace" in native["help"]["stdout"])
            native["trace_supported_by_help"] = trace
            native["binary_revision"] = "unknown: binary hash is authoritative"
            native["status"] = "running"
            save(evidence_path, evidence)
            for workers in (1, 8):
                for name in CASES:
                    native["cases"].append(exercise(
                        native["identity"]["path"], scratch / name,
                        scratch / ("native-" + str(workers) + "-" + Path(name).stem),
                        workers=workers, trace=trace))
                    save(evidence_path, evidence)
            native["status"] = ("passed" if all(row["status"] == "passed" for row in native["cases"])
                                else "failed")
        evidence["status"] = ("passed" if stock["status"] == "passed" and
                              evidence["native"]["status"] in ("passed", "not_run") else "failed")
    except Exception as error:
        evidence.update(status="failed", error=str(error))
        for backend in ("stock", "native"):
            if evidence[backend]["status"] == "running":
                evidence[backend]["status"] = "failed"
    finally:
        evidence["finished_utc"] = datetime.now(timezone.utc).isoformat()
        save(evidence_path, evidence)
    print(str(evidence_path))
    return 0 if evidence["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
