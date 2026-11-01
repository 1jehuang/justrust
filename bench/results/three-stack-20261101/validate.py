#!/usr/bin/env python3
"""Read-only offline validator for the fixed, privacy-projected actual experiment.

Requires Python standard library only. No installed tools, native execution,
network, HOME/PATH, original directories or writable files are needed. The fixed
commitment is a version-controlled integrity anchor, not host attestation.
"""
import sys
sys.dont_write_bytecode = True
import argparse
import gzip
import hashlib
import importlib.util
import json
import os
from pathlib import Path
from unittest import mock

EXPECTED_COMMITMENT_SHA256 = '2072131f4163fc3b18474ac8a71e643d441424836657181393f2419f23dd5c0d'
ORIGINAL_JOURNAL_SHA256 = '3e60bf69a0b0b84a2ef78052ec3eee65df54fce1a1ba078cc8a4981b519f046a'
EXPECTED_REVIEW_SHA256 = 'b320e8c700f846f1bad7065be0e545d85b3b4708ba0e8d87f3b820ff154d4c11'
PRIMARY_ORIGINAL_SHA256 = 'a9f461461e3c659584cb21adcd3c2e32f3ac21c7c84725482a2918d73634d328'
COMMITMENTS = {
    ORIGINAL_JOURNAL_SHA256: EXPECTED_COMMITMENT_SHA256,
    PRIMARY_ORIGINAL_SHA256: '436aa944e47e51c3ba6379168d0d0581606be621ceb0a9dc77f8658b52a23821',
}
SUPPLEMENT_HASHES = {
    'INITIAL-RECORDINGS.json': 'd00503f5829271633d569db7dcd2b9b24fe57b4f96b71a4644de65ecbfcf6745',
    'PRIMARY-RECORDINGS.json': '02dc3e879d21740d7dbd2e36a732dea18e10001e026ee8ecae9732db9864acb3',
    'HISTORY.json': '95bf0ec1f5657b5e6b91e066d77766645109b21effd89b040a95512bb7b5d506',
}


def sha(data):
    return hashlib.sha256(data).hexdigest()


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':'), allow_nan=False).encode()


def safe(root, relative):
    rel = Path(relative)
    if rel.is_absolute() or '..' in rel.parts:
        raise ValueError('unsafe committed path')
    path = root / rel
    if any(p.is_symlink() for p in (path, *path.parents)):
        raise ValueError('evidence symlinks forbidden')
    return path


def commitment(root):
    raw = safe(root, 'COMMITMENT.json').read_bytes()
    value = json.loads(raw)
    if sha(raw) != COMMITMENTS.get(value.get('original_journal_sha256')):
        raise ValueError('fixed derivative commitment changed, including coherent history rewrite')
    for name, expected in value['files'].items():
        if sha(safe(root, name).read_bytes()) != expected:
            raise ValueError('committed artifact changed: ' + name)
    return value


