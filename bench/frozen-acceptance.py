#!/usr/bin/env python3
"""Run normal hosted acceptance using one explicitly pinned client executable.

No installation, account changes, or run-ID override. Raw logs stay in the
caller-selected private output directory. The receipt proves executable and
recorded route identity, not compiler speed or independently audited source.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import time

WORKFLOWS = (
    ("check", ["check"]),
    ("test", ["test"]),
    ("clippy", ["clippy", "--all-targets", "--", "-D", "warnings"]),
    ("recheck", ["check"]),
)


def digest(path):
    h = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()


def verify_client(path, expected):
    if not re.fullmatch(r"[0-9a-f]{64}", expected):
        raise ValueError("client SHA-256 must be 64 lowercase hexadecimal characters")
    if path.is_symlink() or not path.is_file() or not os.access(path, os.X_OK):
        raise ValueError("use an executable regular frozen file, not a mutable CLI symlink")
    if digest(path) != expected:
        raise ValueError("frozen client digest mismatch")


def verify_environment(env):
    if env.get("JUSTRUST_RUN_ID_OVERRIDE"):
        raise ValueError("run-ID overrides can change routing and are forbidden")
    if env.get("JUSTRUST_ON_AGENT"):
        raise ValueError("acceptance must run from the client, not on the agent")
    if not env.get("JCODE_SESSION_ID"):
        raise ValueError("JCODE_SESSION_ID is required for unambiguous run attribution")


def run_ids(root):
    return {p.name for p in root.iterdir() if p.is_dir()} if root.is_dir() else set()


def recorded_run(root, before, args, session, toolchain):
    matches = []
    for rid in run_ids(root) - before:
        path = root / rid / "meta.json"
        try:
            meta = json.loads(path.read_text())
        except (OSError, json.JSONDecodeError):
            continue
        if (isinstance(meta, dict) and meta.get("agent_session") == session
                and meta.get("args") == args):
            if meta.get("id") != rid:
                raise ValueError("invocation metadata run identity mismatch")
            matches.append(meta)
    if len(matches) != 1:
        raise ValueError("expected exactly one new attributable invocation record")
    meta = matches[0]
    if meta.get("place") != "hosted":
        raise ValueError("acceptance did not actually run hosted")
    if meta.get("toolchain") != toolchain:
        raise ValueError("recorded compiler selection differs from the accepted pin")
    return {key: meta.get(key) for key in
            ("id", "place", "toolchain", "ncpu", "args", "agent_session")}


def save_receipt(out, receipt):
    (out / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")


def accept(client, expected, repo, runs, out, env, toolchain, timeout=600):
    verify_client(client, expected)
    verify_environment(env)
    out.mkdir(parents=True, exist_ok=False, mode=0o700)
    receipt = {"schema_version": 1, "client_sha256": expected,
               "started": time.time(), "complete": False, "steps": [],
               "toolchain": toolchain,
               "scope": "hosted functionality and executable identity, not speed"}
    save_receipt(out, receipt)
    child_env = dict(env, JUSTRUST_NO_UPDATE="1")
    try:
        for name, args in WORKFLOWS:
            verify_client(client, expected)
            before = run_ids(runs)
            step = {"name": name, "args": args, "started": time.time()}
            receipt["steps"].append(step)
            save_receipt(out, receipt)
            proc = subprocess.run([str(client), *args], cwd=repo, env=child_env,
                                  stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                  stderr=subprocess.STDOUT, check=False, timeout=timeout)
            (out / (name + ".log")).write_bytes(proc.stdout)
            sys.stdout.buffer.write(proc.stdout)
            sys.stdout.buffer.flush()
            step.update({"exit": proc.returncode, "finished": time.time(),
                         "output_sha256": hashlib.sha256(proc.stdout).hexdigest()})
            save_receipt(out, receipt)
            verify_client(client, expected)
            step["run"] = recorded_run(runs, before, args, env["JCODE_SESSION_ID"], toolchain)
            save_receipt(out, receipt)
            if proc.returncode != 0:
                raise ValueError("workflow failed: " + name)
        receipt["complete"] = True
    except subprocess.TimeoutExpired as error:
        output = error.stdout or b""
        (out / (name + ".log")).write_bytes(output)
        sys.stdout.buffer.write(output)
        sys.stdout.buffer.flush()
        step.update({"exit": None, "finished": time.time(), "timed_out": True,
                     "output_sha256": hashlib.sha256(output).hexdigest()})
        receipt["error"] = "workflow timed out: " + name
        raise ValueError(receipt["error"]) from error
    except (OSError, ValueError) as error:
        receipt["error"] = str(error)
        raise
    finally:
        receipt["finished"] = time.time()
        save_receipt(out, receipt)
    return receipt


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--client", type=Path, required=True)
    parser.add_argument("--sha256", required=True)
    parser.add_argument("--repo", type=Path, required=True)
    parser.add_argument("--runs", type=Path,
                        default=Path(os.environ.get("JUSTRUST_HOME", Path.home() / ".justrust")) / "runs")
    parser.add_argument("--out-dir", type=Path, required=True)
    parser.add_argument("--toolchain", required=True)
    parser.add_argument("--timeout", type=int, default=600)
    args = parser.parse_args()
    if args.timeout < 1:
        parser.error("timeout must be positive")
    try:
        # absolute() deliberately preserves a symlink for verify_client to reject.
        accept(args.client.absolute(), args.sha256, args.repo.absolute(),
               args.runs.absolute(), args.out_dir.absolute(), dict(os.environ),
               args.toolchain, args.timeout)
    except (OSError, ValueError) as error:
        print("frozen acceptance rejected: " + str(error), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
