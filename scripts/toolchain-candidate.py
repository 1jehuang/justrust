#!/usr/bin/env python3
"""Plan an owned-source toolchain candidate. Builds require explicit --execute."""
import argparse
from concurrent.futures import ThreadPoolExecutor
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
# What the compiler and Cargo are built from. A client release bumps the root
# Cargo.toml and Cargo.lock, which the frozen snapshot of the revision never
# reads. The ship tool resumes a tag behind such commits, so this must agree
# with its TOOLCHAIN_SOURCE or a resumed ship can never build (2026-11-27).
TOOLCHAIN_SOURCE = ["toolchain", "scripts/toolchain-candidate.py", "scripts/toolchain-source.py"]


def cargo_archive_argv(root, revision):
    return ["git", "-C", str(root), "archive", revision, "--", "toolchain", ":(exclude)toolchain/rustc"]


def git(root, *args):
    return subprocess.check_output(["git", "-C", str(root), *args], text=True).strip()


def validate_source(root, revision):
    if not re.fullmatch(r"[0-9a-f]{40}", revision):
        raise ValueError("--source-commit must be a full lowercase 40-character repository commit")
    if git(root, "symbolic-ref", "--short", "HEAD") != "stable":
        raise ValueError("candidate builds require the stable branch")
    head = git(root, "rev-parse", "HEAD")
    if head != revision:
        # HEAD may advance through unrelated commits, client releases
        # included (other agents share stable). Only toolchain source drift
        # fails.
        if subprocess.run(["git", "-C", str(root), "merge-base", "--is-ancestor", revision, head]).returncode != 0:
            raise ValueError("--source-commit must be the checked-out HEAD or an ancestor of it")
        if git(root, "diff", "--name-only", revision, head, "--", *TOOLCHAIN_SOURCE):
            raise ValueError("relevant toolchain source changed between --source-commit and HEAD")
    dirty = git(root, "status", "--porcelain", "--untracked-files=no", "--", *RELEVANT)
    if dirty:
        raise ValueError("refusing dirty relevant tracked source:\n" + dirty)


def validate_snapshot_source(root, revision):
    # The build reads only the frozen git-archive snapshot, so commits that
    # land on stable while it runs (other agents) cannot change its inputs.
    # Require only that the revision is still part of stable's history.
    if git(root, "symbolic-ref", "--short", "HEAD") != "stable":
        raise ValueError("candidate builds require the stable branch")
    if subprocess.run(["git", "-C", str(root), "merge-base", "--is-ancestor", revision, "HEAD"]).returncode:
        raise ValueError("source revision is no longer an ancestor of stable HEAD")


def compiler_commands(snapshot, optimize):
    rustc = {"cwd": str(snapshot), "env": {"JR_STAGE": "1", "JR_TOOLS": "1"}}
    if optimize == "none":
        return [dict(rustc, argv=["bash", "toolchain/rustc/build-jr.sh", "release"])]
    # PGO (trained on the bench snapshots) then BOLT on librustc_driver, the
    # same pipeline as the shipped jr-ac20de5e7921. bolt.sh needs llvm-bolt
    # in LLVM_BOLT_DIR (inherited from the caller's environment).
    tc = str(snapshot / "toolchain/rustc")
    return [dict(rustc, argv=["bash", "toolchain/rustc/build-jr.sh", "pgo-gen"]),
            {"argv": ["bash", "./pgo-train.sh"], "cwd": tc, "env": {"JR_STAGE": "1"}},
            dict(rustc, argv=["bash", "toolchain/rustc/build-jr.sh", "pgo-bolt-base"]),
            {"argv": ["bash", "./bolt.sh"], "cwd": tc, "env": {"JR_STAGE": "1"}}]


