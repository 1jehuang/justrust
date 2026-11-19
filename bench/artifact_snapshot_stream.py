"""Offline Linux copy+hash comparison, fresh process per sample. Cold = no offers.

Usage: python3 bench/artifact_snapshot_stream.py cold|warm|mixed kernel|stream EXE...
JCODE_SCRATCH_DIR must be on the filesystem being tested. Sources are warmed while
creating reference IDs. I/O counters are process accounting, not device traffic.
"""
import hashlib
import json
import os
from pathlib import Path
import resource
import sys
import tempfile
import time

from artifact_snapshot_probe import digest


def io_counts():
    return {k: int(v) for k, v in (line.split(':') for line in Path('/proc/self/io').read_text().splitlines())}


def snapshot(src, dst, scope, stream):
    with src.open('rb', buffering=0) as r, dst.open('xb', buffering=0) as w:
        mode = os.fstat(r.fileno()).st_mode & 0o7777
        if stream:
            h = hashlib.sha256(scope)
            h.update(mode.to_bytes(4, 'little'))
            buf = bytearray(65536)
            while n := r.readinto(buf):
                data = memoryview(buf)[:n]
                h.update(data)
                while data:
                    written = w.write(data)
                    if not written:
                        raise OSError('short write')
                    data = data[written:]
        else:
            while os.copy_file_range(r.fileno(), w.fileno(), 1 << 30):
                pass
        os.fchmod(w.fileno(), mode)
        actual = os.fstat(w.fileno()).st_mode & 0o7777
    # Creation policies can strip mode bits. The final snapshot is authoritative.
    return h.digest() if stream and actual == mode else digest(dst, scope)


def replay(scenario, strategy, paths):
    assert scenario in ('cold', 'warm', 'mixed') and strategy in ('kernel', 'stream')
    with tempfile.TemporaryDirectory(dir=os.environ['JCODE_SCRATCH_DIR']) as tmp:
        root = Path(tmp)
        items = []
        for i, p in enumerate(paths):
            scope = hashlib.sha256(json.dumps(['project', 'target/debug/deps', p.name], separators=(',', ':')).encode()).digest()
            ref = digest(p, scope)
            offer = ref if scenario == 'warm' or (scenario == 'mixed' and i % 2 == 0) else None
            items.append((p, scope, ref, offer))
        before = io_counts()
        start = time.monotonic_ns()
        for i, (p, scope, ref, offer) in enumerate(items):
            snap = root / str(i)
            got = snapshot(p, snap, scope, strategy == 'stream')
            assert got == ref and not snap.samefile(p)
            if got == offer:
                snap.unlink()
        elapsed = (time.monotonic_ns() - start) / 1e6
        after = io_counts()
        for i, (_, scope, ref, offer) in enumerate(items):
            if ref != offer:
                assert digest(root / str(i), scope) == ref
        print(scenario, strategy, round(elapsed, 2), resource.getrusage(resource.RUSAGE_SELF).ru_maxrss,
              *(after[k] - before[k] for k in ('read_bytes', 'write_bytes', 'cancelled_write_bytes')))


if __name__ == '__main__':
    replay(sys.argv[1], sys.argv[2], list(map(Path, sys.argv[3:])))
