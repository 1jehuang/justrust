#!/usr/bin/env python3
"""Probe unchanged Adler2 2.0.1, not the adapted recurrence or full Cargo support.

Exit 0: every required native stage passed. Exit 1: native compatibility blocked.
Exit 2: invalid inputs, missing dependencies, broken stock baseline or infrastructure.
No downloads, source rewrites, stock fallback, or compiler installation.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import time
import tomllib
import zlib

PINS = {
    "src/lib.rs": "4acd41668fe30daffa37084e7e223f268957b816afc1864ffb3f5d6d7adf0890",
    "src/algo.rs": "932c2bc591d13fe4470185125617b5aaa660a3898f23b553acc85df0bf49dded",
}
UPSTREAM_REVISION = "89a031a0f42eeff31c70dc598b398cbf31f1680f"


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def executable(value):
    found = shutil.which(value)
    if found is None:
        raise ValueError(f"executable not found: {value}")
    return Path(found).resolve()


def default_repo():
    home = Path(os.environ.get("CARGO_HOME", Path.home() / ".cargo"))
    candidates = sorted((home / "registry/src").glob("*/adler2-2.0.1"))
    return candidates[0] if candidates else None


def validate_repo(repo):
    manifest = tomllib.loads((repo / "Cargo.toml").read_text())
    package = manifest.get("package", {})
    if (package.get("name"), package.get("version"), package.get("edition")) != (
            "adler2", "2.0.1", "2021"):
        raise ValueError("expected Adler2 2.0.1, edition 2021")
    for relative, expected in PINS.items():
        if sha(repo / relative) != expected:
            raise ValueError(f"upstream source pin mismatch: {relative}")
    sources = {str(path.relative_to(repo)): sha(path)
               for path in sorted((repo / "src").rglob("*")) if path.is_file()}
    if set(sources) != set(PINS):
        raise ValueError("unexpected upstream source files; update reviewed pins first")
    return sources


def consumer_source():
    cases = [b"", b"a", b"Wikipedia", bytes(range(256)), bytes([255]) * 5553]
    body = []
    for data in cases:
        literal = ",".join(str(byte) for byte in data)
        expected = zlib.adler32(data)
        body.append(f"let bytes: &[u8] = &[{literal}];")
        body.append(f"assert_eq!(adler2::adler32_slice(bytes), {expected}u32);")
        body.append("let mut state = adler2::Adler32::new();")
        body.append("for part in bytes.chunks(7) { state.write_slice(part); }")
        body.append(f"assert_eq!(state.checksum(), {expected}u32);")
    return 'extern crate adler2;\nfn main() {\n' + "\n".join(body) + (
        '\nprintln!("adler2 probe passed");\n}\n')


class Probe:
    def __init__(self, output, timeout):
        self.output = output
        self.timeout = timeout
        self.steps = []

    def run(self, name, command, artifact=None, expected_stdout=None, cwd=None, env=None,
            expected_test_count=None):
        start = time.monotonic()
        try:
            result = subprocess.run([str(arg) for arg in command], cwd=cwd, env=env,
                                    capture_output=True, timeout=self.timeout)
            stdout, stderr, code = result.stdout, result.stderr, result.returncode
            status = "passed" if code == 0 else "rejected"
        except subprocess.TimeoutExpired as error:
            stdout, stderr, code = error.stdout or b"", error.stderr or b"", None
            status = "timeout"
        except OSError as error:
            stdout, stderr, code = b"", str(error).encode(), None
            status = "infrastructure_error"
        if status == "rejected" and b"not cached under" in stderr:
            status = "dependency_unavailable"
        if status == "passed" and artifact is not None and not artifact.is_file():
            status = "missing_artifact"
        if status == "passed" and expected_stdout is not None and stdout != expected_stdout:
            status = "wrong_output"
        if status == "passed" and expected_test_count is not None:
            summary = f"{expected_test_count} passed; 0 failed; 0 ignored;".encode()
            if summary not in stdout:
                status = "wrong_output"
        (self.output / f"{name}.stdout").write_bytes(stdout)
        (self.output / f"{name}.stderr").write_bytes(stderr)
        step = dict(name=name, command=[str(arg) for arg in command], status=status,
                    exit_code=code, elapsed_seconds=time.monotonic() - start,
                    stdout_file=f"{name}.stdout", stderr_file=f"{name}.stderr",
                    diagnostic=stderr.decode(errors="replace"))
        if artifact is not None:
            step["artifact"] = str(artifact)
        self.steps.append(step)
        print(f"{name}: {status}", flush=True)
        if status != "passed":
            print(stderr.decode(errors="replace"), flush=True)
        return status == "passed"

    def skip(self, name, reason):
        self.steps.append(dict(name=name, status="blocked", reason=reason))
        print(f"{name}: blocked ({reason})", flush=True)


def exercise(probe, repo, native, rustc):
    root = repo / "src/lib.rs"
    consumer = probe.output / "consumer.rs"
    consumer.write_text(consumer_source())
    stock_ok = True
    for mode in ("no_std", "std"):
        cfg = ["--cfg", 'feature="std"'] if mode == "std" else []
        library = probe.output / f"libadler2_{mode}.rlib"
        metadata = probe.output / f"{mode}.rmeta"
        tests = probe.output / f"stock-{mode}-tests"
        binary = probe.output / f"stock-{mode}-consumer"
        base = [rustc, root, "--edition=2021", "--crate-name", "adler2", *cfg]
        stock_ok &= probe.run(f"stock-{mode}-check", [*base, "--crate-type=lib",
                             "--emit=metadata", "-o", metadata], metadata)
        library_ok = probe.run(f"stock-{mode}-build", [*base, "--crate-type=rlib",
                               "-o", library], library)
        stock_ok &= library_ok
        if library_ok:
            built = probe.run(f"stock-{mode}-consumer-build", [rustc, consumer,
                              "--edition=2021", "--extern", f"adler2={library}",
                              "-o", binary], binary)
            stock_ok &= built
            if built:
                stock_ok &= probe.run(f"stock-{mode}-consumer-run", [binary],
                                      expected_stdout=b"adler2 probe passed\n")
        built = probe.run(f"stock-{mode}-tests-build", [*base, "--test", "-o", tests], tests)
        stock_ok &= built
        if built:
            stock_ok &= probe.run(f"stock-{mode}-tests-run", [tests, "--test-threads=1"],
                                  expected_test_count=6 if mode == "std" else 5)

    native_ok = True
    for workers in (1, 8):
        for mode in ("no_std", "std"):
            cfg = ["--cfg", 'feature="std"'] if mode == "std" else []
            features = ["--no-default-features"] if mode == "no_std" else []
            prefix = f"native-w{workers}-{mode}"
            env = dict(os.environ, CARGO_TARGET_DIR=str(probe.output / prefix))
            # Use the native library mode, not rustc's unsupported rlib interface.
            probe.run(f"{prefix}-source-check", [native, root, "--workers", workers,
                                                "--lib", *cfg, "--check"])
            base = ["--manifest-path", repo / "Cargo.toml", "--lib", "-j", workers,
                    *features]
            for command in ("check", "build"):
                native_ok &= probe.run(f"{prefix}-project-{command}",
                                       [native, command, *base], env=env)
            tests = Path(env["CARGO_TARGET_DIR"]) / "jr/debug/adler2-lib-test"
            built = probe.run(f"{prefix}-tests-build", [native, "test", *base, "--no-run"],
                              tests, env=env)
            native_ok &= built
            if built:
                native_ok &= probe.run(f"{prefix}-tests-run", [tests],
                                       expected_test_count=6 if mode == "std" else 5)
            else:
                probe.skip(f"{prefix}-tests-run", "upstream test harness did not build")
            # A separate real Cargo consumer exercises source-crate integration.
            client = probe.output / f"{prefix}-client"
            (client / "src").mkdir(parents=True)
            (client / "src/main.rs").write_text(consumer.read_text())
            dependency = (f'path = {json.dumps(str(repo))}\n'
                          f'default-features = {str(mode == "std").lower()}\n')
            (client / "Cargo.toml").write_text(
                '[package]\nname = "adler2_probe"\nversion = "0.0.0"\n'
                'edition = "2021"\n[dependencies.adler2]\n' + dependency)
            client_base = ["--manifest-path", client / "Cargo.toml", "-j", workers]
            checked = probe.run(f"{prefix}-consumer-check", [native, "check", *client_base],
                                env=env)
            native_ok &= checked
            binary = Path(env["CARGO_TARGET_DIR"]) / "jr/debug/adler2_probe"
            built = probe.run(f"{prefix}-consumer-build", [native, "build", *client_base],
                              binary, env=env)
            native_ok &= built
            if built:
                native_ok &= probe.run(f"{prefix}-consumer-run", [binary],
                                       expected_stdout=b"adler2 probe passed\n")
            else:
                probe.skip(f"{prefix}-consumer-run", "consumer executable did not build")
    return stock_ok, native_ok


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, default=default_repo(),
                        help="unchanged 2.0.1 checkout/package (default: local Cargo registry)")
    parser.add_argument("--native", required=True, help="already-built jr-rust executable")
    parser.add_argument("--rustc", default="rustc", help="stock oracle executable")
    parser.add_argument("--output-dir", type=Path, required=True, help="must not exist")
    parser.add_argument("--timeout", type=float, default=120)
    args = parser.parse_args(argv)
    if not 0 < args.timeout < float("inf"):
        parser.error("timeout must be finite and positive")
    try:
        if args.repo is None:
            raise ValueError("no local Adler2 2.0.1 found; supply --repo")
        repo = args.repo.resolve()
        sources = validate_repo(repo)
        native, rustc = executable(args.native), executable(args.rustc)
        output = args.output_dir.resolve()
        output.mkdir(parents=True, exist_ok=False)
        # Never write build products into the supplied repository. Freeze both
        # source and compiler against concurrent work in the developer checkout.
        original_repo, original_native = repo, native
        repo = output / "upstream"
        shutil.copytree(original_repo, repo, ignore=shutil.ignore_patterns("target", ".git"))
        if validate_repo(repo) != sources:
            raise ValueError("upstream changed while copying")
        native = output / "jr-rust"
        shutil.copy2(original_native, native)
    except (ValueError, OSError, tomllib.TOMLDecodeError) as error:
        parser.error(str(error))
    probe = Probe(output, args.timeout)
    report = dict(schema_version=1, target="adler2 2.0.1", repo=str(repo),
                  original_repo=str(original_repo), original_native=str(original_native),
                  manifest_sha256=sha(repo / "Cargo.toml"),
                  upstream_revision=UPSTREAM_REVISION, source_sha256=sources,
                  native_binary=str(native), native_sha256=sha(native),
                  rustc_binary=str(rustc), rustc_sha256=sha(rustc),
                  status="infrastructure_error", steps=probe.steps,
                  limitations=["Finite target probe, not a complete missing-feature inventory.",
                               "Native manifest/library/consumer stages; stock oracle uses rustc directly.",
                               "No doctests, benchmarks, docsrs or rustc-dep-of-std feature.",
                               "Source-only checks are diagnostic, not library acceptance."])
    try:
        versions_ok = probe.run("stock-version", [rustc, "-Vv"])
        probe.run("native-help", [native, "--help"])
        stock_ok, native_ok = exercise(probe, repo, native, rustc)
        unchanged = validate_repo(repo) == sources
        infrastructure = any(step["status"] in (
            "timeout", "infrastructure_error", "missing_artifact") for step in probe.steps)
        missing_dependency = any(step["status"] == "dependency_unavailable"
                                 for step in probe.steps)
        if not unchanged or not versions_ok or not stock_ok or infrastructure:
            report["status"] = "infrastructure_error"
            code = 2
        elif missing_dependency:
            report["status"] = "environment_blocked"
            code = 2
        else:
            report["status"] = "passed" if native_ok else "compatibility_blocked"
            code = 0 if native_ok else 1
        report["source_unchanged"] = unchanged
    except (OSError, ValueError, tomllib.TOMLDecodeError) as error:
        report["error"] = str(error)
        code = 2
    report["blockers"] = [step for step in probe.steps
                          if step["name"].startswith("native-")
                          and step["status"] not in ("passed", "blocked")]
    report["environment_blockers"] = [step for step in report["blockers"]
                                      if step["status"] == "dependency_unavailable"]
    report["compatibility_blockers"] = [step for step in report["blockers"]
                                        if step["status"] in ("rejected", "wrong_output")]
    (output / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(f"Result: {report['status']}. Report: {output / 'report.json'}")
    return code


if __name__ == "__main__":
    raise SystemExit(main())
