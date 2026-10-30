#!/usr/bin/env python3
"""One-shot historical copy only. No compiler/workload/live source writes."""
import hashlib
import json
from pathlib import Path
import shutil

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parents[2]
SCRATCH = Path('/home/jeremy/.jcode/scratch/blaze-native-engine-20261030')
PAIRS = [('v16', 'rsb05', 'rsa11', 15, 16),
         ('v17', 'rsb06', 'rsa12', 16, 17),
         ('v17-cumulative', 'rsb07', 'rsa13', 4, 17),
         ('v18', 'rsb08', 'rsa14', 17, 18),
         ('v19', 'rsb09', 'rsa15', 4, 19),
         ('v19-reverse', 'rsb10', 'rsa16', 4, 19)]
EXCLUDED = {'collect.py', 'validate.py', 'test_validate.py', 'README.md',
            'manifest.json', 'audit.json', 'offline-tests.log'}


def capture(source, target):
    target = ROOT / target
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(source, target)


def tree(source, target, native=False):
    for path in sorted(source.rglob('*')):
        relative = path.relative_to(source)
        if not path.is_file() or path.is_symlink():
            continue
        if native and 'justrust-home' in relative.parts and not (
            'runs' in relative.parts and path.name in
            {'meta.json', 'summary.json', 'output.jsonl', 'units.jsonl'}
        ):
            continue
        capture(path, target / relative)


