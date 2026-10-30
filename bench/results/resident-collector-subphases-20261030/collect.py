#!/usr/bin/env python3
"""Copy existing frozen v26 evidence only. Never build, test, or run an engine."""
import hashlib
import json
from pathlib import Path

OUT = Path(__file__).resolve().parent
ROOT = Path('/home/jeremy/.jcode/scratch/blaze-native-engine-20261030')
PROFILE = Path('/home/jeremy/.jcode/scratch/resident-semantic-subphases-v26-profile-20261030')


def copy(source, relative):
    target = OUT / relative
    target.parent.mkdir(parents=True, exist_ok=True)
    data = source.read_bytes()
    if target.exists():
        assert target.read_bytes() == data, f'refusing changed artifact: {target}'
    else:
        target.write_bytes(data)


def flat(source, relative):
    for path in sorted(source.iterdir()):
        if path.is_file():
            copy(path, Path(relative) / path.name)


assert (ROOT / 'final-v26-snapshot.exit').read_text().strip() == '0'
flat(PROFILE, 'profile')
for name in ('rsb17', 'rsa23'):
    flat(Path('/home/jeremy') / name, Path('disabled-runtime') / name)
flat(ROOT / 'candidate-unit-v26-snapshot', 'native')
flat(ROOT / 'candidate-build-v26', 'build')
for source, relative in (
    (ROOT / 'candidate-build-v26/frozen-source', Path('build/frozen-source')),
    (ROOT / 'candidate-unit-v26-snapshot/justrust-home/runs', Path('native/raw-runs')),
):
    for path in sorted(source.rglob('*')):
        if path.is_file() and (relative == Path('build/frozen-source') or path.name in ('output.jsonl', 'meta.json', 'summary.json')):
            copy(path, relative / path.relative_to(source))
copy(ROOT / 'candidate-unit-v26-snapshot/vendor/rust-analyzer/Cargo.lock', 'native/private-resolved.Cargo.lock')
ready = ROOT / 'collector-subphases-v26'
for path in sorted(ready.rglob('*')):
    if path.is_file():
        copy(path, Path('source-ready') / path.relative_to(ready))
for name in (
    'final-v26-snapshot.exit', 'suite-v26-snapshot.exit',
    'suite-v26-snapshot-baseline.exit', 'suite-v26-snapshot-candidate.exit',
    'suite-v26-snapshot-baseline.log', 'suite-v26-snapshot-candidate.log',
    'suite-v26-snapshot-compare.log', 'native-unit-v26-snapshot-admission.log',
    'profile-semantic-subphases-v26.log', 'v26-final-offline-regression.log',
    'build-test-candidate-v26.sh', 'native-unit-v26-snapshot.sh',
    'verify-native-unit-v26-snapshot.py', 'prepare-unit-v26-snapshot.py',
    'run-suite-v26-snapshot.sh', 'finish-v26-snapshot.sh',
    'profile-semantic-subphases-v26.py',
):
    copy(ROOT / name, Path('root') / name)

state = json.loads((OUT / 'profile/state.json').read_text())
groups = []
for row in state['rows']:
    # Fixed inventory emission order starts each completed collection with module.
    # Preserve multiple collections in one worker delta instead of summing labels.
    row_groups = []
    for counter in row['collector_inventory']:
        if counter['label'] == 'hir.collect.module_inclusive':
            row_groups.append([])
        assert row_groups
        row_groups[-1].append(counter)
    groups.append({'replicate': row['replicate'], 'collections': row_groups,
                   'callbacks': row['callbacks'], 'transport': row['transport']})
summary = {
    'scope': 'Bounded actual collector inventory, inclusive and non-additive. No speed claim.',
    'status': state['status'], 'unchanged': state['unchanged'],
    'rows': groups,
    'interpretation': 'Collection groups preserve emitter order only. No inferred trace-thread association or critical path.',
}
(OUT / 'inventory-groups.json').write_text(json.dumps(summary, indent=2) + '\n')
manifest = {}
for path in sorted(OUT.rglob('*')):
    if path.is_file() and path.name not in ('SHA256SUMS', 'validation.json') and '__pycache__' not in path.parts:
        manifest[str(path.relative_to(OUT))] = hashlib.sha256(path.read_bytes()).hexdigest()
(OUT / 'SHA256SUMS').write_text(''.join(f'{digest}  {path}\n' for path, digest in manifest.items()))
print(json.dumps({'copied_files': len(manifest), 'profile_rows': len(groups), 'status': 'collected'}))
