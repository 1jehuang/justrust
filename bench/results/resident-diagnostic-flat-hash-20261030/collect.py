#!/usr/bin/env python3
"""Offline collection only. Never invokes a compiler, daemon, or workload."""
import hashlib
import json
import shutil
from pathlib import Path

HERE = Path(__file__).resolve().parent
SCRATCH = Path('/home/jeremy/.jcode/scratch')
ROOT = SCRATCH / 'blaze-native-engine-20261030'
FILES = []


def copy(source, target):
    source = Path(source)
    target = HERE / target
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


for version in ('v22', 'v23'):
    build = ROOT / f'candidate-build-{version}'
    flat(build, f'build/{version}')
    for path in sorted((build / 'frozen-source').rglob('*')):
        if path.is_file():
            copy(path, Path(f'build/{version}/frozen-source') / path.relative_to(build / 'frozen-source'))
for name in ('v23', 'v22-restored'):
    unit = ROOT / f'candidate-unit-{name}'
    flat(unit, f'native/{name}')
    for run in sorted((unit / 'justrust-home/runs').iterdir()):
        if run.is_dir():
            for name2 in ('meta.json', 'summary.json', 'output.jsonl'):
                if (run / name2).exists():
                    copy(run / name2, f'journals/{name}/{run.name}/{name2}')
for name in ('rsb14', 'rsa20'):
    flat(Path('/home/jeremy') / name, f'fixed-five/{name}')
for name, folder in (
    ('interleaved', 'resident-interleaved-v23-20261030'),
    ('interleaved-reverse', 'resident-interleaved-v23-reverse-20261030'),
    ('profile-failed', 'resident-flat-hash-profile-v23-20261030'),
    ('profile-calibrated', 'resident-flat-hash-profile-v23-calibrated-20261030'),
):
    flat(SCRATCH / folder, name)
for name in (
    'interleaved-v23.py', 'interleaved-v23-reverse.py',
    'profile-flat-hash-v23.py', 'profile-flat-hash-v23-calibrated.py',
    'build-test-candidate-v22.sh', 'build-test-candidate-v23.sh',
    'full-native-v22-restored.sh', 'finish-v23.sh', 'run-suite-v23.sh',
    'verify-native-unit-v23.py', 'bench-regression-v23.log', 'bench-regression-v23.exit',
    'suite-v23-baseline.log', 'suite-v23-baseline.exit', 'suite-v23-candidate.log',
    'suite-v23-candidate.exit', 'suite-v23-compare.log', 'suite-v23.exit',
    'final-v23.log', 'final-v23.exit',
):
    copy(ROOT / name, f'provenance/{name}')
flat(ROOT / 'diagnostic-flat-hash-v23', 'source-ready')
for name in ('resident_engine_compare.py', 'blaze_runtime_compare.py', 'blaze_edit_compare.py'):
    copy(HERE.parents[1] / name, f'harness/{name}')
(HERE / 'collection.json').write_text(json.dumps(dict(
    source_commit='bea4cc1dc29dbe65b1c2073c92e43f79e4d735a2',
    scope='Offline byte-identical evidence collection, selected frozen source snapshots, no native reruns',
    files=FILES,
), indent=2) + '\n')
print(f'Collected {len(FILES)} byte-verified artifacts')
