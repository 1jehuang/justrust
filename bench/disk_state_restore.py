#!/usr/bin/env python3
"""Benchmark byte-for-byte copies of existing Rust build state, not synthetic data.

Usage: python3 bench/disk_state_restore.py SOURCE DIRECTORY [--workers 1,4,16]
DIRECTORY is an existing scratch parent on the disk to measure. Only a private
TemporaryDirectory below it is removed. Stop builds that write SOURCE first.
Symlinks and special files are rejected, not followed or silently omitted.
JSON goes to stdout. This is a warm-page-cache benchmark, not durable-storage
throughput: verification warms caches, no caches are dropped and no fsync runs.
"""

import argparse
from concurrent.futures import ThreadPoolExecutor
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import stat
import statistics
import tempfile
import time


CACHE_NOTE = (
    "Warm-page-cache file-copy benchmark. Hash verification and repeated copies "
    "warm the page cache; no global cache dropping, compression, or fsync. "
    "Times measure copy completion, not durable storage writes."
)


def validate_paths(source, directory):
    source = Path(source).resolve(strict=True)
    directory = Path(directory).resolve(strict=True)
    if not source.is_dir() or not directory.is_dir():
        raise ValueError("SOURCE and DIRECTORY must be existing directories")
    if source == directory or source in directory.parents or directory in source.parents:
        raise ValueError("SOURCE and DIRECTORY must not overlap (including ancestors)")
    return source, directory


def manifest(root):
    """Hash regular files and record directories without following symlinks."""
    root = Path(root)
    files = {}
    directories = []

    def walk_error(error):
        raise error

    for parent, dirs, names in os.walk(root, followlinks=False, onerror=walk_error):
        for name in sorted(dirs + names):
            path = Path(parent) / name
            relative = path.relative_to(root).as_posix()
            mode = path.lstat().st_mode
            if stat.S_ISDIR(mode):
                directories.append(relative)
            elif stat.S_ISREG(mode):
                digest = hashlib.sha256()
                size = 0
                with path.open("rb") as stream:
                    for block in iter(lambda: stream.read(1024 * 1024), b""):
                        size += len(block)
                        digest.update(block)
                files[relative] = {"bytes": size, "sha256": digest.hexdigest()}
            else:
                raise ValueError(f"Unsupported symlink or special file: {path}")
    return {"directories": sorted(directories), "files": dict(sorted(files.items()))}


def verify(root, expected):
    if manifest(root) != expected:
        raise ValueError(f"Content verification failed: {root}")


def copy_tree(source, destination, inventory, workers):
    """Copy into a new tree. Caller owns destination's isolated parent."""
    destination.mkdir()
    for relative in inventory["directories"]:
        (destination / relative).mkdir(parents=True, exist_ok=True)

    def copy(relative):
        shutil.copy2(source / relative, destination / relative)

    if workers == 1:
        for relative in inventory["files"]:
            copy(relative)
    else:
        with ThreadPoolExecutor(max_workers=workers) as pool:
            # Consume results so worker exceptions propagate before timing ends.
            for _ in pool.map(copy, inventory["files"]):
                pass


def benchmark(source, directory, workers=(1, 4, 16), repetitions=3):
    source, directory = validate_paths(source, directory)
    workers = list(workers)
    if not workers or any(not isinstance(n, int) or isinstance(n, bool) or n < 1 for n in workers):
        raise ValueError("Worker counts must be positive integers")
    if len(set(workers)) != len(workers):
        raise ValueError("Worker counts must be unique")
    if not isinstance(repetitions, int) or isinstance(repetitions, bool) or repetitions < 3:
        raise ValueError("At least three repetitions are required")
    inventory = manifest(source)
    file_count = len(inventory["files"])
    byte_count = sum(item["bytes"] for item in inventory["files"].values())
    if not file_count:
        raise ValueError("SOURCE must contain at least one regular file")
    runs = []
    with tempfile.TemporaryDirectory(prefix="disk-state-restore-", dir=directory) as owned:
        root = Path(owned)
        snapshot = root / "snapshot"
        started = time.perf_counter()
        copy_tree(source, snapshot, inventory, 1)
        snapshot_seconds = time.perf_counter() - started
        verify(snapshot, inventory)
        verify(source, inventory)
        # Alternate worker order to reduce systematic ordering bias.
        for repetition in range(1, repetitions + 1):
            order = workers if repetition % 2 else list(reversed(workers))
            for count in order:
                destination = root / "restore"
                started = time.perf_counter()
                copy_tree(snapshot, destination, inventory, count)
                elapsed = time.perf_counter() - started
                verify(destination, inventory)  # Deliberately outside timed region.
                runs.append({"workers": count, "repetition": repetition,
                             "seconds": elapsed, "files": file_count,
                             "bytes": byte_count, "verified": True,
                             "bytes_per_second": byte_count / elapsed if elapsed else None})
                shutil.rmtree(destination)  # Only our own private restore tree.
        verify(snapshot, inventory)
        verify(source, inventory)
    summaries = []
    for count in workers:
        seconds = [run["seconds"] for run in runs if run["workers"] == count]
        summaries.append({"workers": count, "repetitions": repetitions,
                          "min_seconds": min(seconds),
                          "median_seconds": statistics.median(seconds),
                          "mean_seconds": statistics.mean(seconds),
                          "max_seconds": max(seconds)})
    return {"schema_version": 1, "cache_note": CACHE_NOTE,
            "copy_method": "shutil.copy2 (platform fast-copy allowed), one task per file",
            "timed_region": "fresh directory creation, file copy, metadata copy, executor startup/shutdown",
            "verification": "SHA-256 of every file plus exact file/directory inventory, outside timed regions",
            "source": str(source), "disk_path": str(directory),
            "machine": {"cpu_count": os.cpu_count(), "platform": platform.platform(),
                        "architecture": platform.machine(), "hostname": platform.node(),
                        "python": platform.python_version(),
                        "disk_device": directory.stat().st_dev},
            "files": file_count, "bytes": byte_count,
            "snapshot": {"seconds": snapshot_seconds, "workers": 1, "verified": True,
                         "files": file_count, "bytes": byte_count},
            "runs": runs, "summaries": summaries}


def parse_workers(value):
    try:
        counts = [int(part) for part in value.split(",")]
    except ValueError as error:
        raise argparse.ArgumentTypeError("Use comma-separated positive worker counts") from error
    if not counts or any(n < 1 for n in counts) or len(set(counts)) != len(counts):
        raise argparse.ArgumentTypeError("Worker counts must be positive and unique")
    return counts


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("directory", type=Path, help="existing, non-overlapping scratch parent")
    parser.add_argument("--workers", type=parse_workers, default=[1, 4, 16])
    parser.add_argument("--repetitions", type=int, default=3)
    args = parser.parse_args(argv)
    try:
        report = benchmark(args.source, args.directory, args.workers, args.repetitions)
    except (OSError, ValueError) as error:
        parser.error(str(error))
    print(json.dumps(report, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