def semantics(root):
    """Frozen unchanged tuple-valued oracles, exercised on projected raw output."""
    root = root.absolute()
    frozen = root / 'frozen'
    sys.path.insert(0, str(frozen))
    try:
        # Only the measured closure is imported, never a live-checkout helper.
        names = ('three_stack', 'resident_engine_compare', 'blaze_runtime_compare', 'blaze_edit_compare')
        for name in names:
            sys.modules.pop(name, None)
        spec = importlib.util.spec_from_file_location('three_stack', frozen / 'three_stack.py')
        harness = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(harness)
        for module in (harness, harness.resident, harness.runtime, harness.runtime.base):
            if Path(module.__file__).parent != frozen:
                raise ValueError('oracle imported outside frozen closure')
        compressed = safe(root, 'journal.jsonl.gz').read_bytes()
        raw = gzip.decompress(compressed)
        projection = json.loads(safe(root, 'PROJECTION.json').read_text())
        if sha(raw) != projection['derivative_journal_uncompressed_sha256']:
            raise ValueError('derivative journal digest changed')
        entries = [json.loads(line) for line in raw.splitlines()]
        header = entries[0]['data']
        if len(header['identities']['toolchain']['files']) != 4020 or len(header['source_before']) != 63:
            raise ValueError('incomplete toolchain/source identity')
        if len(projection['original_source63_sha256']) != 63:
            raise ValueError('missing original source identity')
        if projection['original_journal_sha256'] not in COMMITMENTS:
            raise ValueError('original private journal identity changed')
        for name, expected in header['identities']['harness'].items():
            if sha(safe(root, 'frozen/' + name).read_bytes()) != expected:
                raise ValueError('measured oracle closure changed')
        # Virtualize only the compressed journal filename. Every raw output and
        # source backup is still read and checked from the relocated archive.
        read_bytes = Path.read_bytes
        def virtual_bytes(path):
            return raw if path == root / 'journal.jsonl' else read_bytes(path)
        with mock.patch.object(Path, 'read_bytes', virtual_bytes):
            result = harness.replay(root)
        summary = json.loads(safe(root, 'summary.json').read_text())
        if result != summary:
            raise ValueError('reported statistics do not equal recomputed all-sample statistics')
        rows = [e['data'] for e in entries if e['kind'] == 'outcome']
        states = [e['data'] for e in entries if e['kind'] == 'state']
        receipts = []
        for row in rows:
            if row['stack'] != 'justrust':
                continue
            meta = json.loads(safe(root, row['recorded']['meta']['file']).read_text())
            if (meta.get('place') != 'local' or meta.get('route') != 'JUSTRUST_REMOTE=off'
                    or meta.get('cwd') != '/SOURCE' or meta.get('subcommand') != 'test'
                    or meta.get('args') != row['command'][1:]
                    or set(meta.get('env', {})) - {'CARGO_PROFILE_DEV_DEBUG', 'CARGO_TARGET_DIR'}):
                raise ValueError('shipping local route/compiler receipt mismatch')
            receipts.append(meta)
        nonces = [r['nonce'] for r in rows if r['nonce'] is not None]
        if (len(rows) != 126 or len(receipts) != 42 or len(nonces) != 54
                or len(set(nonces)) != 54 or len(states) != 18):
            raise ValueError('actual run coverage mismatch')
        for name, original_hash in projection['original_source63_sha256'].items():
            item = projection['inventory']['backups/' + name]
            if item['original_sha256'] != original_hash:
                raise ValueError('original source identity/inventory mismatch')
        # Original source fixture projection is explicitly separate from actual
        # source identity. It must never be described as byte-identical source.
        if projection['original_identities'] != header['identities']:
            raise ValueError('original tool identity projection mismatch')
        return {'status': 'PASS', 'actual_timed_calls': 126, 'shipping_local_receipts': 42,
                'fresh_execution_nonces': 54, 'private_states': 18, 'source_files': 63,
                'toolchain_files': 4020, 'seed_files': len(header['seed']),
                'cleanup_receipts': 6, 'statistics': result['statistics'],
                'noise': 'unknown', 'claim': 'controlled-local descriptive only; no broad/default-hosted/significance claim'}
    finally:
        sys.path.pop(0)


