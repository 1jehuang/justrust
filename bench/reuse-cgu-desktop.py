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


def trial(root, cap, phase, digest, arm, pin):
    args = ['justrust', 'test', '--config', f'profile.test.package.jcode-desktop-ui.codegen-units={cap}',
            '-p', 'jcode-desktop-ui', '--lib', '--', 'fps_counter']
    start = time.monotonic()
    env = {**os.environ, 'JUSTRUST_BUILDD': '0', 'JUSTRUST_BUILDD_PREBUILD': '0'}
    if pin:
        env.update(JUSTRUST_TOOLCHAIN=pin, CARGO_TARGET_DIR=str(root / 'target' / f'cgu-bench-{pin}'))
    p = subprocess.run(args, cwd=root, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    wall = time.monotonic() - start
    if p.returncode or '3 tests passed' not in p.stdout:
        raise RuntimeError(p.stdout)
    rid = re.findall(r'justrust show ([\w-]+)`', p.stdout)[-1]
    runs = Path(os.environ.get('JUSTRUST_HOME', Path.home() / '.justrust')) / 'runs'
    record = runs / rid
    summary = json.loads((record / 'summary.json').read_text())
    if pin and summary['toolchain'] != pin:
        raise RuntimeError(f"{rid}: expected {pin}, measured {summary['toolchain']}")
    if phase != 'cold' and summary['units']['compiled'] != 1:
        raise RuntimeError(f"{rid}: not a one-crate rebuild: {summary['units']['compiled']} units")
    units = [json.loads(line) for line in (record / 'units.jsonl').read_text().splitlines()]
    unit, = [u for u in units if u['crate_name'] == 'jcode_desktop_ui' and u['test']]
    counts = {r['outcome']: r['count'] for r in unit['reuse'] if r['query'] == '<cgu>'}
    assert 0 < sum(counts.values()) <= cap, (rid, counts, cap)
    if phase == 'cold':
        assert set(counts) == {'new'}, (rid, counts)
    else:
        assert counts.get('loaded', 0) + counts.get('changed', 0) > 0, (rid, counts)
    link, = [s['secs'] for s in unit['passes'] if s['name'] == 'run_linker']
    row = dict(phase=phase, arm=arm, cap=cap, source=digest, id=rid, toolchain=summary['toolchain'],
               foreground=wall, compile=unit['wall'], rss_mb=unit['max_rss_mb'], link=link, cgu=counts)
    print(json.dumps(row), flush=True)
    return row


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('isolated_desktop', type=Path, help='Disposable copy, never a live checkout')
    ap.add_argument('--rounds', type=int, default=6)
    ap.add_argument('--edit-offset', type=int, default=0, help='Use a distinct sequence when repeating the benchmark')
    ap.add_argument('--candidate', type=int, help='Candidate CGU cap, default 4096 for cap sweeps, 256 for compiler pairs')
    ap.add_argument('--compiler-pair', nargs=2, metavar=('CONTROL_ID', 'CANDIDATE_ID'),
                    help='Full immutable jr-<sha> identities; compare the native policies at the same cap by default')
    ap.add_argument('--output', type=Path, help='Store completed JSON and upload through the benchmark records service')
    ap.add_argument('--warm', action='store_true', help='Both configurations already built the unedited source')
    ap.add_argument('--baseline-warm', action='store_true', help='Already ran the baseline cold build')
    a = ap.parse_args()
    pins = a.compiler_pair or (None, None)
    if a.compiler_pair and (pins[0] == pins[1] or not all(re.fullmatch(r'jr-[a-f0-9]{40}', p) for p in pins)):
        ap.error('--compiler-pair requires two distinct full immutable compiler IDs')
    if a.rounds < 1 or (a.candidate is not None and a.candidate < 1):
        ap.error('rounds and CGU caps must be positive')
    arms = [('control', 256, pins[0]), ('candidate', a.candidate or (256 if a.compiler_pair else 4096), pins[1])]
    rows = []
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
            for arm, cap, pin in (reversed(arms) if i % 2 else arms):
                if i == 0 and (a.warm or (arm == 'control' and a.baseline_warm)):
                    continue
                rows.append(trial(root, cap, i or 'cold', hashlib.sha256(source).hexdigest(), arm, pin))
    finally:
        f.write_bytes(original)
    if a.output:
        import suite
        with a.output.open('x') as out:
            json.dump({'schema': 1, 'cold_scope': 'UI compile, dependency state may differ', 'samples': rows}, out)
        suite.upload_result(a.output)


if __name__ == '__main__':
    main()
