#!/usr/bin/env python3
"""Frozen native arithmetic leaf acceptance, not a benchmark or general reuse claim.

Run only after the coordinator freezes the compiler and approves an idle native
window. Producers use no_core with identical overflow-checks=off flags, retaining
zero external Rust crates under the current identity gate. Every successful
producer object is linked into a newly compiled, fresh C consumer.
Exact output is independently calculated with Python u32 arithmetic.
Raw JSONL, diagnostic streams, artifacts and failure receipts are preserved.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys

import stable_resident as shared
import stable_resident_mir_leaf as leaf
import stable_resident_typed as typed

CLAIM = (
    "Primitive argument-bearing arithmetic MIR leaf acceptance only, with "
    "positive cross-unrelated-edit mir_leaf_hits and exact fresh consumer "
    "outputs. No general reuse, Typeck or speed claim."
)
ROOT = b"""#![feature(no_core, lang_items)]
#![no_core]
#![allow(internal_features, dead_code)]
#[lang = "pointee_sized"] pub trait PointeeSized {}
#[lang = "meta_sized"] pub trait MetaSized: PointeeSized {}
#[lang = "sized"] pub trait Sized: MetaSized {}
#[lang = "copy"] pub trait Copy {}
impl Copy for u32 {}
#[lang = "add"] pub trait Add<Rhs = Self> { type Output; fn add(self, rhs: Rhs) -> Self::Output; }
impl Add for u32 { type Output = u32; fn add(self, rhs: u32) -> u32 { self + rhs } }
#[lang = "sub"] pub trait Sub<Rhs = Self> { type Output; fn sub(self, rhs: Rhs) -> Self::Output; }
impl Sub for u32 { type Output = u32; fn sub(self, rhs: u32) -> u32 { self - rhs } }
#[lang = "mul"] pub trait Mul<Rhs = Self> { type Output; fn mul(self, rhs: Rhs) -> Self::Output; }
impl Mul for u32 { type Output = u32; fn mul(self, rhs: u32) -> u32 { self * rhs } }
#[lang = "bitxor"] pub trait BitXor<Rhs = Self> { type Output; fn bitxor(self, rhs: Rhs) -> Self::Output; }
impl BitXor for u32 { type Output = u32; fn bitxor(self, rhs: u32) -> u32 { self ^ rhs } }
#[lang = "bitand"] pub trait BitAnd<Rhs = Self> { type Output; fn bitand(self, rhs: Rhs) -> Self::Output; }
impl BitAnd for u32 { type Output = u32; fn bitand(self, rhs: u32) -> u32 { self & rhs } }
mod module_a;
mod module_b;
"""
CC = Path("/usr/bin/cc")
INPUTS = (
    (0, 0),
    (1, 2),
    (255, 17),
    (65535, 65536),
    (0xFFFFFFFF, 1),
    (0x80000000, 0xFFFFFFFF),
    (123456789, 987654321),
)
MASK = (1 << 32) - 1


def sources(a=7, b=9, error=False):
    return {
        "fixture.rs": ROOT,
        "module_a.rs": (
            '#[unsafe(no_mangle)]\npub extern "C" fn leaf_a(x: u32, y: u32) -> u32 { '
            "((x + y) ^ %d) * 3 }\n" % a
        ).encode(),
        "module_b.rs": (
            ('#[unsafe(no_mangle)]\npub extern "C" fn leaf_b(x: u32) -> u32 { true }\n')
            if error
            else (
                '#[unsafe(no_mangle)]\npub extern "C" fn leaf_b(x: u32) -> u32 { (x - %d) & 65535 }\n'
                % b
            )
        ).encode(),
    }


def expected_output(a, b):
    return "".join(
        "%d %d\n" % (((((x + y) & MASK) ^ a) * 3) & MASK, ((x - b) & MASK) & 65535)
        for x, y in INPUTS
    ).encode()


def cases():
    return [
        ("initial", sources(), (7, 9), "cold"),
        ("unchanged", sources(), (7, 9), "same"),
        ("edit-b", sources(b=11), (7, 11), "cross"),
        ("after-b-unchanged", sources(b=11), (7, 11), "same"),
        ("edit-a", sources(a=13, b=11), (13, 11), "cross"),
        ("type-error", sources(a=13, b=11, error=True), None, "error"),
        ("error-repeat", sources(a=13, b=11, error=True), None, "error"),
        ("recovery", sources(a=13, b=15), (13, 15), "recovery"),
        ("recovery-unchanged", sources(a=13, b=15), (13, 15), "same"),
    ]


def arguments(root):
    return [
        str(root / "fixture.rs"),
        "--crate-name=arithmetic_fixture",
        "--crate-type=lib",
        "--edition=2024",
        "-Zcodegen-backend=llvm",
        "-Copt-level=0",
        "-Cdebuginfo=0",
        "-Cpanic=abort",
        "-Coverflow-checks=off",
        "-Cincremental=" + str(root / "incremental"),
        "--emit=obj="
        + str(root / "fixture.o")
        + ",llvm-ir="
        + str(root / "fixture.ll")
        + ",mir="
        + str(root / "fixture.mir"),
    ]


def consumer_source():
    calls = "".join(
        'printf("%%" PRIu32 " %%" PRIu32 "\\n", leaf_a(UINT32_C(%d), UINT32_C(%d)), '
        "leaf_b(UINT32_C(%d)));\n" % (x, y, x)
        for x, y in INPUTS
    )
    return (
        "#include <stdint.h>\n#include <inttypes.h>\n#include <stdio.h>\n"
        '_Static_assert(sizeof(uint32_t) == 4, "u32 ABI");\n'
        "extern uint32_t leaf_a(uint32_t, uint32_t);\n"
        "extern uint32_t leaf_b(uint32_t);\n"
        "int main(void) {\n" + calls + "return 0;\n}\n"
    ).encode()


def validate_consumer(record, expected):
    if record.get("timed_out") or record["status"] != 0:
        raise AssertionError("Fresh consumer failed or timed out")
    if record["stdout_b64"] != shared.encoded(expected) or record[
        "stderr_b64"
    ] != shared.encoded(b""):
        raise AssertionError("Fresh consumer exact arithmetic output mismatch")


def run_consumer(root, evidence, prefix, expected, timeout, env, record):
    source, binary = root / "consumer.c", root / "consumer"
    source.write_bytes(consumer_source())
    (evidence / (prefix + ".consumer.c")).write_bytes(source.read_bytes())
    binary.unlink(missing_ok=True)
    argv = [
        str(CC),
        "-std=c11",
        "-O0",
        "-Wall",
        "-Wextra",
        "-Werror",
        str(source),
        str(root / "fixture.o"),
        "-o",
        str(binary),
    ]
    record["consumer_argv"] = argv
    record["consumer_compile"] = shared.run(argv, timeout, cwd=root, env=env)
    compiled = record["consumer_compile"]
    if compiled["status"] != 0 or compiled.get("timed_out"):
        raise AssertionError("Fresh consumer compilation failed")
    # Copy executable bytes before execution or validation so failures retain it.
    (evidence / (prefix + ".consumer.bin")).write_bytes(binary.read_bytes())
    record["consumer_run"] = shared.run([str(binary)], timeout, cwd=root, env=env)
    validate_consumer(record["consumer_run"], expected)


def capture_mode(compiler, root, evidence, mode, timeout):
    evidence.mkdir(mode=0o700)
    env = typed.environment(mode)
    argv = arguments(root)
    result = dict(
        mode=mode,
        observations=[],
        argv=argv,
        cwd=str(root),
        environment_sha256=hashlib.sha256(
            json.dumps(env, sort_keys=True).encode()
        ).hexdigest(),
    )
    backups = {name: (root / name).read_bytes() for name in sources()}
    for name, data in backups.items():
        (evidence / (name + ".rawbackup")).write_bytes(data)
    streams = ("resident.stdout", "resident.stderr", "resident.jsonl")
    for name in streams:
        (root / name).unlink(missing_ok=True)
    worker = None
    try:
        worker = typed.TypedResident(compiler, root, mode, timeout)
        result.update(pid=worker.process.pid, command=worker.command)
        previous, old_sources = None, None
        for index, (name, files, expected, kind) in enumerate(cases()):
            changed = leaf.validate_transition(old_sources, files, kind)
            row = dict(
                name=name,
                kind=kind,
                changed_files=changed,
                expected=expected,
                sources={
                    n: dict(
                        sha256=hashlib.sha256(d).hexdigest(),
                        bytes_b64=shared.encoded(d),
                    )
                    for n, d in files.items()
                },
            )
            result["observations"].append(row)
            for filename, data in files.items():
                if (root / filename).read_bytes() != data:
                    (root / filename).write_bytes(data)
            for engine in ("fresh", "resident"):
                if typed.environment(mode) != env:
                    raise AssertionError("Compiler environment changed within worker")
                bank, active = (
                    root / (mode + "-" + engine + "-incremental"),
                    root / "incremental",
                )
                if bank.exists():
                    bank.rename(active)
                artifacts = ("fixture.ll", "fixture.mir", "fixture.o")
                for artifact in artifacts:
                    (root / artifact).unlink(missing_ok=True)
                try:
                    record = (
                        shared.run([compiler, *argv], timeout, cwd=root, env=env)
                        if engine == "fresh"
                        else worker.compile(argv)
                    )
                    row[engine] = record
                    if record["status"] == 0:
                        if expected is None:
                            raise AssertionError("Type error unexpectedly compiled")
                        for artifact in artifacts:
                            data = (root / artifact).read_bytes()
                            saved = evidence / (
                                "%02d-%s.%s" % (index, engine, artifact)
                            )
                            saved.write_bytes(data)
                            record[artifact] = dict(
                                path=str(saved), sha256=hashlib.sha256(data).hexdigest()
                            )
                            if not data:
                                raise AssertionError(
                                    "Empty producer artifact: " + artifact
                                )
                        run_consumer(
                            root,
                            evidence,
                            "%02d-%s" % (index, engine),
                            expected_output(*expected),
                            timeout,
                            env,
                            record,
                        )
                finally:
                    if active.exists():
                        active.rename(bank)
            typed.validate_pair(
                row["fresh"], row["resident"], expected, worker.process.pid
            )
            if expected is not None:
                for key in ("consumer_compile", "consumer_run"):
                    if shared.observable(row["fresh"][key]) != shared.observable(
                        row["resident"][key]
                    ):
                        raise AssertionError(
                            "Exact fresh/native consumer observables differ"
                        )
            previous = leaf.validate_live(row["resident"], previous, mode, kind)
            old_sources = files
        result["live"] = previous
        closing, worker = worker, None
        closing.close()
        result["verdict"] = "passed"
        return result
    except BaseException as error:
        result.update(verdict="failed", error=f"{type(error).__name__}: {error}")
        raise
    finally:
        failing = sys.exc_info()[0] is not None
        try:
            if worker is not None:
                worker.close()
        except Exception as error:
            result.update(verdict="failed", shutdown_error=str(error))
            if not failing:
                raise
        finally:
            for name in streams:
                if (root / name).exists():
                    (evidence / name).write_bytes((root / name).read_bytes())
            for name, data in backups.items():
                (root / name).write_bytes(data)
            result["sources_restored"] = all(
                (root / n).read_bytes() == d for n, d in backups.items()
            )
            (evidence / "receipt.json").write_text(json.dumps(result, indent=2) + "\n")


def capture(compiler, root, timeout=180):
    compiler, root = Path(compiler).resolve(), Path(root).resolve()
    report = dict(
        schema=1,
        scope="native-primitive-mir-arithmetic",
        verdict="incomplete",
        claim=CLAIM,
        modes=[],
        mode_receipts=[
            str(root / mode / "receipt.json") for mode in ("control", "leaf")
        ],
    )
    try:
        report["identity_before"] = shared.identity(compiler)
        report["consumer_compiler"] = dict(
            path=str(CC.resolve()),
            sha256=hashlib.sha256(CC.resolve().read_bytes()).hexdigest(),
        )
        workspace = root / "workspace"
        workspace.mkdir(mode=0o700)
        for name, data in sources().items():
            (workspace / name).write_bytes(data)
        for mode in ("control", "leaf"):
            report["modes"].append(
                capture_mode(compiler, workspace, root / mode, mode, timeout)
            )
        for control, native in zip(*(m["observations"] for m in report["modes"])):
            for engine in ("fresh", "resident"):
                if shared.observable(control[engine]) != shared.observable(
                    native[engine]
                ):
                    raise AssertionError("Exact control/leaf diagnostics differ")
                if control["expected"] is not None:
                    for key in ("consumer_compile", "consumer_run"):
                        if shared.observable(control[engine][key]) != shared.observable(
                            native[engine][key]
                        ):
                            raise AssertionError(
                                "Exact control/leaf consumer observables differ"
                            )
        if (
            report["consumer_compiler"]["sha256"]
            != hashlib.sha256(CC.resolve().read_bytes()).hexdigest()
        ):
            raise AssertionError("C consumer compiler identity changed")
        report["identity_after"] = shared.identity(compiler)
        if report["identity_before"] != report["identity_after"]:
            raise AssertionError("Frozen compiler identity changed")
        report["verdict"] = "passed"
        return report
    except BaseException as error:
        report.update(verdict="failed", error=f"{type(error).__name__}: {error}")
        raise
    finally:
        if "identity_before" in report and "identity_after" not in report:
            try:
                report["identity_after"] = shared.identity(compiler)
            except Exception as error:
                report["identity_after_error"] = f"{type(error).__name__}: {error}"
        (root / "receipt.json").write_text(json.dumps(report, indent=2) + "\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--compiler", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--timeout", type=float, default=180)
    args = parser.parse_args()
    if not 0 < args.timeout < float("inf"):
        parser.error("timeout must be finite and positive")
    os.umask(0o077)
    args.output.mkdir(mode=0o700, parents=True, exist_ok=False)
    print("Private original evidence: " + str(args.output.resolve()), flush=True)
    try:
        report = capture(args.compiler, args.output, args.timeout)
        print(
            json.dumps(dict(verdict=report["verdict"], claim=report["claim"]), indent=2)
        )
    finally:
        receipt = args.output.resolve() / "receipt.json"
        if receipt.exists():
            try:
                upload = subprocess.run(
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
                print(upload.stdout, end="")
                if upload.returncode:
                    print(
                        "Record upload failed, raw evidence retained: " + upload.stderr
                    )
            except OSError as error:
                print("Record upload failed, raw evidence retained: " + str(error))


if __name__ == "__main__":
    main()
