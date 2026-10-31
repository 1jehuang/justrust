#!/usr/bin/env python3
"""Explicit read-only evidence export. No native execution or cache-tree copies."""
import importlib.util
import json
from pathlib import Path
import re
import shutil
import subprocess

HERE = Path(__file__).resolve().parent
BASE = HERE.parent / 'resident-state-matrix-20261030'
REPO = HERE.parents[2]
ROOT = Path('/home/jeremy/.jcode/scratch/blaze-native-engine-20261030')
spec = importlib.util.spec_from_file_location('demand_evidence_exporter', BASE / 'collect.py')
c = importlib.util.module_from_spec(spec)
spec.loader.exec_module(c)
# Exact experimental controls only. The validator scopes demand=1 to matrix
# candidate rows, and demand=0/1 + stats=1 to the explicit off/on probes.
c.ALLOW = c.ALLOW | {'RA_JIT_DEMAND_STD', 'RA_JIT_STATS'}


def partial_run(source, destination):
    originals = {name: (source / name).read_bytes() for name in ('metadata.json', 'records.json')}
    meta, rows = [json.loads(originals[name]) for name in ('metadata.json', 'records.json')]
    paths = set()
    for row in [*rows, *(value['process'] for value in meta.get('versions', {}).values())]:
        paths.update(Path(value['path']) for value in row.get('raw', {}).values())
    paths.update(Path(value['path']) for value in meta.get('backups', {}).values())
    # Only direct invocation receipt directories, never recursively private stores.
    for parent in [source, *(path for path in source.iterdir() if re.fullmatch(r's\d+', path.name) and path.is_dir())]:
        paths.update(path for path in parent.iterdir() if path.suffix in ('.stdout', '.stderr') and path.is_file())
    for path in paths:
        if not path.is_relative_to(source) or path.is_symlink():
            raise ValueError('unsafe partial-run receipt')
        originals[str(path.relative_to(source))] = path.read_bytes()
    outputs, secrets = {}, set()
    stats = dict(environment_maps=0, omitted_entries=0)
    for name, data in originals.items():
        if name.endswith('.json'):
            outputs[name.replace('.json', '.sanitized.json')] = c.encoded(c.sanitized(json.loads(data), stats, secrets))
        else:
            outputs[name] = data
    for name, data in outputs.items():
        if any(secret in data for secret in secrets):
            raise ValueError('omitted sensitive value outside environment maps')
        c.emit(destination / name, data)
    if any((source / name).read_bytes() != data for name, data in originals.items()):
        raise ValueError('private aborted-run originals changed')
    transform = dict(private_original_location=str(source), environment_allowlist=sorted(c.ALLOW), omitted=stats,
        private_original_sha256={name: c.sha(data) for name, data in originals.items()},
        derivative_sha256={name: c.sha(data) for name, data in outputs.items()},
        policy='Aborted default-mode observation. Missing finally/summary is not fabricated. No original exit/restoration attestation.')
    c.emit(destination / 'transformation.json', c.encoded(transform))
    return transform


