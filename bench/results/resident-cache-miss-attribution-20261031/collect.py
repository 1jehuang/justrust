#!/usr/bin/env python3
"""Retain pre-existing v42/v43 receipts only. No native execution or live Rust reads."""
import hashlib
import json
from pathlib import Path
import shutil

HERE = Path(__file__).resolve().parent
ROOT = Path('/home/jeremy/.jcode/scratch/blaze-native-engine-20261030')
OLD = HERE.parent / 'resident-declarations-cache-20261031'


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def copy(source, target):
    assert source.is_file() and not source.is_symlink(), source
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(source, target)


def tree(source, target):
    for p in sorted(source.rglob('*')):
        if p.is_file():
            copy(p, target / p.relative_to(source))


def emit(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + '\n')


def main():
    gates = []
    for version in (42, 43):
        tree(ROOT / f'cache-miss-attribution-v{version}', HERE / f'frozen/v{version}')
        copy(ROOT / f'build-test-candidate-v{version}.sh', HERE / f'provenance/build-test-candidate-v{version}.sh')
        for kind in ('build', 'unit'):
            group = f'candidate-{kind}-v{version}'
            for p in sorted((ROOT / group).iterdir()):
                if p.is_file():
                    copy(p, HERE / 'gates' / group / p.name)
            for run in sorted((ROOT / group / 'justrust-home/runs').glob('*')):
                if not run.is_dir():
                    continue
                for name in ('meta.json', 'output.jsonl', 'summary.json'):
                    copy(run / name, HERE / 'journals' / run.name / name)
                s = json.loads((run / 'summary.json').read_text())
                slim = {key: s[key] for key in ('id','args','subcommand','start','wall','exit','tests','route')}
                slim['scope'] = 'Derived path-free native receipt, joined to retained raw journal.'
                emit(HERE / 'slim' / (run.name + '.json'), slim)
                receipt = ('codegen-native' if s['tests']['passed'] in (50,53) else 'full-native') if kind == 'unit' else 'check' if s['subcommand'] == 'check' else 'compile'
                gates.append(dict(group=group, run=run.name, receipt=receipt, exit=s['exit'], tests=s['tests']))
    for mode in ('on', 'off'):
        tree(ROOT / f'cache-miss-attribution-v43-{mode}', HERE / 'runtime' / mode)
    for name in ('probe-cache-miss-attribution-v42.py','validate-cache-miss-v43.py'):
        copy(ROOT / name, HERE / 'provenance' / name)
    copy(ROOT / 'cache-miss-v43-validation.json', HERE / 'root-validation.json')
    copy(ROOT / 'v43-bench-regression.log', HERE / 'gates/v43-bench-regression.log')
    for name in ('resident_engine_compare.py','resident_state_matrix.py','resident_workload_compare.py','blaze_runtime_compare.py','blaze_edit_compare.py'):
        copy(OLD / 'provenance' / name, HERE / 'provenance' / name)
    tree(OLD / 'input/source63', HERE / 'input/source63')
    tree(OLD / 'licenses', HERE / 'licenses')
    state = json.loads((HERE / 'runtime/on/state.json').read_text())
    native = {}
    for v in (42,43):
        base = HERE / f'gates/candidate-build-v{v}'
        native[str(v)] = dict(engine_sha=(base/'engine-identity.sha256').read_text().split()[0], version=(base/'engine-version.txt').read_text().strip())
    frozen = {str(p.relative_to(HERE)):sha(p) for p in sorted((HERE/'frozen').rglob('*')) if p.is_file()}
    helpers = {str(p.relative_to(HERE)):sha(p) for p in sorted((HERE/'provenance').glob('*')) if p.is_file()}
    emit(HERE/'commitments.json',dict(frozen=frozen, helpers=helpers,native=native,gates=gates,observations=json.loads((HERE/'root-validation.json').read_text())['modes'],source_before=state['source_before'],toolchain_sha256=hashlib.sha256(json.dumps(state['toolchain_before'],sort_keys=True,separators=(',',':')).encode()).hexdigest(),source_commit='cb9d8a010',scope='Native diagnostic observations, not performance or unique compiled-function losses'))
    emit(HERE/'provenance/retention-receipt.json',dict(source='Existing frozen source packages, original raw journals/state snapshots and prior pinned source/helper/license package only',live_rust_reads=0,native_calls=0,sessions=2,state_rows=14,gate_runs=len(gates),excluded=['engine binaries','targets','private caches','telemetry homes','ambient environment recapture']))
    print(json.dumps(dict(gate_runs=len(gates),state_rows=14,frozen_files=len(frozen))))


if __name__ == '__main__':
    main()
