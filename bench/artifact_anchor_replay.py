"""Local cold selected-test anchor replay. No build, hosted call or record upload.

Pass a retained selected-artifacts.json and --pipeline for one frame of lookahead.
--mbps is an imposed decimal megabits/sec link limit, not a measured hosted rate.
"""
import argparse
from concurrent.futures import ThreadPoolExecutor
import hashlib
import json
from pathlib import Path
import shutil
import stat
import subprocess
import tempfile
import time


def digest(path):
    with path.open('rb') as f:
        return hashlib.file_digest(f, 'sha256').digest()


def replay(paths, root, pipeline, mbps):
    sent = []
    order = []
    stream = hashlib.sha256()
    transferred = anchors = 0

    def prepare(i):
        src = paths[i]
        snap = root / f'{i}.snap'
        shutil.copy2(src, snap)
        digest(snap)  # The selected-test host hashes every snapshot for its id.
        size = snap.stat().st_size
        choices = ((abs(n - size), j) for j, n in enumerate(sent)
                   if min(n, size) >= 1 << 20 and max(n, size) <= 2 * min(n, size))
        base = min(choices, default=(0, None))[1]
        sent.append(size)
        flags = ['-9', '-T0'] if base is None and size >= 32 << 20 else ['-3', '-T0', '--long=31']
        if base is not None:
            flags.append(f'--patch-from={root / f"{base}.snap"}')
        payload = root / f'{i}.zst'
        subprocess.run(['zstd', '-q', '-f', *flags, snap, '-o', payload], check=True)
        return base, payload

    def receive(i, frame):
        nonlocal transferred, anchors
        base, payload = frame
        flags = [] if base is None else [f'--patch-from={root / f"{base}.out"}']
        dst = root / f'{i}.out'
        start = time.monotonic()
        with subprocess.Popen(['zstd', '-qd', '--long=31', *flags, '-', '-o', dst],
                              stdin=subprocess.PIPE) as decoder, payload.open('rb') as f:
            frame_bytes = 0
            while chunk := f.read(1 << 20):
                decoder.stdin.write(chunk)
                stream.update(chunk)
                frame_bytes += len(chunk)
                if mbps:
                    time.sleep(max(0, start + frame_bytes * 8 / (mbps * 1e6) - time.monotonic()))
            decoder.stdin.close()
            if decoder.wait():
                raise RuntimeError(f'decode failed: {paths[i]}')
        dst.chmod(stat.S_IMODE(paths[i].stat().st_mode))
        transferred += frame_bytes
        anchors += base is not None
        order.append(i)
        payload.unlink()

    start = time.monotonic()
    if pipeline:
        with ThreadPoolExecutor(max_workers=1) as encoder:
            pending = encoder.submit(prepare, 0)
            for i in range(len(paths)):
                frame = pending.result()
                if i + 1 < len(paths):
                    pending = encoder.submit(prepare, i + 1)
                receive(i, frame)
    else:
        for i in range(len(paths)):
            receive(i, prepare(i))
    elapsed = time.monotonic() - start
    assert order == list(range(len(paths)))
    for i, src in enumerate(paths):
        dst = root / f'{i}.out'
        assert digest(src) == digest(dst) and src.stat().st_mode == dst.stat().st_mode, src
    print(json.dumps(dict(pipelined=pipeline, files=len(paths), anchors=anchors, bytes=transferred,
                          payload_sha256=stream.hexdigest(), seconds=round(elapsed, 3), mbps=mbps)))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('manifest', type=Path)
    parser.add_argument('--pipeline', action='store_true')
    parser.add_argument('--mbps', type=float, default=0)
    args = parser.parse_args()
    paths = [Path(a['executable']) for a in json.loads(args.manifest.read_text())]
    if not paths or args.mbps < 0:
        parser.error('supply a nonempty manifest and nonnegative link rate')
    with tempfile.TemporaryDirectory() as tmp:
        replay(paths, Path(tmp), args.pipeline, args.mbps)
