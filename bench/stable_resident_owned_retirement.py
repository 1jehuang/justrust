#!/usr/bin/env python3
"""Explicit frozen-compiler owned retirement acceptance, never builds or installs.

Exercises the documented native 64-context lifetime, not a lowered test bound.
This is correctness evidence only, not speed or typed-cache reuse evidence.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

import stable_resident_owned as owned

CAPACITY = 64


def consume_metadata(compiler, root, artifact, label, row, timeout):
    """Snapshot before any validation. Bytes are observations, never an ABI."""
    directory = root / "artifacts" / label
    directory.mkdir(parents=True)
    preserved = directory / "libfixture.rmeta"
    shutil.copyfile(artifact, preserved)
    row.update(
        metadata_path=str(preserved), metadata_sha256=owned.shared.digest(preserved)
    )
    for name, expected in [("positive", 42), ("negative", 43)]:
        source = directory / (name + ".rs")
        source.write_text(
            "extern crate fixture_alias;\n"
            "const _: u64 = fixture_alias::RETIREMENT_VALUE;\n"
            "const _: fn() -> u64 = fixture_alias::retirement_value;\n"
            f"const _: () = assert!(fixture_alias::RETIREMENT_VALUE == {expected});\n"
            f"const _: () = assert!(fixture_alias::retirement_value() == {expected});\n"
        )
        command = [
            str(compiler),
            str(source),
            "--crate-name=retirement_consumer",
            "--crate-type=rlib",
            "--edition=2024",
            "--emit=metadata",
            "--extern",
            "fixture_alias=" + str(preserved),
            "-o",
            str(directory / (name + ".rmeta")),
        ]
        result = row["consumer_" + name] = owned.shared.run(
            command, timeout, cwd=root, env=owned.environment()
        )
        result["command"] = command
        if result.get("timed_out") or type(result.get("status")) is not int:
            raise AssertionError("Metadata consumer did not complete: " + name)
        if name == "positive" and result["status"] != 0:
            raise AssertionError("Metadata consumer rejected exported signature/value")
        if name == "negative" and result["status"] != 1:
            raise AssertionError(
                "Metadata consumer did not reject incompatible expected value normally"
            )


def arguments(root):
    return [
        str(root / "fixture.rs"),
        "--crate-name=owned_retirement_fixture",
        "--crate-type=rlib",
        "--edition=2024",
        "--emit=metadata",
        "-o",
        str(root / "fixture.rmeta"),
    ]


def validate_frame(frame, index, pid, oracle):
    if frame.get("pid") != pid or frame.get("id") != index:
        raise AssertionError("Worker PID/request changed")
    if (
        type(frame.get("status")) is not int
        or frame["status"] != 0
        or frame.get("timed_out")
    ):
        raise AssertionError("Owned compile did not succeed")
    if owned.shared.observable(frame) != owned.shared.observable(oracle):
        raise AssertionError("Fresh and owned compiler observations differ")
    stats = frame.get("owned", {})
    for key in ("contexts", "revisions"):
        if type(stats.get(key)) is not int or stats[key] != index:
            raise AssertionError(
                "Actual owned " + key + " must match successful request count"
            )
    final = index == CAPACITY
    if frame.get("executed") is not True:
        raise AssertionError("Missing executed flag or request was refused")
    if "retirement_reason" not in stats:
        raise AssertionError("Missing retirement reason field")
    if stats["retirement_reason"] != ("context_capacity" if final else None):
        raise AssertionError("Unexpected or early retirement reason")
    rss = stats.get("rss_bytes")
    if type(rss) is not int or not 0 < rss < 2 * 1024 * 1024 * 1024:
        raise AssertionError("Missing RSS or memory budget reached before capacity")


def retire(worker, argv, report, timeout):
    """Keep stdin open: retirement must be autonomous, not induced by EOF."""
    request = dict(id=CAPACITY + 1, args=argv)
    report["extra_request"] = request
    worker.transcript.write(json.dumps({"request": request}).encode() + b"\n")
    worker.transcript.flush()
    try:
        worker.process.stdin.write(json.dumps(request).encode() + b"\n")
        worker.process.stdin.flush()
        report["extra_request_delivery"] = "written"
    except BrokenPipeError:
        report["extra_request_delivery"] = "broken-pipe"
    timed_out = False
    try:
        worker.process.wait(timeout=timeout)
    except subprocess.TimeoutExpired:
        timed_out = True
        worker.process.kill()
        worker.process.wait()
    report["shutdown"] = dict(status=worker.process.returncode, timed_out=timed_out)
    # The final completion frame has already been preserved by OwnedResident.
    # Drain every remaining byte before closing the transport, even on failure.
    trailing = worker.buffer
    try:
        while True:
            if not worker.selector.select(timeout):
                raise AssertionError("Retired response FD failed to reach EOF")
            chunk = os.read(worker.response_fd, 65536)
            if not chunk:
                break
            trailing += chunk
    finally:
        report["trailing_response_b64"] = owned.shared.encoded(trailing)
        (worker.root / "retirement.trailing").write_bytes(trailing)
    if timed_out:
        raise AssertionError("Retired worker shutdown timeout")
    if worker.process.returncode != 0:
        raise AssertionError("Retired worker did not exit cleanly")
    if trailing:
        raise AssertionError(
            "Extra compile accepted or incomplete extra frame after retirement"
        )


def capture(compiler, root, timeout=180):
    compiler, root = Path(compiler).resolve(), Path(root).resolve()
    report = dict(
        schema=2,
        verdict="incomplete",
        capacity=CAPACITY,
        observations=[],
        claim="Native owned-context retirement external correctness only. "
        "Metadata byte differences are observations, not failures. No speed or reuse claim.",
    )
    worker = None
    try:
        report["identity_before"] = owned.shared.identity(compiler)
        (root / "fixture.rs").write_text(
            "pub const RETIREMENT_VALUE: u64 = 42;\n"
            "pub const fn retirement_value() -> u64 { RETIREMENT_VALUE }\n"
        )
        argv = arguments(root)
        report.update(
            argv=argv,
            cwd=str(root),
            environment_sha256=hashlib.sha256(
                json.dumps(owned.environment(), sort_keys=True).encode()
            ).hexdigest(),
        )
        artifact = root / "fixture.rmeta"
        oracle = report["fresh_before"] = owned.shared.run(
            [compiler, *argv], timeout, cwd=root, env=owned.environment()
        )
        consume_metadata(compiler, root, artifact, "fresh-before", oracle, timeout)
        if (
            type(oracle.get("status")) is not int
            or oracle["status"] != 0
            or oracle.get("timed_out")
            or not artifact.is_file()
            or not artifact.stat().st_size
        ):
            raise AssertionError("Fresh compiler oracle failed to produce metadata")
        worker = owned.OwnedResident(compiler, root, timeout)
        report["session"] = dict(pid=worker.process.pid, command=worker.command)
        for index in range(1, CAPACITY + 1):
            artifact.unlink(missing_ok=True)
            frame = worker.compile(argv)
            row = dict(index=index, compile=frame)
            report["observations"].append(row)
            consume_metadata(
                compiler, root, artifact, f"owned-{index:02}", row, timeout
            )
            validate_frame(frame, index, worker.process.pid, oracle)
            row["metadata_matches_fresh_bytes"] = (
                row["metadata_sha256"] == oracle["metadata_sha256"]
            )
            if owned.shared.observable(
                row["consumer_positive"]
            ) != owned.shared.observable(oracle["consumer_positive"]):
                raise AssertionError(
                    "Owned metadata consumer differs from exact fresh baseline"
                )
        retire(worker, argv, report, timeout)
        artifact.unlink(missing_ok=True)
        after = report["fresh_after"] = owned.shared.run(
            [compiler, *argv], timeout, cwd=root, env=owned.environment()
        )
        consume_metadata(compiler, root, artifact, "fresh-after", after, timeout)
        if after.get("timed_out") or owned.shared.observable(
            after
        ) != owned.shared.observable(oracle):
            raise AssertionError("Fresh compiler oracle changed")
        if owned.shared.observable(
            after["consumer_positive"]
        ) != owned.shared.observable(oracle["consumer_positive"]):
            raise AssertionError("Fresh metadata consumer oracle changed")
        report["verdict"] = "passed"
    except BaseException as error:
        report.update(verdict="failed", error=f"{type(error).__name__}: {error}")
        raise
    finally:
        failing = sys.exc_info()[0] is not None
        cleanup_error = None
        if worker is not None:
            try:
                worker.close()
            except Exception as error:
                cleanup_error = error
                report.update(verdict="failed", shutdown_error=str(error))
        try:
            report["identity_after"] = owned.shared.identity(compiler)
            if report.get("identity_before") != report["identity_after"]:
                raise AssertionError("Compiler identity changed")
        except Exception as error:
            cleanup_error = error
            report.update(verdict="failed", identity_error=str(error))
        (root / "receipt.json").write_text(json.dumps(report, indent=2) + "\n")
        if cleanup_error is not None and not failing:
            raise cleanup_error
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--compiler", type=Path, required=True)
    parser.add_argument(
        "--output", type=Path, required=True, help="New private evidence directory"
    )
    parser.add_argument("--timeout", type=float, default=180)
    args = parser.parse_args()
    if not 0 < args.timeout < float("inf"):
        parser.error("timeout must be finite and positive")
    os.umask(0o077)
    args.output.mkdir(mode=0o700, parents=True, exist_ok=False)
    print("Private raw evidence: " + str(args.output.resolve()), flush=True)
    try:
        report = capture(args.compiler, args.output, args.timeout)
        print(json.dumps(dict(verdict=report["verdict"], claim=report["claim"])))
    finally:
        receipt = args.output.resolve() / "receipt.json"
        if receipt.exists():
            try:
                uploaded = subprocess.run(
                    [
                        "justrust",
                        "records",
                        "put",
                        "bench",
                        args.output.resolve().name,
                        str(receipt),
                    ],
                    capture_output=True,
                    text=True,
                )
                print(uploaded.stdout, end="")
                if uploaded.returncode:
                    print(
                        "Record upload failed, private receipt retained: "
                        + uploaded.stderr
                    )
            except OSError as error:
                print("Record upload failed, private receipt retained: " + str(error))


if __name__ == "__main__":
    main()
