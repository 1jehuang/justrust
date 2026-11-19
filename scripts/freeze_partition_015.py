#!/usr/bin/env python3
"""Freeze the selective 0.1.5 partition candidate without changing the checkout."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile

BASE = '4f130cf20aa90de08dc3bcd56da7fd9771dc762f'
DONOR = 'd332efc26be88a051461b3685e5631aadf8e6d6a'
FILE = 'toolchain/rustc/compiler/rustc_monomorphize/src/partitioning.rs'
ROOT = Path(__file__).resolve().parents[1]


def replace_one(text, before, after):
    if text.count(before) != 1:
        raise ValueError(f'missing or ambiguous context: {before[:80]!r}')
    return text.replace(before, after)


def freeze():
    with tempfile.TemporaryDirectory() as tmp:
        env = dict(os.environ, GIT_INDEX_FILE=f'{tmp}/index')

        def git(*args, data=None):
            return subprocess.check_output(['git', '-C', ROOT, *args], input=data, env=env)

        def source(rev):
            return git('show', f'{rev}:{FILE}').decode()

        base, parent, donor = source(BASE), source(DONOR+'^'), source(DONOR)
        # The donor parent already contained unrelated FxHashMap and StdEntry
        # imports. Port only its new FxBuildHasher import onto the old list.
        old = 'use rustc_data_structures::fx::{FxIndexMap, FxIndexSet};'
        new = old.replace('FxIndexMap', 'FxBuildHasher, FxIndexMap')
        anchor = '    let max_codegen_units = cx.tcx.sess.codegen_units().as_usize();\n'
        end = '    while codegen_units.len() > max_codegen_units {'
        block = donor.split(anchor)[1].split(end)[0]
        def apply(text, old, new):
            for a, b in [(old, new), ('use std::io::Write;', 'use std::hash::BuildHasher;\nuse std::io::Write;'), ('DumpMonoStatsFormat, SwitchWithOptPath', 'DumpMonoStatsFormat, OptLevel, SwitchWithOptPath'), (anchor+end, anchor+block+end)]:
                text = replace_one(text, a, b)
            return text
        parent_import = old.replace('FxIndexMap, FxIndexSet', 'FxHashMap, FxIndexMap, FxIndexSet, StdEntry')
        if apply(parent, parent_import, parent_import.replace('FxHashMap', 'FxBuildHasher, FxHashMap')) != donor:
            raise ValueError('transplant does not reconstruct the complete donor delta')
        content = apply(base, old, new).encode()
        blob = git('hash-object', '-w', '--stdin', data=content).decode().strip()
        git('read-tree', BASE)
        git('update-index', '--cacheinfo', '100644', blob, FILE)
        tree = git('write-tree').decode().strip()
        if git('diff', '--name-only', BASE, tree).decode().splitlines() != [FILE]:
            raise ValueError('effective tree differs outside partitioning.rs')
        return dict(base=BASE, donor=DONOR, tree=tree, path=FILE, blob=blob,
                    sha256=hashlib.sha256(content).hexdigest(),
                    cargo_tree=git('rev-parse', tree+':toolchain/cargo').decode().strip())


if __name__ == '__main__':
    print(json.dumps(freeze(), indent=2))
