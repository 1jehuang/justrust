#!/usr/bin/env python3
"""Plan an owned-source toolchain candidate. Builds require explicit --execute."""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tarfile
import tomllib

ROOT = Path(__file__).resolve().parents[1]
RELEVANT = ["toolchain", "scripts/toolchain-candidate.py", "scripts/toolchain-source.py",
            "Cargo.toml", "Cargo.lock", "build.rs", ".cargo"]


def git(root, *args):
    return subprocess.check_output(["git", "-C", str(root), *args], text=True).strip()


def validate_source(root, revision):
    if not re.fullmatch(r"[0-9a-f]{40}", revision):
        raise ValueError("--source-commit must be a full lowercase 40-character repository commit")
    if git(root, "symbolic-ref", "--short", "HEAD") != "stable":
        raise ValueError("candidate builds require the stable branch")
    if git(root, "rev-parse", "HEAD") != revision:
        raise ValueError("--source-commit must equal the checked-out repository HEAD")
    dirty = git(root, "status", "--porcelain", "--untracked-files=no", "--", *RELEVANT)
    if dirty:
        raise ValueError("refusing dirty relevant tracked source:\n" + dirty)


def make_plan(root, revision, output):
    root = Path(root).resolve()
    validate_source(root, revision)
    version_data = git(root, "show", f"{revision}:toolchain/version.toml")
    version = tomllib.loads(version_data).get("version", "")
    if not isinstance(version, str) or not re.fullmatch(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)", version):
        raise ValueError("committed toolchain/version.toml must contain a numeric major.minor.patch version")
    candidate = "jr-" + revision
    directory = Path(output).resolve() / candidate
    snapshot = directory / "source"
    target = snapshot / "toolchain/cargo/target"
    return {
        "schema_version": 1,
        "candidate_id": candidate,
        "source_revision": revision,
        "version": version,
        "release_tag": "toolchain-v" + version,
        "source_root": str(root),
        "state": "planned",
        "provenance_directory": str(directory),
        "build_root": str(snapshot),
        "source_metadata": git(root, "show", "-s", "--format=%H%n%h%n%cs", "--abbrev=9", revision),
        "compiler_identity": "Archive metadata identifies integrated repository source; verify actual rustc -vV before packaging",
        "commands": [
            {"argv": ["bash", "toolchain/rustc/build-jr.sh", "release"],
             "cwd": str(snapshot), "env": {"JR_STAGE": "1", "JR_TOOLS": "1"}},
            {"argv": ["bash", "toolchain/cargo/build-jr.sh", "nightly-2026-10-08"],
             "cwd": str(snapshot), "env": {"JUSTRUST_REMOTE": "off", "CARGO_TARGET_DIR": str(target),
                                       "JR_CARGO_FEATURES": "all-static"}},
        ],
        "artifacts": {"compiler_sysroot": str(snapshot / "toolchain/rustc/build/jr/release"),
                      "jr_cargo": str(target / "release/cargo")},
    }


def execute(plan):
    root = Path(plan["source_root"])
    validate_source(root, plan["source_revision"])
    directory = Path(plan["provenance_directory"])
    # Never overwrite a prior attempt, including a failed one.
    directory.mkdir(parents=True, exist_ok=False)
    record = directory / "provenance.json"
    record.write_text(json.dumps(plan, indent=2) + "\n")
    result = dict(plan, state="building")
    try:
        # Freeze committed bytes rather than compiling a concurrently edited checkout.
        archive = directory / "source.tar"
        with archive.open("wb") as output:
            subprocess.run(["git", "-C", str(root), "archive", plan["source_revision"]],
                           stdout=output, check=True, stdin=subprocess.DEVNULL)
        snapshot = Path(plan["build_root"])
        snapshot.mkdir()
        with tarfile.open(archive) as source:
            source.extractall(snapshot, filter="data")
        archive.unlink()
        # Only the private snapshot changes. Never rewrite live tracked metadata.
        (snapshot / "toolchain/rustc/git-commit-info").write_text(plan["source_metadata"] + "\n")
        for command in plan["commands"]:
            validate_source(root, plan["source_revision"])
            env = dict(os.environ, **command["env"], CI="1", GIT_TERMINAL_PROMPT="0")
            subprocess.run(command["argv"], cwd=command["cwd"], env=env,
                           stdin=subprocess.DEVNULL, stdout=sys.stderr, stderr=sys.stderr, check=True)
        validate_source(root, plan["source_revision"])
        for artifact in (Path(plan["artifacts"]["compiler_sysroot"]) / "bin/rustc",
                         Path(plan["artifacts"]["jr_cargo"])):
            if not artifact.is_file() or not os.access(artifact, os.X_OK):
                raise ValueError(f"build helper did not produce executable artifact: {artifact}")
        compiler = Path(plan["artifacts"]["compiler_sysroot"]) / "bin/rustc"
        identity = subprocess.check_output([str(compiler), "-vV"], text=True)
        if f"commit-hash: {plan['source_revision']}" not in identity.splitlines():
            raise ValueError("built compiler identity does not match integrated source revision")
        result.update(state="built", rustc_verbose_version=identity)
    except (OSError, subprocess.CalledProcessError, ValueError) as error:
        result.update(state="failed", error=str(error))
        raise
    finally:
        record.write_text(json.dumps(result, indent=2) + "\n")
    return result


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-commit", required=True)
    parser.add_argument("--output-dir", type=Path, default=ROOT / "target/toolchain-candidates")
    parser.add_argument("--execute", action="store_true", help="run the real compiler and Cargo build helpers")
    args = parser.parse_args(argv)
    try:
        plan = make_plan(ROOT, args.source_commit, args.output_dir)
        print(json.dumps(execute(plan) if args.execute else plan, indent=2))
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        print(f"toolchain-candidate: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
