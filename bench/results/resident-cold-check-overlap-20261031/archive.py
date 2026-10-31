#!/usr/bin/env python3
"""Archive completed v54 evidence only. Never runs a native command."""
import hashlib
import json
from pathlib import Path
import shutil
import subprocess

ROOT = Path('/home/jeremy/.jcode/scratch/blaze-native-engine-20261030')
REPO = Path('/home/jeremy/justrust')
OUT = Path(__file__).resolve().parent

def copy(src, dst):
    dst.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(src, dst)

for version in ('v54', 'v54b'):
    shutil.copytree(ROOT / ('cold-check-overlap-' + version), OUT / 'frozen' / version, dirs_exist_ok=True)
for kind in ('build', 'unit'):
    name = 'candidate-' + kind + '-v54'
    src = ROOT / name
    for path in src.iterdir():
        if path.is_file():
            copy(path, OUT / 'native' / name / path.name)
    shutil.copytree(src / 'justrust-home/runs', OUT / 'native' / name / 'runs', dirs_exist_ok=True)
rows = []
for suffix in ('off', 'on', 'on-repeat', 'off-repeat'):
    case = 'cold-check-overlap-v54-' + suffix
    shutil.copytree(ROOT / case, OUT / 'raw' / case, dirs_exist_ok=True)
    state = json.loads((ROOT / case / 'state.json').read_text())
    rows.extend(dict(case=case, **row) for row in state['rows'])
(OUT / 'rows28.json').write_text(json.dumps(rows, indent=2) + '\n')
shutil.copytree(ROOT / 'cold-check-overlap-v54-fixture-observations', OUT / 'fixtures', dirs_exist_ok=True)
for name in ('build-test-candidate-v54.sh', 'run-cold-check-overlap-v54-pairs.sh', 'probe-cold-check-overlap-v54.py', 'validate-cold-check-overlap-v54.py', 'cold-check-overlap-v54-validation.json', 'cold-check-overlap-v54-pairs.exit', 'probe-cold-check-fixtures-v54.py', 'validate-cold-check-fixtures-v54.py', 'cold-check-fixtures-v54-validation.json'):
    copy(ROOT / name, OUT / 'provenance' / name)
for name in ('resident_state_matrix', 'resident_engine_compare', 'resident_workload_compare', 'blaze_runtime_compare', 'blaze_edit_compare'):
    copy(REPO / 'bench' / (name + '.py'), OUT / 'oracle' / (name + '.py'))
copy(REPO / 'bench/results/resident-linear-adjacency-20261031/snapshots/source63.tar.gz', OUT / 'snapshots/source63.tar.gz')
for name in ('LICENSE-MIT', 'LICENSE-APACHE', 'README.md'):
    copy(REPO / 'toolchain/resident' / name, OUT / 'licenses' / name)
copy(REPO / 'LICENSE', OUT / 'licenses/justrust-LICENSE')
copy(REPO / 'toolchain/resident/Cargo.lock', OUT / 'provenance/production-Cargo.lock')
copy(REPO / 'toolchain/resident/vendor/rust-analyzer/crates/ide-db/src/lib.rs', OUT / 'provenance/ide-db-lib.rs')
path = 'toolchain/resident/crates/ra-jit/src/main.rs'
live = (REPO / path).read_bytes()
original = (OUT / 'frozen/v54b/original' / path).read_bytes()
head = subprocess.check_output(['git', 'show', 'HEAD:' + path], cwd=REPO)
assert live == original == head
(OUT / 'restoration.json').write_text(json.dumps(dict(status='RESTORED', observed='2026-10-31T06:56:09Z', released='2026-10-31T06:56:18Z', base_head=subprocess.check_output(['git','rev-parse','HEAD'], cwd=REPO, text=True).strip(), files={path: dict(sha256=hashlib.sha256(live).hexdigest(), cmp_original=True, cmp_head=True)}), indent=2) + '\n')
(OUT / 'decision.json').write_text(json.dumps(dict(decision='REJECTED', promotion=False, latency_claim=False, zero_overhead_claim=False, noise='unknown', strict_rows=28, fixture_rows=30, worker_snapshots_are_cumulative=True, injected_panic_unset_procmacro_tests=False, alternate_lint_borrow_controls=False, long_lived_include_recovery=False), indent=2) + '\n')
print('Archived full v54/v54b lineages, native journals, all28 runtime and30 fixture raw observations, exact restoration')
