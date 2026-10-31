#!/usr/bin/env python3
"""Copy already-completed pool evidence. Never launches native commands."""
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

for version in ('v52', 'v53'):
    shutil.copytree(ROOT / ('builder-context-pool-' + version), OUT / 'frozen' / version, dirs_exist_ok=True)
    for kind in ('build', 'unit'):
        name = 'candidate-' + kind + '-' + version
        src = ROOT / name
        for path in src.iterdir():
            if path.is_file():
                copy(path, OUT / 'native' / name / path.name)
        shutil.copytree(src / 'justrust-home/runs', OUT / 'native' / name / 'runs', dirs_exist_ok=True)
rows = []
for suffix in ('off', 'on', 'on-repeat', 'off-repeat'):
    case = 'builder-context-v53-' + suffix
    shutil.copytree(ROOT / case, OUT / 'raw' / case, dirs_exist_ok=True)
    state = json.loads((ROOT / case / 'state.json').read_text())
    rows.extend(dict(case=case, **row) for row in state['rows'])
(OUT / 'rows28.json').write_text(json.dumps(rows, indent=2) + '\n')
for name in ('build-test-candidate-v52.sh', 'build-test-candidate-v53.sh', 'run-builder-context-v53-pairs.sh', 'probe-builder-context-v52.py', 'validate-builder-context-v53.py', 'builder-context-v53-validation.json', 'builder-context-v53-pairs.exit'):
    copy(ROOT / name, OUT / 'provenance' / name)
for name in ('resident_state_matrix', 'resident_engine_compare', 'resident_workload_compare', 'blaze_runtime_compare', 'blaze_edit_compare'):
    copy(REPO / 'bench' / (name + '.py'), OUT / 'oracle' / (name + '.py'))
copy(REPO / 'bench/results/resident-linear-adjacency-20261031/snapshots/source63.tar.gz', OUT / 'snapshots/source63.tar.gz')
for name in ('LICENSE-MIT', 'LICENSE-APACHE', 'README.md'):
    copy(REPO / 'toolchain/resident' / name, OUT / 'licenses' / name)
copy(REPO / 'LICENSE', OUT / 'licenses/justrust-LICENSE')
frontend = Path('/home/jeremy/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cranelift-frontend-0.136.2')
copy(frontend / 'src/frontend.rs', OUT / 'provenance/cranelift-frontend-0.136.2-frontend.rs')
copy(frontend / 'LICENSE', OUT / 'licenses/cranelift-frontend-LICENSE')
copy(frontend / 'Cargo.toml', OUT / 'provenance/cranelift-frontend-0.136.2-Cargo.toml')
copy(REPO / 'toolchain/resident/Cargo.lock', OUT / 'provenance/production-Cargo.lock')
path = 'toolchain/resident/crates/hir-ty-codegen/mod.rs'
live = (REPO / path).read_bytes()
original = (OUT / 'frozen/v52/original.rs').read_bytes()
head = subprocess.check_output(['git', 'show', 'HEAD:' + path], cwd=REPO)
assert live == original == head
(OUT / 'restoration.json').write_text(json.dumps(dict(status='RESTORED', observed='2026-10-31T06:11:49Z', released='2026-10-31T06:12:01Z', base_head=subprocess.check_output(['git','rev-parse','HEAD'], cwd=REPO, text=True).strip(), files={path: dict(sha256=hashlib.sha256(live).hexdigest(), cmp_original=True, cmp_head=True)}), indent=2) + '\n')
(OUT / 'decision.json').write_text(json.dumps(dict(decision='REJECTED', promotion=False, latency_claim=False, zero_overhead_claim=False, noise='unknown', strict_rows=28, debug_finalize_panic_executed=False, worker_snapshots_are_cumulative=True), indent=2) + '\n')
print('Archived full v52/v53 sources, native journals, all28 raw rows, original semantic oracles, pinned63 workload, restoration')
