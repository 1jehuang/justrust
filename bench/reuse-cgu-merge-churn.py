#!/usr/bin/env python3
"""Isolate size-based CGU merge churn from ordinary red dep nodes (jr-rustc)."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time


def exercise(compiler, root, modules, cap):
    root.mkdir()
    prefix = '#[inline(never)] pub fn f(x:u64)->u64 {'
    source = ''.join(f'mod m{i:04} {{ {prefix} x       }} }}\n' for i in range(modules))
    source += 'fn main(){ let mut x=0;' + ''.join(f'x=m{i:04}::f(x);' for i in range(modules)) + 'println!("{x}");}\n'
    # Equal-width edit leaves all other source spans intact, but changes MIR size.
    edited = source.replace(' x       }', ' x + 1   }', 1)
    assert len(edited) == len(source) and edited != source
    for name, text, expected in [('cold', source, '0'), ('control', source, '0'),
                                 ('edit', edited, '1'), ('revert', source, '0')]:
        (root / 'probe.rs').write_text(text)
        start = time.monotonic()
        subprocess.run([compiler, 'probe.rs', '-Cincremental=incr', f'-Ccodegen-units={cap}',
                        '-Zincremental-verify-ich'], cwd=root, check=True,
                       env={**os.environ, 'JUSTRUST_RUSTC_PHASES_FILE': str(root / name)})
        seconds = time.monotonic() - start
        assert subprocess.check_output([root / 'probe'], text=True).strip() == expected
        ledger = [line.split('\t') for line in (root / f'{name}.reuse').read_text().splitlines()]
        counts = {o: int(n) for o, q, _, n in ledger if q == '<cgu>'}
        if not counts or (name == 'control' and set(counts) != {'loaded'}):
            raise RuntimeError(f'invalid control or missing ledger: {counts}')
        print(json.dumps(dict(cap=cap, phase=name, seconds=round(seconds, 3), cgu=counts,
                              blockers={q: int(n) for o, q, _, n in ledger if o == 'cgu_blocker'})), flush=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('rustc', type=Path)
    parser.add_argument('--modules', type=int, default=384)
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(dir=os.environ.get('JCODE_SCRATCH_DIR')) as temp:
        for cap in (256, args.modules + 32):
            exercise(args.rustc.resolve(), Path(temp) / str(cap), args.modules, cap)
