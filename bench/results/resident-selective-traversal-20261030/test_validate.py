#!/usr/bin/env python3
"""Relocation and semantic mutations after integrity re-signing, offline only."""
import argparse
import importlib.util
import json
import os
import shutil
import sys
import tempfile
from pathlib import Path

sys.dont_write_bytecode = True
HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('selective_evidence', HERE / 'validate.py')
v = importlib.util.module_from_spec(spec)
spec.loader.exec_module(v)


def dump(path, data):
    path.write_text(json.dumps(data, indent=2) + '\n')


def update_json(root, name, callback):
    path = root / name
    data = json.loads(path.read_text())
    callback(data)
    dump(path, data)


def resign(root):
    # Recompute all outer collection integrity. Individual mutations also update
    # stream checksums where relevant so failures must reach semantic checks.
    path = root / 'collection.json'
    data = json.loads(path.read_text())
    for item in data['files']:
        file = root / item['artifact']
        if file.is_file() and not file.is_symlink():
            item['sha256'] = v.sha(file.read_bytes())
    dump(path, data)


def marker(root, duplicate=False):
    name = 'interleaved/edit-0-new.stdout'
    path = root / name
    old = 'JR_RUNTIME_EXECUTED_1801'
    path.write_text(path.read_text().replace(old, old + ' ' + old if duplicate else 'JR_RUNTIME_EXECUTED_9999'))
    def change(state):
        row = next(r for r in state['rows'] if r['phase'] == 'edit' and r['replicate'] == 0 and r['stack'] == 'new')
        row['stdout_sha256'] = v.sha(path.read_bytes())
    update_json(root, 'interleaved/state.json', change)


def zero_native(root):
    name = 'journals/v24-snapshot/20261030-130140108-3164280/output.jsonl'
    path = root / name
    path.write_text(path.read_text().replace('test result: ok. 3 passed;', 'test result: ok. 0 passed;'))


def negative_status(root):
    def change(state):
        row = next(r for r in state['rows'] if r['phase'] == 'type-error' and r['stack'] == 'old')
        row['status'] = 0
    update_json(root, 'interleaved/state.json', change)


def lost_original_failure(root):
    path = root / 'journals/v24/20261030-125716550-3148215/output.jsonl'
    path.write_text(path.read_text().replace('not a member of the workspace', 'failure evidence removed'))


def patch_drift(root):
    path = root / 'source-ready/source.patch'
    data = path.read_text()
    old = '+    if bodies.is_none() {'
    assert old in data
    path.write_text(data.replace(old, '+    if bodies.is_some() {', 1))


def copy_drift(root):
    def change(state):
        state['files_after']['crates/ide-diagnostics/src/lib.rs'] = '0' * 64
    update_json(root, 'native/v24-snapshot/source-copy-after.json', change)


def fake_math(root):
    def change(state):
        state['medians_seconds']['new'] /= 2
    update_json(root, 'interleaved-reverse/state.json', change)


def missing_recovery(root):
    def change(state):
        state['rows'] = [r for r in state['rows'] if not (r['phase'] == 'assert-recovery' and r['stack'] == 'new')]
    update_json(root, 'interleaved-reverse/state.json', change)


def wrong_order(root):
    def change(state):
        i = next(i for i, r in enumerate(state['rows']) if r['phase'] == 'edit')
        state['rows'][i], state['rows'][i + 1] = state['rows'][i + 1], state['rows'][i]
    update_json(root, 'interleaved/state.json', change)


def lock_drift(root):
    path = root / 'snapshot/Cargo.lock.after'
    path.write_text(path.read_text() + '\n# mutated lock\n')


def main(scratch):
    report = dict(scope='offline relocation and semantic mutation controls only, no native execution',
                  original=v.validate(HERE), controls=[])
    with tempfile.TemporaryDirectory(prefix='selective-v24-audit-', dir=scratch) as temp:
        relocated = Path(temp) / 'relocated'
        shutil.copytree(HERE, relocated, ignore=shutil.ignore_patterns('__pycache__'))
        report['relocation'] = v.validate(relocated)['verdict']
        # Forbid subprocess execution entirely in these validator controls.
        sys.addaudithook(lambda event, args: (_ for _ in ()).throw(RuntimeError('native execution forbidden'))
                         if event in ('subprocess.Popen', 'os.system', 'os.posix_spawn') else None)
        controls = [
            ('wrong-exact-marker-resigned', lambda root: marker(root)),
            ('duplicate-exact-marker-resigned', lambda root: marker(root, duplicate=True)),
            ('zero-native-api-tests-resigned', zero_native),
            ('negative-control-status-resigned', negative_status),
            ('original-workspace-failure-erased-resigned', lost_original_failure),
            ('source-patch-reconstruction-resigned', patch_drift),
            ('private-copy-source-drift-resigned', copy_drift),
            ('summary-math-corruption-resigned', fake_math),
            ('missing-recovery-row-resigned', missing_recovery),
            ('permutation-order-corruption-resigned', wrong_order),
            ('private-lock-corruption-resigned', lock_drift),
        ]
        for name, mutate in controls:
            # Mutations are confined to a fresh throwaway evidence copy.
            target = Path(temp) / name
            shutil.copytree(relocated, target)
            mutate(target)
            resign(target)
            try:
                v.validate(target)
            except (ValueError, KeyError, OSError, TypeError) as error:
                report['controls'].append(dict(name=name, rejected=True, reason=str(error)))
            else:
                raise AssertionError('mutation incorrectly accepted: ' + name)
            shutil.rmtree(target)
    report['verdict'] = 'passed'
    return report


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--scratch', type=Path, default=os.environ.get('JCODE_SCRATCH_DIR'))
    args = parser.parse_args()
    if args.scratch is None:
        parser.error('set JCODE_SCRATCH_DIR or pass --scratch for throwaway evidence copies')
    args.scratch.mkdir(parents=True, exist_ok=True)
    print(json.dumps(main(args.scratch), indent=2))
