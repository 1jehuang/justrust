#!/usr/bin/env python3
"""Package Cargo test executables without executing them, then run on a matching host."""
import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import platform
import shutil
import stat
import subprocess
import sys

MAX_JSON = 4 * 1024 * 1024
MAX_TESTS = 4096
# Deliberately explicit: no guessing OS/architecture from arbitrary target strings.
TARGETS = {
    "x86_64-unknown-linux-gnu": ("linux", "x86_64"),
    "aarch64-unknown-linux-gnu": ("linux", "aarch64"),
    "x86_64-unknown-linux-musl": ("linux", "x86_64"),
    "aarch64-unknown-linux-musl": ("linux", "aarch64"),
    "x86_64-pc-windows-msvc": ("win32", "x86_64"),
    "aarch64-pc-windows-msvc": ("win32", "aarch64"),
    "x86_64-apple-darwin": ("darwin", "x86_64"),
    "aarch64-apple-darwin": ("darwin", "aarch64"),
}


def fail(message):
    raise ValueError(message)


def regular(path):
    """Reject symlinks in every path component, including the final file."""
    path = Path(os.path.abspath(path))
    for part in (path, *path.parents):
        if part.is_symlink():
            fail(f"symlink not permitted: {part}")
    if not stat.S_ISREG(path.stat().st_mode):
        fail(f"not a regular file: {path}")
    return path


def digest(path):
    h = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def write_json_new(path, data):
    with Path(path).open("x", encoding="utf-8") as stream:
        json.dump(data, stream, indent=2, allow_nan=False)
        stream.write("\n")


def bundle(args):
    if args.target not in TARGETS:
        fail("unsupported target triple")
    root = Path(args.artifact_root).resolve(strict=True)
    if not root.is_dir():
        fail("artifact root must be a directory")
    artifacts = []
    finished = []
    with regular(args.messages).open(encoding="utf-8") as stream:
        while True:
            line = stream.readline(MAX_JSON + 1)
            if not line:
                break
            if len(line.encode("utf-8")) > MAX_JSON:
                fail("Cargo JSON line too large")
            event = json.loads(line)
            if not isinstance(event, dict):
                fail("Cargo event must be an object")
            if event.get("reason") == "build-finished":
                finished.append(event.get("success"))
            if event.get("reason") == "compiler-artifact" and isinstance(event.get("profile"), dict) and event["profile"].get("test") is True and event.get("executable"):
                source = regular(event["executable"])
                if not source.is_relative_to(root):
                    fail("test executable is outside artifact root")
                if source not in artifacts:
                    artifacts.append(source)
                if len(artifacts) > MAX_TESTS:
                    fail("too many test executables")
    if not finished or any(value is not True for value in finished):
        fail("requires successful build-finished event(s)")
    if not artifacts:
        fail("no test executables in Cargo messages")
    out = Path(args.out)
    out.mkdir(parents=False, exist_ok=False)
    tests = []
    try:
        (out / "bin").mkdir()
        for index, source in enumerate(artifacts):
            relative = f"bin/test-{index:04d}" + (".exe" if TARGETS[args.target][0] == "win32" else "")
            destination = out / relative
            with source.open("rb") as src, destination.open("xb") as dst:
                shutil.copyfileobj(src, dst)
            destination.chmod(0o755)
            tests.append({"path": relative, "sha256": digest(destination), "size": destination.stat().st_size})
        manifest = {"schema_version": 1, "target": args.target, "build_status": "compiled", "test_status": "not-tested", "tests": tests}
        write_json_new(out / "manifest.json", manifest)
    except BaseException:
        shutil.rmtree(out)
        raise
    print(f"Bundled {len(tests)} executable(s): compiled, NOT TESTED")