def main():
    transforms = {'rsm05': c.export_run(Path('/home/jeremy/rsm05'), HERE / 'runs/rsm05'),
                  'rsm03': partial_run(Path('/home/jeremy/rsm03'), HERE / 'runs/rsm03')}
    shutil.copytree(BASE / 'input', HERE / 'input')
    c.emit(HERE / 'provenance/matrix-evidence-validator.py', (BASE / 'validate.py').read_bytes())
    for name in c.HELPERS:
        data = subprocess.check_output(['git', 'show', '9e6f21120:bench/' + name], cwd=REPO, timeout=30)
        c.emit(HERE / 'provenance' / name, data)
    # Retained source-only diagnostic runners are independently pinned by the base.
    for name in ('matrix-typeerror-probe.py', 'matrix-typeerror-probe-v2.py'):
        c.emit(HERE / 'provenance' / name, (BASE / 'provenance' / name).read_bytes())
    for name in ('LICENSE-MIT', 'LICENSE-APACHE'):
        c.emit(HERE / 'proof' / name, (BASE / 'proof' / name).read_bytes())
    originals = {}
    for version in (31, 32, 33):
        for path in sorted((ROOT / f'demand-std-v{version}').iterdir()):
            if path.name in ('main.rs.original', 'main.rs.v31', 'main.rs.v32', 'main.rs.candidate', 'source.patch', 'v31.patch', 'v32.patch'):
                originals[f'proof/v{version}/{path.name}'] = (path, path.read_bytes())
        for kind, names in (
            ('build', ('main.rs.frozen', 'lock-before.sha256', 'all-source-before.sha256', 'check.log', 'check.exit', 'compile.log', 'compile.exit', 'engine-version.txt', 'engine-identity.sha256', 'orchestration-exit')),
            ('unit', ('full-native.log', 'full-native.exit', 'all-source-after.log', 'lock-after.log', 'exit'))):
            directory = ROOT / f'candidate-{kind}-v{version}'
            for name in names:
                path = directory / name
                originals[f'journals/v{version}/{kind}/{name}'] = (path, path.read_bytes())
            run_root = directory / 'justrust-home/runs'
            for run in sorted(run_root.iterdir()):
                if not run.is_dir() or not re.fullmatch(r'\d{8}-\d+-\d+', run.name):
                    raise ValueError('unexpected native journal directory')
                for name in ('meta.json', 'output.jsonl', 'units.jsonl', 'live.json', 'cpu.json', 'samples.jsonl', 'processes.json', 'summary.json'):
                    path = run / name
                    originals[f'journals/v{version}/{kind}/{run.name}/{name}'] = (path, path.read_bytes())
        for name in (f'build-test-candidate-v{version}.sh', f'probe-demand-std-v{version}.py'):
            path = ROOT / name
            originals['provenance/' + name] = (path, path.read_bytes())
        for path in sorted((ROOT / f'demand-std-v{version}-native').iterdir()):
            if path.name == 'state.json' or re.fullmatch(r'(?:off|on)(?:\.(?:stdout|stderr)|-worker-\d+\.log)', path.name):
                name = 'state.sanitized.json' if path.name == 'state.json' else path.name
                originals[f'probes/v{version}/{name}'] = (path, path.read_bytes())
    for run, task in (('rsm03', '0382269fjo'), ('rsm04', '055622sxaq')):
        path = Path('/tmp/jcode-bg-tasks') / (task + '.output')
        originals[f'launches/{run}.log'] = (path, path.read_bytes())
    outputs, receipts, secrets = {}, {}, set()
    for name, (path, original) in originals.items():
        stats = dict(environment_maps=0, omitted_entries=0)
        if path.suffix == '.json':
            data = c.encoded(c.sanitized(json.loads(original), stats, secrets))
        elif path.suffix == '.jsonl':
            data = b''.join((json.dumps(c.sanitized(json.loads(line), stats, secrets)) + '\n').encode()
                            for line in original.splitlines() if line)
        else:
            data = original
        outputs[name] = data
        receipts[name] = dict(original_path=str(path), original_sha256=c.sha(original), retained_sha256=c.sha(data), omitted=stats)
    for name, data in outputs.items():
        if any(secret in data for secret in secrets):
            raise ValueError('omitted sensitive value appears in selected raw evidence')
        c.emit(HERE / name, data)
    if any(path.read_bytes() != data for path, data in originals.values()):
        raise ValueError('private source/build/probe originals changed during export')
    c.emit(HERE / 'evidence.json', c.encoded(dict(version=1, harness_commit='9e6f21120', source_commit='f97fcc2',
        private_commitments={run: transform['private_original_sha256'] for run, transform in transforms.items()},
        receipts=receipts, rsm04_private_output_exists=Path('/home/jeremy/rsm04').exists(),
        scope='Experimental candidate-only demand-std matrix, original exclusions preserved; v31/v32 opt-in failures, rsm03 aborted default-mode and rsm04 pristine admission rejection remain distinct. No native re-execution or general/default promotion.',
        restoration_context='Coordinator reported cp of all3 rsm03 durable backups at 2026-10-31T00:21:41Z. This is reported operational context, not a fabricated original finally/exit attestation.')))


if __name__ == '__main__':
    main()
