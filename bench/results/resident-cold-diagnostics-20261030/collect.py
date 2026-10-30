#!/usr/bin/env python3
"""Read-only export of distinct v30 diagnostic evidence, never native execution."""
import difflib
import importlib.util
import json
from pathlib import Path
import shutil

HERE = Path(__file__).resolve().parent
BASE = HERE.parent / 'resident-state-matrix-20261030'
ROOT = Path('/home/jeremy/.jcode/scratch/blaze-native-engine-20261030')
SCRATCH = Path('/home/jeremy/.jcode/scratch/resident-matrix-native-20261030')
spec = importlib.util.spec_from_file_location('matrix_exporter', BASE / 'collect.py')
c = importlib.util.module_from_spec(spec)
spec.loader.exec_module(c)


def main():
    receipts = {}
    def retain(path, relative):
        original = path.read_bytes()
        stats, secrets = dict(environment_maps=0, omitted_entries=0), set()
        if path.suffix == '.json':
            data = c.encoded(c.sanitized(json.loads(original), stats, secrets))
        elif path.suffix == '.jsonl':
            data = b''.join(c.encoded(c.sanitized(json.loads(line), stats, secrets)).replace(b'\n', b'') + b'\n'
                            for line in original.splitlines() if line)
        else:
            data = original
        if any(secret in data for secret in secrets):
            raise ValueError('omitted credential outside filtered maps')
        c.emit(HERE / relative, data)
        if path.read_bytes() != original:
            raise ValueError('original changed during export')
        receipts[relative] = dict(original_path=str(path), original_sha256=c.sha(original),
                                 retained_sha256=c.sha(data), omitted=stats)
    transforms = {name: c.export_run(Path('/home/jeremy') / name, HERE / 'runs' / name)
                  for name in ('rstp03', 'p')}
    for directory in ('provenance', 'input'):
        shutil.copytree(BASE / directory, HERE / directory)
    retain(BASE / 'validate.py', 'provenance/matrix-evidence-validator.py')
    retain(BASE / 'runs/rsm01/metadata.sanitized.json', 'input/rsm01-metadata.sanitized.json')
    retain(SCRATCH / 'matrix-typeerror-v30-probe.py', 'provenance/matrix-typeerror-v30-probe.py')
    retain(ROOT / 'cold-diagnostics-v30/main.rs.original', 'proof/main.rs.original')
    retain(ROOT / 'candidate-build-v30/main.rs.frozen', 'proof/main.rs.frozen')
    for license in ('LICENSE-MIT', 'LICENSE-APACHE'):
        retain(BASE / 'proof' / license, 'proof/' + license)
    old = (HERE / 'proof/main.rs.original').read_text().splitlines(keepends=True)
    new = (HERE / 'proof/main.rs.frozen').read_text().splitlines(keepends=True)
    patch = ''.join(difflib.unified_diff(old, new, fromfile='main.rs.original', tofile='main.rs.frozen'))
    c.emit(HERE / 'proof/cold-diagnostics.patch', patch.encode())
    for name in ('all-source-before.sha256', 'check.exit', 'check.log', 'compile.exit', 'compile.log',
                 'engine-identity.sha256', 'engine-version.txt', 'lock-before.sha256', 'orchestration-exit'):
        retain(ROOT / 'candidate-build-v30' / name, 'journals/build/' + name)
    for name in ('all-source-after.log', 'exit', 'full-native.exit', 'full-native.log', 'lock-after.log'):
        retain(ROOT / 'candidate-unit-v30' / name, 'journals/unit/' + name)
    retain(ROOT / 'build-test-candidate-v30.sh', 'journals/build-test-candidate-v30.sh')
    run_ids = [('build', '20261030-163119597-1821357'), ('build', '20261030-163136337-1823465'),
               ('unit', '20261030-163303318-1833909')]
    names = ('meta.json', 'output.jsonl', 'units.jsonl', 'live.json', 'cpu.json', 'samples.jsonl', 'processes.json', 'summary.json')
    for kind, run in run_ids:
        for name in names:
            retain(ROOT / ('candidate-' + kind + '-v30/justrust-home/runs') / run / name,
                   'journals/' + kind + '/' + run + '/' + name)
    c.emit(HERE / 'evidence.json', c.encoded(dict(version=1, source_commit='f97fcc2',
        native_fix_commit='91220ff43', original_matrix_bundle_commit='99049bcf1',
        runs=['rstp03', 'p'], receipts=receipts,
        private_commitments={n: t['private_original_sha256'] for n, t in transforms.items()},
        scope='Bounded cold diagnostic correctness, not performance. Earlier path-limit failure and baseline initial-cold rejection remain rejected. No original matrix relabeling.')))


if __name__ == '__main__':
    main()
