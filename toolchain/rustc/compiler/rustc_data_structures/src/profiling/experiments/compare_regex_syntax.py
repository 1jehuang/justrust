"""Compare unmodified and indexed compilers on a real regex-syntax metadata build.

Pass the two complete stage1 bin/rustc paths and regex-syntax-0.8.8/src/lib.rs.
Use taskset externally if desired. No project source or persistent outputs are modified.
"""

import argparse
import hashlib
import os
from pathlib import Path
import statistics
import subprocess
import tempfile
import time


def measure(rustc, src, indexed, work):
    output = work / 'out.rmeta'
    ledger = work / 'phases.tsv'
    for path in (ledger, output):
        path.unlink(missing_ok=True)
    env = dict(os.environ, JUSTRUST_RUSTC_PHASES_FILE=str(ledger),
               JUSTRUST_EXPERIMENT_INDEXED_PHASES=str(int(indexed)))
    args = [rustc, src, '--crate-name', 'regex_syntax', '--edition=2021',
            '--crate-type=rlib', '--emit=metadata', '-o', output]
    features = ('std unicode unicode-age unicode-bool unicode-case unicode-gencat '
                'unicode-perl unicode-script unicode-segment')
    for feature in features.split():
        args.extend(['--cfg', f'feature="{feature}"'])
    start = time.perf_counter_ns()
    subprocess.run(args, env=env, check=True)
    elapsed_ms = (time.perf_counter_ns() - start) / 1e6
    rows = [row.split('\t') for row in ledger.read_text().splitlines()]
    assert rows[-1][0] == 'total' and rows[-1][2] == '1'
    assert all(len(row) == 3 and float(row[1]) >= 0 for row in rows)
    counts = {name: int(count) for name, _, count in rows}
    assert len(counts) == len(rows)
    assert rows[:-1] == sorted(rows[:-1], key=lambda row: -float(row[1]))
    expand_ms = next(float(v) * 1000 for name, v, _ in rows if name == 'macro_expand_crate')
    return (elapsed_ms, expand_ms), counts, hashlib.sha256(output.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for arg in ('baseline', 'candidate', 'source'):
        parser.add_argument(arg, type=Path)
    parser.add_argument('--pairs', type=int, default=16)
    args = parser.parse_args()
    if args.pairs < 2:
        parser.error('at least two measured pairs required')
    expected = None
    digests = {}
    versions = {arm: subprocess.check_output([getattr(args, arm).resolve(), '-vV'])
                for arm in ('baseline', 'candidate')}
    samples = []
    with tempfile.TemporaryDirectory() as temp:
        for i in range(args.pairs + 2):
            pair = {}
            for arm in (('baseline', 'candidate') if i % 2 == 0 else ('candidate', 'baseline')):
                timing, counts, digest = measure(getattr(args, arm).resolve(), args.source.resolve(),
                                                 arm == 'candidate', Path(temp))
                if expected is None:
                    expected = counts
                if counts != expected:
                    raise ValueError(f'{arm} iteration {i}: phase counts differ')
                if digests.setdefault(arm, digest) != digest:
                    raise ValueError(f'{arm} iteration {i}: nondeterministic metadata')
                if versions['baseline'] == versions['candidate'] and len(set(digests.values())) != 1:
                    raise ValueError('same compiler identity produced different metadata')
                pair[arm] = timing
            print(i, pair, flush=True)
            if i >= 2:
                samples.append(pair)
    print('nonzero expand labels:', {k: v for k, v in expected.items() if k.startswith('expand_')})
    print('artifact sha256 by compiler identity:', digests)
    for col, label in enumerate(('wall', 'macro_expand_crate')):
        deltas = [r['baseline'][col] - r['candidate'][col] for r in samples]
        print(label, 'median baseline/candidate ms:',
              *(statistics.median(r[k][col] for r in samples) for k in ('baseline', 'candidate')),
              'paired median saving/range ms:', statistics.median(deltas), min(deltas), max(deltas))


if __name__ == '__main__':
    main()
