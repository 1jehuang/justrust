#!/usr/bin/env python3
"""Offline receipt collection only. Does not execute retained workload runners."""
import hashlib
import json
import shutil
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = Path('/home/jeremy/.jcode/scratch/blaze-native-engine-20261030')
PROFILE = Path('/home/jeremy/.jcode/scratch/resident-semantic-subphases-v25-profile-20261030')
FILES = []


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def copy(source, target):
    source, target = Path(source), HERE / target
    assert source.is_file() and not source.is_symlink(), source
    target.parent.mkdir(parents=True, exist_ok=True)
    if target.exists():
        assert digest(source) == digest(target), target
    else:
        shutil.copyfile(source, target)
    assert digest(source) == digest(target)
    FILES.append(dict(origin=str(source), artifact=str(target.relative_to(HERE)), sha256=digest(target)))


def flat(source, target):
    for path in sorted(source.iterdir()):
        if path.is_file() and not path.is_symlink():
            copy(path, Path(target) / path.name)


def tree(source, target):
    for path in sorted(source.rglob('*')):
        if path.is_file() and not path.is_symlink():
            copy(path, Path(target) / path.relative_to(source))


def journals(source, target):
    for run in sorted(source.iterdir()):
        if run.is_dir():
            flat(run, Path(target) / run.name)


# Frozen pre-validation source handoff, never recaptured from live production source.
tree(ROOT / 'semantic-subphases-v25', 'source-ready')
for version in ('v23', 'v25'):
    build = ROOT / f'candidate-build-{version}'
    flat(build, f'build/{version}')
    tree(build / 'frozen-source', f'build/{version}/frozen-source')
    journals(build / 'justrust-home/runs', f'journals/build-{version}')
for version in ('v25', 'v25-snapshot'):
    unit = ROOT / f'candidate-unit-{version}'
    flat(unit, f'native/{version}')
    journals(unit / 'justrust-home/runs', f'journals/{version}')
copy(Path('bench/results/resident-selective-traversal-20261030/snapshot/Cargo.lock.before'), 'snapshot/Cargo.lock.original')
copy(ROOT / 'candidate-unit-v25-snapshot/vendor/rust-analyzer/Cargo.lock', 'snapshot/Cargo.lock.private')
copy(ROOT / 'candidate-unit-v25-snapshot/vendor/rust-analyzer/Cargo.toml', 'snapshot/Cargo.toml')
for name in ('rsb16', 'rsa22'):
    flat(Path('/home/jeremy') / name, f'fixed-five/{name}')
flat(PROFILE, 'profile')
for name in (
    'build-test-candidate-v25.sh', 'build-test-candidate-v25.launch.log',
    'queue-unit-v25.sh', 'queue-unit-v25.log', 'queue-runtime-v25.sh', 'queue-runtime-v25.log',
    'prepare-unit-v25-snapshot.py', 'native-unit-v25-snapshot.sh',
    'verify-native-unit-v25-snapshot.py', 'native-unit-v25-snapshot-admission.log',
    'run-suite-v25-snapshot.sh', 'suite-v25-snapshot-baseline.log',
    'suite-v25-snapshot-baseline.exit', 'suite-v25-snapshot-candidate.log',
    'suite-v25-snapshot-candidate.exit', 'suite-v25-snapshot-compare.log',
    'suite-v25-snapshot.exit', 'finish-v25-snapshot.sh', 'finish-v25-snapshot.log',
    'final-v25-snapshot.exit', 'profile-semantic-subphases-v25.py',
    'profile-semantic-subphases-v25.log', 'bench-regression-v25.log', 'bench-regression-v25.exit',
):
    copy(ROOT / name, Path('provenance') / name)
for name in ('suite.py', 'resident_engine_compare.py', 'blaze_runtime_compare.py', 'blaze_edit_compare.py'):
    copy(Path('bench') / name, Path('harness') / name)
export = []
for path in sorted((HERE / 'journals').glob('*/*/summary.json')):
    summary = json.loads(path.read_text())
    export.append(dict(id=summary['id'], source=str(path.relative_to(HERE)),
                       summary_sha256=digest(path), args=summary['args'],
                       exit=summary['exit'], tests=summary.get('tests'),
                       method='offline projection of retained raw summary, not a justrust invocation'))
(HERE / 'offline-export.json').write_text(json.dumps(export, indent=2) + '\n')
(HERE / 'collection.json').write_text(json.dumps(dict(
    source_commit='90164925c', scope='Frozen sources and offline actual native receipts only',
    excludes=['full2444 source snapshot', 'targets', 'binaries', 'caches'],
    files=FILES,
), indent=2) + '\n')
print(json.dumps(dict(copied=len(FILES), bytes=sum((HERE / x['artifact']).stat().st_size for x in FILES))))
