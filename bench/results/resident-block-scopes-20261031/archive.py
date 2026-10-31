#!/usr/bin/env python3
"""Archive actual historical evidence only, never run a native command."""
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tarfile

ROOT = Path('/home/jeremy/.jcode/scratch/blaze-native-engine-20261030')
REPO = Path('/home/jeremy/justrust')
OUT = Path(__file__).resolve().parent

def copy(src, dst):
    dst.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(src, dst)

def journal(src, dst):
    if not src.exists():
        return
    for p in src.iterdir():
        if p.is_file():
            copy(p, dst / p.name)
    for run in (src / 'justrust-home/runs').glob('*'):
        for name in ('meta.json', 'output.jsonl', 'summary.json', 'units.jsonl'):
            p = run / name
            if p.is_file():
                copy(p, dst / 'runs' / run.name / name)

def archive_tree(src, dst):
    dst.parent.mkdir(parents=True, exist_ok=True)
    with tarfile.open(dst, 'w:gz') as tar:
        for p in sorted(src.rglob('*')):
            if p.is_file():
                tar.add(p, arcname=p.relative_to(src).as_posix(), recursive=False)

for version, name in [('v46', 'block-identity-v46'), ('v47', 'block-scopes-v47'), ('v49', 'block-scopes-v49')]:
    shutil.copytree(ROOT / name, OUT / 'frozen' / version, dirs_exist_ok=True)
for name in ('candidate-build-v47', 'candidate-build-v49', 'candidate-unit-v49', 'candidate-build-v43', 'candidate-unit-v43'):
    journal(ROOT / name, OUT / 'native' / name)
for version in ('v48', 'v49'):
    src = ROOT / ('candidate-unit-' + version + '-snapshot')
    journal(src, OUT / 'native' / ('snapshot-' + version))
    archive_tree(src / 'vendor/rust-analyzer', OUT / 'snapshots' / ('rust-analyzer-' + version + '.tar.gz'))
    for name in ('native-unit-' + version + '-snapshot.sh', 'prepare-unit-' + version + '-snapshot.py'):
        copy(ROOT / name, OUT / 'provenance' / name)
for name in ('build-test-candidate-v46.sh', 'build-test-candidate-v47.sh', 'build-test-candidate-v48.sh', 'build-test-candidate-v49.sh', 'build-test-candidate-v43.sh', 'run-block-scopes-v49-pairs.sh', 'probe-cache-miss-attribution-v42.py', 'validate-block-scopes-v49.py', 'block-scopes-v49-validation.json', 'block-scopes-v49-pairs.exit', 'block-scopes-v47-baseline.exit'):
    copy(ROOT / name, OUT / 'provenance' / name)
rows = []
for case in ('block-scopes-v47-baseline', 'block-scopes-v49-candidate', 'block-scopes-v49-candidate-repeat', 'block-scopes-v49-baseline-repeat'):
    src = ROOT / case
    for p in src.iterdir():
        if p.is_file():
            copy(p, OUT / 'raw' / case / p.name)
    for row in json.loads((src / 'state.json').read_text())['rows']:
        rows.append(dict(case=case, **row))
(OUT / 'rows28.json').write_text(json.dumps(rows, indent=2) + '\n')
for name in ('resident_state_matrix', 'resident_engine_compare', 'resident_workload_compare', 'blaze_runtime_compare', 'blaze_edit_compare'):
    copy(REPO / 'bench' / (name + '.py'), OUT / 'oracle' / (name + '.py'))
archive_tree(Path('/home/jeremy/.jcode/scratch/resident-matrix-native-20261030/source63'), OUT / 'snapshots/source63.tar.gz')
for name in ('LICENSE-MIT', 'LICENSE-APACHE', 'README.md'):
    copy(REPO / 'toolchain/resident' / name, OUT / 'licenses' / name)
copy(REPO / 'LICENSE', OUT / 'licenses/justrust-LICENSE')
copy(REPO / 'toolchain/resident/Cargo.lock', OUT / 'provenance/production-Cargo.lock')
copy(REPO / 'toolchain/resident/vendor/rust-analyzer/Cargo.lock', OUT / 'provenance/original-vendor-Cargo.lock')
restored = {}
base = 'toolchain/resident/vendor/rust-analyzer/crates/hir-def/src/'
for tail in ('lib.rs', 'expr_store/body.rs', 'expr_store/lower.rs', 'expr_store/lower/format_args.rs', 'nameres/frozen.rs', 'expr_store/tests/body/block.rs'):
    name = base + tail
    live = (REPO / name).read_bytes()
    original = (ROOT / 'block-scopes-v49/original' / name).read_bytes()
    head = subprocess.check_output(['git', 'show', 'HEAD:' + name], cwd=REPO)
    assert live == original == head, name
    restored[name] = dict(sha256=hashlib.sha256(live).hexdigest(), cmp_original=True, cmp_head=True)
(OUT / 'restoration.json').write_text(json.dumps(dict(status='RESTORED', observed='2026-10-31T04:54:38Z', base_head=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=REPO, text=True).strip(), files=restored, source_release='2026-10-31T04:54:45Z'), indent=2) + '\n')
print('Retained actual28 rows, frozen originals/candidates, native failures and successes, full source snapshots and restoration')
