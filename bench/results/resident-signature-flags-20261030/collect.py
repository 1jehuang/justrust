#!/usr/bin/env python3
"""One-shot copy of historical receipts. No builds, workloads, or live source edits."""
import hashlib
import json
from pathlib import Path
import shutil
import subprocess

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parents[2]
SCRATCH = Path('/home/jeremy/.jcode/scratch/blaze-native-engine-20261030')
RECEIPTS = {
    13: Path('/home/jeremy/.jcode/scratch/resident-signature-flags-20261030-Bx60Ea'),
    14: Path('/home/jeremy/.jcode/scratch/resident-signature-flags-v14-20261030'),
    15: Path('/home/jeremy/.jcode/scratch/resident-signature-flags-v15-20261030'),
}
EXPORTS = Path('/home/jeremy/.jcode/scratch/signature-flags-exports-20261030')
BASE = '8905ce5931b352545ec4a18a3bbb8e6f13bd1b63'
CHANGED = ['toolchain/resident/vendor/rust-analyzer/crates/hir-def/src/signatures.rs',
           'toolchain/resident/crates/hir-ty-codegen/mod.rs',
           'toolchain/resident/crates/ra-jit/src/main.rs']


def capture(source, target):
    target = ROOT / target
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(source, target)


def main():
    if (ROOT / 'manifest.json').exists():
        raise SystemExit('Capture is immutable. Do not recapture live sources.')
    for session in ['rsb04', 'rsa10']:
        for path in (Path('/home/jeremy') / session).iterdir():
            if path.is_file():
                capture(path, Path('sessions') / session / path.name)
    for version in [13, 14, 15]:
        for kind in ['build', 'unit']:
            folder = SCRATCH / f'candidate-{kind}-v{version}'
            for path in folder.rglob('*'):
                rel = path.relative_to(folder)
                if path.is_file() and ('justrust-home' not in rel.parts or
                        ('runs' in rel.parts and path.name in
                         ['meta.json', 'output.jsonl', 'summary.json', 'units.jsonl'])):
                    capture(path, Path('native') / folder.name / rel)
        for path in RECEIPTS[version].rglob('*'):
            if path.is_file():
                capture(path, Path('source-ready') / f'v{version}' / path.relative_to(RECEIPTS[version]))
        capture(SCRATCH / f'build-test-candidate-v{version}.sh', Path('provenance') / f'build-test-candidate-v{version}.sh')
    for path in EXPORTS.rglob('*.jsonl'):
        capture(path, Path('exports') / path.relative_to(EXPORTS))
    for name in ['source-before.sha256', 'source-after.log', 'engine-identity.sha256', 'engine-version.txt', 'exit']:
        capture(SCRATCH / 'candidate-build-v4' / name, Path('native/baseline-v4') / name)
    for name in ['run-suite-v13.sh', 'run-suite-v14.sh', 'run-suite-v15.sh',
                 'suite-v14.exit', 'suite-v15.exit', 'suite-v15-compare.log',
                 'suite-v15-baseline.log', 'suite-v15-baseline.exit',
                 'suite-v15-candidate.log', 'suite-v15-candidate.exit',
                 'verify-native-unit-v14.py', 'verify-native-unit-v15.py',
                 'native-unit-v15-admission.log', 'full-native-v15.sh']:
        capture(SCRATCH / name, Path('provenance') / name)
    for name in ['suite.py', 'resident_suite.py', 'resident_engine_compare.py']:
        capture(REPO / 'bench' / name, Path('harness-captured') / name)
    for name in ['LICENSE-APACHE', 'LICENSE-MIT']:
        capture(REPO / 'toolchain/resident' / name, Path('source-ready') / name)
    for path in CHANGED:
        target = ROOT / 'baseline-head' / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(subprocess.check_output(['git', 'show', f'{BASE}:{path}'], cwd=REPO))
    provenance = {
        'schema': 'resident_signature_flags_evidence/v1',
        'decision': 'NOT_A_PERFORMANCE_WIN', 'promotion': False,
        'baseline_revision': BASE, 'final_pair': ['rsb04', 'rsa10'],
        'scope': 'Native licensed Blaze-derived resident, not clean-room. Scoped descriptive timing only.',
        'source_capture': 'Historical worker exact-byte SOURCE READY receipts v13/v14/v15 plus original root before-build frozen sources, copied without reconstruction. Current live sources intentionally untouched.',
        'source_ready_message_files': 'No original DM text files were created. Original per-version SHA256SUMS and complete available source bytes/source.patch are the ready receipts, not invented messages.',
        'older_source_claim': 'Only v13-v15 genuinely captured files are present. No earlier full sources reconstructed.',
        'raw_policy': 'Original top-level runtime files and native meta/output/summary/units retained byte-for-byte. No binaries, cache trees, target trees, install IDs, or telemetry state.',
        'harness_policy': 'Harness copied at evidence capture time, not asserted to be an original historical harness snapshot.',
        'working_source_policy': 'No source restoration or source commit. Root continues independent coarse-root evaluation on uncommitted signature base.',
        'validation_policy': 'Offline portable replay only. Historical absolute paths are provenance, never dependencies.',
    }
    (ROOT / 'provenance.json').write_text(json.dumps(provenance, indent=2) + '\n')
    files = {str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest()
             for p in sorted(ROOT.rglob('*')) if p.is_file() and p.name not in
             {'manifest.json', 'collect.py', 'validate.py', 'test_validate.py', 'README.md', 'audit.json', 'offline-tests.log'}
             and '__pycache__' not in p.parts}
    (ROOT / 'manifest.json').write_text(json.dumps({'sha256': files}, indent=2) + '\n')
    print(f'Captured {len(files)} immutable artifacts')


if __name__ == '__main__':
    main()
