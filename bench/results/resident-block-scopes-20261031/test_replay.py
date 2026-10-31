#!/usr/bin/env python3
"""Offline coherent-mutation and empty HOME/PATH relocation controls, never native jobs."""
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

def save(p, value):
    p.write_text(json.dumps(value, indent=2) + '\n')

def reindex(root):
    rows = []
    for case in replay.CASES:
        for row in replay.read_json(root / 'raw' / case / 'state.json')['rows']:
            rows.append(dict(case=case, **row))
    save(root / 'rows28.json', rows)

def mutate_row(root, index, change, stream=None):
    p = root / 'raw/block-scopes-v49-candidate/state.json'
    state = replay.read_json(p)
    row = state['rows'][index]
    if stream:
        raw = p.parent / f'{index:02}-{row["phase"]}.{stream}'
        raw.write_bytes(change(raw.read_bytes(), row))
        row['raw'][stream].update(bytes=raw.stat().st_size, sha256=replay.digest(raw.read_bytes()))
        text = '\n'.join((p.parent / f'{index:02}-{row["phase"]}.{s}').read_text() for s in ('stdout', 'stderr'))
        row['parsed'] = replay.matrix.shared.runtime.test_results(text)
    else:
        change(row)
    save(p, state)
    reindex(root)

def bad_tc(root):
    for p in (root / 'raw').glob('*/state.json'):
        state = replay.read_json(p)
        for side in ('before', 'after'):
            state['toolchain_' + side]['files']['lib/rustlib/multirust-channel-manifest.toml']['sha256'] = '0' * 64
        save(p, state)
    reindex(root)

def bad_native(root):
    p = next((root / 'native/snapshot-v49/runs').glob('20261030-214133450-2047116/output.jsonl'))
    text = p.read_text()
    old = 'resident_block_scopes_codec_identity_without_body_providers'
    assert old in text
    p.write_text(text.replace(old, 'resident_block_scopes_not_the_codec_fixture'))

def bad_order(root):
    p = root / 'raw/block-scopes-v49-candidate/state.json'
    state = replay.read_json(p)
    state['rows'].reverse()
    save(p, state)
    reindex(root)

def bad_lock_delta(root):
    p = root / 'native/snapshot-v49/source-copy-before.json'
    data = replay.read_json(p)
    data['selected']['crates/hir-def/src/lib.rs'] = '0' * 64
    save(p, data)
    p = root / 'native/snapshot-v49/source-copy-after.json'
    data = replay.read_json(p)
    data['selected']['crates/hir-def/src/lib.rs'] = '0' * 64
    save(p, data)

def bad_decision(root):
    p = root / 'provenance/block-scopes-v49-validation.json'
    data = replay.read_json(p)
    data.update(decision='ACCEPTED', promotion=True)
    save(p, data)

def bad_restoration(root):
    p = root / 'restoration.json'
    data = replay.read_json(p)
    next(iter(data['files'].values()))['cmp_original'] = False
    save(p, data)

def run():
    report = []
    cases = [
        ('runtime-freshness', lambda p: mutate_row(p, 1, lambda b,r: b.replace(('JR_MATRIX_RUNTIME_' + r['nonce']).encode(), b'JR_MATRIX_RUNTIME_stale'), 'stdout'), 'original actual semantic oracle'),
        ('type-error-freshness', lambda p: mutate_row(p, 3, lambda b,r: b.replace(('JR_MATRIX_TYPE_' + r['nonce']).encode(), b'JR_MATRIX_TYPE_stale'), 'stderr'), 'original actual semantic oracle'),
        ('assertion-freshness', lambda p: mutate_row(p, 5, lambda b,r: b.replace(('JR_MATRIX_FAILURE_' + r['nonce']).encode(), b'JR_MATRIX_FAILURE_stale'), 'stderr'), 'original actual semantic oracle'),
        ('missing-runtime-test', lambda p: mutate_row(p, 1, lambda b,r: b.replace(b'summary::tests::parses_libtest_results', b'summary::tests::not_requested'), 'stdout'), 'original actual semantic oracle'),
        ('success-exit', lambda p: mutate_row(p, 1, lambda r: r.update(status=101)), 'original actual semantic oracle'),
        ('failure-exit', lambda p: mutate_row(p, 3, lambda r: r.update(status=0)), 'original actual semantic oracle'),
        ('exact-input-recipe', lambda p: mutate_row(p, 2, lambda r: r['inputs'].update({'Cargo.toml': '0' * 64})), 'actual exact source input recipe'),
        ('unknown-noise', lambda p: mutate_row(p, 1, lambda r: r['noise'].update(contamination='quiet')), 'retain unknown noise'),
        ('coherent-toolchain-change', bad_tc, 'pinned toolchain manifest'),
        ('exact-native-fixture', bad_native, 'all12 exact native fixture identities'),
        ('phase-order', bad_order, 'seven phases in order'),
        ('second-private-source-delta', bad_lock_delta, 'only private lock differs'),
        ('promotion-invention', bad_decision, 'retain rejection/noise/promotion boundary'),
        ('restoration-receipt', bad_restoration, 'exact original restoration'),
    ]
    scratch = Path(os.environ.get('JCODE_SCRATCH_DIR', str(BASE.parent)))
    with tempfile.TemporaryDirectory(prefix='block-scopes-offline-', dir=scratch) as name:
        temp = Path(name)
        relocated = temp / 'relocated bundle with spaces'
        shutil.copytree(BASE, relocated, ignore=shutil.ignore_patterns('__pycache__'))
        guard = '''import sys, runpy, os
script = sys.argv[1]
def audit(event, args):
    if event in ('subprocess.Popen', 'os.system', 'socket.__new__', 'socket.connect'):
        raise RuntimeError('native/network forbidden: ' + event)
    if event == 'open' and isinstance(args[0], (str, bytes, os.PathLike)):
        path = os.fsdecode(args[0])
        if path.startswith(('/home/jeremy/justrust/', '/home/jeremy/.jcode/scratch/blaze-native-engine-20261030/', '/home/jeremy/.blaze/', '/home/jeremy/.jcode/scratch/resident-matrix-native-20261030/')):
            raise RuntimeError('original checkout/scratch/toolchain forbidden: ' + path)
sys.addaudithook(audit)
sys.argv = [script]
runpy.run_path(script, run_name='__main__')
'''
        home = temp / 'empty-home'
        home.mkdir()
        completed = subprocess.run([sys.executable, '-I', '-B', '-c', guard, str(relocated / 'replay.py')], cwd=temp, env={'PATH': '', 'HOME': str(home)}, text=True, capture_output=True)
        if completed.returncode:
            raise AssertionError(completed.stderr)
        assert json.loads(completed.stdout)['raw_rows'] == 28
        report.append(dict(case='empty-home-path-isolated-relocation', status='PASS', forbidden_original_checkout_scratch_toolchain=True, forbidden_native_network=True))
        for name, mutate, expected in cases:
            trial = temp / name
            shutil.copytree(BASE, trial, ignore=shutil.ignore_patterns('__pycache__'))
            mutate(trial)
            replay.seal(trial)
            replay.verify_seal(trial)
            try:
                replay.validate(trial)
            except ValueError as error:
                if expected not in str(error):
                    raise AssertionError(f'{name}: wrong rejection: {error}')
                report.append(dict(case=name, status='REJECTED_AS_EXPECTED', reason=str(error), transport_checksums='coherently resigned PASS'))
            else:
                raise AssertionError(name + ' was incorrectly admitted')
    return dict(status='PASS', native_jobs=False, cases=report)

if __name__ == '__main__':
    print(json.dumps(run(), indent=2))
