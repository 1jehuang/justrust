#!/usr/bin/env python3
"""Measure Linux lock-protocol contention without launching Rust builds.

This is a kernel/protocol diagnostic, NOT actual Cargo acceptance, hosted
performance, or evidence of an end-to-end speedup. For real target-lock Cargo
acceptance use contention.py. Each case uses private temporary lock files,
never the user's Cargo cache. No downloads, cache deletion, or target writes.
Results use suite.upload_result's existing account-private records mechanism.

Owned semantics: toolchain/cargo/src/util/cache_lock.rs (CacheState::lock,
CACHE_LOCK_NAME, MUTATE_NAME), compiler/layout.rs (Layout::new lock selection),
and util/flock.rs (FileLock). Mutate takes mutate then download, and rolls back
mutate if download cannot be acquired. Download does not exclude Shared.
Target controls model default coarse build locking with colocated build and
artifact roots, not fine-grain locking, NFS, separate build-dir configuration,
read-only fallback, recursive locking, or the full Cargo acquisition sequence.
acquire_ns includes opening lock files and acquiring the requested lock set,
not compiler work or a fabricated duration inferred from a log message.
"""
from __future__ import annotations

import argparse
from contextlib import ExitStack
import fcntl
import hashlib
import json
import multiprocessing
from pathlib import Path
import platform
import sys
import tempfile
import time

import suite

PROTOCOL = "cargo-package-target-flock-matrix-v1"
MODES = {
    "download": (("home/.package-cache", fcntl.LOCK_EX),),
    "shared": (("home/.package-cache-mutate", fcntl.LOCK_SH),),
    "mutate": (("home/.package-cache-mutate", fcntl.LOCK_EX),
               ("home/.package-cache", fcntl.LOCK_EX)),
    "target": (("target/debug/.cargo-build-lock", fcntl.LOCK_EX),
               ("target/debug/.cargo-lock", fcntl.LOCK_SH)),
    "other-target": (("other-target/debug/.cargo-build-lock", fcntl.LOCK_EX),
                     ("other-target/debug/.cargo-lock", fcntl.LOCK_SH)),
}
SOURCES = ("src/util/cache_lock.rs", "src/util/flock.rs", "src/compiler/layout.rs")


def acquire(root, mode, nonblocking=False):
    """Return all held handles, rolling back partial acquisition on any error."""
    stack = ExitStack()
    try:
        for name, operation in MODES[mode]:
            path = Path(root) / name
            path.parent.mkdir(parents=True, exist_ok=True)
            handle = stack.enter_context(path.open("a+b"))
            fcntl.flock(handle, operation | (fcntl.LOCK_NB if nonblocking else 0))
        return stack
    except BaseException:
        stack.close()
        raise


def conflicts(holder, contender):
    return any(a == b and (x == fcntl.LOCK_EX or y == fcntl.LOCK_EX)
               for a, x in MODES[holder] for b, y in MODES[contender])


def probe(root, mode, channel):
    try:
        start = time.monotonic_ns()
        try:
            locks = acquire(root, mode, nonblocking=True)
        except BlockingIOError:
            channel.send({"event": "blocked"})
            start = time.monotonic_ns()
            with acquire(root, mode):
                acquired = time.monotonic_ns()
        else:
            acquired = time.monotonic_ns()
            locks.close()
            channel.send({"event": "compatible"})
        channel.send({"event": "acquired", "acquire_ns": acquired - start})
    except BaseException as error:
        channel.send({"event": "error", "error": repr(error)})
    finally:
        channel.close()


def receive(channel, timeout=10):
    if not channel.poll(timeout):
        raise ValueError("package contention: child handshake timed out")
    result = channel.recv()
    if result.get("event") == "error":
        raise ValueError(result["error"])
    return result


def measure(holder, contender, hold_seconds=0.1):
    # spawn prevents inherited holder descriptors from keeping its lock alive.
    context = multiprocessing.get_context("spawn")
    with tempfile.TemporaryDirectory(prefix="justrust-package-lock-") as root:
        parent, child = context.Pipe()
        locks = acquire(root, holder)
        process = context.Process(target=probe, args=(root, contender, child))
        process.start()
        child.close()
        try:
            event = receive(parent)["event"]
            blocked = event == "blocked"
            if event not in ("blocked", "compatible") or blocked != conflicts(holder, contender):
                raise ValueError(f"unexpected lock compatibility: {holder}/{contender}: {event}")
            hold_start = time.monotonic_ns()
            if blocked:
                # A nonblocking syscall proved contention before the hold begins.
                time.sleep(hold_seconds)
                locks.close()
            observed = receive(parent)
            hold_end = time.monotonic_ns()
            if observed.get("event") != "acquired":
                raise ValueError("missing acquisition evidence")
            process.join(timeout=10)
            if process.is_alive() or process.exitcode != 0:
                raise ValueError("lock probe did not exit successfully")
            return {"holder": holder, "contender": contender, "blocked": blocked,
                    "acquire_ns": observed["acquire_ns"],
                    "parent_observation_ns": hold_end - hold_start,
                    "configured_hold_seconds": hold_seconds,
                    "clock": "monotonic_ns", "scope": "local-kernel-protocol"}
        finally:
            locks.close()
            if process.is_alive():
                process.terminate()
                process.join(timeout=10)
            parent.close()


def source_identity():
    root = Path(__file__).resolve().parent.parent / "toolchain/cargo"
    return {str(path): hashlib.sha256((root / path).read_bytes()).hexdigest()
            for path in SOURCES}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", required=True)
    parser.add_argument("--hold-seconds", type=float, default=0.1)
    parser.add_argument("-n", type=int, default=1)
    args = parser.parse_args()
    if sys.platform != "linux" or not 0 < args.hold_seconds <= 10 or args.n < 1:
        parser.error("requires Linux, positive -n, and 0 < --hold-seconds <= 10")
    sources = source_identity()
    samples = [measure(a, b, args.hold_seconds) for _ in range(args.n)
               for a in MODES for b in MODES]
    if sources != source_identity():
        raise ValueError("owned Cargo lock source changed during measurement")
    result = {"schema": PROTOCOL, "scope": "local kernel protocol, not Cargo or build speed",
              "finished": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
              "harness_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
              "host": platform.node(), "platform": platform.platform(),
              "source_sha256": sources, "samples": samples}
    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(result, indent=2) + "\n")
    for sample in samples:
        print(f"{sample['holder']:12} -> {sample['contender']:12} "
              f"blocked={sample['blocked']} acquire_ms={sample['acquire_ns'] / 1e6:.3f}")
    print(f"saved {out}")
    suite.upload_result(out)


if __name__ == "__main__":
    main()
