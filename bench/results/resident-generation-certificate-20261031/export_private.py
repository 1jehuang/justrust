#!/usr/bin/env python3
"""Offline, explicit-allowlist derivatives. Never run the engine or copy environments."""
import argparse
import hashlib
import json
import re
from pathlib import Path

GROUPS = (
    'whole-generation-v64-off-seven', 'whole-generation-v64-on-seven',
    'manifest-cfg-reload-v64', 'native-link-reload-v62-off',
    'native-link-reload-v62-on', 'native-link-reload-v63-on',
    'native-link-reload-v64-on',
)


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def selected(text):
    """Full-line patterns only. Unrecognized lines are never exported."""
    patterns = (
        r'running \d+ tests',
        r'test summary::tests::[a-z_]+ \.\.\. (?:ok|FAILED|JR_MATRIX_RUNTIME_[0-9a-f]{32})',
        r'(?:ok|FAILED)',
        r'test result: (?:ok|FAILED)\. \d+ passed; \d+ failed; \d+ ignored; \d+ measured; \d+ filtered out; finished in [0-9.]+s',
        r'NATIVE_[0-9a-f]{32}_\d+',
        r'GEN_OK_[0-9a-f]{32}',
        r'error\[E0308\]: expected (?:u32|u64), found &\x27static str',
        r'error: could not compile `(?:justrust|generation-fixture)` \((?:lib test|bin "generation-fixture")\) due to 1 previous error',
        r'generation-certificate result=(?:known|unknown) stage=[a-z-]+',
        r'assertion `left == right` failed: JR_MATRIX_FAILURE_[0-9a-f]{32}',
    )
    out = [line for line in text.splitlines() if any(re.fullmatch(p, line) for p in patterns)]
    # Only the synthetic fixture literals are selected, not arbitrary source/diagnostics.
    out.extend(sorted(set(re.findall(r'JR_(?:MATRIX_TYPE|GENERATION_ERROR)_[0-9a-f]{32}', text))))
    return '\n'.join(out) + ('\n' if out else '')


def daemon(text):
    out = []
    for line in text.splitlines():
        if re.fullmatch(r'generation-parent reason=[a-z-]+(?: [a-z_]+=(?:true|false))+', line):
            out.append(line)
        elif re.fullmatch(r'whole-generation retained-load success pid=\d+ generation=\d+', line):
            out.append(line)
        elif re.fullmatch(r'retained load failed, restarting cold: error\[E0308\]: expected (?:u32|u64), found &\x27static str', line):
            out.append(line)
        else:
            match = re.match(r'worker [0-9a-f]+ of pid (\d+) for ', line)
            if match:
                out.append('worker-start pid=' + match[1])
    return out


def export(root):
    result = {'schema': 1, 'decision': 'rejected-restored', 'timing_claim': 'descriptive-uncontrolled-no-significance', 'groups': {}, 'compilers': [], 'units': [], 'source_inventories': {}, 'process_stops': []}
    for name in GROUPS:
        directory = root / name
        state = json.loads((directory / 'state.json').read_text())
        group = {'state_sha256': digest(directory / 'state.json'), 'recorded_status': state['status'], 'rows': []}
        if isinstance(state.get('unchanged'), bool):
            group['unchanged'] = state['unchanged']
        if name.startswith('whole-generation') or name.startswith('native-link'):
            group['engine_sha256'] = state['tool_before']
        for index, source in enumerate(state['rows']):
            phase = source.get('phase', str(index) + '-' + source['choice'] if 'choice' in source else '')
            if name.startswith('whole-generation'):
                prefix = f'{index:02d}-{phase}'
            elif name == 'manifest-cfg-reload-v64':
                prefix = source['mode'] + '-' + phase
            else:
                prefix = str(index) + '-' + source['choice']
            row = {key: source[key] for key in ('mode', 'nonce', 'literal', 'choice', 'expected', 'status', 'timed_out', 'elapsed_seconds', 'accepted') if key in source}
            row['phase'] = phase
            row['transport_error'] = source.get('error') is not None or source.get('raw_error') is not None
            row['raw_sha256'] = {}
            for stream in ('stdout', 'stderr'):
                path = directory / (prefix + '.' + stream)
                row['raw_sha256'][stream] = digest(path)
                row[stream] = selected(path.read_text(errors='strict'))
            path = directory / (prefix + '-daemon.log')
            row['daemon_events'] = daemon(path.read_text()) if path.exists() else []
            if path.exists():
                row['raw_sha256']['daemon'] = digest(path)
            path = directory / (prefix + '-status.stdout')
            row['worker_pids'] = [int(pid) for pid in re.findall(r'^  worker: pid (\d+),', path.read_text(), re.M)] if path.exists() else []
            if path.exists():
                row['raw_sha256']['status'] = digest(path)
            noise = source.get('noise', {})
            row['noise'] = {key: noise[key] for key in ('contamination', 'child_cpu_seconds', 'other_cpu_seconds_estimate') if key in noise}
            group['rows'].append(row)
        result['groups'][name] = group
    for version in range(60, 65):
        directory = root / f'candidate-build-v{version}'
        log = directory / 'check.log'
        text = log.read_text()
        result['compilers'].append({'version': version, 'check_exit': int((directory / 'check.exit').read_text()), 'check_sha256': digest(log), 'error_codes': re.findall(r'^error\[(E\d+)\]:', text, re.M), 'compile_exit': int((directory / 'compile.exit').read_text()) if (directory / 'compile.exit').exists() else None})
        if version >= 62:
            for kind in ('codegen-native', 'full-native'):
                directory = root / f'candidate-unit-v{version}'
                log = directory / (kind + '.log')
                matches = re.findall(r'^  tests\s+(\d+) passed, (\d+) failed, (\d+) ignored, (\d+) filtered out, (\d+) binary', log.read_text(), re.M)
                result['units'].append({'version': version, 'kind': kind, 'exit': int((directory / (kind + '.exit')).read_text()), 'summaries': [[int(value) for value in summary] for summary in matches], 'log_sha256': digest(log)})
        inventory = root / f'generation-certificate-v{version}' / 'SHA256SUMS'
        if inventory.exists():
            result['source_inventories'][str(version)] = {'inventory_sha256': digest(inventory), 'files': [{'path': line.split(None, 1)[1], 'sha256': line.split(None, 1)[0]} for line in inventory.read_text().splitlines()]}
    stop = json.loads((root / 'completed-generation-v64-stop/receipt.json').read_text())
    result['stores_deleted'] = stop['stores_deleted']
    for row in stop['rows']:
        result['process_stops'].append({key: row[key] for key in ('case', 'post_measurement_only', 'status', 'timed_out')})
    result['restoration'] = {
        'accepted_base': '69669f0c7b28721fc3130eb801311e7e7afca962',
        'verified_at_utc': '2026-10-31T10:18:22Z',
        'method': 'write-from-immutable-originals-then-cmp-and-git-diff-HEAD',
        'rejected_live_ten_preserved': True,
        'files': [{'path': item['path'], 'sha256': digest(root / 'generation-certificate-v60/base-originals' / item['path'])}
                  for item in result['source_inventories']['64']['files'] if item['path'].endswith('.rs')],
    }
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('private_root', type=Path)
    parser.add_argument('--output', type=Path, default=Path(__file__).with_name('observations.json'))
    args = parser.parse_args()
    args.output.write_text(json.dumps(export(args.private_root), indent=2, sort_keys=True) + '\n')


if __name__ == '__main__':
    main()
