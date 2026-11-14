#!/usr/bin/env python3
"""Restore independent zstd chunks made from existing build artifacts.

Usage: python3 bench/compressed_state_restore.py SOURCE SCRATCH_DIRECTORY
Requires libzstd and Linux/POSIX pwrite. Stop writers to SOURCE before running.
Only isolated temporary paths are removed. No synthetic payloads or cache drops.
This measures disk-backed chunk decode/write, NOT rustc query deserialization.
"""
import argparse
from concurrent.futures import ThreadPoolExecutor
import ctypes
import ctypes.util
import json
import os
from pathlib import Path
import platform
import shutil
import statistics
import tempfile
import time

try:
    from .disk_state_restore import EVICT_CACHE_NOTE, evict_snapshot_cache, read_bytes_counter, manifest, parse_workers, validate_paths, verify
except ImportError:
    from disk_state_restore import EVICT_CACHE_NOTE, evict_snapshot_cache, read_bytes_counter, manifest, parse_workers, validate_paths, verify


CACHE_NOTE = (
    "Warm-page-cache compressed-chunk extraction benchmark. Encoding, verification "
    "and repeated restores warm caches. No global cache dropping or fsync. "
    "Times measure restoration completion, not durable storage writes."
)


class Zstd:
    """Small one-shot libzstd binding. CDLL releases the GIL during codec calls."""
    def __init__(self):
        name = ctypes.util.find_library("zstd")
        if not name:
            raise ValueError("libzstd is required")
        self.lib = ctypes.CDLL(name)
        size = ctypes.c_size_t
        pointer = ctypes.c_void_p
        signatures = {
            "ZSTD_compressBound": ([size], size),
            "ZSTD_compress": ([pointer, size, pointer, size, ctypes.c_int], size),
            "ZSTD_decompress": ([pointer, size, pointer, size], size),
            "ZSTD_isError": ([size], ctypes.c_uint),
            "ZSTD_getErrorName": ([size], ctypes.c_char_p),
            "ZSTD_versionString": ([], ctypes.c_char_p),
        }
        for symbol, (arguments, result) in signatures.items():
            function = getattr(self.lib, symbol)
            function.argtypes = arguments
            function.restype = result
        self.version = self.lib.ZSTD_versionString().decode()

    def checked(self, result):
        if self.lib.ZSTD_isError(result):
            raise ValueError("zstd: " + self.lib.ZSTD_getErrorName(result).decode())
        return result

    def compress(self, data, level):
        capacity = self.lib.ZSTD_compressBound(len(data))
        output = ctypes.create_string_buffer(capacity)
        size = self.checked(self.lib.ZSTD_compress(output, capacity, data, len(data), level))
        return output.raw[:size]

    def decompress(self, data, expected_size):
        output = ctypes.create_string_buffer(expected_size)
        size = self.checked(self.lib.ZSTD_decompress(output, expected_size, data, len(data)))
        if size != expected_size:
            raise ValueError("zstd decoded chunk length mismatch")
        return output.raw[:size]


def encode(source, destination, inventory, codec, chunk_bytes, level):
    destination.mkdir()
    chunks = []
    codec_seconds = 0.0
    for relative in inventory["files"]:
        with (source / relative).open("rb") as stream:
            offset = 0
            while True:
                data = stream.read(chunk_bytes)
                if not data:
                    break
                started = time.perf_counter()
                compressed = codec.compress(data, level)
                codec_seconds += time.perf_counter() - started
                name = f"{len(chunks):09d}.zst"
                (destination / name).write_bytes(compressed)
                chunks.append({"frame": name, "file": relative, "offset": offset,
                               "bytes": len(data), "compressed_bytes": len(compressed)})
                offset += len(data)
    return chunks, codec_seconds


def restore(snapshot, destination, inventory, chunks, codec, workers):
    destination.mkdir()
    for relative in inventory["directories"]:
        (destination / relative).mkdir(parents=True, exist_ok=True)
    for relative, item in inventory["files"].items():
        with (destination / relative).open("xb") as stream:
            stream.truncate(item["bytes"])

    def decode(chunk):
        compressed = (snapshot / chunk["frame"]).read_bytes()
        started = time.perf_counter()
        data = codec.decompress(compressed, chunk["bytes"])
        codec_seconds = time.perf_counter() - started
        fd = os.open(destination / chunk["file"], os.O_WRONLY)
        try:
            written = 0
            while written < len(data):
                count = os.pwrite(fd, memoryview(data)[written:], chunk["offset"] + written)
                if count <= 0:
                    raise OSError("pwrite made no progress")
                written += count
        finally:
            os.close(fd)
        return codec_seconds

    if workers == 1:
        return sum(decode(chunk) for chunk in chunks)
    with ThreadPoolExecutor(max_workers=workers) as pool:
        return sum(pool.map(decode, chunks))


