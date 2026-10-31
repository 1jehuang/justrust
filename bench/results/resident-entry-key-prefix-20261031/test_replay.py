#!/usr/bin/env python3
"""Offline adversarial replay tests. Mutations are coherently re-signed, never native runs."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
sys.dont_write_bytecode = True
import replay

BASE = Path(__file__).resolve().parent

def save(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')

def reindex(root):
    rows = []
    for path in sorted((root / 'raw').glob('*/*/state.json')):
        for row in replay.read_json(path)['rows']:
            rows.append(dict(version=path.parent.parent.name, mode=path.parent.name, **row))
    save(root / 'rows56.json', rows)

def mutate_row(root, index, change, stream=None):
    path = root / 'raw/v45/on/state.json'
    data = replay.read_json(path)
    row = data['rows'][index]
    if stream:
        raw = path.parent / f'{index:02}-{row["phase"]}.{stream}'
        raw.write_bytes(change(raw.read_bytes(), row))
        row['raw'][stream].update(bytes=raw.stat().st_size, sha256=replay.digest(raw.read_bytes()))
        text = '\n'.join((path.parent / f'{index:02}-{row["phase"]}.{s}').read_text() for s in ('stdout', 'stderr'))
        row['parsed'] = replay.matrix.shared.runtime.test_results(text)
    else:
        change(row)
    save(path, data)
    reindex(root)

def drop_activation(root):
    path = root / 'raw/v45/on/00-cold-prepare-worker-0.log'
    path.write_bytes(path.read_bytes().replace(replay.MARKER + b'\n', b''))

def invent_off_activation(root):
    path = root / 'raw/v45/off/00-cold-prepare-worker-0.log'
    path.write_bytes(path.read_bytes() + replay.MARKER + b'\n')

def change_state(root, change):
    path = root / 'raw/v45/on/state.json'
    data = replay.read_json(path)
    change(data)
    save(path, data)
    reindex(root)

def bad_tc(root):
    for path in (root / 'raw').glob('*/*/state.json'):
        data = replay.read_json(path)
        for side in ('before', 'after'):
            data['toolchain_' + side]['files']['lib/rustlib/multirust-channel-manifest.toml']['sha256'] = '0' * 64
        save(path, data)
    reindex(root)

def bad_native(root):
    path = next((root / 'native/v45/unit/runs').glob('*/output.jsonl'))
    path.write_text(path.read_text().replace('entry_key_prefix_gate_is_exact', 'not_the_requested_fixture'))

def run():
    report = []
    cases = [
        ('fresh-runtime-nonce', lambda p: mutate_row(p, 1, lambda b,r:b.replace(('JR_MATRIX_RUNTIME_' + r['nonce']).encode(), b'JR_MATRIX_RUNTIME_stale'), 'stdout'), 'exact original semantic oracle'),
        ('type-error-nonce', lambda p: mutate_row(p, 3, lambda b,r:b.replace(('JR_MATRIX_TYPE_' + r['nonce']).encode(), b'JR_MATRIX_TYPE_stale'), 'stderr'), 'exact original semantic oracle'),
        ('assertion-nonce', lambda p: mutate_row(p, 5, lambda b,r:b.replace(('JR_MATRIX_FAILURE_' + r['nonce']).encode(), b'JR_MATRIX_FAILURE_stale'), 'stderr'), 'exact original semantic oracle'),
        ('missing-test', lambda p: mutate_row(p, 1, lambda b,r:b.replace(b'summary::tests::parses_libtest_results', b'summary::tests::other_test'), 'stdout'), 'exact original semantic oracle'),
        ('success-exit', lambda p: mutate_row(p, 1, lambda r:r.update(status=101)), 'exact original semantic oracle'),
        ('failure-exit', lambda p: mutate_row(p, 3, lambda r:r.update(status=0)), 'exact original semantic oracle'),
        ('input-recipe', lambda p: mutate_row(p, 2, lambda r:r['inputs'].update({'Cargo.toml':'0'*64})), 'exact input recipe'),
        ('unknown-noise', lambda p: mutate_row(p, 1, lambda r:r['noise'].update(contamination='quiet')), 'retain unknown noise'),
        ('activation-on-missing', drop_activation, 'activation positive/negative control'),
        ('activation-off-invented', invent_off_activation, 'activation positive/negative control'),
        ('toolchain-coherently-changed', bad_tc, 'pinned toolchain manifest'),
        ('native-fixture-renamed', bad_native, 'four native prefix fixtures actually passed'),
        ('phase-order', lambda p: change_state(p, lambda d:d['rows'].reverse()), 'phase inventory/order'),
    ]
    scratch = Path(os.environ.get('JCODE_SCRATCH_DIR', '/home/jeremy/.jcode/scratch'))
    with tempfile.TemporaryDirectory(prefix='entry-prefix-offline-', dir=scratch) as directory:
        temp = Path(directory)
        relocated = temp / 'relocated bundle with spaces'
        shutil.copytree(BASE, relocated, ignore=shutil.ignore_patterns('__pycache__'))
        # Isolated Python, unrelated cwd, no repository/scratch lookups by replay.
        guard = '''import sys, runpy, os
script = sys.argv[1]
def audit(event, args):
    if event in ('subprocess.Popen', 'os.system', 'socket.__new__', 'socket.connect'):
        raise RuntimeError('native/network forbidden: ' + event)
    if event == 'open' and isinstance(args[0], (str, bytes, os.PathLike)):
        path = os.fsdecode(args[0])
        if path.startswith(('/home/jeremy/justrust/', '/home/jeremy/.jcode/scratch/blaze-native-engine-20261030/')):
            raise RuntimeError('original checkout/scratch forbidden: ' + path)
sys.addaudithook(audit)
sys.argv = [script]
runpy.run_path(script, run_name='__main__')
'''
        empty_home = temp / 'empty-home'
        empty_home.mkdir()
        result = subprocess.run([sys.executable, '-I', '-B', '-c', guard, str(relocated / 'replay.py')], cwd=temp, env={'PATH':'', 'HOME':str(empty_home)}, text=True, capture_output=True)
        if result.returncode:
            raise AssertionError(result.stderr)
        assert json.loads(result.stdout)['raw_rows'] == 56
        report.append(dict(case='isolated-relocation', status='PASS', empty_home_path=True, forbid_original_checkout_scratch=True, forbid_native_network=True))
        for name, mutate, expected in cases:
            copy = temp / name
            shutil.copytree(BASE, copy, ignore=shutil.ignore_patterns('__pycache__'))
            mutate(copy)
            replay.seal(copy)
            replay.verify_seal(copy)
            try:
                replay.validate(copy)
            except ValueError as error:
                if expected not in str(error):
                    raise AssertionError(f'{name}: wrong rejection: {error}')
                report.append(dict(case=name, status='REJECTED_AS_EXPECTED', semantic_reason=str(error), transport_checksums='coherently resigned PASS'))
            else:
                raise AssertionError(name + ' was incorrectly admitted')
    return dict(status='PASS', native_jobs=False, cases=report)

if __name__ == '__main__':
    print(json.dumps(run(), indent=2))
