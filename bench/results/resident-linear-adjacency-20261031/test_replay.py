#!/usr/bin/env python3
"""Offline coherent re-sealing, semantic falsification, and empty HOME/PATH relocation."""
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
    save(root / 'rows35.json', [dict(case=case, **row) for case in replay.CASES for row in replay.read_json(root / 'raw' / case / 'state.json')['rows']])

def mutate_row(root, index, change, stream=None):
    path = root / 'raw' / replay.CASES[1] / 'state.json'
    state = replay.read_json(path)
    row = state['rows'][index]
    if stream:
        raw = path.parent / f'{index:02}-{row["phase"]}.{stream}'
        raw.write_bytes(change(raw.read_bytes(), row))
        row['raw'][stream].update(bytes=raw.stat().st_size, sha256=replay.digest(raw.read_bytes()))
        text = '\n'.join((path.parent / f'{index:02}-{row["phase"]}.{s}').read_text() for s in ('stdout', 'stderr'))
        row['parsed'] = replay.matrix.shared.runtime.test_results(text)
    else:
        change(row)
    save(path, state)
    reindex(root)

def decision(root, **change):
    path = root / 'decision.json'
    value = replay.read_json(path)
    value.update(change)
    save(path, value)

def bad_toolchain(root):
    for path in (root / 'raw').glob('*/state.json'):
        state = replay.read_json(path)
        for side in ('before', 'after'):
            state['toolchain_' + side]['files']['lib/rustlib/multirust-channel-manifest.toml']['sha256'] = '0' * 64
        save(path, state)
    reindex(root)

def bad_native(root):
    path = next((root / 'native/candidate-unit-v51/runs').glob('20261030-223204180-2445807/output.jsonl'))
    old = 'cached::tests::native_adjacency_diagnostic_counts_actual_shadow_inspections'
    assert old in path.read_text()
    path.write_text(path.read_text().replace(old, 'cached::tests::not_the_shadow_fixture'))

def bad_feature(root):
    for path in (root / 'native/candidate-build-v51/runs').glob('*/meta.json'):
        value = replay.read_json(path)
        value['args'].remove('--features')
        value['args'].remove(replay.FEATURE)
        save(path, value)

def bad_order(root):
    path = root / 'raw' / replay.CASES[1] / 'state.json'
    value = replay.read_json(path)
    value['rows'].reverse()
    save(path, value)
    reindex(root)

def bad_snapshot_summary(root):
    path = root / 'provenance/linear-adjacency-v51-snapshot-validation.json'
    value = replay.read_json(path)
    value['rows'][3]['worker_snapshot_reset_or_replaced'] = False
    save(path, value)

def bad_snapshot_bound(root):
    path = root / 'raw' / replay.CASES[-1] / '02-manifest-feature-add-worker-0.log'
    text = path.read_text()
    match = next(m for m in replay.PATTERN.finditer(text) if int(m.group(9)) > 0)
    candidate = int(match.group(9))
    # Re-seal transport, but force an impossible actual candidate > original bound.
    old = match.group(0)
    new = old.rsplit('original_inspections=', 1)[0] + 'original_inspections=' + str(max(0, candidate - 1))
    path.write_text(text.replace(old, new, 1))

def bad_restoration(root):
    path = root / 'restoration.json'
    value = replay.read_json(path)
    next(iter(value['files'].values()))['cmp_head'] = False
    save(path, value)

