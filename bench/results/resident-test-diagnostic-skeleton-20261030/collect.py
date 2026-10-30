#!/usr/bin/env python3
"""Retain frozen root-owned evidence only. No builds, engines or subprocesses."""
from pathlib import Path

OUT = Path(__file__).resolve().parent
ROOT = Path('/home/jeremy/.jcode/scratch/blaze-native-engine-20261030')
SCRATCH = ROOT.parent


def copy(source, relative):
    assert source.is_file() and not source.is_symlink(), source
    target = OUT / relative
    target.parent.mkdir(parents=True, exist_ok=True)
    payload = source.read_bytes()
    if target.exists():
        assert target.read_bytes() == payload, f'refusing changed frozen payload: {target}'
    else:
        target.write_bytes(payload)


def flat(source, relative):
    for path in sorted(source.iterdir()):
        if path.is_file():
            copy(path, Path(relative) / path.name)


def tree(source, relative, names=None):
    for path in sorted(source.rglob('*')):
        if path.is_file() and '__pycache__' not in path.parts and (names is None or path.name in names):
            copy(path, Path(relative) / path.relative_to(source))


for version in ('v27', 'v28'):
    tree(ROOT / f'test-diagnostic-skeleton-{version}', f'source-ready/{version}')
    for category in ('build', 'unit'):
        source = ROOT / f'candidate-{category}-{version}'
        relative = f'{category}/{version}'
        flat(source, relative)
        tree(source / 'justrust-home/runs', f'{relative}/raw-runs', {'meta.json', 'summary.json', 'output.jsonl'})
        if category == 'build':
            tree(source / 'frozen-source', f'{relative}/frozen-source')
for session in ('rsb18', 'rsa24', 'rsa25', 'rsb19'):
    flat(Path('/home/jeremy') / session, f'suites/{session}')
for suffix, relative in (
    ('resident-interleaved-v28-20261030', 'warm/forward'),
    ('resident-interleaved-v28-reverse-20261030', 'warm/reverse'),
    ('resident-test-skeleton-v28-profile-20261030', 'profile'),
):
    flat(SCRATCH / suffix, relative)
for name in (
    'candidate-admission-v28.json', 'bench-v28-regression.log',
    'build-test-candidate-v27.sh', 'build-test-candidate-v28.sh',
    'build-test-candidate-v27.launch.log', 'build-test-candidate-v28.launch.log',
    'suite-v27-baseline.log', 'suite-v27-baseline.exit',
    'suite-v28-candidate.log', 'suite-v28-candidate.exit', 'suite-v28-compare.log',
    'suite-v28-repeat-compare.log', 'suite-v28-repeat.exit',
    'suite-v28-repeat-baseline.log', 'suite-v28-repeat-baseline.exit',
    'suite-v28-repeat-candidate.log', 'suite-v28-repeat-candidate.exit',
    'queue-runtime-v28.py', 'interleaved-v28.py', 'interleaved-v28-reverse.py',
    'profile-test-skeleton-v28.py', 'run-suite-v28-repeat.sh',
):
    source = ROOT / name
    if source.exists():
        copy(source, Path('root') / name)
for name in ('engine-identity.sha256', 'engine-version.txt'):
    copy(ROOT / 'candidate-build-v26' / name, Path('root/old-engine') / name)
print('Frozen payloads retained. Generate SHA256SUMS after writing validator/docs.')
