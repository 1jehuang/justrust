#!/usr/bin/env python3
"""One-shot archival capture, not a build or benchmark. Historical paths are inputs."""
import hashlib
import json
from pathlib import Path
import shutil
import subprocess

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parents[2]
SCRATCH = Path('/home/jeremy/.jcode/scratch/blaze-native-engine-20261030')


def capture(source, target):
    target = ROOT / target
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(source, target)


def main():
    if (ROOT / 'manifest.json').exists():
        raise SystemExit('Immutable capture already exists. Do not recapture after source restoration.')
    for name in ['rsb02', 'rsa07', 'rsa08', 'rsb03', 'rsa09']:
        for source in (Path('/home/jeremy') / name).iterdir():
            if source.is_file():
                capture(source, Path('sessions') / name / source.name)
    for version in [9, 10, 11, 12]:
        for kind in ['build', 'unit']:
            folder = SCRATCH / f'candidate-{kind}-v{version}'
            for source in folder.rglob('*'):
                relative = source.relative_to(folder)
                if source.is_file() and ('justrust-home' not in relative.parts or
                        ('runs' in relative.parts and source.name in
                         ['meta.json', 'output.jsonl', 'summary.json', 'units.jsonl'])):
                    capture(source, Path('native') / folder.name / relative)
        for name in [f'build-test-candidate-v{version}.sh',
                     f'root-classification-source-ready-v{version}.txt']:
            capture(SCRATCH / name, Path('provenance') / name)
    for folder in ['candidate-build-v4']:
        for source in (SCRATCH / folder).iterdir():
            if source.is_file():
                capture(source, Path('native') / folder / source.name)
    for name in [
        'build-test-candidate-v4.sh', 'run-suite-after-v5.sh',
        'run-suite-after-v11.sh', 'run-suite-after-v12.sh', 'full-native-v12.sh',
        'repeat-native-primitive-v12.sh', 'suite-after-v11.log', 'suite-after-v11.exit',
        'suite-after-v12.log', 'suite-after-v12.exit',
        'suite-v4-v11-compare.log', 'suite-v4-v12-compare.log',
        'suite-v12-repeat-before.log', 'suite-v12-repeat-before.exit',
        'suite-v12-repeat-after.log', 'suite-v12-repeat-after.exit',
        'suite-v12-repeat-compare.log', 'suite-v12-repeat.exit',
    ]:
        capture(SCRATCH / name, Path('provenance') / name)
    for name in ['main.rs', 'hir-ty-codegen-mod.rs', 'resident-root-classification.md']:
        capture(SCRATCH / 'root-classification-source-v12' / name, Path('candidate-v12') / name)
    paths = ['toolchain/resident/crates/ra-jit/src/main.rs',
             'toolchain/resident/crates/hir-ty-codegen/mod.rs']
    patch = subprocess.check_output(['git', 'diff', '--binary', 'HEAD', '--', *paths], cwd=REPO)
    (ROOT / 'candidate-v12' / 'candidate.patch').write_bytes(patch)
    for path, name in zip(paths, ['main.rs', 'hir-ty-codegen-mod.rs']):
        data = subprocess.check_output(['git', 'show', f'HEAD:{path}'], cwd=REPO)
        target = ROOT / 'baseline-head' / name
        target.parent.mkdir(exist_ok=True)
        target.write_bytes(data)
    for name in ['suite.py', 'resident_suite.py', 'resident_engine_compare.py']:
        capture(REPO / 'bench' / name, Path('harness-captured') / name)
    for name in ['LICENSE-APACHE', 'LICENSE-MIT']:
        capture(REPO / 'toolchain/resident' / name, Path('candidate-v12') / name)
    provenance = {
        'schema': 'resident_root_classification_evidence/v1',
        'decision': 'REJECTED', 'promotion': False,
        'scope': 'Licensed Blaze-derived engine, not clean-room. Descriptive scoped timings, no general speedup claim.',
        'capture_head': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=REPO, text=True).strip(),
        'older_full_source': {'v9': 'unknown/not captured', 'v10': 'unknown/not captured', 'v11': 'unknown/not captured'},
        'source_reconstruction_claim': False,
        'final_pair': ['rsb03', 'rsa09'],
        'sessions': ['rsb02', 'rsa07', 'rsa08', 'rsb03', 'rsa09'],
        'source_original_paths': paths,
        'capture_notes': 'Copied original top-level session artifacts byte-for-byte, excluding private cache/build directories. Native run output/meta/summary retained, install IDs and telemetry delivery state excluded. Harness source captured at evidence time, not asserted to be a historical snapshot. No older source reconstruction. Machine paths in original receipts are historical, not needed by validator.',
    }
    (ROOT / 'provenance.json').write_text(json.dumps(provenance, indent=2) + '\n')
    files = {}
    for path in sorted(ROOT.rglob('*')):
        if path.is_file() and path.name not in ['manifest.json', 'collect.py', 'validate.py', 'test_validate.py', 'README.md', 'audit.json'] and '__pycache__' not in path.parts:
            files[str(path.relative_to(ROOT))] = hashlib.sha256(path.read_bytes()).hexdigest()
    (ROOT / 'manifest.json').write_text(json.dumps({'sha256': files}, indent=2) + '\n')
    print(f'Captured {len(files)} immutable files')


if __name__ == '__main__':
    main()