def run():
    cases = [
        ('runtime-freshness', lambda p: mutate_row(p, 1, lambda b,r: b.replace(('JR_MATRIX_RUNTIME_' + r['nonce']).encode(), b'JR_MATRIX_RUNTIME_stale'), 'stdout'), 'original actual semantic oracle'),
        ('type-freshness', lambda p: mutate_row(p, 3, lambda b,r: b.replace(('JR_MATRIX_TYPE_' + r['nonce']).encode(), b'JR_MATRIX_TYPE_stale'), 'stderr'), 'original actual semantic oracle'),
        ('assert-freshness', lambda p: mutate_row(p, 5, lambda b,r: b.replace(('JR_MATRIX_FAILURE_' + r['nonce']).encode(), b'JR_MATRIX_FAILURE_stale'), 'stderr'), 'original actual semantic oracle'),
        ('exact-runtime-test', lambda p: mutate_row(p, 1, lambda b,r: b.replace(b'summary::tests::parses_libtest_results', b'summary::tests::not_requested'), 'stdout'), 'original actual semantic oracle'),
        ('success-exit', lambda p: mutate_row(p, 1, lambda r: r.update(status=101)), 'original actual semantic oracle'),
        ('failure-exit', lambda p: mutate_row(p, 3, lambda r: r.update(status=0)), 'original actual semantic oracle'),
        ('exact-input-recipe', lambda p: mutate_row(p, 2, lambda r: r['inputs'].update({'Cargo.toml': '0' * 64})), 'actual source edit recipe'),
        ('unknown-noise', lambda p: mutate_row(p, 1, lambda r: r['noise'].update(contamination='quiet')), 'unknown noise retained'),
        ('coherent-toolchain', bad_toolchain, 'pinned actual toolchain manifest'),
        ('exact-native-shadow-fixture', bad_native, 'exact shadow inspection fixture passed'),
        ('feature-identity', bad_feature, 'explicit native diagnostic build feature'),
        ('phase-order', bad_order, 'exact phase order'),
        ('promotion-invention', lambda p: decision(p, decision='ACCEPTED', promotion=True), 'rejection and no latency/fresh attribution boundary'),
        ('diagnostic-timing-invention', lambda p: decision(p, diagnostic_timings_valid=True, latency_claim=True), 'rejection and no latency/fresh attribution boundary'),
        ('fresh-phase-invention', lambda p: decision(p, fresh_phase_attribution=True, snapshot_counts_overlap_or_reset=False), 'rejection and no latency/fresh attribution boundary'),
        ('snapshot-hidden-reset', bad_snapshot_summary, 'exact corrected snapshot replay'),
        ('actual-shadow-bound', bad_snapshot_bound, 'actual dual selector scan bounds'),
        ('failed-analysis-overwrite', lambda p: (p / 'provenance/linear-adjacency-v51-validation.json').write_text('{"status":"passed"}\n'), 'retain failed validator empty output'),
        ('restoration-receipt', bad_restoration, 'exact source restoration receipt'),
    ]
    report = []
    scratch = Path(os.environ.get('JCODE_SCRATCH_DIR', str(BASE.parent)))
    with tempfile.TemporaryDirectory(prefix='linear-adjacency-offline-', dir=scratch) as name:
        temp = Path(name)
        relocated = temp / 'relocated bundle with spaces'
        shutil.copytree(BASE, relocated, ignore=shutil.ignore_patterns('__pycache__'))
        home = temp / 'empty-home'
        home.mkdir()
        guard = '''import sys,runpy,os
script=sys.argv[1]
def audit(event,args):
    if event in ('subprocess.Popen','os.system','socket.__new__','socket.connect'):
        raise RuntimeError('native/network forbidden: '+event)
    if event=='open' and isinstance(args[0],(str,bytes,os.PathLike)):
        path=os.fsdecode(args[0])
        if path.startswith(('/home/jeremy/justrust/','/home/jeremy/.jcode/scratch/blaze-native-engine-20261030/','/home/jeremy/.blaze/','/home/jeremy/.jcode/scratch/resident-matrix-native-20261030/')):
            raise RuntimeError('original checkout/scratch/toolchain forbidden: '+path)
sys.addaudithook(audit)
sys.argv=[script]
runpy.run_path(script,run_name='__main__')
'''
        result = subprocess.run([sys.executable, '-I', '-B', '-c', guard, str(relocated / 'replay.py')], cwd=temp, env={'PATH': '', 'HOME': str(home)}, text=True, capture_output=True)
        assert result.returncode == 0, result.stderr
        assert json.loads(result.stdout)['raw_rows'] == 35
        report.append(dict(case='empty-home-path-isolated-relocation', status='PASS', original_checkout_scratch_toolchain_forbidden=True, native_network_forbidden=True))
        for name, mutate, expected in cases:
            trial = temp / name
            shutil.copytree(BASE, trial, ignore=shutil.ignore_patterns('__pycache__'))
            mutate(trial)
            replay.seal(trial)
            replay.verify_seal(trial)
            try:
                replay.validate(trial)
            except ValueError as error:
                assert expected in str(error), f'{name}: wrong semantic rejection: {error}'
                report.append(dict(case=name, status='REJECTED_AS_EXPECTED', reason=str(error), transport_checksums='coherently resealed PASS'))
            else:
                raise AssertionError(name + ' incorrectly admitted')
    return dict(status='PASS', native_jobs=False, cases=report)

if __name__ == '__main__':
    print(json.dumps(run(), indent=2))
