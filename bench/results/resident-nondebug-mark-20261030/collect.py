#!/usr/bin/env python3
"""One-shot light capture of frozen root evidence. No subprocess/native execution."""
from pathlib import Path

OUT = Path(__file__).resolve().parent
ROOT = Path('/home/jeremy/.jcode/scratch/blaze-native-engine-20261030')


def copy(source, relative):
    assert source.is_file() and not source.is_symlink(), source
    target = OUT / relative
    target.parent.mkdir(parents=True, exist_ok=True)
    data = source.read_bytes()
    if target.exists():
        assert target.read_bytes() == data, f'refusing changed frozen bytes: {target}'
    else:
        target.write_bytes(data)


def flat(source, relative):
    for path in sorted(source.iterdir()):
        if path.is_file():
            copy(path, Path(relative) / path.name)


def tree(source, relative):
    for path in sorted(source.rglob('*')):
        if path.is_file() and '__pycache__' not in path.parts:
            copy(path, Path(relative) / path.relative_to(source))


def capture():
    assert not (OUT / 'manifest.json').exists(), 'sealed bundle must not be recaptured'
    tree(ROOT / 'nondebug-mark-v29', 'source-ready/v29')
    tree(ROOT / 'nondebug-mark-v29b', 'source-ready/v29b')
    for category in ('build', 'unit'):
        source = ROOT / f'candidate-{category}-v29b'
        relative = f'native/{category}'
        flat(source, relative)
        for run in sorted((source / 'justrust-home/runs').iterdir()):
            if run.is_dir():
                for name in ('meta.json', 'summary.json', 'output.jsonl'):
                    copy(run / name, f'{relative}/raw-runs/{run.name}/{name}')
        if category == 'build':
            tree(source / 'frozen-source', f'{relative}/frozen-source')
    for session in ('rsb20', 'rsa26'):
        flat(Path('/home/jeremy') / session, f'sessions/{session}')
    for name in ('build-test-candidate-v29b.sh', 'suite-v29b-compare.log'):
        copy(ROOT / name, f'provenance/{name}')
    for name in ('engine-version.txt', 'engine-identity.sha256'):
        copy(ROOT / 'candidate-build-v28' / name, f'provenance/baseline-v28/{name}')
    for name in ('rejected-v29b-Cargo.lock', 'rejected-v29b-lock.patch'):
        copy(ROOT / 'cold-diagnostics-v30' / name, f'provenance/{name}')
    for name in ('LICENSE-APACHE', 'LICENSE-MIT', 'JUSTRUST-SOURCE.md'):
        copy(Path('/home/jeremy/justrust/toolchain/resident') / name, f'licenses/{name}')
    for name in ('suite.py', 'resident_suite.py'):
        source = Path('/home/jeremy/justrust/bench') / name
        if source.exists():
            copy(source, f'harness-captured/{name}')
    print('Frozen evidence copied. No native workload or audit controls executed.')


if __name__ == '__main__':
    capture()
