#!/usr/bin/env python3
"""Offline re-signed mutation controls and isolated relocated replay, no native calls."""
import copy
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('semantic_v25_validate', HERE / 'validate.py')
v = importlib.util.module_from_spec(spec)
spec.loader.exec_module(v)


def load(root, name):
    return json.loads((root / name).read_text())


def save(root, name, data):
    (root / name).write_text(json.dumps(data, indent=2) + '\n')


def replace(root, name, old, new):
    path = root / name
    text = path.read_text()
    assert old in text, (name, old)
    path.write_text(text.replace(old, new, 1))


def mutate_json(root, name, change):
    data = load(root, name)
    change(data)
    save(root, name, data)


def resign(root):
    """Repair all mutable receipt hashes so controls exercise semantic constraints."""
    for name in ('rsb16', 'rsa22'):
        path = f'fixed-five/{name}/suite.json'
        suite = load(root, path)
        for artifact in suite['artifacts_sha256']:
            suite['artifacts_sha256'][artifact] = v.sha(root / f'fixed-five/{name}' / artifact)
        save(root, path, suite)
    state = load(root, 'profile/state.json')
    for index, row in enumerate(state['rows']):
        trace = root / f'profile/{index}-new.trace.json'
        row['trace_sha256'] = v.sha(trace)
        row['worker_delta_sha256'] = v.sha(root / f'profile/{index}-new.worker-delta.log')
        row['events'] = [e for e in json.loads(trace.read_text())['traceEvents'] if e.get('ph') == 'X' and e.get('name', '').startswith('diagnostics.')]
    save(root, 'profile/state.json', state)
    exports = load(root, 'offline-export.json')
    for item in exports:
        summary = load(root, item['source'])
        item.update(summary_sha256=v.sha(root / item['source']), tests=summary.get('tests'), exit=summary['exit'], args=summary['args'])
    save(root, 'offline-export.json', exports)
    collection = load(root, 'collection.json')
    for item in collection['files']:
        item['sha256'] = v.sha(root / item['artifact'])
    save(root, 'collection.json', collection)
    v.sign(root)


def trace_change(root, change, index=2):
    path = f'profile/{index}-new.trace.json'
    data = load(root, path)
    change(data['traceEvents'])
    save(root, path, data)


def source_guard(root):
    replace(root, 'source-ready/frozen/' + v.LIB,
            'parse.syntax().descendants().filter(|_| bodies.is_none())',
            'parse.syntax().descendants().filter(|_| true)')
    path = root / 'source-ready/frozen.sha256'
    path.write_text(''.join(f'{v.sha(root / "source-ready/frozen" / name)}  ./{name}\n' for name in v.ACCEPTED))


def dropped(root):
    replace(root, 'profile/2-new.worker-delta.log', 'dropped=0 cap=4000', 'dropped=1 cap=4000')
    mutate_json(root, 'profile/state.json', lambda s: s['rows'][2].update(transport=[[9, 0, 1]]))


def collapse_files(root):
    path = root / 'profile/0-new.worker-delta.log'
    lines = path.read_text().splitlines()
    lines = [line for line in lines if 'FileId(33)' not in line]
    lines = [line.replace('asked_callback_calls=373', 'asked_callback_calls=604') for line in lines]
    path.write_text('\n'.join(lines) + '\n')
    mutate_json(root, 'profile/state.json', lambda s: s['rows'][0].update(callbacks=[[604, 393471]]))


def wrong_thread(root):
    trace_change(root, lambda es: next(e for e in es if e.get('name') == v.LABELS[3]).update(tid=999999))


def outside_outer(root):
    trace_change(root, lambda es: next(e for e in es if e.get('name') == v.LABELS[3]).update(ts=1))


def overlap(root):
    def change(es):
        event = next(e for e in es if e.get('name') == v.LABELS[2])
        event['ts'] -= 10
    trace_change(root, change)


def duplicate_phase(root):
    def change(es):
        es.append(copy.deepcopy(next(e for e in es if e.get('name') == v.LABELS[3])))
    trace_change(root, change)


def remove_marker(root):
    replace(root, 'fixed-five/rsa22/engine-edit-0.stdout', 'JR_RUNTIME_EXECUTED_1', 'REMOVED_RUNTIME_MARKER')


