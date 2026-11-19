#!/usr/bin/env python3
"""Compare a package-local CGU cap on an isolated, held Desktop checkout."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import time


def trial(root, cap, phase, digest):
    args = ['justrust', 'test', '--config', f'profile.test.package.jcode-desktop-ui.codegen-units={cap}',
            '-p', 'jcode-desktop-ui', '--lib', '--', 'fps_counter']
    start = time.monotonic()
    env = {**os.environ, 'JUSTRUST_BUILDD': '0', 'JUSTRUST_BUILDD_PREBUILD': '0'}
    p = subprocess.run(args, cwd=root, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    wall = time.monotonic() - start
    if p.returncode or '3 tests passed' not in p.stdout:
        raise RuntimeError(p.stdout)
    rid = re.findall(r'justrust show ([\w-]+)`', p.stdout)[-1]
    runs = Path(os.environ.get('JUSTRUST_HOME', Path.home() / '.justrust')) / 'runs'
    record = runs / rid
    summary = json.loads((record / 'summary.json').read_text())
    if phase != 'cold' and summary['units']['compiled'] != 1:
        raise RuntimeError(f"{rid}: not a one-crate rebuild: {summary['units']['compiled']} units")
    units = [json.loads(line) for line in (record / 'units.jsonl').read_text().splitlines()]
    unit, = [u for u in units if u['crate_name'] == 'jcode_desktop_ui' and u['test']]
    counts = {r['outcome']: r['count'] for r in unit['reuse'] if r['query'] == '<cgu>'}
    assert counts and (phase == 'cold' or counts.get('loaded', 0) + counts.get('changed', 0) > 0), (rid, counts)
    link, = [s['secs'] for s in unit['passes'] if s['name'] == 'run_linker']
    print(json.dumps(dict(phase=phase, cap=cap, source=digest, id=rid, toolchain=summary['toolchain'], foreground=wall,
                          compile=unit['wall'], rss_mb=unit['max_rss_mb'], link=link, cgu=counts)), flush=True)


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('isolated_desktop', type=Path, help='Disposable copy, never a live checkout')
    ap.add_argument('--rounds', type=int, default=6)
    ap.add_argument('--edit-offset', type=int, default=0, help='Use a distinct sequence when repeating the benchmark')
    ap.add_argument('--candidate', type=int, default=4096)
    ap.add_argument('--warm', action='store_true', help='Both configurations already built the unedited source')
    ap.add_argument('--baseline-warm', action='store_true', help='Already ran the baseline cold build')
    a = ap.parse_args()
    root = a.isolated_desktop.resolve()
    f = root / 'crates/jcode-desktop-ui/src/fps_counter.rs'
    original = f.read_bytes()
    anchor = b'let current = snapshot();'
    assert original.count(anchor) == 1
    try:
        for i in range(a.rounds + 1):
            # Alternate a structural edit and an equal-width no-op placeholder.
            # Different literals/comments per pair prevent whole-build result hits.
            n = i + a.edit_offset
            replacement = (f'std::hint::black_box({n:02}u64);' if i % 2 else f'/* unchanged {n:02} */').ljust(36)
            source = original if not i else original.replace(anchor, anchor + replacement.encode())
            if i:
                f.write_bytes(source)
            for cap in ((a.candidate, 256) if i % 2 else (256, a.candidate)):
                if i == 0 and (a.warm or (cap == 256 and a.baseline_warm)):
                    continue
                trial(root, cap, i or 'cold', hashlib.sha256(source).hexdigest())
    finally:
        f.write_bytes(original)


if __name__ == '__main__':
    main()
