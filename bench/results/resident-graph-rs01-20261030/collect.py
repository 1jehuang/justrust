#!/usr/bin/env python3
"""Explicit allowlisted evidence retention. Does not run any native workload."""
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parents[2]
SCRATCH = Path('/home/jeremy/.jcode/scratch/blaze-native-engine-20261030')
PRIOR = REPO / 'bench/results/resident-parity-re01-20261030'

def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def copy(src, dst):
    assert src.is_file() and not src.is_symlink(), src
    assert not src.read_bytes().startswith(b'\x7fELF'), src
    out = ROOT / dst
    out.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(src, out)

for session in ('rsb01', 'rsa01'):
    source = Path('/home/jeremy') / session
    for name in ('metadata.json', 'records.json', 'summary.json', 'source.original', 'suite.json'):
        copy(source / name, session + '/' + name)
    for stack in ('reference', 'engine'):
        for phase, count in [('version', 1), ('prepare', 1), ('type-error', 1), ('type-recovery', 1), ('assert-failure', 1), ('assert-recovery', 1), ('edit', 5)]:
            for rep in range(count):
                for suffix in ('stdout', 'stderr'):
                    name = f'{stack}-{phase}-{rep}.{suffix}'
                    copy(source / name, session + '/' + name)
meta = json.loads((ROOT / 'rsb01/metadata.json').read_text())
assert len(meta['source_before']) == 63
for name, sha in meta['source_before'].items():
    src = Path('/home/jeremy/.jcode/scratch/ba30/source') / name
    assert digest(src) == sha, name
    copy(src, 'source/' + name)
for name in ('resident_suite.py', 'resident_engine_compare.py', 'blaze_runtime_compare.py', 'blaze_edit_compare.py', 'suite.py'):
    copy(REPO / 'bench' / name, 'harness/' + name)
for name in ('workload-tree.txt', 'baseline-source-manifest.json', 'candidate-source-manifest.json', 'candidate-commit.txt', 'channel-manifest.toml', 'LICENSE-APACHE', 'LICENSE-MIT'):
    target = 'baseline-v2-' + name if name.startswith('candidate-') else name
    copy(PRIOR / 'provenance' / name, 'provenance/' + target)
# Freeze hashes only, not compiler source copies. Original seven pre-build hashes must match.
frozen = ROOT / 'provenance/candidate-v4-source-manifest.json'
manifest = json.loads(frozen.read_text()) if frozen.exists() else {}
if not manifest:
    tree = REPO / 'toolchain/resident'
    for path in sorted(tree.rglob('*')):
        rel = path.relative_to(tree)
        if any(p in ('.git', 'target', 'node_modules', '.wrangler') for p in rel.parts):
            continue
        if path.is_file() and not path.is_symlink():
            manifest[rel.as_posix()] = digest(path)
for line in (SCRATCH / 'candidate-build-v4/source-before.sha256').read_text().splitlines():
    sha, name = line.split(None, 1)
    assert manifest[name] == sha, ('v4 source changed', name)
(ROOT / 'provenance/candidate-v4-source-manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
for directory, names in {
    'candidate-build-v3': ['build.log', 'compile-exit', 'engine-identity.sha256', 'engine-version.txt', 'exit', 'source-after.log', 'source-before.sha256'],
    'candidate-build-v4': ['build.log', 'compile-exit', 'engine-identity.sha256', 'engine-version.txt', 'exit', 'source-after.log', 'source-before.sha256'],
    'candidate-unit-v3': ['exit', 'graph-exit', 'graph.log'],
    'candidate-unit-v4': ['exit', 'source-after.log', 'resident_graph_reuse.log', 'resident_graph_reuse.exit', 'the_roots_of_a_listed_crate.log', 'the_roots_of_a_listed_crate.exit', 'telemetry.log', 'telemetry.exit'],
    'baseline-build': ['rustc-version.txt', 'tool-identities.sha256', 'engine-version.txt', 'exit', 'build.log'],
    'candidate-build-v2': ['engine-identity.sha256', 'engine-version.txt', 'exit', 'source-before.sha256', 'source-after.log', 'compile-exit', 'build.log'],
}.items():
    for name in names:
        copy(SCRATCH / directory / name, 'build/' + directory + '/' + name)
    # Full captured stdout/stderr and exact recorded commands, never account or cache files.
    for name in names:
        if not name.endswith('.log') or name == 'source-after.log':
            continue
        text = (SCRATCH / directory / name).read_text()
        ids = re.findall(r'justrust log (\d{8}-\d{9}-\d+)', text)
        assert len(ids) == 1, (directory, name, ids)
        run_id = ids[0]
        for receipt in ('meta.json', 'output.jsonl', 'summary.json'):
            src = SCRATCH / directory / 'justrust-home/runs' / run_id / receipt
            # Older baseline/v2 raw run stores were intentionally pruned after their receipts.
            if src.is_file():
                copy(src, f'build/{directory}/runs/{run_id}/{receipt}')
for name in ('build-test-candidate-v4.sh', 'build-candidate-v3.sh', 'test-candidate-v3.sh', 'run-suite-before.sh', 'suite-before-v3.log', 'suite-before-v3.exit'):
    copy(SCRATCH / name, 'build/' + name)
# Record current source state for attribution, not an assertion that Git HEAD contains v4.
(ROOT / 'provenance/retention-head.txt').write_bytes(subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=REPO))
(ROOT / 'provenance/candidate-working-diff-stat.txt').write_bytes(subprocess.check_output(['git', 'diff', '--stat', '--', 'toolchain/resident'], cwd=REPO))
print('Retained rsb01/rsa01 full gates, source63, and allowlisted build/unit receipts')
