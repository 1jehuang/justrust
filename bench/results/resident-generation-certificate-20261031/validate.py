#!/usr/bin/env python3
"""Validate portable observations offline. No native commands or private files required."""
import argparse
import hashlib
import json
import math
import re
from pathlib import Path

PHASES = ['cold-prepare', 'repeated-noop-1', 'manifest-feature-add', 'type-error', 'type-recovery', 'assert-failure', 'assert-recovery']
TESTS = ['summary::tests::parses_libtest_results', 'summary::tests::shares_overlapping_time', 'summary::tests::splits_passes_without_double_counting', 'summary::tests::test_failures_are_not_compile_errors']
HEX = re.compile(r'[0-9a-f]{64}')


def require(condition, message):
    if not condition:
        raise ValueError(message)


def parsed_tests(text):
    tests = []
    pending = None
    for line in text.splitlines():
        match = re.fullmatch(r'test (summary::tests::[a-z_]+) \.\.\. (ok|FAILED|JR_MATRIX_RUNTIME_[0-9a-f]{32})', line)
        if match:
            require(pending is None, 'unfinished test line')
            if match[2].startswith('JR_'):
                pending = match[1]
            else:
                tests.append([match[1], match[2]])
        elif pending and line in ('ok', 'FAILED'):
            tests.append([pending, line])
            pending = None
    require(pending is None, 'missing test result after nonce')
    summaries = re.findall(r'^test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured; (\d+) filtered out; finished in [0-9.]+s$', text, re.M)
    return tests, summaries


def common(row):
    require(row['accepted'] is True, 'original harness did not accept row')
    require(row['timed_out'] is False and row['transport_error'] is False, 'timeout/transport failure')
    require(re.fullmatch(r'[0-9a-f]{32}', row['nonce']) is not None, 'nonce invalid')
    require(isinstance(row['elapsed_seconds'], (float, int)) and math.isfinite(row['elapsed_seconds']) and row['elapsed_seconds'] >= 0, 'invalid duration')
    require(all(HEX.fullmatch(value) for value in row['raw_sha256'].values()), 'raw digest invalid')
    require(all(type(pid) is int and pid > 0 for pid in row['worker_pids']), 'invalid PID')


