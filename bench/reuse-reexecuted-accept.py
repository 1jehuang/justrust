#!/usr/bin/env python3
"""Native retrospective value classification acceptance, never a savings benchmark."""
import argparse
from collections import Counter
import importlib.util
import json
import os
from pathlib import Path
import sys

import query_profile

_spec = importlib.util.spec_from_file_location('reuse_query_accept', Path(__file__).with_name('reuse-query-accept.py'))
_helpers = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_helpers)
run, digest, compare = _helpers.run, _helpers.digest, _helpers.compare

MARKER_FILTER = 'generic-activity,function-args'
MARKERS = tuple('jr_incremental_provider_' + family + '_' + outcome
                for family in ('reexecuted', 'eval_always')
                for outcome in ('same_value', 'different_value', 'no_hash'))
BASE = '''#![allow(dead_code)]
fn value() -> u32 { 7 }
fn untouched() -> u32 { 2 }
fn main() { println!("{}", value() + untouched()); }
'''


def cases():
    return [('initial', BASE, '9\n'), ('unchanged', BASE, '9\n'),
            ('body-edit', BASE.replace('{ 7 }', '{ 8 }'), '10\n'),
            ('type-error', BASE.replace('{ 7 }', '{ "wrong" }'), None),
            ('recovery', BASE, '9\n')]


def inventory(records, resolve):
    """Retain exact counts only. Absent markers are unknown, not measured zero."""
    counts = {marker: Counter() for marker in MARKERS}
    for record in records:
        label = record.get('label')
        if label not in counts:
            continue
        if record.get('kind') != 'GenericActivity' or record.get('record_type') != 'interval':
            raise AssertionError('Native classification marker must be a GenericActivity interval')
        parts = resolve(record['event_id']).split('\x1e')
        if len(parts) != 2 or parts[0] != label or not parts[1] or not parts[1].isidentifier():
            raise AssertionError('Missing or malformed static dep-kind argument')
        counts[label][parts[1]] += 1
    return {marker: dict(evidence='observed' if values else 'unknown-marker-absent',
                         interval_records=sum(values.values()) if values else None,
                         query_name_interval_records=dict(sorted(values.items())))
            for marker, values in counts.items()}


def validate_counts(observations):
    totals = Counter()
    for observation in observations:
        markers = observation['modes']['on']['profile']['markers']
        for marker, value in markers.items():
            count = value['interval_records']
            if count is not None:
                if count != sum(value['query_name_interval_records'].values()):
                    raise AssertionError('Marker partition count mismatch')
                if observation['name'] == 'initial':
                    raise AssertionError('Fresh incremental directory unexpectedly had prior-node markers')
                totals[marker] += count
    # This fixture must exercise real retrospective ordinary equality and change.
    # Other families may be absent; absence never becomes zero-work evidence.
    for suffix in ('same_value', 'different_value'):
        if not totals['jr_incremental_provider_reexecuted_' + suffix]:
            raise AssertionError('Missing native ordinary ' + suffix + ' evidence')
    return dict(totals)


def capture(rustc, root):
    root.mkdir(mode=0o700, parents=False, exist_ok=False)
    report = dict(schema=2, scope='provider-only-retrospective-reexecuted-value-classification',
                  allocation_origin='query-provider-only-feed-and-nonquery-tasks-excluded',
                  verify_ich=True, self_profile_events=MARKER_FILTER, verdict='incomplete', observations=[],
                  provider_elapsed_ns=None, time_saved_ns=None, inputs_changed=None,
                  provider_avoidable=None, capture_complete=None)
    try:
        version, sysroot = run([rustc, '-vV']), run([rustc, '--print', 'sysroot'])
        report['compiler'] = dict(path=str(rustc), sha256=digest(rustc), version=version)
        report['sysroot_query'] = sysroot
        if version['status'] or sysroot['status']:
            raise AssertionError('Compiler provenance query failed')
        drivers = sorted((Path(sysroot['stdout'].strip()) / 'lib').glob('*rustc_driver*'))
        report['drivers'] = [dict(path=str(p.resolve()), sha256=digest(p)) for p in drivers if p.is_file()]
        if not report['drivers']:
            raise AssertionError('Missing exact rustc_driver provenance')
        for mode in ('off', 'on'):
            (root / mode).mkdir(mode=0o700)
        for name, source, expected in cases():
            observation = dict(name=name, source=source, expected_stdout=expected, modes={})
            report['observations'].append(observation)
            for mode in ('off', 'on'):
                work = root / mode
                fixture, binary = work / 'fixture.rs', work / 'fixture'
                fixture.write_text(source)
                prefix = work / name
                command = [rustc, fixture, '--crate-name=reexecuted_fixture', '--edition=2024',
                           '-Zincremental-verify-ich', '-Cdebuginfo=0',
                           '-C', f'incremental={work / "incremental"}', '-o', binary]
                if mode == 'on':
                    command += [f'-Zself-profile={prefix}', f'-Zself-profile-events={MARKER_FILTER}']
                record = dict(compile=run(command))
                observation['modes'][mode] = record
                if not record['compile']['status']:
                    record['artifact_sha256'] = digest(binary)
                    record['runtime'] = run([binary])
                profiles = sorted(work.glob(name + '*.mm_profdata'))
                if prefix.is_dir():
                    profiles += sorted(prefix.glob('*.mm_profdata'))
                if mode == 'off':
                    if profiles:
                        raise AssertionError('Profile-off unexpectedly emitted a capture')
                else:
                    if len(profiles) != 1:
                        raise AssertionError(f'{name}: expected one profile, got {profiles}')
                    raw = query_profile.read_input(str(profiles[0]))
                    events, strings = query_profile.prepare(raw, False, False)
                    record['profile'] = dict(path=str(profiles[0]), sha256=digest(profiles[0]),
                                             bytes=len(raw), markers=inventory(
                                                 query_profile.iter_prepared_records(events, strings, False), strings.resolve))
            off, on = (observation['modes'][m] for m in ('off', 'on'))
            compare(off['compile'], on['compile'], root / 'off', root / 'on', name)
            if expected is None:
                if off['compile']['status'] != 1:
                    raise AssertionError('Expected ordinary type-error status 1')
            else:
                if off['compile']['status']:
                    raise AssertionError('Unexpected compile failure: ' + json.dumps(observation))
                compare(off['runtime'], on['runtime'], root / 'off', root / 'on', name + '/runtime')
                runtime = off['runtime']
                if (runtime['status'], runtime['stdout'], runtime['stderr']) != (0, expected, ''):
                    raise AssertionError('Incorrect runtime result')
        report['observed_marker_totals'] = validate_counts(report['observations'])
        report['verdict'] = 'passed'
    except Exception as error:
        report['verdict'], report['error'] = 'failed', str(error)
        raise
    finally:
        (root / 'acceptance.json').write_text(json.dumps(report, indent=2) + '\n')
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--rustc', '--compiler', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    os.umask(0o077)
    report = capture(args.rustc.resolve(), args.output.resolve())
    print(json.dumps(dict(verdict=report['verdict'], output=str(args.output),
                          observed_marker_totals=report['observed_marker_totals']), indent=2))


if __name__ == '__main__':
    try:
        main()
    except Exception as error:
        print(str(error), file=sys.stderr)
        sys.exit(1)