def make_plan(root, revision, output, optimize="none"):
    root = Path(root).resolve()
    validate_source(root, revision)
    version_data = git(root, "show", f"{revision}:toolchain/version.toml")
    version = tomllib.loads(version_data).get("version", "")
    if not isinstance(version, str) or not re.fullmatch(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)", version):
        raise ValueError("committed toolchain/version.toml must contain a numeric major.minor.patch version")
    candidate = "jr-" + revision
    directory = Path(output).resolve() / candidate
    snapshot = directory / "source"
    # jr-cargo builds wherever justrust routes builds (hosted when local
    # compiles are off) and the binary is fetched back. The hosted mirror root
    # is the nearest `.git` ancestor, and jr-cargo reads toolchain/shared and
    # toolchain/resident-inputs outside its own workspace, so it gets a small
    # frozen tree with a git root: everything under toolchain/ but the compiler.
    cargo_root = directory / "cargo-source"
    target = cargo_root / "toolchain/cargo/target"
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
        "cargo_build_root": str(cargo_root),
        "source_metadata": git(root, "show", "-s", "--format=%H%n%h%n%cs", "--abbrev=9", revision),
        "compiler_identity": "Archive metadata identifies integrated repository source; verify actual rustc -vV before packaging",
        "optimize": optimize,
        "commands": compiler_commands(snapshot, optimize) + [
            {"argv": ["bash", "toolchain/cargo/build-jr.sh", "nightly-2026-10-08"],
             "cwd": str(cargo_root), "env": {"CARGO_TARGET_DIR": str(target),
                                         "JR_CARGO_FEATURES": "all-static"},
             # The helper exits 0 when a remote build brings no binary back.
             "produces": str(target / "release/cargo")},
        ],
        "artifacts": {"compiler_sysroot": str(snapshot / "toolchain/rustc/build/jr" / ("pgo-bolt" if optimize == "pgo-bolt" else "release")),
                      "jr_cargo": str(target / "release/cargo")},
    }


CARGO_BUILD_ATTEMPTS = 3


def build_stages(commands, run):
    """Overlap the pinned Cargo build with bootstrap, never with PGO training.

    The Cargo build runs wherever justrust routes builds, so a network drop can
    fail it while bootstrap is fine (2026-11-27: one rsync error cost a
    17 minute compiler stage). It is idempotent, so a failed one is repeated
    after bootstrap, still before training starts.
    """
    compiler, cargo = commands[:-1], commands[-1]
    with ThreadPoolExecutor(max_workers=1) as pool:
        cargo_result = pool.submit(run, cargo)
        run(compiler[0])
        for attempt in range(1, CARGO_BUILD_ATTEMPTS + 1):
            try:
                cargo_result.result() if attempt == 1 else run(cargo)
                break
            except (subprocess.CalledProcessError, OSError) as error:
                if attempt == CARGO_BUILD_ATTEMPTS:
                    raise
                print(f"toolchain-candidate: Cargo build failed (attempt {attempt} of "
                      f"{CARGO_BUILD_ATTEMPTS}), repeating it: {error}", file=sys.stderr, flush=True)
    for command in compiler[1:]:
        run(command)


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
        cargo_root = Path(plan["cargo_build_root"])
        cargo_root.mkdir()
        cargo_archive = directory / "cargo-source.tar"
        with cargo_archive.open("wb") as output:
            subprocess.run(cargo_archive_argv(root, plan["source_revision"]),
                           stdout=output, check=True, stdin=subprocess.DEVNULL)
        with tarfile.open(cargo_archive) as source:
            source.extractall(cargo_root, filter="data")
        cargo_archive.unlink()
        subprocess.run(["git", "init", "-q", str(cargo_root)], check=True, stdin=subprocess.DEVNULL)
        # Only the private snapshot changes. Never rewrite live tracked metadata.
        (snapshot / "toolchain/rustc/git-commit-info").write_text(plan["source_metadata"] + "\n")
        # Tells bootstrap to report that revision instead of a source content hash.
        (snapshot / "toolchain/rustc/justrust-release-archive").write_text(plan["source_revision"] + "\n")
        def run(command):
            validate_snapshot_source(root, plan["source_revision"])
            env = dict(os.environ, **command["env"], CI="1", GIT_TERMINAL_PROMPT="0")
            subprocess.run(command["argv"], cwd=command["cwd"], env=env,
                           stdin=subprocess.DEVNULL, stdout=sys.stderr, stderr=sys.stderr, check=True)
            produced = command.get("produces")
            if produced and not os.access(produced, os.X_OK):
                raise ValueError(f"build helper did not produce executable artifact: {produced}")
        build_stages(plan["commands"], run)
        validate_snapshot_source(root, plan["source_revision"])
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
    parser.add_argument("--optimize", choices=["none", "pgo-bolt"], default="none",
                        help="pgo-bolt: train PGO and BOLT the compiler (the production pipeline)")
    parser.add_argument("--execute", action="store_true", help="run the real compiler and Cargo build helpers")
    args = parser.parse_args(argv)
    try:
        plan = make_plan(ROOT, args.source_commit, args.output_dir, args.optimize)
        print(json.dumps(execute(plan) if args.execute else plan, indent=2))
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        print(f"toolchain-candidate: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