def validate(data):
    require(data['schema'] == 1 and data['decision'] == 'rejected-restored', 'not rejection evidence')
    require(data['timing_claim'] == 'descriptive-uncontrolled-no-significance', 'unsupported timing claim')
    serialized = json.dumps(data)
    require('/home/' not in serialized and 'private_root' not in serialized and 'environment' not in serialized, 'private metadata exported')
    require(set(data) == {'schema', 'decision', 'timing_claim', 'groups', 'compilers', 'units', 'source_inventories', 'process_stops', 'stores_deleted', 'restoration'}, 'unexpected top-level metadata')
    groups = data['groups']
    expected_groups = {'whole-generation-v64-off-seven', 'whole-generation-v64-on-seven', 'manifest-cfg-reload-v64', 'native-link-reload-v62-off', 'native-link-reload-v62-on', 'native-link-reload-v63-on', 'native-link-reload-v64-on'}
    require(set(groups) == expected_groups, 'group coverage changed')
    nonces = set()
    for group in groups.values():
        require(group['recorded_status'] == 'passed', 'original group failed')
        require(HEX.fullmatch(group['state_sha256']), 'state digest invalid')
        require(set(group) <= {'state_sha256', 'recorded_status', 'rows', 'unchanged', 'engine_sha256'}, 'unexpected group metadata')
        for row in group['rows']:
            require(set(row) <= {'mode', 'nonce', 'literal', 'choice', 'expected', 'status', 'timed_out', 'elapsed_seconds', 'accepted', 'phase', 'transport_error', 'raw_sha256', 'stdout', 'stderr', 'daemon_events', 'worker_pids', 'noise'}, 'unexpected row metadata')
            require(set(row['noise']) <= {'contamination', 'child_cpu_seconds', 'other_cpu_seconds_estimate'}, 'unexpected noise metadata')
            require(set(row['raw_sha256']) == {'stdout', 'stderr', 'daemon', 'status'}, 'raw evidence coverage changed')
            common(row)
            require(row['nonce'] not in nonces, 'nonce replay')
            nonces.add(row['nonce'])
    off = groups['whole-generation-v64-off-seven']
    on = groups['whole-generation-v64-on-seven']
    require(off['engine_sha256'] == on['engine_sha256'] and HEX.fullmatch(on['engine_sha256']), 'not same-binary OFF/ON')
    for group in (off, on):
        require(group['unchanged'] is True, 'fixture/toolchain restoration failed')
        require([row['phase'] for row in group['rows']] == PHASES, 'seven-state coverage/order changed')
        for row in group['rows']:
            phase = row['phase']
            tests, summaries = parsed_tests(row['stdout'])
            if phase == 'type-error':
                require(row['status'] == 101 and not tests and not summaries, 'type error status/test leak')
                require('error[E0308]' in row['stderr'] and 'JR_MATRIX_TYPE_' + row['nonce'] in row['stderr'], 'type error diagnostic/literal missing')
            else:
                failed = phase == 'assert-failure'
                require(row['status'] == (101 if failed else 0), 'wrong test exit')
                expected = [[name, 'FAILED' if failed and name == TESTS[1] else 'ok'] for name in TESTS]
                require(tests == expected, 'exact four-test tuple changed')
                require(summaries == [('FAILED' if failed else 'ok', '3' if failed else '4', '1' if failed else '0', '0', '0', '144')], 'exact test totals changed')
                require(row['stdout'].count('JR_MATRIX_RUNTIME_' + row['nonce']) == 1, 'runtime nonce absent/replayed')
                require('error[E0308]' not in row['stderr'], 'unexpected compiler error')
                if failed:
                    require('JR_MATRIX_FAILURE_' + row['nonce'] in row['stderr'], 'assertion diagnostic missing')
            require(row['noise'].get('contamination') == 'unknown', 'unsupported quiet-machine claim')
    # Positive changed-generation retention is completion + unchanged actual status PID,
    # not the preceding eligibility line. Cumulative daemon snapshots are compared.
    before, changed = on['rows'][1:3]
    require(len(before['worker_pids']) == 1 and changed['worker_pids'] == before['worker_pids'], 'worker was not retained')
    new_events = changed['daemon_events'][len(before['daemon_events']):]
    require(changed['daemon_events'][:len(before['daemon_events'])] == before['daemon_events'], 'daemon prefix changed')
    marker = [re.fullmatch(r'whole-generation retained-load success pid=(\d+) generation=(\d+)', event) for event in new_events]
    marker = [match for match in marker if match]
    require(len(marker) == 1 and int(marker[0][1]) == before['worker_pids'][0], 'completed retention marker absent/wrong PID')
    require(any('generation_equal=false' in event and 'reason=retained-load-candidate' in event for event in new_events), 'not changed-generation request')
    fixture = groups['manifest-cfg-reload-v64']['rows']
    require([(row['mode'], row['phase']) for row in fixture] == [(mode, phase) for mode in ('original', 'off', 'on') for phase in ('valid', 'manifest-error', 'recovery')], 'manifest fallback coverage changed')
    for row in fixture:
        failing = row['phase'] == 'manifest-error'
        require(row['status'] == (101 if failing else 0), 'manifest diagnostic status lost')
        if failing:
            require('error[E0308]' in row['stderr'] and row['literal'] in row['stderr'], 'manifest error diagnostic missing')
            require('GEN_OK_' not in row['stdout'], 'invalid manifest ran program')
        else:
            require(row['stdout'] == 'GEN_OK_' + row['nonce'] + '\n', 'manifest runtime nonce mismatch')
    valid, failing, recovery = fixture[-3:]
    additions = failing['daemon_events'][len(valid['daemon_events']):]
    require(failing['daemon_events'][:len(valid['daemon_events'])] == valid['daemon_events'], 'fallback prefix changed')
    require(sum(event.startswith('retained load failed, restarting cold: error[E0308]') for event in additions) == 1, 'single cold fallback not established')
    require(sum(event.startswith('worker-start pid=') for event in additions) == 1, 'fallback did not start exactly one cold worker')
    require(not any('retained-load success' in event for event in additions), 'failed load incorrectly marked successful')
    require(valid['worker_pids'] and recovery['worker_pids'] and valid['worker_pids'] != recovery['worker_pids'], 'recovery reused failed worker')
    for name in sorted(expected_groups):
        if not name.startswith('native-link'):
            continue
        rows = groups[name]['rows']
        require(len(rows) == 3 and [row['choice'] for row in rows] == ['a', 'b', 'a'], 'native A/B/A coverage changed')
        for row, expected in zip(rows, (11, 22, 11)):
            require(row['status'] == 0 and row['expected'] == expected and row['stdout'] == f'NATIVE_{row["nonce"]}_{expected}\n', 'native value/nonce/status mismatch')
            if name == 'native-link-reload-v63-on':
                require('generation-certificate result=unknown stage=plan' in row['stderr'], 'client plan failure evidence absent')
        if name == 'native-link-reload-v64-on':
            require(all(row['worker_pids'] == rows[0]['worker_pids'] and len(row['worker_pids']) == 1 for row in rows), 'native-script worker retention absent')
    compilers = data['compilers']
    require([row['version'] for row in compilers] == list(range(60, 65)), 'compiler iteration coverage changed')
    require(compilers[0]['check_exit'] == 101 and compilers[0]['error_codes'] == ['E0599', 'E0599'], 'v60 actual Context failure absent')
    require(compilers[1]['check_exit'] == 101 and compilers[1]['error_codes'] == ['E0282'], 'v61 actual type failure absent')
    for row in compilers[2:]:
        require(row['check_exit'] == 0 and row['compile_exit'] == 0 and row['error_codes'] == [], 'successful compiler gate changed')
    require(len(data['units']) == 6, 'unit coverage changed')
    for row, expected in zip(data['units'], ((62, 'codegen-native', 53), (62, 'full-native', 374), (63, 'codegen-native', 53), (63, 'full-native', 374), (64, 'codegen-native', 53), (64, 'full-native', 375))):
        version, kind, count = expected
        require((row['version'], row['kind'], row['exit'], row['summaries']) == (version, kind, 0, [[count, 0, 0, 0, 1]]), 'native test count/status softened')
    require(data['stores_deleted'] is False and len(data['process_stops']) == 2, 'stop receipt coverage changed')
    require(all(row['post_measurement_only'] is True and row['status'] == 0 and row['timed_out'] is False for row in data['process_stops']), 'post-validation stop failed')
    restoration = data['restoration']
    require(restoration['accepted_base'] == '69669f0c7b28721fc3130eb801311e7e7afca962' and restoration['rejected_live_ten_preserved'] is True, 'accepted base/live preservation changed')
    require(len(restoration['files']) == 9 and len({item['path'] for item in restoration['files']}) == 9, 'nine-source restoration coverage changed')
    for inventory in list(data['source_inventories'].values()) + [restoration]:
        for item in inventory['files']:
            require(not Path(item['path']).is_absolute() and '..' not in Path(item['path']).parts and HEX.fullmatch(item['sha256']), 'nonportable source inventory')
    return {'correctness_rows': sum(len(group['rows']) for group in groups.values()), 'same_binary_seven_state_rows': 14, 'decision': data['decision'], 'speedup_claim': False}


def verify_manifest(directory):
    manifest = directory / 'SHA256SUMS'
    for line in manifest.read_text().splitlines():
        expected, name = line.split('  ', 1)
        require(Path(name).name == name and name != 'SHA256SUMS', 'unsafe manifest path')
        require(hashlib.sha256((directory / name).read_bytes()).hexdigest() == expected, 'package checksum mismatch: ' + name)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--observations', type=Path, default=Path(__file__).with_name('observations.json'))
    parser.add_argument('--manifest', action='store_true')
    args = parser.parse_args()
    if args.manifest:
        verify_manifest(args.observations.parent)
    print(json.dumps(validate(json.loads(args.observations.read_text())), sort_keys=True))


if __name__ == '__main__':
    main()
