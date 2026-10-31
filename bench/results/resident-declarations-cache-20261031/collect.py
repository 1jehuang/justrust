#!/usr/bin/env python3
"""One-shot retention from existing frozen receipts only. Not a native benchmark."""
import hashlib
import json
from pathlib import Path
import shutil
import subprocess

HERE = Path(__file__).resolve().parent
ROOT = Path('/home/jeremy/.jcode/scratch/blaze-native-engine-20261030')
OLD = HERE.parent / 'resident-demand-inventory-20261031'
REPO = Path('/home/jeremy/justrust')


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def copy(source, target):
    assert source.is_file() and not source.is_symlink(), source
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(source, target)


def tree(source, target):
    for path in sorted(source.rglob('*')):
        if path.is_file():
            copy(path, target / path.relative_to(source))


def emit(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + '\n')


def main():
    for version in (39, 40, 41):
        tree(ROOT / f'declarations-cache-v{version}', HERE / f'frozen/v{version}')
        copy(ROOT / f'build-test-candidate-v{version}.sh', HERE / f'provenance/build-test-candidate-v{version}.sh')
    gate_rows = []
    for group in ('candidate-build-v39', 'candidate-build-v40', 'candidate-unit-v40', 'candidate-build-v41', 'candidate-unit-v41'):
        for path in sorted((ROOT / group).iterdir()):
            if path.is_file():
                copy(path, HERE / 'gates' / group / path.name)
        for run in sorted((ROOT / group / 'justrust-home/runs').glob('*')):
            if not run.is_dir():
                continue
            for name in ('meta.json', 'output.jsonl', 'summary.json'):
                copy(run / name, HERE / 'journals' / run.name / name)
            summary = json.loads((run / 'summary.json').read_text())
            slim = {key: summary[key] for key in ('id', 'args', 'subcommand', 'start', 'wall', 'exit', 'tests', 'route') if key in summary}
            slim['scope'] = 'Derived path-free receipt. Full original journals retained, never native re-execution.'
            emit(HERE / 'slim' / (run.name + '.json'), slim)
            gate_rows.append(dict(group=group, run=run.name, exit=summary['exit'], tests=summary.get('tests')))
    sessions = ['v40-off', 'v40-on', 'v41-on', 'v41-off', 'v41-off-repeat', 'v41-on-repeat']
    for session in sessions:
        tree(ROOT / f'declarations-cache-{session}', HERE / 'runtime' / session)
    copy(ROOT / 'probe-declarations-cache-v39.py', HERE / 'provenance/probe-declarations-cache-v39.py')
    for name in ('resident_engine_compare.py', 'resident_state_matrix.py', 'resident_workload_compare.py', 'blaze_runtime_compare.py', 'blaze_edit_compare.py'):
        copy(REPO / 'bench' / name, HERE / 'provenance' / name)
    tree(OLD / 'input/source63', HERE / 'input/source63')
    tree(OLD / 'licenses', HERE / 'licenses')
    # This codec is unchanged across the experiment. Retrieve historical Git object bytes,
    # not restored live source, then bind it to the native 3,757-file receipt.
    codec = subprocess.check_output(['git', 'show', 'HEAD:toolchain/resident/vendor/rust-analyzer/crates/hir-def/src/nameres/frozen.rs'], cwd=REPO)
    (HERE / 'frozen/hir-def-frozen.rs').write_bytes(codec)
    committed = {}
    for path in sorted((HERE / 'frozen').rglob('*')):
        if path.is_file() and path.suffix == '.rs':
            committed[str(path.relative_to(HERE))] = digest(path)
    helpers = {str(path.relative_to(HERE)): digest(path) for path in sorted((HERE / 'provenance').glob('*')) if path.is_file()}
    native = {}
    for version in (40, 41):
        base = HERE / f'gates/candidate-build-v{version}'
        native[str(version)] = dict(engine_sha=(base / 'engine-identity.sha256').read_text().split()[0], version=(base / 'engine-version.txt').read_text().strip())
    state = json.loads((HERE / 'runtime/v41-on/state.json').read_text())
    tc = json.dumps(state['toolchain_before'], sort_keys=True, separators=(',', ':')).encode()
    emit(HERE / 'commitments.json', dict(frozen_sources=committed, helpers=helpers, native=native, gates=gate_rows, toolchain_sha256=hashlib.sha256(tc).hexdigest(), source_before=state['source_before'], decision='REJECT/no promotion', expected_v41_native_hits=21, runtime_rows=42))
    emit(HERE / 'provenance/retention-receipt.json', dict(scope='Existing frozen files and historical Git object only. No native jobs, binary/target/private-home copy or ambient environment recapture.', sessions=sessions, gate_runs=len(gate_rows)))
    print(json.dumps(dict(gates=len(gate_rows), sessions=len(sessions), frozen_sources=len(committed))))


if __name__ == '__main__':
    main()
