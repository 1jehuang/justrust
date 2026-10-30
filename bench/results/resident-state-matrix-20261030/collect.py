#!/usr/bin/env python3
"""Read-only private-to-portable export. No engines, builds, cache copies or source edits."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[2]
ALLOW = frozenset(('HOME', 'JR_ENGINE_HOME', 'RA_JIT_HOME', 'RA_JIT_CACHE', 'CARGO_HOME',
    'CARGO_TARGET_DIR', 'RUSTUP_HOME', 'BLAZE_HOME', 'BLAZE_CACHE', 'BLAZE_BACKEND', 'BLAZE_OPT',
    'BLAZE_REMOTE', 'RA_JIT_REMOTE', 'BLAZE_PROC_MACRO_SERVER', 'RA_JIT_PROC_MACRO_SERVER',
    'XDG_RUNTIME_DIR', 'XDG_CACHE_HOME', 'XDG_CONFIG_HOME', 'XDG_DATA_HOME')) | frozenset(
    f'CARGO_PROFILE_{p}_{k}' for p in ('DEV', 'TEST') for k in ('OPT_LEVEL', 'DEBUG', 'INCREMENTAL'))
HELPERS = ('resident_state_matrix.py', 'test_resident_state_matrix.py',
           'resident_workload_compare.py', 'resident_engine_compare.py',
           'blaze_runtime_compare.py', 'blaze_edit_compare.py')


def sha(data):
    return hashlib.sha256(data).hexdigest()


def emit(path, data):
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open('xb') as stream:
        stream.write(data)


def encoded(value):
    return (json.dumps(value, indent=2) + '\n').encode()


def git(*args):
    return subprocess.check_output(['git', *args], cwd=REPO, timeout=30)


def sanitized(value, stats, secrets):
    """Filter all nested environment maps, including version-process receipts."""
    if isinstance(value, list):
        return [sanitized(item, stats, secrets) for item in value]
    if not isinstance(value, dict):
        return value
    result = {}
    for key, item in value.items():
        if key in ('environment', 'environments', 'env', 'envs') and isinstance(item, dict):
            def env(mapping):
                if not all(isinstance(v, str) for v in mapping.values()):
                    return {k: env(v) if isinstance(v, dict) else sanitized(v, stats, secrets)
                            for k, v in mapping.items()}
                stats['environment_maps'] += 1
                stats['omitted_entries'] += len(set(mapping) - ALLOW)
                for k, v in mapping.items():
                    if k not in ALLOW and len(v) >= 8 and re.search(r'TOKEN|SECRET|PASSWORD|CREDENTIAL|AUTH|PRIVATE_KEY', k, re.I):
                        secrets.add(v.encode())
                return {k: v for k, v in mapping.items() if k in ALLOW}
            result[key] = env(item)
        else:
            result[key] = sanitized(item, stats, secrets)
    return result


def export_run(source, destination):
    raw = {name: (source / name).read_bytes() for name in ('metadata.json', 'records.json', 'summary.json')}
    meta, rows = json.loads(raw['metadata.json']), json.loads(raw['records.json'])
    paths = set()
    for row in [*rows, *(v['process'] for v in meta.get('versions', {}).values())]:
        paths.update(Path(item['path']) for item in row.get('raw', {}).values())
    for item in meta.get('backups', {}).values():
        paths.add(Path(item['path']))
    groups = list(meta.get('initial_worker_logs', {}).values()) + [r.get('worker_logs', []) for r in rows]
    for entries in groups:
        for entry in entries:
            for label in ('before', 'after', 'delta'):
                if label in entry:
                    paths.add(Path(entry[label]['path']))
    for path in paths:
        if not path.is_relative_to(source) or path.is_symlink() or not path.is_file():
            raise ValueError('unsafe explicit receipt path: ' + str(path))
        raw[str(path.relative_to(source))] = path.read_bytes()
    stats, secrets = {'environment_maps': 0, 'omitted_entries': 0}, set()
    outputs = {}
    for name, data in raw.items():
        if name in ('metadata.json', 'records.json', 'summary.json'):
            outputs[name.replace('.json', '.sanitized.json')] = encoded(sanitized(json.loads(data), stats, secrets))
        else:
            outputs[name] = data
    for name, data in outputs.items():
        if any(secret in data for secret in secrets):
            raise ValueError('omitted sensitive ambient value occurs outside filtered maps')
        emit(destination / name, data)
    if any((source / name).read_bytes() != data for name, data in raw.items()):
        raise ValueError('private originals changed during export')
    transform = dict(private_original_location=str(source), environment_allowlist=sorted(ALLOW),
        omitted=stats, private_original_sha256={n: sha(d) for n, d in raw.items()},
        derivative_sha256={n: sha(d) for n, d in outputs.items()},
        policy='Separate allowlisted derivative. Private originals unchanged. No binaries or cache trees copied. Raw diagnostics/private paths retained. SHA256 is a commitment, not authenticity attestation.')
    emit(destination / 'transformation.json', encoded(transform))
    return transform


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--private-parent', type=Path, default=Path('/home/jeremy'))
    parser.add_argument('--scratch-root', type=Path, required=True)
    parser.add_argument('--root-file', action='append', default=[], help='relative-label=absolute-file, explicit receipts only')
    args = parser.parse_args()
    transforms = {run: export_run(args.private_parent / run, HERE / 'runs' / run)
                  for run in ('rsm01', 'rstp01', 'rstp02')}
    provenance = {}
    for name in HELPERS:
        data = git('show', '5ec494e60:bench/' + name)
        emit(HERE / 'provenance' / name, data)
        provenance[name] = sha(data)
    for name in ('matrix-typeerror-probe.py', 'matrix-typeerror-probe-v2.py'):
        data = (args.scratch_root / name).read_bytes()
        emit(HERE / 'provenance' / name, data)
        provenance[name] = sha(data)
    tree = []
    for entry in git('ls-tree', '-rz', 'f97fcc2').split(b'\0'):
        if not entry:
            continue
        info, filename = entry.split(b'\t', 1)
        mode, kind, oid = info.decode().split()
        name = filename.decode()
        if kind != 'blob' or mode not in ('100644', '100755'):
            raise ValueError('unsupported pinned source entry')
        data = git('show', 'f97fcc2:' + name)
        emit(HERE / 'input/source63' / name, data)
        tree.append(dict(path=name, mode=mode, git_blob=oid, sha256=sha(data)))
    emit(HERE / 'input/tree.json', encoded(tree))
    proof = {}
    for source_name, retained_name in (
        ('toolchain/resident/crates/ra-jit/src/main.rs', 'ra-jit-main-5ec494e60.rs'),
        ('toolchain/resident/LICENSE-MIT', 'LICENSE-MIT'),
        ('toolchain/resident/LICENSE-APACHE', 'LICENSE-APACHE')):
        data = git('show', '5ec494e60:' + source_name)
        emit(HERE / 'proof' / retained_name, data)
        proof[retained_name] = dict(source_commit='5ec494e60', source_path=source_name, sha256=sha(data))
    extras = {}
    for spec in args.root_file:
        label, filename = spec.split('=', 1)
        if Path(label).is_absolute() or '..' in Path(label).parts or not Path(filename).is_absolute():
            raise ValueError('invalid root receipt specification')
        path = Path(filename)
        original = path.read_bytes()
        stats, secrets = {'environment_maps': 0, 'omitted_entries': 0}, set()
        data = encoded(sanitized(json.loads(original), stats, secrets)) if path.suffix == '.json' else original
        if any(s in data for s in secrets):
            raise ValueError('sensitive root receipt value outside filtered maps')
        emit(HERE / 'root' / label, data)
        if path.read_bytes() != original:
            raise ValueError('private root receipt changed')
        extras[label] = dict(original_path=str(path), original_sha256=sha(original),
                             retained_sha256=sha(data), omitted=stats)
    emit(HERE / 'evidence.json', encoded(dict(version=1, source_commit=git('rev-parse', 'f97fcc2').decode().strip(),
        harness_commit=git('rev-parse', '5ec494e60').decode().strip(), provenance=provenance,
        private_commitments={r: t['private_original_sha256'] for r, t in transforms.items()},
        runs=['rsm01', 'rstp01', 'rstp02'], root_receipts=extras, cold_ordering_proof=proof,
        counts=dict(matrix_rows=140, matrix_accepted=130, matrix_rejected=10, probe_rows=16,
                    source_files=63),
        scope='Failed descriptive state matrix and diagnostic probes. No comprehensive/default/significance or general faster-than-Blaze claim.')))


if __name__ == '__main__':
    main()
