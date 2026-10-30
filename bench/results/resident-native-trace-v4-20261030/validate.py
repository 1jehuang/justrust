#!/usr/bin/env python3
"""Offline verification of retained native telemetry acceptance, not a speed gate."""
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent
EXPECTED = 'deafad0c76f20059db6e1a07f4ed9665013ee61600db0089aec3667c38069df8'


def require(condition, message):
    if not condition:
        raise ValueError(message)


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def validate_trace(data):
    require(isinstance(data.get('traceEvents'), list), 'missing trace events')
    events = [event for event in data['traceEvents'] if event.get('ph') == 'X']
    require(bool(events), 'empty phase events')
    for event in events:
        require(isinstance(event.get('name'), str), 'unnamed phase')
        require(isinstance(event.get('dur'), (int, float)) and event['dur'] >= 0, 'invalid duration')
        require(isinstance(event.get('ts'), (int, float)), 'invalid start')
    names = {event['name'] for event in events}
    require('client command' in names, 'missing owning command phase')
    meta = data['raJit']
    require(meta['total_us'] >= 0, 'negative wall time')
    require(sum(row[1] for row in meta['breakdown']) == meta['total_us'], 'breakdown does not sum to wall time')
    return names


def verify_trace(path, expected_sha):
    require(sha(path) == expected_sha, f'changed trace: {path}')
    return validate_trace(json.loads(path.read_text()))


def main():
    state = json.loads((ROOT / 'acceptance.json').read_text())
    require(state['status'] == 'passed' and state['source_unchanged'], 'lightweight gate failed')
    require(state['tool_sha256'] == state['tool_sha256_after'] == EXPECTED, 'wrong frozen binary')
    require(state['source_sha256_before'] == state['source_sha256_after'], 'source changed')
    expected = {'off': 0, 'stats-only': 0, 'trace-only': 0, 'alias-trace-only': 0,
                'returned-error': 1, 'refused-command': 101, 'trace-write-error': 0, 'off-again': 0}
    require(len(state['rows']) == len(expected), 'missing mode rows')
    require({row['name']: row['status'] for row in state['rows']} == expected, 'mode status mismatch')
    for row in state['rows']:
        name = row['name']
        require(row['worker_identity_unchanged'] and row['artifact_identity_unchanged'], 'identity changed')
        stderr = (ROOT / f'{name}.stderr').read_text()
        require(('blaze time:' in stderr) == (name == 'stats-only'), 'stats output escaped opt-in')
        if 'trace_sha256' in row:
            verify_trace(ROOT / f'{name}.json', row['trace_sha256'])
    limitation = state['known_metadata_hard_exit']
    require(limitation['status'] == 101 and limitation['trace_missing'], 'lost metadata hard-exit control')
    require(not (ROOT / 'metadata-hard-exit.json').exists(), 'metadata hard-exit unexpectedly changed')
    off = ROOT / 'root-off-on-off'
    old = json.loads((off / 'state.json').read_text())
    require(old['status'] == 'passed' and old['source_restored'], 'root off/on/off failed')
    require(old['tool_before'] == old['tool_after'] == EXPECTED, 'root binary changed')
    baseline = next(row['artifacts'] for row in old['rows'] if row['mode'] == 'off')
    require([row['mode'] for row in old['rows']] == ['settle', 'off', 'trace', 'alias', 'off-again'], 'root modes missing')
    for row in old['rows']:
        require(row['status'] == 0 and not row['timed_out'], 'root command failed')
        if row['mode'] != 'settle':
            require(row['artifacts'] == baseline, 'root executable identity changed')
        if 'trace_sha256' in row:
            verify_trace(off / f"{row['mode']}.trace.json", row['trace_sha256'])
    errors = ROOT / 'root-errors-recovery'
    error_state = json.loads((errors / 'state.json').read_text())
    require(error_state['status'] == 'passed' and error_state['source_restored'], 'error recovery failed')
    require({row['mode']: row['status'] for row in error_state['rows']} == {
        'body': 0, 'type-error': 101, 'type-recovery': 0, 'assert-failure': 101, 'assert-recovery': 0}, 'error statuses changed')
    for row in error_state['rows']:
        names = verify_trace(errors / f"{row['mode']}.json", row['trace_sha256'])
        require(('client test execution' in names) == (row['mode'] != 'type-error'), 'wrong actual execution phases')
        if row['mode'] == 'body':
            require(any(name.startswith('worker ') for name in names) and 'daemon' in names, 'missing remote process spans')
    compile_stderr = (errors / 'type-error.stderr').read_text()
    require('error[E0308]: expected u64' in compile_stderr and 'JR_RUNTIME_TYPE_ERROR' in compile_stderr,
            'compile diagnostic missing')
    require('test result: FAILED.' in (errors / 'assert-failure.stdout').read_text(), 'test failure diagnostic missing')
    control = json.loads((ROOT / 'old-missing-trace-control' / 'state.json').read_text())
    require(control['status'] == 'failed' and not control['rows'], 'lost old failed trace control')
    failed = json.loads((ROOT / 'initial-unsettled-control' / 'acceptance.json').read_text())
    require(failed['status'] == 'failed' and not failed['rows'], 'lost PATH identity negative control')
    print(json.dumps({'status': 'passed', 'lightweight_modes': len(expected),
                      'root_opt_in_rows': len(old['rows']), 'root_edit_error_rows': len(error_state['rows']),
                      'known_limitation': 'metadata hard exit preserves status101 but skips trace'}))


if __name__ == '__main__':
    main()
