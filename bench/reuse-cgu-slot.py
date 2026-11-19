#!/usr/bin/env python3
"""Exercise rustc's codegen cache across hosted-style output slot switches (Linux)."""
import argparse
import os
from pathlib import Path
import subprocess
import tempfile

SOURCE = '''mod a { #[inline(never)] pub fn f(x: u32) -> u32 { x + 10 } }
mod b { #[inline(never)] pub fn f(x: u32) -> u32 { x * 3 } }
fn main() { println!("{}", b::f(a::f(2))); }
'''


def exercise(compiler, root, split, baseline):
    root.mkdir()
    src = root / 'example.rs'
    src.write_text(SOURCE)
    for slot in ['a', 'b']:
        (root / slot).mkdir()
    config = [str(compiler), 'example.rs', '-Cdebuginfo=2', '-Ccodegen-units=16',
              f'-Csplit-debuginfo={split}']
    def build(label, slot, cache):
        ledger = root / label
        subprocess.run([*config, '--out-dir', slot, f'-Cincremental={cache}'], cwd=root,
                       env={**os.environ, 'JUSTRUST_RUSTC_PHASES_FILE': str(ledger)}, check=True)
        rows = [s.split('\t') for s in Path(f'{ledger}.reuse').read_text().splitlines()]
        if label == 'moved' and split == 'unpacked' and not baseline:
            blocked = sum(int(n) for outcome, q, _, n in rows
                          if outcome == 'cgu_blocker' and q == 'output_filenames')
            assert blocked == sum(int(n) for outcome, q, _, n in rows
                                  if outcome == 'changed' and q == '<cgu>') > 0
        return {outcome: int(n) for outcome, q, _, n in rows if q == '<cgu>'}
    first = build('cold', 'a', 'cache')
    n = first['new']
    assert n > 1, first
    assert build('same', 'a', 'cache') == {'loaded': n}
    moved = build('moved', 'b', 'cache')
    expected = 'loaded' if split == 'off' and not baseline else 'changed'
    assert moved == {expected: n}, (split, moved)
    assert subprocess.check_output([root / 'b/example']) == b'36\n'
    if split == 'off':
        assert (root / 'a/example').read_bytes() == (root / 'b/example').read_bytes()
    src.write_text(SOURCE.replace('+ 10', '+ 11'))
    edited = build('edited', 'b', 'cache')
    assert edited.get('changed', 0) > 0, edited
    assert subprocess.check_output([root / 'b/example']) == b'39\n'
    incremental = (root / 'b/example').read_bytes()
    # Split DWARF references incremental paths. Use the same now-empty cache
    # path so the clean comparison does not introduce another path difference.
    (root / 'cache').rename(root / 'saved-cache')
    build('clean', 'b', 'cache')
    if split == 'off':
        assert incremental == (root / 'b/example').read_bytes()
    # Split debug paths may include rustc's fresh session directory. Check
    # execution and invalidation instead of treating those bytes as deterministic.
    assert subprocess.check_output([root / 'b/example']) == b'39\n'
    print(f'{split}: slot switch {moved}; real edit {edited}; clean execution matched')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('rustc', type=Path)
    parser.add_argument('--baseline', action='store_true', help='expect old all-dirty behavior')
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(dir=os.environ.get('JCODE_SCRATCH_DIR')) as temp:
        for split in ['off', 'unpacked']:
            exercise(args.rustc.resolve(), Path(temp) / split, split, args.baseline)