def validate(root):
    root = root.absolute()
    supplement_root = root if (root / 'INITIAL-RECORDINGS.json').is_file() else root.parent
    if (supplement_root / 'INITIAL-RECORDINGS.json').is_file():
        for name, expected in SUPPLEMENT_HASHES.items():
            if sha(safe(supplement_root, name).read_bytes()) != expected:
                raise ValueError('additive recording/history review changed')
        for name, total, extra in (('INITIAL-RECORDINGS.json', 88, 46), ('PRIMARY-RECORDINGS.json', 42, 0)):
            proof = json.loads(safe(supplement_root, name).read_text())
            if (proof['total_recordings'] != total or proof['measured_recordings'] != 42
                    or proof['background_prebuild_recordings'] != extra
                    or proof['original_source_before_sha256'] != proof['original_source_after_sha256']
                    or proof['original_identity_before_sha256'] != proof['original_identity_after_sha256']):
                raise ValueError('original source/tool restoration or private inventory mismatch')
    manifest = commitment(root)
    if manifest['original_journal_sha256'] == PRIMARY_ORIGINAL_SHA256:
        result = semantics(root)
        proof = json.loads(safe(root, 'recording-inventory.json').read_text())
        if len(proof) != 6 or [p['sample'] for p in proof] != list(range(6)):
            raise ValueError('missing exact corrected recording inventory')
        projected = json.loads(safe(root, 'PROJECTION.json').read_text())
        known = set()
        for sample in proof:
            if (sample['count'] != 7 or len(sample['recordings']) != 7 or sample['buildd_entries']
                    or any(r['background_prebuild'] for r in sample['recordings'])):
                raise ValueError('corrected run contains speculative or extra work')
            for receipt in sample['recordings']:
                if receipt['id'] in known or receipt['place'] != 'local' or receipt['route'] != 'JUSTRUST_REMOTE=off':
                    raise ValueError('corrected recording identity/local route mismatch')
                known.add(receipt['id'])
        raw = gzip.decompress(safe(root, 'journal.jsonl.gz').read_bytes())
        entries = [json.loads(line) for line in raw.splitlines()]
        if entries[0]['data']['schema'] != 'three-stack-v91-1':
            raise ValueError('corrected run has wrong measured schema')
        recorded = set()
        for e in entries:
            if e['kind'] == 'state':
                if any(e['data']['environment'].get(k) != '0' for k in ('JUSTRUST_BUILDD', 'JUSTRUST_BUILDD_PREBUILD')):
                    raise ValueError('speculative daemon controls not disabled')
            if e['kind'] == 'outcome' and e['data']['stack'] == 'justrust':
                ref = e['data']['recorded']['meta']
                meta = json.loads(safe(root, ref['file']).read_text())
                recorded.add(meta['id'])
                candidates = [r for s in proof for r in s['recordings'] if r['id'] == meta['id']]
                if (len(candidates) != 1 or candidates[0]['args'] != meta['args']
                        or candidates[0]['meta_original_sha256'] != projected['inventory'][ref['file']]['original_sha256']):
                    raise ValueError('corrected measured/private inventory mismatch')
        if recorded != known:
            raise ValueError('unmeasured extra justrust recordings')
        result['status'] = 'INTEGRITY-AND-SEMANTICS-PASS-CORRECTED-PRIMARY'
        result['speculative_background_recordings'] = 0
        result['justrust_recordings_per_round'] = [7] * 6
        result['original_journal_sha256'] = PRIMARY_ORIGINAL_SHA256
        return result
    review_raw = safe(root, 'REVIEW.json').read_bytes()
    if sha(review_raw) != EXPECTED_REVIEW_SHA256:
        raise ValueError('fixed confounded-run review changed')
    review = json.loads(review_raw)
    result = semantics(root)
    result['status'] = 'INTEGRITY-PASS-CONFOUNDED-NOT-FAIR-BENCHMARK'
    result['review'] = review
    result['diagnostic_first_run_statistics'] = result.pop('statistics')
    result['claim'] = 'first-run command latency with unaccounted speculative background work; not fair edit-to-result or headline evidence'
    return result


def deny_side_effects():
    def audit(event, args):
        if (event.startswith(('subprocess.', 'socket.', 'os.exec', 'os.spawn', 'os.posix_spawn'))
                or event in ('os.system', 'os.fork', 'os.forkpty', 'os.mkdir', 'os.remove',
                             'os.rmdir', 'os.rename', 'os.truncate', 'os.chmod', 'os.utime', 'os.symlink', 'os.link')):
            raise PermissionError('offline read-only validator denied ' + event)
        if event == 'open':
            _, mode, flags = args
            if (isinstance(mode, str) and any(c in mode for c in 'wax+')) or (
                    isinstance(flags, int) and flags & (os.O_WRONLY | os.O_RDWR | os.O_CREAT | os.O_TRUNC | os.O_APPEND)):
                raise PermissionError('offline validator denied file write')
    sys.addaudithook(audit)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--archive', type=Path, default=Path(__file__).parent)
    parser.add_argument('--run', choices=('primary', 'initial'), default='primary')
    args = parser.parse_args(argv)
    deny_side_effects()
    archive = args.archive
    if args.run == 'primary' and (archive / 'primary-v91').is_dir():
        archive = archive / 'primary-v91'
    result = validate(archive)
    print(json.dumps(result, indent=2))
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
