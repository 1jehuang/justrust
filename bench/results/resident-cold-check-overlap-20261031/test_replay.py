#!/usr/bin/env python3
"""Resealed semantic negative controls and isolated relocation, never native jobs."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parent
sys.path.insert(0, str(ROOT))
import replay


def save(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')


def state_path(root, case):
    return root / 'raw' / replay.CASES[case] / 'state.json'


def mutate_row(root, case, index, change):
    path = state_path(root, case)
    state = replay.read_json(path)
    change(state['rows'][index])
    save(path, state)


def raw_edit(root, case, index, suffix, change):
    directory = state_path(root, case).parent
    state = replay.read_json(directory / 'state.json')
    row = state['rows'][index]
    path = directory / f'{index:02}-{row["phase"]}.{suffix}'
    path.write_bytes(change(path.read_bytes(), row))
    data = path.read_bytes()
    row['raw'][suffix].update(bytes=len(data), sha256=replay.digest(data))
    text = '\n'.join((directory / f'{index:02}-{row["phase"]}.{s}').read_text(errors='replace') for s in ('stdout', 'stderr'))
    row['parsed'] = replay.matrix.shared.runtime.test_results(text)
    save(directory / 'state.json', state)


def log_edit(root, case, index, change):
    directory = state_path(root, case).parent
    phase = replay.PHASES[index]
    path, = directory.glob(f'{index:02}-{phase}-worker-*.log')
    path.write_bytes(change(path.read_bytes()))


def decision(root, key, value):
    path = root / 'decision.json'
    obj = replay.read_json(path)
    obj[key] = value
    save(path, obj)


def coherent_toolchain(root):
    for case in range(4):
        path = state_path(root, case)
        obj = replay.read_json(path)
        for name in ('toolchain_before', 'toolchain_after'):
            obj[name]['files']['bin/rustc']['sha256'] = '0' * 64
        save(path, obj)


def renamed_native_fixture(root):
    for path in (root / 'native/candidate-unit-v54/runs').glob('*/output.jsonl'):
        path.write_text(path.read_text().replace('test tests::', 'test renamed::'))


def coherent_frozen_source(root):
    directory = root / 'frozen/v54b'
    name = 'candidate/toolchain/resident/crates/ra-jit/src/main.rs'
    path = directory / name
    path.write_bytes(path.read_bytes() + b'\n// changed\n')
    hashes = replay.manifest(directory / 'SHA256SUMS')
    hashes[name] = replay.digest(path.read_bytes())
    (directory / 'SHA256SUMS').write_text(''.join(f'{sha}  {name}\n' for name, sha in hashes.items()))


def fixture_edit(root, case, mode, change):
    path = root / 'fixtures/state.json'
    state = replay.read_json(path)
    row = next(r for r in state['rows'] if r['case'] == case and r['mode'] == mode)
    raw = root / 'fixtures' / (case+'-'+mode+'-raw.stderr')
    raw.write_bytes(change(raw.read_bytes()))
    row['raw']['stderr'].update(bytes=raw.stat().st_size,sha256=replay.digest(raw.read_bytes()))
    save(path,state)


def fixture_row(root, case, mode, change):
    path = root / 'fixtures/state.json'
    state = replay.read_json(path)
    change(next(r for r in state['rows'] if r['case'] == case and r['mode'] == mode))
    save(path,state)


def coherent_diagnostics(root):
    for mode in ('original','off','on'):
        fixture_edit(root,'unused_error',mode,lambda b:b.replace(b'E0308',b'E0000'))


def index_and_seal(root):
    rows = []
    for case in replay.CASES:
        rows.extend(dict(case=case, **row) for row in replay.read_json(root / 'raw' / case / 'state.json')['rows'])
    save(root / 'rows28.json', rows)
    replay.seal(root)


def relocated(root, temporary):
    # Run only the relocated payload. Deny original checkout/scratch/toolchain reads,
    # external application reads, networking and native process launches in replay.
    bootstrap = '''
import os, pathlib, runpy, sys
bundle = pathlib.Path(sys.argv[1]).resolve()
standard = pathlib.Path(sys.base_prefix).resolve()
def audit(event, args):
    if event == 'open' and isinstance(args[0], (str, bytes, os.PathLike)):
        path = pathlib.Path(os.fsdecode(args[0])).resolve()
        if not path.is_relative_to(bundle) and not path.is_relative_to(standard):
            raise RuntimeError('forbidden external read: ' + str(path))
    if event.startswith(('subprocess.', 'socket.', 'os.exec', 'os.spawn')) or event == 'os.system':
        raise RuntimeError('forbidden native/network action: ' + event)
sys.addaudithook(audit)
sys.argv = [str(bundle / 'replay.py')]
runpy.run_path(str(bundle / 'replay.py'), run_name='__main__')
'''
    env = dict(HOME=str(temporary / 'empty home'), PATH='', LANG='C.UTF-8')
    result = subprocess.run([sys.executable, '-I', '-B', '-c', bootstrap, str(root)], cwd=temporary, env=env, text=True, capture_output=True, check=False)
    if result.returncode:
        raise AssertionError('isolated relocation failed:\n' + result.stdout + result.stderr)
    answer = json.loads(result.stdout)
    assert answer['status'] == 'PASS' and answer['strict_rows'] == 28
    return dict(status='PASS', isolated_python=True, empty_home_path=True, audited_external_reads=True, native_network_forbidden=True)


def main():
    controls = [
        ('stale runtime nonce', lambda r: raw_edit(r, 1, 1, 'stdout', lambda b, row: b.replace(('JR_MATRIX_RUNTIME_' + row['nonce']).encode(), b'JR_MATRIX_RUNTIME_stale')), 'original semantic oracle'),
        ('duplicate runtime challenge', lambda r: raw_edit(r, 1, 1, 'stdout', lambda b, row: b + ('\nJR_MATRIX_RUNTIME_' + row['nonce'] + '\n').encode()), 'original semantic oracle'),
        ('wrong exact test identity', lambda r: raw_edit(r, 1, 1, 'stdout', lambda b, row: b.replace(b'summary::tests::parses_libtest_results', b'summary::tests::unrelated_test')), 'original semantic oracle'),
        ('missing diagnostic code', lambda r: raw_edit(r, 1, 3, 'stderr', lambda b, row: b.replace(b'E0308', b'E0000')), 'original semantic oracle'),
        ('type error must not run', lambda r: raw_edit(r, 1, 3, 'stdout', lambda b, row: b + ('JR_MATRIX_RUNTIME_' + row['nonce'] + '\n').encode()), 'original semantic oracle'),
        ('type error wrong exit', lambda r: mutate_row(r, 1, 3, lambda row: row.update(status=0)), 'original semantic oracle'),
        ('stale assertion freshness', lambda r: raw_edit(r, 1, 5, 'stderr', lambda b, row: b.replace(('JR_MATRIX_FAILURE_' + row['nonce']).encode(), b'JR_MATRIX_FAILURE_stale')), 'original semantic oracle'),
        ('assertion wrong exit', lambda r: mutate_row(r, 1, 5, lambda row: row.update(status=0)), 'original semantic oracle'),
        ('source edit recipe drift', lambda r: mutate_row(r, 1, 2, lambda row: row['inputs'].update({'Cargo.toml': '0' * 64})), 'actual phase input recipe'),
        ('positive reuse removed', lambda r: log_edit(r, 1, 1, lambda b: b.replace(replay.MARKER + b'\n', b'')), 'exact cumulative asynchronous startup marker snapshots'),
        ('disabled reuse invented', lambda r: log_edit(r, 0, 1, lambda b: b + replay.MARKER + b'\n'), 'exact cumulative asynchronous startup marker snapshots'),
        ('duplicate per-process marker', lambda r: log_edit(r, 1, 1, lambda b: b + replay.MARKER + b'\n'), 'exact cumulative asynchronous startup marker snapshots'),
        ('marker not fixed line', lambda r: log_edit(r, 1, 1, lambda b: b.replace(replay.MARKER, replay.MARKER + b' extra')), 'exact cumulative asynchronous startup marker snapshots'),
        ('coherent toolchain mutation', coherent_toolchain, 'exact whole4020 pinned toolchain anchor'),
        ('native differential fixture renamed', renamed_native_fixture, 'exact native test identities/statuses'),
        ('coherent frozen source mutation', coherent_frozen_source, 'candidate anchor'),
        ('phase order drift', lambda r: mutate_phase_order(r), 'exact seven phase order'),
        ('noise certainty invented', lambda r: mutate_row(r, 1, 1, lambda row: row['noise'].update(contamination='quiet')), 'finite elapsed/unknown noise boundary'),
        ('promotion invented', lambda r: decision(r, 'promotion', True), 'rejection and missing-coverage boundaries'),
        ('speed claim invented', lambda r: decision(r, 'latency_claim', True), 'rejection and missing-coverage boundaries'),
        ('zero overhead claim invented', lambda r: decision(r, 'zero_overhead_claim', True), 'rejection and missing-coverage boundaries'),
        ('debug gate pass invented', lambda r: decision(r, 'injected_panic_unset_procmacro_tests', True), 'rejection and missing-coverage boundaries'),
        ('restoration claim corrupted', corrupt_restoration, 'exact main restoration'),
        ('original analysis rewritten', corrupt_analysis, 'exact original runtime analysis'),
    ]
    controls.extend([
        ('fixture precise diagnostic changed', lambda r: fixture_edit(r,'unused_error','on',lambda b:b.replace(b'E0308',b'E0000')), 'complete original/OFF/ON diagnostics'),
        ('coherent all-mode diagnostic mutation', coherent_diagnostics, 'complete original diagnostic anchor'),
        ('included file diagnostics changed', lambda r: fixture_edit(r,'include_error','on',lambda b:b.replace(b'E0308',b'E0000')), 'complete original/OFF/ON diagnostics'),
        ('macro diagnostics changed', lambda r: fixture_edit(r,'macro_error','on',lambda b:b.replace(b'E0308',b'E0000')), 'complete original/OFF/ON diagnostics'),
        ('fixture wrong exit', lambda r: fixture_row(r,'unused_error','on',lambda row:row.update(status=0)), 'fixture exact status'),
        ('fixture stop failed', lambda r: fixture_row(r,'valid','on',lambda row:row['post_observation_stop'].update(status=1)), 'actual post-observation stop'),
        ('fixture startup flag changed', lambda r: fixture_row(r,'valid','on',lambda row:row['startup_controls'].update(RA_JIT_COLD_CHECK_OVERLAP='0')), 'fixture startup controls'),
        ('alternate lint controls invented', lambda r: decision(r,'alternate_lint_borrow_controls',True), 'rejection and missing-coverage boundaries'),
        ('long lived include proof invented', lambda r: decision(r,'long_lived_include_recovery',True), 'rejection and missing-coverage boundaries'),
    ])
    observed = []
    scratch = Path(os.environ.get('JCODE_SCRATCH_DIR', str(Path.home() / '.jcode/scratch')))
    scratch.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='overlap evidence relocation ', dir=scratch) as temporary_name:
        temporary = Path(temporary_name)
        clean = temporary / 'standalone relocated bundle'
        shutil.copytree(ROOT, clean)
        relocation = relocated(clean, temporary)
        for name, change, reason in controls:
            mutated = temporary / 'mutated bundle'
            shutil.copytree(clean, mutated)
            change(mutated)
            index_and_seal(mutated)
            try:
                replay.validate(mutated)
            except ValueError as error:
                assert reason in str(error), (name, reason, str(error))
                observed.append(dict(control=name, rejection=str(error), resealed=True))
            else:
                raise AssertionError('semantically invalid resealed evidence admitted: ' + name)
            shutil.rmtree(mutated)
    print(json.dumps(dict(status='PASS', relocation=relocation, negative_controls=len(observed), controls=observed), indent=2))


def mutate_phase_order(root):
    path = state_path(root, 1)
    state = replay.read_json(path)
    state['rows'][1], state['rows'][2] = state['rows'][2], state['rows'][1]
    save(path, state)


def corrupt_restoration(root):
    path = root / 'restoration.json'
    value = replay.read_json(path)
    value['files']['toolchain/resident/crates/ra-jit/src/main.rs']['cmp_head'] = False
    save(path, value)


def corrupt_analysis(root):
    path = root / 'provenance/cold-check-overlap-v54-validation.json'
    value = replay.read_json(path)
    value['rows'][0]['first_percent'] = -99
    save(path, value)


if __name__ == '__main__':
    main()
