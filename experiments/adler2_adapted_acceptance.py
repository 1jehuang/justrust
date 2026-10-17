#!/usr/bin/env python3
"""Exact upstream/stock/adapted/native Adler2 differential, not a benchmark."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import zlib

ROOT = Path(__file__).resolve().parents[1]
FIXTURE = ROOT / "fixtures/adler2_adapted"
PINS = {
    "lib.rs": "4acd41668fe30daffa37084e7e223f268957b816afc1864ffb3f5d6d7adf0890",
    "algo.rs": "932c2bc591d13fe4470185125617b5aaa660a3898f23b553acc85df0bf49dded",
}


def run(argv):
    result = subprocess.run([str(x) for x in argv], capture_output=True, text=True,
                            timeout=180)
    if result.returncode:
        raise RuntimeError(f"command failed ({result.returncode}): {argv}\n"
                           f"{result.stdout}\n{result.stderr}")
    return result.stdout


def vectors():
    cases = [("empty", [[]]), ("one_zero", [[0]]), ("one_ff", [[255]]),
             ("four", [[0, 1, 128, 255]]),
             ("sixteen", [list(range(16))]),
             ("mixed_257", [[(i * 73 + 19) % 256 for i in range(257)]])]
    for length in (5551, 5552, 5553, 22207, 22208, 22209, 44417):
        cases.append((f"ff_{length}", [[255] * length]))
    cases.extend([
        ("split_empty", [[], [], []]),
        ("split_mixed", [[0, 255], [], list(range(16)), [255, 128, 1]]),
        ("split_chunks", [[255] * 5552, [], [255] * 16657]),
    ])
    return cases


def array(data):
    if not data:
        return "[]"
    if len(set(data)) == 1:
        return f"[{data[0]}; {len(data)}]"
    return "[" + ", ".join(map(str, data)) + "]"


def declarations(parts):
    return "\n".join(f"let p{i}: [u8; {len(data)}] = {array(data)};"
                     for i, data in enumerate(parts))


def upstream_source(parts):
    calls = "\n".join(f"state.write_slice(&p{i});" for i in range(len(parts)))
    return ("extern crate adler2;\nfn main() {\n" + declarations(parts)
            + "\nlet mut state = adler2::Adler32::new();\n" + calls
            + '\nprintln!("{}", state.checksum());\n}\n')


def adapted_source(name, parts, expected):
    calls = "\n".join(f"state = adler2_adapted_update(&p{i}, state);"
                       for i in range(len(parts)))
    # jr-rust supports literal println, not integer formatting. A checksum is
    # printed only after runtime equality against the exact upstream u32 value.
    return ((FIXTURE / "recurrence.rs").read_text() + "\nfn main() {\n"
            + declarations(parts) + "\nlet mut state: u32 = 1;\n" + calls
            + f'\nif state == {expected} {{ println!("{name}:{expected}"); }}'
            + ' else { println!("CHECKSUM_MISMATCH"); }\n}\n')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rustc", default="rustc")
    parser.add_argument("--native", type=Path,
                        help="integrated jr-rust public CLI; omitted means stock-only")
    parser.add_argument("--output-dir", required=True, type=Path)
    parser.add_argument("--skip", action="append", default=[],
                        help="vector name excluded from this run (recorded in evidence)")
    args = parser.parse_args()
    args.output_dir.mkdir(parents=True, exist_ok=True)
    evidence = args.output_dir / "evidence.json"
    # A failed rerun must not leave a previous success report behind.
    evidence.unlink(missing_ok=True)
    for filename, expected in PINS.items():
        actual = hashlib.sha256((FIXTURE / "upstream" / filename).read_bytes()).hexdigest()
        if actual != expected:
            raise RuntimeError(f"upstream source checksum mismatch: {filename}")
    report = {"schema": 1, "claim": "adapted Adler2 recurrence, not unchanged crate",
              "recorded_at": datetime.now(timezone.utc).isoformat(),
              "runner_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
              "recurrence_sha256": hashlib.sha256((FIXTURE / "recurrence.rs").read_bytes()).hexdigest(),
              "rustc": run([args.rustc, "-Vv"]), "upstream_sha256": PINS,
              "native_status": "not_run" if args.native is None else "requested",
              "cases": []}
    if args.native is not None:
        report["native_binary_sha256"] = hashlib.sha256(args.native.read_bytes()).hexdigest()
    # Never reuse binaries from an earlier run. Persistent generated source is
    # evidence only; executables always live in a fresh private directory.
    with tempfile.TemporaryDirectory(prefix="adler2-oracle-") as temporary:
        work = Path(temporary)
        library = work / "libadler2.rlib"
        run([args.rustc, FIXTURE / "upstream/lib.rs", "--crate-name", "adler2",
             "--crate-type", "rlib", "--edition=2021", "-C", "opt-level=0",
             "-C", "overflow-checks=yes", "-o", library])
        for name, parts in vectors():
            if name in args.skip:
                report.setdefault("skipped", []).append(name)
                print(f"SKIP {name}")
                continue
            oracle = args.output_dir / f"{name}.oracle.rs"
            oracle.write_text(upstream_source(parts))
            binary = work / "oracle"
            run([args.rustc, oracle, "--crate-name", "oracle", "--edition=2021",
                 "--extern", f"adler2={library}", "-C", "overflow-checks=yes",
                 "-o", binary])
            checksum = int(run([binary]).strip())
            # Independently cross-check the incremental upstream oracle against
            # Python's standard zlib over the concatenated exact bytes.
            if checksum != zlib.adler32(bytes(x for part in parts for x in part)):
                raise RuntimeError(f"upstream/zlib checksum mismatch: {name}")
            source = args.output_dir / f"{name}.rs"
            source.write_text(adapted_source(name, parts, checksum))
            expected = f"{name}:{checksum}\n"
            run([args.rustc, source, "--crate-name", "adapted", "--edition=2021",
                 "-C", "overflow-checks=yes", "-o", binary])
            observed = run([binary])
            if observed != expected:
                raise RuntimeError(f"stock adapted mismatch {name}: {observed!r}")
            row = {"name": name, "lengths": list(map(len, parts)),
                   "input_sha256": [hashlib.sha256(bytes(part)).hexdigest() for part in parts],
                   "oracle_source_sha256": hashlib.sha256(oracle.read_bytes()).hexdigest(),
                   "checksum": checksum, "stock_stdout": observed,
                   "source_sha256": hashlib.sha256(source.read_bytes()).hexdigest(),
                   "native": []}
            if args.native is not None:
                for workers in (1, 8):
                    executable = work / f"{name}-{workers}"
                    check = [args.native.resolve(), source.resolve(), "--workers",
                             str(workers), "--check"]
                    run(check)
                    build = [args.native.resolve(), source.resolve(), "--workers",
                             str(workers), "-o", executable]
                    run(build)
                    output = run([executable])
                    if output != expected:
                        raise RuntimeError(f"native mismatch {name}/{workers}: {output!r}")
                    row["native"].append({"workers": workers, "check": "pass",
                                          "build": "pass", "stdout": output})
            report["cases"].append(row)
            print(f"PASS {name} checksum={checksum} native={len(row['native'])}")
    if args.native is not None:
        report["native_status"] = ("passed_workers_1_and_8" if not args.skip
                                   else "passed_workers_1_and_8_with_skips")
    evidence.write_text(json.dumps(report, indent=2) + "\n")
    print(f"Evidence: {args.output_dir / 'evidence.json'}")


if __name__ == "__main__":
    main()
