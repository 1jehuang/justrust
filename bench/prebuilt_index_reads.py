#!/usr/bin/env python3
"""Offline syscall-model paired replay, not compiled Rust or end-to-end timings."""
import argparse
import os
from pathlib import Path
import random
from statistics import median
import tempfile
import time


def search(buf, key):
    lo, hi = 0, len(buf) // 33
    while lo < hi:
        mid = (lo + hi) // 2
        probe = buf[mid * 33:mid * 33 + 32]
        if key == probe:
            return True
        if probe < key:
            lo = mid + 1
        else:
            hi = mid
    return False


def lookup(path, key, bounded):
    fd = os.open(path, os.O_RDONLY)
    try:
        size = os.fstat(fd).st_size
        if not bounded or size <= 65536:
            data = os.read(fd, size)
            assert len(data) == size and not os.read(fd, 1)
            return search(data, key), size, 2
        lo, hi = 0, size // 33
        n = calls = 0
        while hi - lo > 256:
            mid = (lo + hi) // 2
            probe = os.pread(fd, 32, mid * 33)
            calls += 1
            n += len(probe)
            if probe == key:
                return True, n, calls
            if probe < key:
                lo = mid + 1
            else:
                hi = mid
        buf = os.pread(fd, 33 * (hi - lo), 33 * lo)
        return search(buf, key) or os.fstat(fd).st_size != size, n + len(buf), calls + 1
    finally:
        os.close(fd)


def replay(path):
    keys = path.read_bytes().splitlines()
    assert keys == sorted(keys) and all(len(k) == 32 for k in keys)
    rng = random.Random(13)
    hits = rng.choices(keys, k=256)
    # Adjacent hex keys give in-range misses even in the small table.
    queries = hits + [f'{int(k, 16) ^ 1:032x}'.encode() for k in hits]
    rng.shuffle(queries)
    truth = set(keys)
    elapsed = [[], []]
    for rep in range(8):
        for mode in [rep % 2, 1 - rep % 2]:
            total = calls = 0
            start = time.perf_counter_ns()
            for key in queries:
                found, n, c = lookup(path, key, mode)
                assert found == (key in truth)
                total += n
                calls += c
            elapsed[mode].append((time.perf_counter_ns() - start) / len(queries) / 1000)
            if rep == 0:
                print(f'{path.stat().st_size=} {mode=} bytes={total/len(queries):.1f} reads={calls/len(queries):.2f}')
    print('warm batch median us/lookup (whole, bounded):', *(round(median(x), 2) for x in elapsed))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('index', type=Path)
    path = parser.parse_args().index
    replay(path)
    with tempfile.TemporaryDirectory(dir=os.getenv('JCODE_SCRATCH_DIR')) as tmp:
        small = Path(tmp) / 'small'
        with path.open('rb') as src:
            small.write_bytes(src.read((48 * 1024 // 33) * 33))
        replay(small)
