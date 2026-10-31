#!/usr/bin/env python3
"""Archive existing native adjacency experiment evidence only, never native jobs."""
import hashlib
import json
from pathlib import Path
import shutil
import subprocess

ROOT = Path('/home/jeremy/.jcode/scratch/blaze-native-engine-20261030')
REPO = Path('/home/jeremy/justrust')
OUT = Path(__file__).resolve().parent
CASES = [f'linear-adjacency-v50-{suffix}' for suffix in ('baseline', 'candidate', 'candidate-repeat', 'baseline-repeat')] + ['linear-adjacency-v51-diagnostic']

def copy(src, dst):
    dst.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(src, dst)

def journal(src, dst):
    for p in src.iterdir():
        if p.is_file():
            copy(p, dst / p.name)
    for run in (src / 'justrust-home/runs').glob('*'):
        for name in ('meta.json', 'output.jsonl', 'summary.json', 'units.jsonl'):
            p = run / name
            if p.is_file():
                copy(p, dst / 'runs' / run.name / name)

for version in ('v50', 'v51'):
    shutil.copytree(ROOT / ('linear-adjacency-' + version), OUT / 'frozen' / version, dirs_exist_ok=True)
for name in ('candidate-build-v43', 'candidate-unit-v43', 'candidate-build-v50', 'candidate-unit-v50', 'candidate-build-v51', 'candidate-unit-v51'):
    journal(ROOT / name, OUT / 'native' / name)
rows = []
for case in CASES:
    for p in (ROOT / case).iterdir():
        if p.is_file():
            copy(p, OUT / 'raw' / case / p.name)
    rows.extend(dict(case=case, **row) for row in json.loads((ROOT / case / 'state.json').read_text())['rows'])
(OUT / 'rows35.json').write_text(json.dumps(rows, indent=2) + '\n')
for name in ('build-test-candidate-v43.sh', 'build-test-candidate-v50.sh', 'build-test-candidate-v51.sh', 'run-linear-adjacency-v50-pairs.sh', 'probe-cache-miss-attribution-v42.py', 'validate-linear-adjacency-v50.py', 'linear-adjacency-v50-validation.json', 'linear-adjacency-v50-pairs.exit', 'validate-linear-adjacency-v51.py', 'linear-adjacency-v51-validation.json', 'validate-linear-adjacency-v51-snapshots.py', 'linear-adjacency-v51-snapshot-validation.json', 'cleanup-completed-scope-and-adjacency.py'):
    copy(ROOT / name, OUT / 'provenance' / name)
for p in (ROOT / 'completed-scope-and-adjacency-stop').glob('*'):
    if p.is_file():
        copy(p, OUT / 'cleanup' / p.name)
for name in ('resident_state_matrix', 'resident_engine_compare', 'resident_workload_compare', 'blaze_runtime_compare', 'blaze_edit_compare'):
    copy(REPO / 'bench' / (name + '.py'), OUT / 'oracle' / (name + '.py'))
# The existing archive is the exact same pinned63 workload, checked against every state by replay.
copy(REPO / 'bench/results/resident-block-scopes-20261031/snapshots/source63.tar.gz', OUT / 'snapshots/source63.tar.gz')
for name in ('LICENSE-MIT', 'LICENSE-APACHE', 'README.md'):
    copy(REPO / 'toolchain/resident' / name, OUT / 'licenses' / name)
copy(REPO / 'LICENSE', OUT / 'licenses/justrust-LICENSE')
copy(REPO / 'toolchain/resident/Cargo.lock', OUT / 'provenance/production-Cargo.lock')
restored = {}
for name, original in [('cached.rs', ROOT / 'linear-adjacency-v50/original.rs'), ('Cargo.toml', ROOT / 'linear-adjacency-v51/original/Cargo.toml')]:
    path = 'toolchain/resident/crates/hir-ty-codegen/' + name
    live = (REPO / path).read_bytes()
    head = subprocess.check_output(['git', 'show', 'HEAD:' + path], cwd=REPO)
    assert live == original.read_bytes() == head, path
    restored[path] = dict(sha256=hashlib.sha256(live).hexdigest(), cmp_original=True, cmp_head=True)
(OUT / 'restoration.json').write_text(json.dumps(dict(status='RESTORED', observed='2026-10-31T05:36:34Z', source_release='2026-10-31T05:36:45Z', base_head=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=REPO, text=True).strip(), files=restored), indent=2) + '\n')
(OUT / 'decision.json').write_text(json.dumps(dict(decision='REJECTED', promotion=False, noise='unknown', latency_claim=False, fresh_phase_attribution=False, diagnostic_timings_valid=False, snapshot_counts_overlap_or_reset=True, strict_timing_rows=28, diagnostic_rows=7), indent=2) + '\n')
print('Archived actual35 rows, default/feature native journals, frozen sources, failed/corrected analysis and restoration')
