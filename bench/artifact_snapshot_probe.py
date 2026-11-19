"""Linux offline scoped-digest probe replay. Pass real executable paths, scratch on same fs.

Reports wall milliseconds, logical hash reads/copies, peak live snapshot blocks and
process max RSS (KiB). Blocks include reflink-shared extents, not exclusive space.
Cold means no offers, stale means valid offers with different content, not cold OS cache.
"""
import hashlib
import json
import os
from pathlib import Path
import resource
import sys
import tempfile
import time


def digest(path, scope):
    h = hashlib.sha256(scope)
    with path.open('rb') as f:
        st = os.fstat(f.fileno())
        h.update((st.st_mode & 0o7777).to_bytes(4, 'little'))
        while chunk := f.read(65536):
            h.update(chunk)
        def key(s):
            return s.st_dev, s.st_ino, s.st_mode, s.st_size, s.st_mtime_ns, s.st_ctime_ns
        assert key(st) == key(os.fstat(f.fileno())) == key(path.stat())
    return h.digest()


def replay(paths, scenario, probe):
    scopes = [hashlib.sha256(json.dumps(['project', 'target/debug/deps', p.name], separators=(',', ':')).encode()).digest() for p in paths]
    offers = [digest(p, s) if scenario == 'warm' or (scenario == 'mixed' and i % 2 == 0) else None if scenario == 'cold' else bytes(32) for i, (p, s) in enumerate(zip(paths, scopes))]
    start = time.monotonic_ns()
    hashed = copied = peak = live = 0
    with tempfile.TemporaryDirectory(dir=os.environ['JCODE_SCRATCH_DIR']) as tmp:
        for i, (p, s, offer) in enumerate(zip(paths, scopes, offers)):
            n = p.stat().st_size
            if probe and offer is not None:
                hashed += n
                if digest(p, s) == offer:
                    continue
            snap = Path(tmp) / str(i)
            with p.open('rb') as src, snap.open('wb') as dst:
                left = n
                while left:
                    count = os.copy_file_range(src.fileno(), dst.fileno(), min(left, 1 << 30))
                    assert count
                    left -= count
                os.fchmod(dst.fileno(), os.fstat(src.fileno()).st_mode & 0o7777)
            copied += n
            hashed += n
            blocks = snap.stat().st_blocks * 512
            peak = max(peak, live + blocks)
            if digest(snap, s) == offer:
                snap.unlink()
            else:
                live += blocks
    print(scenario, probe, round((time.monotonic_ns()-start)/1e6), hashed, copied, peak, resource.getrusage(resource.RUSAGE_SELF).ru_maxrss)


if __name__ == '__main__':
    replay(list(map(Path, sys.argv[3:])), sys.argv[1], bool(int(sys.argv[2])))
