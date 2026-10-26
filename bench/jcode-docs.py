#!/usr/bin/env python3
"""Pinned README edit benchmark. Candidate runs require --baseline-result.

Examples:
  python3 bench/jcode-docs.py --out /path/baseline.json
  python3 bench/jcode-docs.py --baseline-result /path/baseline.json \
    --patch candidate.patch --allow-file Cargo.toml --allow-file crates/... \
    --out /path/candidate.json
Compare the saved files with bench/suite.py compare. Sources and targets are
isolated per invocation, never built in the live checkout. Results embed the
exact applied patch, not merely the unchanged baseline snapshot label.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import subprocess
import tempfile
import time
import uuid
from pathlib import Path

import suite

PIN = "146b6f4d84a7e76f212695c89936d69fe36b3e3c"
NAME = "jcode-docs-readme-edit"
ANCHOR = "High-performance coding agents."
CMD = ["test", "-p", "jcode-tui", "--lib", "justrust"]


def git(repo, *args):
    return subprocess.check_output(["git", "-C", str(repo), *args])


def candidate_patch(args):
    if args.patch:
        return Path(args.patch).read_bytes(), {"kind": "patch"}
    if args.source_commit:
        commit = git(args.repo, "rev-parse", args.source_commit + "^{commit}").decode().strip()
        return git(args.repo, "diff", "--binary", PIN, commit), {"kind": "commit", "commit": commit}
    return b"", {"kind": "baseline"}


def apply_candidate(root, patch, allowed):
    """Use an isolated index to validate *all* paths, including rename sources."""
    if not patch:
        raise ValueError("candidate patch is empty")
    git(root, "init", "-q")
    git(root, "add", "-A")
    tree = git(root, "write-tree").decode().strip()
    subprocess.run(["git", "-C", str(root), "apply", "--index", "--binary", "-"],
                   input=patch, check=True)
    paths = git(root, "diff", "--cached", "--no-renames", "--name-only", "-z", tree).decode().split("\0")
    paths = sorted(p for p in paths if p)
    if not paths or set(paths) - set(allowed):
        raise ValueError(f"candidate changes outside explicit --allow-file list: {paths}")
    if "README.md" in paths:
        raise ValueError("candidate may not change the benchmark README workload")
    return paths


def validate_pair(baseline, info, workload):
    if baseline.get("candidate", {}).get("kind") != "baseline" or not baseline.get("valid"):
        raise ValueError("--baseline-result must be a baseline from this driver")
    if baseline.get("workload") != workload:
        raise ValueError("baseline workload, pin, sample count or warmup differs")
    for key in ("execution", "toolchain", "rustc", "cargo", "justrust_binary_sha256", "optimization_env"):
        if baseline["env"].get(key) != info.get(key):
            raise ValueError(f"baseline differs in {key}")


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--repo", type=Path, default=Path.home() / "jcode")
    group = p.add_mutually_exclusive_group()
    group.add_argument("--patch", type=Path)
    group.add_argument("--source-commit")
    p.add_argument("--allow-file", action="append", default=[])
    p.add_argument("--baseline-result", type=Path)
    p.add_argument("--out", type=Path, required=True)
    p.add_argument("-n", type=int, default=3)
    p.add_argument("--warmup", type=int, default=1)
    p.add_argument("--mode", choices=("hosted", "local"), default="hosted")
    p.add_argument("--toolchain", default="system")
    args = p.parse_args(argv)
    if args.n < 1 or args.warmup < 1:
        p.error("n and warmup must be positive")
    if os.environ.get("JUSTRUST_BENCH_SRC"):
        p.error("JUSTRUST_BENCH_SRC is unsupported: sources must come from git archive")
    is_candidate = bool(args.patch or args.source_commit)
    if is_candidate != bool(args.baseline_result):
        p.error("candidate requires --baseline-result, baseline cannot supply it")
    if args.out.exists():
        p.error("refusing to overwrite an existing result")
    resolved = git(args.repo, "rev-parse", PIN + "^{commit}").decode().strip()
    if resolved != PIN:
        raise ValueError("baseline pin did not resolve exactly")
    patch, provenance = candidate_patch(args)
    suite.REMOTE = args.mode == "hosted"
    suite.TOOLCHAIN = args.toolchain
    if args.toolchain != "system":
        subprocess.run([suite.J, "toolchain", "install", args.toolchain], check=True)
    # Keep snapshot labels identical for suite.compare. Actual modifications are
    # disclosed independently in candidate, while physical paths never overlap.
    suite.SNAPSHOTS = {NAME: {"repos": {"jcode": (args.repo.resolve(), PIN)}, "cwd": "jcode"}}
    staging = Path(os.environ.get("JUSTRUST_BENCH_WORK", Path.home() / ".cache" / "jcode-docs-bench"))
    if any(part in {".justrust", ".cargo", ".git", "target"} for part in staging.resolve().parts):
        p.error("benchmark staging must be outside hosted registry excluded directories")
    staging.mkdir(parents=True, exist_ok=True)
    suite.WORK = Path(tempfile.mkdtemp(prefix="jcode-docs-", dir=staging))
    root = suite.prepare(NAME)
    paths = apply_candidate(root / "jcode", patch, args.allow_file) if is_candidate else []
    if is_candidate:
        shutil.rmtree(root / "jcode" / ".git")
    provenance.update({"patch_sha256": hashlib.sha256(patch).hexdigest(), "patch": patch.decode(),
                       "changed_files": paths, "baseline_commit": PIN})
    info = suite.env_info()
    info["execution"] = "remote" if suite.REMOTE else "local"
    workload = {"pin": PIN, "command": CMD, "file": "README.md", "anchor": ANCHOR,
                "edit": "unique markdown body suffix", "n": args.n, "warmup": args.warmup}
    if is_candidate:
        validate_pair(json.loads(args.baseline_result.read_text()), info, workload)
    nonce = uuid.uuid4().hex
    scenario = suite.Scenario(NAME, NAME, CMD, suite.Edit("jcode/README.md", ANCHOR,
        ANCHOR + f" Benchmark documentation revision {nonce}-{{n}}."),
        "README body edit to filtered jcode-tui library test result")
    result = {"started": time.strftime("%Y-%m-%dT%H:%M:%S%z"), "env": info,
              "runs_per_scenario": args.n, "workload": workload, "candidate": provenance,
              "isolation": {"root": str(root), "edit_nonce": nonce}, "scenarios": {}}
    result["scenarios"][NAME] = suite.run_scenario(scenario, args.n, args.warmup, 0)
    measured = result["scenarios"][NAME]["samples"]
    result["valid"] = bool(measured) and all(
        s.get("tests_passed", 0) > 0 and s.get("tests_failed", 0) == 0 for s in measured)
    suite.verify_binary_identity(info)
    result["finished"] = time.strftime("%Y-%m-%dT%H:%M:%S%z")
    args.out.parent.mkdir(parents=True, exist_ok=True)
    with args.out.open("x") as f:
        json.dump(result, f, indent=2)
        f.write("\n")
    print(suite.table(result))
    print(f"saved {args.out}")
    if not result["valid"]:
        raise SystemExit("benchmark invalid: no passing filtered tests or test failures (result retained)")


if __name__ == "__main__":
    main()
