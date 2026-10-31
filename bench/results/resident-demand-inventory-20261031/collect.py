#!/usr/bin/env python3
"""One-shot evidence retention. Reads frozen receipts and git objects, never live toolchains."""
import hashlib
import json
from pathlib import Path
import shutil
import subprocess

HERE = Path(__file__).resolve().parent
ROOT = Path('/home/jeremy/.jcode/scratch/blaze-native-engine-20261030')
OLD = HERE.parent / 'resident-demand-std-v33-20261031'
COMMIT = 'a59351f652c9b266b7f29734f2218cbb19a9aaad'

def copy(src, dst):
    dst.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(src, dst)

def tree(src, dst):
    for p in sorted(src.rglob('*')):
        if p.is_file():
            assert not p.is_symlink(), p
            copy(p, dst / p.relative_to(src))

def main():
    for version in (35, 36):
        tree(ROOT / f'demand-library-artifact-v{version}', HERE / f'frozen/v{version}')
        copy(ROOT / f'build-test-candidate-v{version}.sh', HERE / f'provenance/build-test-candidate-v{version}.sh')
    # v34 is history, not a successful implementation or a substitute for v35 failure.
    tree(ROOT / 'demand-library-artifact-v34', HERE / 'frozen/v34-history')
    for group in ('candidate-build-v35', 'candidate-build-v36', 'candidate-unit-v36'):
        for p in sorted((ROOT / group).iterdir()):
            if p.is_file():
                copy(p, HERE / 'gates' / group / p.name)
        runs = ROOT / group / 'justrust-home/runs'
        for run in sorted(runs.glob('*')):
            if not run.is_dir():
                continue
            for name in ('meta.json', 'output.jsonl', 'summary.json'):
                copy(run / name, HERE / 'journals' / run.name / name)
            summary = json.loads((run / 'summary.json').read_text())
            slim = {k: summary[k] for k in ('id', 'args', 'subcommand', 'start', 'wall', 'exit', 'tests', 'route') if k in summary}
            slim['scope'] = 'Derived receipt, full original summary retained in journals. No paths or source.'
            dest = HERE / 'slim' / (run.name + '.json')
            dest.parent.mkdir(parents=True, exist_ok=True)
            dest.write_text(json.dumps(slim, indent=2) + '\n')
    tree(ROOT / 'demand-inventory-v36-native', HERE / 'probe')
    copy(ROOT / 'probe-demand-inventory-v36.py', HERE / 'provenance/probe-demand-inventory-v36.py')
    for name in ('rsb21', 'rsa27'):
        for p in sorted((Path('/home/jeremy') / name).iterdir()):
            if p.is_file():
                copy(p, HERE / 'runtime' / name / p.name)
    tree(OLD / 'input', HERE / 'input')
    for p in (OLD / 'provenance').glob('*.py'):
        copy(p, HERE / 'provenance' / p.name)
    for name in ('LICENSE-MIT', 'LICENSE-APACHE'):
        copy(OLD / 'proof' / name, HERE / 'licenses' / name)
    for name in ('32891831af', '306723vuyn'):
        copy(Path('/tmp/jcode-bg-tasks') / (name + '.output'), HERE / 'outer' / (name + '.output'))
    paths = {'codegen-mod.rs': 'toolchain/resident/crates/hir-ty-codegen/mod.rs',
             'main.rs': 'toolchain/resident/crates/ra-jit/src/main.rs',
             'doc.md': 'docs/resident-demand-library-artifact.md'}
    for name, path in paths.items():
        dest = HERE / 'accepted' / name
        dest.parent.mkdir(parents=True, exist_ok=True)
        dest.write_bytes(subprocess.check_output(['git', 'show', f'{COMMIT}:{path}'], cwd=HERE))
    (HERE / 'accepted/change.patch').write_bytes(subprocess.check_output(['git', 'show', '--format=fuller', '--binary', COMMIT], cwd=HERE))
    (HERE / 'accepted/commit.txt').write_text(COMMIT + '\n')
    print('Retained frozen inputs only. No workload or live source/toolchain reads.')

if __name__ == '__main__':
    main()
