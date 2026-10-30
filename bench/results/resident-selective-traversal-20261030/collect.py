#!/usr/bin/env python3
"""One-shot offline byte-identical collection, no builds or workloads."""
import hashlib
import json
import shutil
from pathlib import Path

HERE = Path(__file__).resolve().parent
SCRATCH = Path('/home/jeremy/.jcode/scratch')
ROOT = SCRATCH / 'blaze-native-engine-20261030'
FILES = []


def copy(source, target):
    source, target = Path(source), HERE / target
    assert source.is_file() and not source.is_symlink(), source
    assert not target.exists(), target
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(source, target)
    sha = hashlib.sha256(source.read_bytes()).hexdigest()
    assert hashlib.sha256(target.read_bytes()).hexdigest() == sha
    FILES.append(dict(origin=str(source), artifact=str(target.relative_to(HERE)), sha256=sha))


def flat(source, target):
    for path in sorted(source.iterdir()):
        if path.is_file() and not path.is_symlink():
            copy(path, Path(target) / path.name)


def tree(source, target):
    for path in sorted(source.rglob('*')):
        if path.is_file() and not path.is_symlink():
            copy(path, Path(target) / path.relative_to(source))


# Exact rejected sources and baseline, including the original pending-gates documentation.
tree(ROOT / 'selective-traversal-v24', 'source-ready')
for version in ('v23', 'v24'):
    build = ROOT / f'candidate-build-{version}'
    flat(build, f'build/{version}')
    tree(build / 'frozen-source', f'build/{version}/frozen-source')
    home = build / 'justrust-home/runs'
    for run in sorted(home.iterdir()):
        if run.is_dir():
            for name in ('meta.json', 'summary.json', 'output.jsonl'):
                if (run / name).exists():
                    copy(run / name, f'journals/build-{version}/{run.name}/{name}')
for name in ('v24', 'v24-nested', 'v24-private-lock', 'v24-snapshot'):
    unit = ROOT / f'candidate-unit-{name}'
    flat(unit, f'native/{name}')
    home = unit / 'justrust-home/runs'
    if home.exists():
        for run in sorted(home.iterdir()):
            if run.is_dir():
                for item in ('meta.json', 'summary.json', 'output.jsonl'):
                    if (run / item).exists():
                        copy(run / item, f'journals/{name}/{run.name}/{item}')
copy(ROOT / 'candidate-unit-v24-private-lock/Cargo.lock', 'snapshot/Cargo.lock.before')
copy(ROOT / 'candidate-unit-v24-snapshot/vendor/rust-analyzer/Cargo.lock', 'snapshot/Cargo.lock.after')
copy(ROOT / 'candidate-unit-v24-snapshot/vendor/rust-analyzer/Cargo.toml', 'snapshot/Cargo.toml')
for name in ('rsb15', 'rsa21'):
    flat(Path('/home/jeremy') / name, f'fixed-five/{name}')
for name, folder in (
    ('interleaved', 'resident-interleaved-v24-20261030'),
    ('interleaved-reverse', 'resident-interleaved-v24-reverse-20261030'),
):
    flat(SCRATCH / folder, name)
for name in (
    'build-test-candidate-v24.sh', 'verify-native-unit-v24.py',
    'run-suite-v24.sh', 'finish-v24.sh', 'final-v24.log', 'final-v24.exit',
    'native-unit-v24-nested.sh', 'native-unit-v24-nested.log',
    'native-unit-v24-snapshot.sh', 'native-unit-v24-snapshot.log',
    'verify-native-unit-v24-snapshot.py', 'run-suite-v24-snapshot.sh',
    'finish-v24-snapshot.sh', 'final-v24-snapshot.log', 'final-v24-snapshot.exit',
    'native-unit-v24-snapshot-admission.log', 'suite-v24-snapshot-baseline.log',
    'suite-v24-snapshot-baseline.exit', 'suite-v24-snapshot-candidate.log',
    'suite-v24-snapshot-candidate.exit', 'suite-v24-snapshot-compare.log',
    'suite-v24-snapshot.exit', 'interleaved-v24.py', 'interleaved-v24-reverse.py',
):
    copy(ROOT / name, f'provenance/{name}')
for name in ('resident_engine_compare.py', 'blaze_runtime_compare.py', 'blaze_edit_compare.py', 'suite.py'):
    copy(HERE.parents[1] / name, f'harness/{name}')
(HERE / 'collection.json').write_text(json.dumps(dict(
    baseline_commit='8a04340343a67e0152b8c5b4d7d52d18dd8cf13a',
    decision='reject, latency benefit not independently replicated; production restored',
    scope='Offline byte-identical copies, selected source snapshots and raw evidence, no binaries or caches',
    files=FILES,
), indent=2) + '\n')
print(f'Collected {len(FILES)} byte-verified artifacts')
