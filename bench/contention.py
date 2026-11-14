#!/usr/bin/env python3
"""Focused real hosted target-lock acceptance, not a broad speed benchmark.

Run the published baseline first, then the candidate by explicit path, without
installing either. Both results use suite.py's comparison and private records:

  python3 bench/contention.py --out bench/results/contention-before.json
  JUSTRUST=/absolute/candidate python3 bench/contention.py --expect-isolated \
      --out bench/results/contention-after.json
  python3 bench/suite.py compare bench/results/contention-{before,after}.json

A tiny content-pinned fixture owns its own persistent hosted workspace. A real
`justrust test --target-dir target` executable locks Cargo's shared build
directory, then B runs
`justrust test` without forcing a target directory. A's ledger checks shared
artifacts are unchanged while locked. B and an uncontended follow-up must pass
both independent unit tests. --local is only a harness diagnostic, not evidence
of the hosted ext4 fix. All Cargo invocations are real recorded justrust runs.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import threading
import time

import prerequisites
import suite

FIXTURE = Path(__file__).resolve().parent / "fixtures" / "contention"
SCENARIO = "hosted-ext4-target-contention"
PROTOCOL = "target-flock-shared-ledger-v1"
READY = "BENCH_LOCK_READY"
SAFE = "BENCH_SHARED_ARTIFACTS_UNCHANGED"
EXT4_MAGIC = "ef53"


def fixture_files():
    return sorted(path for path in FIXTURE.rglob("*")
                  if path.is_file() and "target" not in path.relative_to(FIXTURE).parts)


def fixture_digest():
    digest = hashlib.sha256()
    for path in fixture_files():
        digest.update(str(path.relative_to(FIXTURE)).encode() + b"\0" + path.read_bytes() + b"\0")
    return digest.hexdigest()


def prepare(digest):
    # Content pinning, not the live project checkout. No symlink to user files,
    # worktree, external dependencies, or large target directories.
    root = suite.benchmark_work_root(True) / "contention" / digest[:20]
    root.mkdir(parents=True, exist_ok=True)
    for source in fixture_files():
        dest = root / source.relative_to(FIXTURE)
        dest.parent.mkdir(parents=True, exist_ok=True)
        if not dest.exists() or dest.read_bytes() != source.read_bytes():
            dest.write_bytes(source.read_bytes())
    return root


def child_env(run_id, remote, session):
    env = dict(os.environ)
    # Target overrides disable slots. Reject them in the CLI, and defensively
    # remove every ambient target selector from child commands.
    for key in ("CARGO_TARGET_DIR", "CARGO_BUILD_TARGET_DIR", "JUSTRUST_DISABLE"):
        env.pop(key, None)
    env.update({"JUSTRUST_RUN_ID_OVERRIDE": run_id,
                "JUSTRUST_REMOTE": "force" if remote else "0",
                "JUSTRUST_TOOLCHAIN": "system", "JUSTRUST_QUIET": "1",
                "JUSTRUST_NO_UPDATE": "1", "JUSTRUST_BUILDD": "0",
                "JUSTRUST_PREBUILT": "off", "JUSTRUST_TEST_PARALLEL": "0",
                "JUSTRUST_TEST_SPLIT": "0", "JUSTRUST_EARLY": "off",
                "JUSTRUST_SLOT_SESSION": session, "JCODE_SESSION_ID": session})
    return env


class Run:
    def __init__(self, command, root, remote, session):
        self.id = suite.new_id()
        self.command = command
        self.lines = []
        self.ready = threading.Event()
        self.ready_at = None
        self.released_at = None
        self.started = time.monotonic()
        self.process = subprocess.Popen([suite.J, *command], cwd=root,
            env=child_env(self.id, remote, session), stdout=subprocess.PIPE,
            stderr=subprocess.PIPE, text=True, umask=prerequisites.CHILD_UMASK)
        self.threads = [threading.Thread(target=self.read, args=(stream,), daemon=True)
                        for stream in (self.process.stdout, self.process.stderr)]
        for thread in self.threads:
            thread.start()

    def read(self, stream):
        for line in stream:
            self.lines.append(line)
            if READY in line:
                self.ready_at = time.monotonic()
                self.ready.set()
            if SAFE in line:
                self.released_at = time.monotonic()
            # Keep actual diagnostics intact, not hidden behind a benchmark log.
            print(line, end="", file=sys.stderr, flush=True)

    def finish(self, remote, timeout=180):
        try:
            code = self.process.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            self.process.terminate()
            raise ValueError(f"contention: timed out, inspect justrust log {self.id}")
        ended = time.monotonic()
        for thread in self.threads:
            thread.join(timeout=10)
        path = suite.JR_HOME / "runs" / self.id / "summary.json"
        if code or not path.exists():
            raise ValueError(f"contention: run {self.id} failed (exit {code})\n{''.join(self.lines)}")
        summary = json.loads(path.read_text())
        if remote:
            suite.remote_measurement(summary)
        if summary.get("tests", {}).get("failed", 0):
            raise ValueError(f"contention: failing tests in {self.id}")
        return summary, ended - self.started, ended


def sample(summary, run, wall, remote):
    resources = summary.get("resources", {})
    slot = summary.get("slot") or {}
    units = summary.get("units", {})
    return {"id": run.id, "wall": round(wall, 3),
            "recorded_wall": summary["wall"],
            "startup": summary["phases"].get("startup", 0),
            "compile": summary["phases"].get("compile", 0),
            "test_run": summary["phases"].get("test_run", 0),
            "cpu": summary.get("cpu_secs", 0), "link_secs": 0,
            "split": dict.fromkeys(("frontend", "codegen", "link", "incremental", "other"), 0),
            "units": units.get("compiled", 0),
            "tests_passed": summary.get("tests", {}).get("passed", 0),
            "tests_failed": summary.get("tests", {}).get("failed", 0),
            "first_error": None, "first_fail": None,
            "noisy": resources.get("avg_other_cores", 0) > suite.NOISY_CORES,
            "other_cores": resources.get("avg_other_cores", 0),
            "lock_wait": summary["phases"].get("lock_wait"),
            "cargo_target_lock_message": any("Blocking waiting for file lock on build directory" in line
                                             for line in run.lines),
            "slot": slot,
            **({"remote": suite.remote_measurement(summary)} if remote else {})}


def validate_acceptance(holder, contender, followup, finished_under_lock, isolated):
    ready = re.search(r"BENCH_LOCK_READY filesystem_magic=([0-9a-f]+) files=(\d+)", "".join(holder.lines))
    if not ready or SAFE not in "".join(holder.lines):
        raise ValueError("contention: missing real lock readiness or shared-artifact safety evidence")
    for name, summary in (("contender", contender), ("followup", followup)):
        if summary.get("tests", {}).get("passed") != 2:
            raise ValueError(f"contention: {name} must pass both independent probe tests")
    if (followup.get("slot") or {}).get("slot") is not None:
        raise ValueError("contention: followup must verify the shared target, not a slot")
    if followup.get("units", {}).get("compiled", 0) != 0:
        raise ValueError("contention: unchanged shared-target followup unexpectedly rebuilt artifacts")
    if isolated:
        if not finished_under_lock:
            raise ValueError("contention: contender did not finish while the shared lock was held")
        if (contender.get("slot") or {}).get("slot") is None:
            raise ValueError("contention: contender did not acquire an isolated slot")
        if contender["phases"].get("lock_wait", 0) > 0.5:
            raise ValueError("contention: isolated contender still waited for the Cargo target lock")
    return ready.group(1)


def comparison_issues(a, b):
    """Do not let the generic named-scenario reporter mix different lock loads."""
    issues = []
    for label, result in (("A", a), ("B", b)):
        if result.get("schema") != "contention_suite_v1":
            issues.append(f"{label}: compare contention results with contention results only")
            continue
        env = result.get("env", {})
        if not env.get("contention"):
            issues.append(f"{label}: missing contention protocol provenance")
        samples = result.get("scenarios", {}).get(SCENARIO, {}).get("samples", [])
        if not samples:
            issues.append(f"{label}: missing contention samples")
        for sample in samples:
            if (sample.get("shared_artifacts_unchanged") is not True
                    or sample.get("tests_passed") != 2 or sample.get("tests_failed") != 0
                    or sample.get("followup_tests_passed") != 2 or sample.get("followup_units") != 0
                    or sample.get("followup_target") != "shared default target"
                    or not sample.get("holder_id") or not sample.get("followup_id")):
                issues.append(f"{label}: incomplete lock-owner or test-safety evidence")
            if env.get("execution") == "remote" and (
                    sample.get("filesystem_magic") != EXT4_MAGIC
                    or not sample.get("remote", {}).get("machine")):
                issues.append(f"{label}: missing hosted ext4 execution evidence")
    for key in ("contention", "execution", "host", "cpu", "ncpu", "optimization_env"):
        if a.get("env", {}).get(key) != b.get("env", {}).get(key):
            issues.append(f"different {key}: use identical contention workload and execution mode")
    machines = [{sample.get("remote", {}).get("machine")
                 for sample in result.get("scenarios", {}).get(SCENARIO, {}).get("samples", [])}
                for result in (a, b)]
    if machines[0] != machines[1]:
        issues.append("different hosted machines")
    return issues


def run(args):
    if sys.platform != "linux":
        raise ValueError("contention: the controlled flock fixture requires Linux")
    if args.n < 1:
        raise ValueError("contention: positive -n required")
    for key in ("CARGO_TARGET_DIR", "CARGO_BUILD_TARGET_DIR", "JUSTRUST_SLOTS_ALL"):
        if os.environ.get(key):
            raise ValueError(f"contention: unset {key} to preserve target-lock semantics")
    remote = not args.local
    digest = fixture_digest()
    root = prepare(digest)
    info = suite.env_info()
    # A standalone fixture is pinned by bytes, not unrelated suite snapshots.
    info.update({"execution": "remote" if remote else "local",
                 "snapshots": {"contention": digest}, "snapshot_format": PROTOCOL,
                 "snapshot_fingerprints": {"contention": digest},
                 "contention": {"protocol": PROTOCOL, "hold_seconds": 20,
                                "target": "default, slots eligible", "fixture": digest}})
    started = time.strftime("%Y-%m-%dT%H:%M:%S%z")
    # Warm the shared default target explicitly. Only the lock owner forces
    # this target. The measured contender never forces a target directory.
    warm = Run(["test", "--lib", "--target-dir", "target", "--", "probe_"],
               root, remote, "bench-contention-warm")
    warm_summary, _, _ = warm.finish(remote)
    samples = []
    for iteration in range(args.n):
        holder = Run(["test", "--test", "lock-holder", "--target-dir", "target", "--",
                      "--exact", "hold_shared_target", "--nocapture"],
                     root, remote, f"bench-contention-holder-{iteration}")
        contender = None
        try:
            if not holder.ready.wait(timeout=120):
                raise ValueError(f"contention: lock owner did not become ready, run {holder.id}")
            contender = Run(["test", "--lib", "--", "probe_"], root, remote,
                            f"bench-contention-probe-{iteration}")
            contender_summary, wall, ended = contender.finish(remote)
            holder_summary, _, _ = holder.finish(remote)
            # Output timestamps are conservative: success must reach the caller
            # before the holder's release marker reaches it.
            under_lock = holder.released_at is not None and ended < holder.released_at
            follow = Run(["test", "--lib", "--target-dir", "target", "--", "probe_"], root, remote,
                         f"bench-contention-probe-{iteration}")
            follow_summary, follow_wall, _ = follow.finish(remote)
            fs_magic = validate_acceptance(holder, contender_summary, follow_summary,
                                           under_lock, args.expect_isolated)
            if remote and fs_magic != EXT4_MAGIC:
                raise ValueError(f"contention: hosted filesystem is {fs_magic}, expected ext4 ef53")
            machines = {(sm.get("remote") or {}).get("machine")
                        for sm in (warm_summary, holder_summary, contender_summary, follow_summary)}
            if remote and len(machines) != 1:
                raise ValueError("contention: commands ran on different hosted machines")
            measured = sample(contender_summary, contender, wall, remote)
            measured.update({"holder_id": holder.id, "followup_id": follow.id,
                             "followup_wall": round(follow_wall, 3),
                             "followup_target": "shared default target",
                             "followup_units": follow_summary.get("units", {}).get("compiled", 0),
                             "followup_tests_passed": follow_summary.get("tests", {}).get("passed", 0),
                             "lock_ready_delay": round(contender.started - holder.ready_at, 3),
                             "finished_before_release_secs": round(holder.released_at - ended, 3),
                             "finished_under_lock": under_lock, "shared_artifacts_unchanged": True,
                             "filesystem_magic": fs_magic})
            samples.append(measured)
        finally:
            # Never leave a fixture command running after an invalid sample.
            # A remotely disconnected lock owner self-releases after <=120s.
            for active in (contender, holder):
                if active is not None and active.process.poll() is None:
                    active.process.terminate()
                    active.process.wait(timeout=15)
    suite.verify_binary_identity(info)
    result = {"schema": "contention_suite_v1", "started": started,
              "finished": time.strftime("%Y-%m-%dT%H:%M:%S%z"), "env": info,
              "runs_per_scenario": args.n, "warmup_id": warm.id,
              "scope": "controlled hosted ext4 target-lock contention only, not broad speedup",
              "scenarios": {SCENARIO: {"what": __doc__.splitlines()[0],
                  "expect_test_fail": False, "samples": samples, "stats": suite.stats(samples)}}}
    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(result, indent=1) + "\n")
    print(suite.table(result))
    print(f"saved {out}")
    suite.upload_result(out)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("-n", type=int, default=3)
    parser.add_argument("--expect-isolated", action="store_true")
    parser.add_argument("--local", action="store_true", help="local harness diagnostic only")
    parser.add_argument("--out", required=True)
    args = parser.parse_args()
    try:
        run(args)
    except (ValueError, OSError, subprocess.SubprocessError) as error:
        sys.exit(str(error))


if __name__ == "__main__":
    main()