def negative_succeeds(root):
    mutate_json(root, 'fixed-five/rsa22/records.json',
                lambda rows: next(r for r in rows if r['stack'] == 'engine' and r['phase'] == 'type-error').update(status=0))


def snapshot_delta(root):
    mutate_json(root, 'native/v25-snapshot/source-copy-before.json',
                lambda data: data['selected'].update({'crates/ide-diagnostics/src/lib.rs': '0' * 64}))


def native_count(root):
    mutate_json(root, 'journals/v25-snapshot/20261030-133629708-3652589/summary.json',
                lambda s: s['tests'].update(passed=350))


CONTROLS = [
    ('reintroduced_semantic_guard', source_guard, 'accepted frozen source anchor'),
    ('altered_four_file_patch', lambda r: replace(r, 'source-ready/four-paths.patch', 'Some(&mut intervals)', 'None'), 'four-file patch anchor'),
    ('snapshot_nonlock_delta', snapshot_delta, 'private delta not lock-only'),
    ('private_lock_corrupt', lambda r: replace(r, 'snapshot/Cargo.lock.private', 'version = 4', 'version = 3'), 'retained lock mismatch'),
    ('full351_count_downgrade', native_count, 'native test counts/exit'),
    ('actual_api_test_missing', lambda r: replace(r, 'journals/v25-snapshot/20261030-133307142-3479390/output.jsonl',
                                               'resident_semantic_subphases_all_bodies_exact', 'renamed_not_actual_api'), 'actual API test absent'),
    ('runtime_marker_missing', remove_marker, 'runtime edit marker absent/duplicated'),
    ('type_error_false_success', negative_succeeds, 'expected negative gate succeeded'),
    ('phase_label_missing', lambda r: trace_change(r, lambda es: next(e for e in es if e.get('name') == v.LABELS[3]).update(name='diagnostics.semantic.wrong')), 'same-thread five actual phases missing/duplicated'),
    ('phase_wrong_thread', wrong_thread, 'same-thread five actual phases missing/duplicated'),
    ('phase_outside_outer', outside_outer, 'phase outside actual same-thread semantic outer'),
    ('phase_overlap', overlap, 'actual phases not contiguous in label order'),
    ('phase_duplicate', duplicate_phase, 'same-thread five actual phases missing/duplicated'),
    ('transport_drop', dropped, 'profile drops/stale/retained counts'),
    ('two_files_collapsed', collapse_files, 'profile actual file callback rows'),
]


def main():
    scratch = Path(os.environ.get('JCODE_SCRATCH_DIR', '/home/jeremy/.jcode/scratch'))
    report = dict(scope=__doc__, baseline=v.validate(HERE)['verdict'], mutations=[])
    for name, mutation, message in CONTROLS:
        with tempfile.TemporaryDirectory(prefix='semantic-v25-control-', dir=scratch) as temporary:
            target = Path(temporary) / 'evidence'
            shutil.copytree(HERE, target, ignore=shutil.ignore_patterns('__pycache__'))
            mutation(target)
            resign(target)
            try:
                v.validate(target)
            except ValueError as error:
                assert message in str(error), (name, message, str(error))
                report['mutations'].append(dict(name=name, verdict='rejected', reason=str(error), all_mutable_hashes_resigned=True))
            else:
                raise AssertionError(f'mutation accepted: {name}')
    with tempfile.TemporaryDirectory(prefix='semantic-v25-relocated-', dir=scratch) as temporary:
        target = Path(temporary) / 'elsewhere' / 'evidence'
        shutil.copytree(HERE, target, ignore=shutil.ignore_patterns('__pycache__'))
        result = subprocess.run([sys.executable, '-B', str(target / 'validate.py')],
                                cwd=temporary, text=True, capture_output=True, check=True)
        validation = json.loads(result.stdout)
        assert validation['verdict'] == 'passed'
        report['relocation'] = dict(verdict='passed', independent_cwd=True,
                                    actual_validator_from_relocated_bundle=True,
                                    no_original_scratch_or_repository_reads=True)
    report['verdict'] = 'passed'
    print(json.dumps(report, indent=2))


if __name__ == '__main__':
    main()