def load_manifest(directory):
    path = regular(directory / "manifest.json")
    if path.stat().st_size > MAX_JSON:
        fail("manifest too large")
    def unique(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                fail("duplicate JSON key")
            result[key] = value
        return result
    with path.open("r", encoding="utf-8") as stream:
        text = stream.read(MAX_JSON + 1)
    if len(text.encode("utf-8")) > MAX_JSON:
        fail("manifest too large")
    manifest = json.loads(text, object_pairs_hook=unique)
    if not isinstance(manifest, dict) or set(manifest) != {"schema_version", "target", "build_status", "test_status", "tests"}:
        fail("invalid manifest fields")
    if type(manifest["schema_version"]) is not int or manifest["schema_version"] != 1:
        fail("unsupported schema version")
    if not isinstance(manifest["target"], str) or manifest["target"] not in TARGETS:
        fail("unsupported target triple")
    if manifest["build_status"] != "compiled" or manifest["test_status"] != "not-tested":
        fail("invalid build/test status")
    tests = manifest["tests"]
    if not isinstance(tests, list) or not 1 <= len(tests) <= MAX_TESTS:
        fail("invalid test count")
    seen = set()
    for test in tests:
        if not isinstance(test, dict) or set(test) != {"path", "size", "sha256"}:
            fail("invalid test entry")
        name = test["path"]
        if not isinstance(name, str) or len(name) > 256 or "\\" in name or ":" in name:
            fail("unsafe test path")
        parts = PurePosixPath(name)
        if parts.is_absolute() or ".." in parts.parts or parts.as_posix() != name or len(parts.parts) != 2 or parts.parts[0] != "bin":
            fail("unsafe test path")
        if name in seen:
            fail("duplicate test path")
        seen.add(name)
        if type(test["size"]) is not int or test["size"] < 0:
            fail("invalid size")
        sha = test["sha256"]
        if not isinstance(sha, str) or len(sha) != 64 or any(c not in "0123456789abcdef" for c in sha):
            fail("invalid sha256")
        executable = regular(directory / name)
        if executable.stat().st_size != test["size"] or digest(executable) != sha:
            fail(f"hash/size mismatch: {name}")
    return manifest


def run(args):
    directory = Path(os.path.abspath(args.bundle))
    manifest = load_manifest(directory)  # Validate EVERY executable before running ANY.
    machine = platform.machine().lower()
    machine = {"amd64": "x86_64", "arm64": "aarch64"}.get(machine, machine)
    if TARGETS[manifest["target"]] != (sys.platform, machine):
        fail("target OS/architecture does not match this host")
    result_path = Path(os.path.abspath(args.results))
    for part in (result_path, *result_path.parents):
        if part.is_symlink():
            fail("result path must not contain symlinks")
    if result_path.resolve().is_relative_to(directory.resolve()):
        fail("results must be outside the immutable bundle")
    # Reserve the output before execution, refusing overwrite including dangling symlinks.
    with result_path.open("x", encoding="utf-8") as output:
        result = {"schema_version": 1, "target": manifest["target"], "manifest_sha256": digest(directory / "manifest.json"), "test_status": "failed", "tests": []}
        print("WARNING: executing trusted bundle code. Never run an untrusted bundle.", file=sys.stderr)
        forwarded = args.test_args
        if forwarded[:1] == ["--"]:
            forwarded = forwarded[1:]
        try:
            for test in manifest["tests"]:
                entry = {"path": test["path"]}
                try:
                    completed = subprocess.run([str(directory / test["path"]), *forwarded], cwd=args.cwd, shell=False, check=False)
                    entry["exit_code"] = completed.returncode
                except OSError as error:
                    entry["exit_code"] = None
                    entry["error"] = str(error)
                result["tests"].append(entry)
            if all(test["exit_code"] == 0 for test in result["tests"]):
                result["test_status"] = "passed"
        finally:
            json.dump(result, output, indent=2)
            output.write("\n")
    return 0 if result["test_status"] == "passed" else 1


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    pack = commands.add_parser("bundle", help="package compiled tests without running them")
    for flag in ("target", "messages", "out", "artifact-root"):
        pack.add_argument("--" + flag, required=True)
    execute = commands.add_parser("run", help="execute a TRUSTED bundle on a matching host")
    execute.add_argument("--bundle", required=True)
    execute.add_argument("--results", required=True)
    execute.add_argument("--cwd", help="test working directory (default: invoking directory)")
    execute.add_argument("test_args", nargs=argparse.REMAINDER)
    args = parser.parse_args(argv)
    try:
        return (bundle(args) or 0) if args.command == "bundle" else run(args)
    except (ValueError, OSError, TypeError, RecursionError) as error:
        print(f"cross-test-bundle: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
