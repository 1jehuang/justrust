#!/usr/bin/env python3
"""Offline Unix protocol fixture, not a Rust client benchmark. Count index I/O only."""
import argparse
import fcntl
import multiprocessing as mp
import os
from pathlib import Path
import tempfile
import time


def probe(root, coordinated, ready, release, output):
    idx = root / 'index'
    def fresh():
        return idx.exists() and idx.stat().st_mtime > 1

    if fresh():
        output.put(('cached', 0))
        return
    with (root / 'refresh-lock').open('a+b') as lock:
        if coordinated:
            try:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError:
                output.put(('probe-manifest', 0))
                return
            if fresh():
                output.put(('cached', 0))
                return
        ready.put(True)
        deadline = time.monotonic() + 15
        while not release.exists():
            if time.monotonic() > deadline:
                raise TimeoutError('fake store stalled')
            time.sleep(.005)
        data = (root / 'store').read_bytes()
        tmp = root / f'index.{os.getpid()}'
        tmp.write_bytes(data)
        tmp.replace(idx)
        output.put(('index-get', len(data)))


def wave(root, count, coordinated, crash=False):
    ready, release, output = mp.Queue(), root / 'release', mp.Queue()
    release.unlink(missing_ok=True)
    def start():
        p = mp.Process(target=probe, args=(root, coordinated, ready, release, output))
        p.start()
        return p
    leader = start()
    ready.get(timeout=10)
    others = [start() for _ in range(count - 1)]
    if coordinated:
        for _ in others:
            assert output.get(timeout=10) == ('probe-manifest', 0)
    else:
        for _ in others:
            ready.get(timeout=10)
    if crash:
        leader.kill()
        leader.join(10)
        assert leader.exitcode is not None and leader.exitcode < 0
        leader = start()
        ready.get(timeout=10)
    release.touch()
    for p in [leader, *others]:
        p.join(10)
        assert p.exitcode == 0
    fetched = [output.get(timeout=10) for _ in range(1 if coordinated else count)]
    assert all(kind == 'index-get' for kind, _ in fetched)
    assert (root / 'index').read_bytes() == (root / 'store').read_bytes()
    print(f'coordinated={coordinated} crash={crash} requests={len(fetched)+int(crash)} '
          f'completed_bytes={sum(size for _, size in fetched)}')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('retained_index', type=Path)
    args = parser.parse_args()
    data = args.retained_index.read_bytes()
    assert data.endswith(b'\n') and len(data) > 65536
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        for coordinated, crash in [(False, False), (True, False), (True, True)]:
            # Stale copy lacks the final (newly published) key.
            (root / 'index').write_bytes(data[:-33])
            os.utime(root / 'index', (1, 1))
            (root / 'store').write_bytes(data)
            wave(root, 16, coordinated, crash)
