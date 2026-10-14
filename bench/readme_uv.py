#!/usr/bin/env python3
"""Focused uv README evidence, not the comprehensive speedup/default gate.

    python3 bench/readme_uv.py --rounds 3 --output /path/to/uv.json

Each round rotates stock, local, remote within each row. Independent git
archives get fresh workspace targets, but existing shared dependency caches
remain enabled and untouched. These are NOT cold builds. The warm edit uses
the check workspace after an explicit prewarm, excluded from aggregation.
Logs and extracted sources stay in scratch. No installation or config changes.
"""
from __future__ import annotations

import argparse
from contextlib import contextmanager
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import statistics
import subprocess
import tarfile
import tempfile
import time
import uuid

PIN = "44b2e5877ef752e360acc02c7d198ba71ddf948d"
DEFAULT_SOURCE = Path.home() / ".jcode/scratch/e2e/uv"
STACKS = ("stock", "local", "remote")
ROWS = {
    "fresh-check": ["check", "--workspace", "--locked"],
    "fresh-test-no-run": ["test", "-p", "uv-resolver", "--no-run", "--locked"],
    "warm-edit-check": ["check", "-p", "uv-resolver", "--locked"],
}
EDIT_FILE = "crates/uv-resolver/src/manifest.rs"
EDIT_ANCHOR = "    pub fn num_requirements(&self) -> usize {\n"
ANSI = re.compile(r"\x1b\[[0-?]*[ -/]*[@-~]")


def utc_now():
    return datetime.now(timezone.utc).isoformat()


def scratch_root():
    return Path(os.environ.get("JCODE_SCRATCH_DIR", Path.home() / ".jcode/scratch")).expanduser().resolve()


def run_text(command, cwd=None, env=None):
    return subprocess.check_output(command, cwd=cwd, env=env, text=True, stderr=subprocess.STDOUT).strip()