def main():
    if (ROOT / 'manifest.json').exists():
        raise SystemExit('Immutable capture already exists, never recapture live sources')
    if not (SCRATCH / 'suite-v19-reverse.exit').exists() or not (SCRATCH / 'candidate-unit-v19/full-native.exit').exists():
        raise SystemExit('Wait for coordinator timing isolation release')
    for _, old, new, _, _ in PAIRS:
        for session in [old, new]:
            for path in sorted((Path('/home/jeremy') / session).iterdir()):
                if path.is_file():
                    capture(path, Path('sessions') / session / path.name)
    for version in [4, 15, 16, 17, 18, 19]:
        for kind in ['build', 'unit'] if version in [16, 17, 18, 19] else ['build']:
            name = f'candidate-{kind}-v{version}'
            tree(SCRATCH / name, Path('native') / name, native=True)
    archived = REPO / 'bench/results/resident-signature-flags-20261030/baseline-head'
    tree(archived, Path('baseline-head'))
    baseline_map = {}
    for line in (SCRATCH / 'candidate-build-v4/source-before.sha256').read_text().splitlines():
        digest, relative = line.split(None, 1)
        source = (archived / 'toolchain/resident' / relative if relative == 'crates/ra-jit/src/main.rs'
                  else SCRATCH / 'candidate-build-v15/frozen-source' / relative)
        if hashlib.sha256(source.read_bytes()).hexdigest() != digest:
            raise SystemExit('Historical baseline bytes do not match declared source hash: ' + relative)
        capture(source, Path('native/candidate-build-v4/hash-matched-source') / relative)
        baseline_map[relative] = str(source)
    (ROOT / 'baseline-v4-byte-provenance.json').write_text(json.dumps({
        'policy':'Exact declared-v4 bytes selected by SHA equality from existing historical archives, not asserted an original v4 full-tree freeze. No live source read for candidate reconstruction.',
        'sources':baseline_map},indent=2)+'\n')
    for version, directory in [(16, 'root-snapshot'), (17, 'root-snapshot-v17'),
                               (18, 'root-snapshot-v18'), (19, 'root-snapshot-v19')]:
        tree(SCRATCH / directory, Path('source-ready') / f'v{version}')
        receipt = 'root-snapshot-source-ready' + ('' if version == 16 else f'-v{version}') + '.txt'
        capture(SCRATCH / receipt, Path('source-ready') / receipt)
    for version in [16, 17, 18]:
        name = f'resident-root-snapshot-profile-v{version}-20261030'
        tree(SCRATCH.parent / name, Path('traces') / f'v{version}')
        for script in [f'profile-root-snapshot-v{version}.py',
                       f'root-snapshot-phase-summary-v{version}.py',
                       f'build-test-candidate-v{version}.sh',
                       f'full-native-v{version}.sh',
                       f'verify-native-unit-v{version}.py',
                       f'native-unit-v{version}-admission.log']:
            capture(SCRATCH / script, Path('provenance') / script)
    for script in ['build-test-candidate-v19.sh', 'full-native-v19.sh', 'verify-native-unit-v19.py', 'native-unit-v19-admission.log']:
        capture(SCRATCH / script, Path('provenance') / script)
    for label, _, _, _, _ in PAIRS:
        for suffix in ['baseline.log', 'baseline.exit', 'candidate.log', 'candidate.exit',
                       'compare.log', 'exit']:
            name = f'suite-{label}' + ('.exit' if suffix == 'exit' else '-' + suffix)
            capture(SCRATCH / name, Path('provenance') / name)
        script = f'run-suite-{label}.sh'
        capture(SCRATCH / script, Path('provenance') / script)
    for pattern in ['suite-v16*', 'suite-v17*', 'suite-v18*', 'suite-v19*', 'native-unit-v16*', 'native-unit-v17*', 'native-unit-v18*', 'native-unit-v19*', 'queue-suite-v16.sh', 'queue-suite-v17.sh', 'queue-suite-v18.sh', 'queue-suite-v19.sh', 'v19-reverse-launch.log']:
        for path in sorted(SCRATCH.glob(pattern)):
            if path.is_file(): capture(path, Path('provenance') / path.name)
    capture(SCRATCH / 'cleanup-completed-before-v19.py', Path('provenance/cleanup-completed-before-v19.py'))
    capture(SCRATCH.parent / 'resident-private-cleanup-before-v19-20261030.json', Path('provenance/resident-private-cleanup-before-v19-20261030.json'))
    for name in ['LICENSE-APACHE', 'LICENSE-MIT']:
        capture(REPO / 'toolchain/resident' / name, Path('source-ready') / name)
    for name in ['suite.py', 'resident_suite.py', 'resident_engine_compare.py',
                 'blaze_runtime_compare.py', 'blaze_compare.py']:
        path = REPO / 'bench' / name
        if path.is_file():
            capture(path, Path('harness-captured') / name)
    data = {
        'schema': 'resident_root_snapshot_evidence/v1',
        'decision': 'REJECTED_NOT_REPLICATED', 'promotion': False,
        'noise': 'unknown', 'floor_percent': 10,
        'scope': 'licensed Blaze-derived native resident, scoped local filtered-libtest diagnostic, not cleanroom, shipping, broad workload or default-promotion gate',
        'pairs': [{'label': label, 'baseline': old, 'candidate': new,
                   'baseline_version': ov, 'candidate_version': nv}
                  for label, old, new, ov, nv in PAIRS],
        'raw_policy': 'Top-level suites, native raw journals, complete existing trace files and original full-source/patch/ready receipts copied byte-for-byte. No binaries, caches, targets, install IDs or reconstructed historical source.',
        'harness_policy': 'Copied at retention time, not asserted historical per-run snapshots.',
        'validation_policy': 'Portable offline Python standard-library checks only. Historical absolute paths are provenance, not external dependencies.',
        'source_policy': 'Evidence retention never commits/restores live main.rs. Authorized source worker v18->v17+cfg(test) v19 transition preserved by exact original snapshots/patches; compiler source retention is root decision, separate from evidence.',
        'exports_policy': 'Original meta/summary/output/units journals retained directly. No slim exports required and no compiler workload invoked.',
        'rejection': 'First v19 absolute -18.51% was not replicated candidate-first: -1.16% raw, +1.67% reference-normalized. No robust source promotion. Root restored three production files separately, not this evidence worker.',
        'v19_cleanup_policy': 'Root stopped 34 completed own exact private namespaces before final v19 pair, without deleting stores. Memory pressure changed from historical pairs. Both fresh baseline and candidate run under the new cleanup policy. Final pair is separate, never pooled with historical pairs or solely attributed to the source change.',
    }
    (ROOT / 'provenance.json').write_text(json.dumps(data, indent=2) + '\n')
    hashes = {str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest()
              for p in sorted(ROOT.rglob('*')) if p.is_file()
              and p.name not in EXCLUDED and '__pycache__' not in p.parts}
    (ROOT / 'manifest.json').write_text(json.dumps({'sha256': hashes}, indent=2) + '\n')
    print(f'Captured {len(hashes)} exact artifacts')


if __name__ == '__main__':
    main()