def benchmark(source, directory, workers=(1, 4, 16), repetitions=3,
              chunk_bytes=4 * 1024 * 1024, level=3, evict_source_cache=False):
    source, directory = validate_paths(source, directory)
    workers = list(workers)
    if not workers or any(type(n) is not int or n < 1 for n in workers) or len(set(workers)) != len(workers):
        raise ValueError("Worker counts must be positive unique integers")
    if type(repetitions) is not int or repetitions < 3:
        raise ValueError("At least three repetitions are required")
    if type(chunk_bytes) is not int or chunk_bytes < 1:
        raise ValueError("chunk_bytes must be a positive integer")
    if type(level) is not int or not 1 <= level <= 19:
        raise ValueError("zstd level must be between 1 and 19")
    inventory = manifest(source)
    files = len(inventory["files"])
    size = sum(item["bytes"] for item in inventory["files"].values())
    if not files or not size:
        raise ValueError("SOURCE must contain nonempty real files")
    codec = Zstd()
    runs = []
    with tempfile.TemporaryDirectory(prefix="compressed-state-restore-", dir=directory) as owned:
        root = Path(owned)
        snapshot = root / "chunks"
        started = time.perf_counter()
        chunks, encoding_codec_seconds = encode(source, snapshot, inventory, codec, chunk_bytes, level)
        encoding_seconds = time.perf_counter() - started
        verify(source, inventory)
        compressed_bytes = sum(chunk["compressed_bytes"] for chunk in chunks)
        for repetition in range(1, repetitions + 1):
            order = workers if repetition % 2 else list(reversed(workers))
            for count in order:
                destination = root / "restore"
                preparation_seconds = 0.0
                if evict_source_cache:
                    preparation_started = time.perf_counter()
                    evict_snapshot_cache(snapshot, {"files": {c["frame"]: {} for c in chunks}})
                    preparation_seconds = time.perf_counter() - preparation_started
                read_before = read_bytes_counter()
                started = time.perf_counter()
                codec_seconds = restore(snapshot, destination, inventory, chunks, codec, count)
                elapsed = time.perf_counter() - started
                read_after = read_bytes_counter()
                verify(destination, inventory)
                runs.append({"workers": count, "repetition": repetition, "seconds": elapsed,
                             "codec_thread_seconds_sum": codec_seconds, "files": files,
                             "bytes": size, "compressed_bytes": compressed_bytes,
                             "verified": True, "cache_preparation_seconds": preparation_seconds,
                             "read_bytes_delta": (read_after - read_before
                                                  if read_before is not None and read_after is not None else None),
                             "bytes_per_second": size / elapsed if elapsed else None})
                shutil.rmtree(destination)
        verify(source, inventory)
    return {"schema_version": 1, "source": str(source), "disk_path": str(directory),
            "scope": "independent compressed file chunks, not compiler query/heap deserialization",
            "cache_note": EVICT_CACHE_NOTE if evict_source_cache else CACHE_NOTE,
            "evict_source_cache": evict_source_cache,
            "read_bytes_note": "Guest storage-read accounting around extraction, not proof of physical-device cold reads.",
            "timed_region": "fresh tree creation/preallocation, compressed reads, zstd decode, pwrite, executor lifecycle; no hashes or deletion",
            "codec_timing_note": "codec thread seconds are summed elapsed call times, may exceed wall time under parallelism, not CPU time",
            "metadata_note": "file contents and empty directories restored; file metadata not restored",
            "verification": "SHA-256 and exact file/directory inventory outside timed restore",
            "machine": {"cpu_count": os.cpu_count(), "platform": platform.platform(),
                        "architecture": platform.machine(), "hostname": platform.node(),
                        "python": platform.python_version(), "disk_device": directory.stat().st_dev},
            "codec": {"name": "zstd", "version": codec.version, "level": level,
                      "chunk_bytes": chunk_bytes, "chunks": len(chunks)},
            "files": files, "bytes": size, "compressed_bytes": compressed_bytes,
            "compressed_to_original_ratio": compressed_bytes / size,
            "encoding": {"workers": 1, "seconds": encoding_seconds,
                         "codec_seconds": encoding_codec_seconds},
            "runs": runs,
            "summaries": [{"workers": count, "repetitions": repetitions,
                           "median_seconds": statistics.median(r["seconds"] for r in runs if r["workers"] == count),
                           "mean_seconds": statistics.mean(r["seconds"] for r in runs if r["workers"] == count)}
                          for count in workers]}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--workers", type=parse_workers, default=[1, 4, 16])
    parser.add_argument("--repetitions", type=int, default=3)
    parser.add_argument("--chunk-bytes", type=int, default=4 * 1024 * 1024)
    parser.add_argument("--level", type=int, default=3)
    parser.add_argument("--evict-source-cache", action="store_true")
    args = parser.parse_args(argv)
    try:
        report = benchmark(args.source, args.directory, args.workers, args.repetitions,
                           args.chunk_bytes, args.level, args.evict_source_cache)
    except (ValueError, OSError) as error:
        parser.error(str(error))
    print(json.dumps(report, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
