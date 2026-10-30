#!/usr/bin/env python3
"""One-time read-only private receipt export. Never executes engines or edits originals."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[2]
HISTORICAL = '0dba7c33e'
FOLLOWUP = 'fb9e3f14c'
ALLOW = frozenset((
    'HOME', 'JR_ENGINE_HOME', 'RA_JIT_HOME', 'RA_JIT_CACHE', 'CARGO_HOME',
    'CARGO_TARGET_DIR', 'RUSTUP_HOME', 'BLAZE_HOME', 'BLAZE_CACHE',
    'BLAZE_BACKEND', 'BLAZE_OPT', 'BLAZE_REMOTE', 'RA_JIT_REMOTE',
    'BLAZE_PROC_MACRO_SERVER', 'RA_JIT_PROC_MACRO_SERVER',
    'XDG_RUNTIME_DIR', 'XDG_CACHE_HOME', 'XDG_CONFIG_HOME', 'XDG_DATA_HOME',
)) | frozenset(f'CARGO_PROFILE_{p}_{k}' for p in ('DEV', 'TEST')
              for k in ('OPT_LEVEL', 'DEBUG', 'INCREMENTAL'))


def digest(data):
    return hashlib.sha256(data).hexdigest()


def emit(path, data):
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open('xb') as stream:
        stream.write(data)


def emit_json(path, data):
    emit(path, (json.dumps(data, indent=2) + '\n').encode())


def git(*args):
    return subprocess.check_output(['git', *args], cwd=REPO, timeout=30)


def sanitize(metadata, records):
    """Do not record omitted key names/values, only counts and policy."""
    omitted = {'metadata_environment_entries': 0, 'process_environment_entries': 0}
    for env in metadata['environments'].values():
        omitted['metadata_environment_entries'] += len(set(env) - ALLOW)
        for key in list(env):
            if key not in ALLOW:
                del env[key]
    for row in records:
        env = row['environment']
        omitted['process_environment_entries'] += len(set(env) - ALLOW)
        row['environment'] = {k: v for k, v in env.items() if k in ALLOW}
    return omitted


def export_run(source, destination):
    # Read only explicitly named top-level receipts, never private cache trees.
    names = ['metadata.json', 'records.json', 'summary.json']
    names += sorted(p.name for p in source.iterdir()
                    if p.is_file() and (p.suffix in ('.stdout', '.stderr')
                                        or re.fullmatch(r'original-\d+\.rs', p.name)))
    originals = {name: (source / name).read_bytes() for name in names}
    metadata = json.loads(originals['metadata.json'])
    records = json.loads(originals['records.json'])
    secrets = set()
    for env in [*metadata['environments'].values(), *(r['environment'] for r in records)]:
        for key, value in env.items():
            if key not in ALLOW and re.search(r'TOKEN|SECRET|PASSWORD|CREDENTIAL|AUTH|PRIVATE_KEY', key, re.I):
                if isinstance(value, str) and len(value) >= 8:
                    secrets.add(value.encode())
    omissions = sanitize(metadata, records)
    outputs = {'metadata.sanitized.json': (json.dumps(metadata, indent=2) + '\n').encode(),
               'records.sanitized.json': (json.dumps(records, indent=2) + '\n').encode()}
    outputs.update({name: data for name, data in originals.items()
                    if name not in ('metadata.json', 'records.json')})
    for name, data in outputs.items():
        if any(secret in data for secret in secrets):
            raise ValueError('sensitive ambient value appears outside omitted maps, export refused')
        emit(destination / name, data)
    # Originals must remain bit-for-bit identical after export.
    if any((source / name).read_bytes() != data for name, data in originals.items()):
        raise ValueError('private original changed during read-only export')
    emit_json(destination / 'transformation.json', dict(
        version=1, private_original_location=str(source),
        private_original_sha256={name: digest(data) for name, data in originals.items()},
        derivative_sha256={name: digest(data) for name, data in outputs.items()},
        environment_allowlist=sorted(ALLOW), omitted=omissions,
        policy='Only environment maps in metadata and process rows were filtered by explicit allowlist. Omitted keys/values are not retained. All other fields and raw stdout/stderr/summary/backup bytes unchanged. Originals untouched and private. SHA256s attest content identity, not independent authenticity.',
        source_restored_derivation='Historical harness had no source_restored field. Compare complete source_before/source_after manifests; do not add a retroactive raw receipt.',
        sensitive_value_scan='Rejected export if any omitted sensitive ambient value of length >=8 appeared in exported bytes. This is not a general secret detector.'))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--private-parent', type=Path, default=Path('/home/jeremy'))
    parser.add_argument('--root-log', type=Path, required=True)
    args = parser.parse_args()
    for run in ('rwc01', 'rwc02'):
        export_run(args.private_parent / run, HERE / 'runs' / run)
    provenance = {}
    helper_names = ('resident_workload_compare.py', 'resident_engine_compare.py',
                    'blaze_runtime_compare.py', 'blaze_edit_compare.py',
                    'test_resident_workload_compare.py')
    for revision, names in ((HISTORICAL, helper_names),
                            (FOLLOWUP, ('resident_workload_compare.py', 'test_resident_workload_compare.py'))):
        provenance[revision] = {'commit': git('rev-parse', revision).decode().strip(), 'files': {}}
        for name in names:
            data = git('show', f'{revision}:bench/{name}')
            emit(HERE / 'provenance' / revision / name, data)
            provenance[revision]['files'][name] = digest(data)
    tree = []
    for record in git('ls-tree', '-rz', 'f97fcc2').split(b'\0'):
        if not record:
            continue
        info, raw_name = record.split(b'\t', 1)
        mode, kind, oid = info.decode().split()
        name = raw_name.decode()
        if kind != 'blob' or mode not in ('100644', '100755'):
            raise ValueError('unsupported source tree entry')
        data = git('show', f'f97fcc2:{name}')
        emit(HERE / 'input' / 'source63' / name, data)
        tree.append(dict(path=name, mode=mode, git_blob=oid, sha256=digest(data)))
    emit_json(HERE / 'input' / 'tree.json', tree)
    emit(HERE / 'root' / 'bench-broader-v28-regression.log', args.root_log.read_bytes())
    emit_json(HERE / 'evidence.json', dict(
        version=1, source_commit=git('rev-parse', 'f97fcc2').decode().strip(),
        historical=provenance, runs=['rwc01', 'rwc02'],
        scope='Two private-store orders of four pinned real modules/body edits only. No comprehensive/default/significance claim.',
        expected_counts=dict(rows=164, timed_edits=80, raw_streams=328, source_files=63),
        root_regression='root/bench-broader-v28-regression.log',
        launch_provenance='Exact direct engine commands are retained per row. Outer launcher provenance, if separately supplied, is retained under root/.'))


if __name__ == '__main__':
    main()