def sha256(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def executable_identity(path, cwd=None, env=None):
    path = Path(path).resolve(strict=True)
    before = sha256(path)
    version = run_text([str(path), "--version", "--verbose"], cwd, env)
    if sha256(path) != before:
        raise ValueError(f"executable changed while identifying {path}")
    return {"path": str(path), "sha256": before, "version": version}


def justrust_binary():
    overrides = [os.environ[key] for key in ("JUSTRUST", "JUSTRUST_BIN") if key in os.environ]
    paths = [shutil.which(value) for value in overrides or ["justrust"]]
    if any(path is None for path in paths):
        raise ValueError("justrust executable override is missing or not executable")
    resolved = [str(Path(path).resolve()) for path in paths]
    if len(set(resolved)) != 1:
        raise ValueError("JUSTRUST and JUSTRUST_BIN select different executables")
    return resolved[0]


def sanitize(text, work=None):
    text = ANSI.sub("", text)
    # Logs remain intact in private scratch. JSON does not expose credentials.
    text = re.sub(r"(?i)(bearer\s+)[\w.+=/-]+", r"\1<redacted>", text)
    text = re.sub(r"(?i)((?:token|api[_-]?key|password|secret)\s*[=:]\s*)[^\s,\"']+", r"\1<redacted>", text)
    text = re.sub(r"(https?://)[^/\s@]+@", r"\1<redacted>@", text)
    text = re.sub(r"(?i)([?&](?:token|key|signature|credential)[^=\s]*=)[^&\s]+", r"\1<redacted>", text)
    if work:
        text = text.replace(str(work), "<work>")
    return text.replace(str(Path.home()), "~")


def sanitized_data(data, work=None):
    if isinstance(data, str):
        return sanitize(data, work)
    if isinstance(data, list):
        return [sanitized_data(value, work) for value in data]
    if isinstance(data, dict):
        return {key: sanitized_data(value, work) for key, value in data.items()}
    return data


def load_snapshot():
    result = {"loadavg": list(os.getloadavg()), "cpu_count": os.cpu_count()}
    try:
        result["proc_stat_cpu"] = Path("/proc/stat").read_text().splitlines()[0]
        result["rustc_pids"] = sorted(
            int(entry.name) for entry in Path("/proc").iterdir()
            if entry.name.isdecimal() and (entry / "comm").exists()
            and (entry / "comm").read_text().strip() == "rustc"
        )
    except OSError:
        result["proc_scan_incomplete"] = True
    return result


def rotating_order(round_index, stacks=STACKS):
    offset = round_index % len(stacks)
    return stacks[offset:] + stacks[:offset]


def sample_environment(stack, target, run_id):
    env = dict(os.environ)
    env["CARGO_TARGET_DIR"] = str(target)
    env["CARGO_TERM_COLOR"] = "never"
    env.pop("JUSTRUST_RUN_ID_OVERRIDE", None)
    # Prevent inherited wrappers contaminating the stock baseline. Also remove
    # them for justrust so only that stack's own recording shim is selected.
    for key in ("RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER"):
        env.pop(key, None)
    if stack == "stock":
        env["RUSTC"] = "/usr/bin/rustc"
        env["RUSTDOC"] = "/usr/bin/rustdoc"
    else:
        env["JUSTRUST_NO_UPDATE"] = "1"
        env["JUSTRUST_RUN_ID_OVERRIDE"] = run_id
        env.pop("JUSTRUST_DISABLE", None)
    return env


def validate_summary(stack, summary, run_id):
    """Missing evidence or a remote fallback is a failure, even on exit 0."""
    if stack == "stock":
        return None
    if not isinstance(summary, dict):
        raise ValueError("missing justrust summary")
    if summary.get("id") != run_id:
        raise ValueError("justrust summary run ID mismatch")
    if summary.get("exit") != 0:
        raise ValueError(f"justrust summary exit {summary.get('exit')}")
    remote = summary.get("remote")
    machine = remote.get("machine") if isinstance(remote, dict) else None
    if stack == "remote" and (not isinstance(machine, str) or not machine.strip()):
        raise ValueError("forced remote sample fell back locally: no remote.machine")
    if stack == "local" and machine:
        raise ValueError("local sample unexpectedly ran remotely")
    # Do not invent equality with the baseline, or mistake the client's rustc
    # for the hosted compiler. This is the actual host-recorded selection.
    return {"selected": summary.get("toolchain") or "system", "machine": machine,
            "identity_source": "host summary" if stack == "remote" else "local summary"}


def aggregate(samples, rounds, stacks=STACKS):
    results = {}
    for row in ROWS:
        results[row] = {}
        for stack in stacks:
            group = [sample for sample in samples
                     if sample["row"] == row and sample["stack"] == stack and sample.get("measured", True)]
            good = [sample["wall_seconds"] for sample in group if sample.get("status") == "ok"]
            complete = len(group) == rounds and len(good) == rounds
            results[row][stack] = {
                "expected": rounds, "samples": len(group), "successful": len(good),
                "all_ok": complete,
                # Never turn failed or partially collected rows into README claims.
                "median_seconds": statistics.median(good) if complete else None,
                "min_seconds": min(good) if complete else None,
                "max_seconds": max(good) if complete else None,
                "run_ids": [sample.get("run_id") for sample in group],
            }
    return results


def save_result(output, result):
    result["aggregate"] = aggregate(result["samples"], result["rounds"], result.get("stacks", STACKS))
    result["all_ok"] = bool(result["status"] == "complete" and
                            all(cell["all_ok"] for row in result["aggregate"].values() for cell in row.values()))
    output.parent.mkdir(parents=True, exist_ok=True)
    temporary = output.with_name(output.name + ".tmp")
    temporary.write_text(json.dumps(result, indent=2) + "\n")
    os.replace(temporary, output)


def make_archive(source, destination):
    run_text(["git", "-C", str(source), "archive", "--format=tar", f"--output={destination}", PIN])
    return sha256(destination)


def extract_checkout(archive, destination):
    destination.mkdir(parents=True, exist_ok=False)
    with tarfile.open(archive) as stream:
        # Python's data filter rejects path traversal and unsafe links.
        stream.extractall(destination, filter="data")
    # An archive may live beneath another Git repo. An isolated, unborn repo
    # stops discovery there, and indexes even gitignored archived sources for
    # hosted git ls-files synchronization. No worktree, commit, or source edits.
    run_text(["git", "init", "--quiet", str(destination)])
    run_text(["git", "add", "--force", "--all"], cwd=destination)
    if not (destination / "Cargo.lock").is_file():
        raise ValueError("pinned source is missing Cargo.lock")
    return destination


@contextmanager
def body_edit(checkout, marker):
    path = checkout / EDIT_FILE
    original = path.read_bytes()
    text = original.decode()
    if text.count(EDIT_ANCHOR) != 1:
        raise ValueError("pinned uv-resolver function anchor is not unique")
    line = f"        std::hint::black_box(\"readme_uv_{marker}\");\n"
    try:
        path.write_bytes(text.replace(EDIT_ANCHOR, EDIT_ANCHOR + line).encode())
        yield {"file": EDIT_FILE, "function": "Manifest::num_requirements", "line": line.strip(),
               "original_sha256": hashlib.sha256(original).hexdigest(), "edited_sha256": sha256(path)}
    finally:
        path.write_bytes(original)


def read_evidence(run_home, run_id):
    directory = run_home / "runs" / run_id
    summary = json.loads((directory / "summary.json").read_text())
    meta = json.loads((directory / "meta.json").read_text())
    # Never embed meta.env, which can contain credentials.
    return summary, {key: meta.get(key) for key in
                     ("id", "toolchain", "real_cargo", "real_rustc", "place", "args", "route")}


def execute(result, output, stack, row, round_index, checkout, binaries, work, measured=True, edit=None):
    sample_id = "readme-uv-" + uuid.uuid4().hex
    run_id = sample_id if stack != "stock" else None
    args = ROWS[row]
    command = ([binaries["cargo"]["path"], *args] if stack == "stock" else
               [binaries["justrust"]["path"], args[0], "--" + stack, *args[1:]])
    log = work / "logs" / (sample_id + ".log")
    log.parent.mkdir(exist_ok=True)
    target = checkout / "target"
    env = sample_environment(stack, target, run_id)
    sample = {"sample_id": sample_id, "run_id": run_id, "stack": stack, "row": row,
              "round": round_index + 1, "measured": measured, "command": command,
              "checkout": str(checkout), "target": str(target), "log": str(log),
              "started_utc": utc_now(), "load_before": load_snapshot(), "status": "running",
              "edit": edit, "summary": None, "exit": None}
    result["samples"].append(sample)
    save_result(output, result)
    start = time.monotonic_ns()
    errors = []
    try:
        for identity in binaries.values():
            if sha256(identity["path"]) != identity["sha256"]:
                raise ValueError("binary changed since harness startup")
        with log.open("wb") as stream:
            start = time.monotonic_ns()
            completed = subprocess.run(command, cwd=checkout, env=env, stdout=stream, stderr=subprocess.STDOUT)
            end = time.monotonic_ns()
        sample["exit"] = completed.returncode
        sample["monotonic_start_ns"] = start
        sample["monotonic_end_ns"] = end
        sample["wall_seconds"] = (end - start) / 1e9
        if completed.returncode:
            errors.append(f"command exit {completed.returncode}")
        if stack != "stock":
            home = Path(env.get("JUSTRUST_HOME", Path.home() / ".justrust"))
            summary, meta = read_evidence(home, run_id)
            sample["summary"] = sanitized_data(summary, work)
            sample["run_meta"] = sanitized_data(meta, work)
            sample["toolchain"] = validate_summary(stack, summary, run_id)
            if stack == "local":
                sample["compiler_identity"] = executable_identity(meta["real_rustc"], checkout, env)
                sample["cargo_identity"] = executable_identity(meta["real_cargo"], checkout, env)
            else:
                sample["toolchain"]["real_rustc"] = meta.get("real_rustc")
                sample["toolchain"]["real_cargo"] = meta.get("real_cargo")
                sample["toolchain"]["note"] = "Host compiler paths and selection, not a client binary version/SHA claim"
        else:
            sample["compiler_identity"] = binaries["rustc"]
            sample["cargo_identity"] = binaries["cargo"]
        for identity in binaries.values():
            if sha256(identity["path"]) != identity["sha256"]:
                errors.append("binary changed during sample")
    except (OSError, ValueError, subprocess.SubprocessError, TypeError) as error:
        errors.append(str(error))
    finally:
        sample.setdefault("wall_seconds", (time.monotonic_ns() - start) / 1e9)
        sample["load_after"] = load_snapshot()
        sample["finished_utc"] = utc_now()
        sample["status"] = "failed" if errors else "ok"
        sample["errors"] = [sanitize(error, work) for error in errors]
        if errors and log.exists():
            with log.open("rb") as stream:
                stream.seek(max(0, log.stat().st_size - 12000))
                sample["error_tail"] = sanitize(stream.read().decode(errors="replace"), work)
        save_result(output, result)
    return sample


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--stacks", default="stock,local,remote", help="comma-separated stacks, e.g. stock,local when hosted is blocked")
    parser.add_argument("--rounds", type=int, default=3)
    parser.add_argument("--source", type=Path, default=DEFAULT_SOURCE)
    parser.add_argument("--work", type=Path, help="new directory inside JCODE_SCRATCH_DIR or ~/.jcode/scratch")
    parser.add_argument("--output", type=Path, help="JSON receipt, incrementally atomically written")
    args = parser.parse_args(argv)
    stacks = tuple(args.stacks.split(","))
    if not stacks or len(set(stacks)) != len(stacks) or any(stack not in STACKS for stack in stacks):
        parser.error("--stacks must select distinct stock,local,remote entries")
    if args.rounds < 1:
        parser.error("--rounds must be positive")
    root = scratch_root()
    root.mkdir(parents=True, exist_ok=True)
    work = args.work.expanduser().resolve() if args.work else Path(tempfile.mkdtemp(prefix="readme-uv-", dir=root))
    allowed = (root, (Path.home() / ".jcode/scratch").resolve())
    if not any(work != base and work.is_relative_to(base) for base in allowed):
        parser.error("--work must be an isolated directory inside scratch")
    if args.work:
        work.mkdir(parents=True, exist_ok=False)
    source = args.source.expanduser().resolve()
    output = args.output.expanduser().resolve() if args.output else work / "result.json"
    result = {"schema": 1, "scope": "focused uv README evidence, not a general speedup",
              "source": {"path": str(source), "commit": PIN}, "rounds": args.rounds, "stacks": stacks,
              "work": str(work), "started_utc": utc_now(), "status": "preparing", "samples": [],
              "cache_policy": "Fresh archive workspace targets, existing shared dependency caches retained and uncontrolled. Not cold.",
              "remote_target_policy": "Unique checkout paths per round/row, check target reused only for its prewarm and edit",
              "prewarm_policy": "Explicit uv-resolver check immediately before body edit, excluded from measured medians",
              "git_policy": "git archive, isolated unborn git index for correct hosted source sync, no worktrees or commits"}
    save_result(output, result)
    try:
        commit = run_text(["git", "-C", str(source), "rev-parse", "44b2e58^{commit}"])
        if commit != PIN:
            raise ValueError(f"source commit mismatch: {commit}")
        archive = work / "uv.tar"
        result["source"]["archive_sha256"] = make_archive(source, archive)
        probe = extract_checkout(archive, work / "identity-probe")
        result["source"]["rust_toolchain_toml"] = (probe / "rust-toolchain.toml").read_text()
        stock_env = sample_environment("stock", probe / "target", None)
        client = Path(justrust_binary())
        frozen = work / "binaries" / "justrust"
        frozen.parent.mkdir()
        client_sha = sha256(client)
        shutil.copy2(client, frozen)
        if sha256(frozen) != client_sha or sha256(client) != client_sha:
            raise ValueError("justrust changed while freezing its binary")
        result["client_original"] = {"path": str(client), "sha256": client_sha}
        identity_env = dict(os.environ, JUSTRUST_NO_UPDATE="1")
        binaries = {"cargo": executable_identity("/usr/bin/cargo", probe, stock_env),
                    "rustc": executable_identity("/usr/bin/rustc", probe, stock_env),
                    "justrust": executable_identity(frozen, probe, identity_env)}
        result["binaries"] = binaries
        result["status"] = "running"
        save_result(output, result)
        for round_index in range(args.rounds):
            order = rotating_order(round_index, stacks)
            checks = {}
            for row in ("fresh-check", "fresh-test-no-run"):
                for stack in order:
                    checkout = extract_checkout(archive, work / f"round-{round_index + 1}" / stack / row)
                    if row == "fresh-check":
                        checks[stack] = checkout
                    execute(result, output, stack, row, round_index, checkout, binaries, work)
            for stack in order:
                checkout = checks[stack]
                prewarm = execute(result, output, stack, "warm-edit-check", round_index,
                                  checkout, binaries, work, measured=False)
                prewarm["phase"] = "untimed-prewarm"
                save_result(output, result)
                if prewarm["status"] != "ok":
                    result["samples"].append({"row": "warm-edit-check", "stack": stack,
                                              "round": round_index + 1, "status": "failed", "measured": True,
                                              "exit": None, "wall_seconds": None, "run_id": None,
                                              "errors": ["prewarm failed, edit measurement blocked"]})
                    save_result(output, result)
                    continue
                with body_edit(checkout, uuid.uuid4().hex) as edit:
                    execute(result, output, stack, "warm-edit-check", round_index,
                            checkout, binaries, work, edit=edit)
                result["samples"][-1]["edit_restored"] = sha256(checkout / EDIT_FILE) == edit["original_sha256"]
                save_result(output, result)
        result["status"] = "complete"
    except (OSError, ValueError, subprocess.SubprocessError, tarfile.TarError) as error:
        result["status"] = "failed"
        result["error"] = sanitize(str(error), work)
    except KeyboardInterrupt:
        result["status"] = "interrupted"
    finally:
        result["finished_utc"] = utc_now()
        save_result(output, result)
    print(f"uv receipt: {output}, all_ok={result['all_ok']}")
    return 0 if result["all_ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
