#!/usr/bin/env python3
"""Export fixed allowlisted derivatives of existing v65 evidence. Never executes native tools."""
import argparse
import hashlib
import json
import re
from pathlib import Path

ENV = {
    'BLAZE_BACKEND': 'cranelift', 'BLAZE_OPT': '0',
    'CARGO_PROFILE_DEV_DEBUG': '0', 'CARGO_PROFILE_DEV_INCREMENTAL': 'true',
    'CARGO_PROFILE_DEV_OPT_LEVEL': '0', 'CARGO_PROFILE_TEST_DEBUG': '0',
    'CARGO_PROFILE_TEST_INCREMENTAL': 'true', 'CARGO_PROFILE_TEST_OPT_LEVEL': '0',
}
CONTROLS = ('RA_JIT_DEMAND_STD', 'RA_JIT_CACHE_MISS_STATS', 'RA_JIT_STATS',
            'RA_JIT_WHOLE_RELOAD', 'RA_JIT_READY_NOTIFY')
CASES = ('off', 'on', 'on-repeat', 'off-repeat')
MARKER = 'ra-jit ready-notify: observed loading transition'
NATIVE = ('ready_notify_ready_before_wait_and_terminal_errors',
          'ready_notify_wrong_key_generation_and_loading_timeout',
          'ready_notify_during_wait_uses_the_predicate_mutex',
          'ready_notify_generation_transition_during_wait')


def digest(data):
    return hashlib.sha256(data).hexdigest()


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':')).encode()


def collect(root, output):
    output.mkdir(parents=True, exist_ok=True)
    provenance = {}
    read_paths = {}

    def read(relative):
        path = root / relative
        raw = path.read_bytes()
        provenance[relative] = {'sha256': digest(raw), 'bytes': len(raw)}
        read_paths[path] = digest(raw)
        return raw

    def load(relative):
        return json.loads(read(relative))

    def copy(relative, destination):
        path = output / destination
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(read(relative))

    def identity(value):
        return digest(canonical(value))

    result = {'schema': 1, 'decision': 'rejected', 'promotion': False,
              'noise': 'unknown', 'speed_claim': False, 'causal_regression_claim': False,
              'cases': {}, 'native': {}, 'analysis_attempts': [], 'cleanup': {}}
    for name in CASES:
        prefix = f'ready-notify-v65-{name}-seven'
        state = load(prefix + '/state.json')
        rows = []
        for index, row in enumerate(state['rows']):
            phase = row['phase']
            stem = f'{prefix}/{index:02}-{phase}'
            stdout = read(stem + '.stdout').decode()
            stderr = read(stem + '.stderr').decode()
            daemon = read(stem + '-daemon.log').decode()
            # stdout consists of native test output and fresh challenge markers, not env.
            # stderr is a full-line allowlist of diagnostics, never a raw env dump.
            selected = [line for line in stderr.splitlines() if (
                line.startswith('error[E0308]:') or
                re.fullmatch(r'\s*\d+ \| let _: u64 = "JR_MATRIX_TYPE_[0-9a-f]{32}";', line) or
                re.fullmatch(r'assertion `left == right` failed: JR_MATRIX_FAILURE_[0-9a-f]{32}', line))]
            assert 'environment' not in stdout.lower()
            env = {key: row['environment'][key] for key in ENV if key in row['environment']}
            assert env == ENV
            rows.append({key: row[key] for key in (
                'phase', 'nonce', 'accepted', 'status', 'error', 'timed_out',
                'elapsed_seconds', 'raw_error', 'parsed', 'input_unchanged', 'failures')})
            rows[-1].update(stdout=stdout, diagnostic_lines=selected,
                            marker_lines=[line for line in daemon.splitlines() if line == MARKER],
                            safe_environment=env,
                            omitted_environment_entries=len(row['environment']) - len(env),
                            input_identity=identity(row['inputs']),
                            noise_contamination=row['noise']['contamination'])
        result['cases'][name] = {
            'status': state['status'], 'unchanged': state['unchanged'],
            'controls': {key: state['startup_controls'][key] for key in CONTROLS},
            'source_before': identity(state['source_before']),
            'source_after': identity(state['source_after']),
            'tool_before': state['tool_before'], 'tool_after': state['tool_after'],
            'toolchain_before': identity(state['toolchain_before']),
            'toolchain_after': identity(state['toolchain_after']), 'rows': rows,
        }
    baseline = load('ready-notify-v65-baseline-seven/state.json')
    result['baseline'] = {
        'status': baseline['status'], 'unchanged': baseline['unchanged'],
        'binary_sha256': baseline['tool_after'],
        'rows': [{key: row[key] for key in ('phase', 'status', 'accepted', 'elapsed_seconds')}
                 for row in baseline['rows']],
        'scope': 'accepted v58 baseline, distinct executable, not part of same-binary OFF/ON pairs',
    }
    for name in ('daemon.rs.original', 'daemon.rs.candidate', 'candidate.patch',
                 'resident-ready-notify.md.candidate', 'resident-ready-notify.md.original.ABSENT',
                 'FROZEN'):
        copy('ready-notify-v65/' + name, 'frozen/' + name)
    # Preserve distinct historical scripts and empty outputs. Paths in scripts are
    # sanitized derivatives, with exact-original hashes bound in provenance.
    for version in ('v65', 'v65b', 'v65c'):
        script = read(f'validate-ready-notify-{version}.py').decode()
        script = script.replace(str(root), '<PRIVATE_EXPERIMENT_ROOT>')
        script = script.replace('/home/jeremy/justrust', '<PRIVATE_CHECKOUT>')
        dest = output / 'analyses' / f'validate-{version}.py.txt'
        dest.parent.mkdir(exist_ok=True)
        dest.write_text(script)
        copy(f'ready-notify-{version}-validation.json', f'analyses/{version}-output.json')
        result['analysis_attempts'].append({
            'version': version, 'outcome': 'passed' if version == 'v65c' else 'failed',
            'reason': ('raw parser tuple reconstruction before unchanged strict oracle'
                       if version == 'v65c' else 'JSON list versus parser tuple representation mismatch'),
            'stderr_retained': False, 'historical_rerun': False,
        })
    result['root_validation'] = load('ready-notify-v65c-validation.json')
    for gate, folder in (('check', 'candidate-build-v65'), ('compile', 'candidate-build-v65'),
                         ('codegen-native', 'candidate-unit-v65'), ('full-native', 'candidate-unit-v65')):
        raw = read(f'{folder}/{gate}.log').decode()
        code = int(read(f'{folder}/{gate}.exit'))
        summaries = [line for line in raw.splitlines() if line.startswith('justrust: ok:')
                     or line.startswith('  tests   ')]
        result['native'][gate] = {'exit': code, 'summaries': summaries}
    journal = read('candidate-unit-v65/justrust-home/runs/20261031-034253111-448775/output.jsonl')
    result['native']['fixtures'] = [json.loads(line)['l'] for line in journal.splitlines()
                                    if any(('::' + name + ' ...').encode() in line for name in NATIVE)]
    result['native']['source_check_count'] = len(read('candidate-build-v65/all-source-before.sha256').splitlines())
    result['native']['source_check_ok'] = all(line.endswith(b': OK') for line in read('candidate-unit-v65/all-source-after.log').splitlines())
    result['native']['lock_check_ok'] = all(line.endswith(b': OK') for line in read('candidate-unit-v65/lock-after.log').splitlines())
    result['native']['engine_version'] = read('candidate-build-v65/engine-version.txt').decode().strip()
    result['native']['root_lock_sha256'] = read('candidate-build-v65/lock-before.sha256').decode().split()[0]
    result['native']['benchmark_regression_lines'] = [line for line in read('v65-final-bench-regression.log').decode().splitlines() if re.search(r'Ran 733 tests|^OK$', line)]
    # Cleanup is strictly a post-validation operational receipt, not timing evidence.
    for name, prefix in (('baseline', 'completed-ready-notify-v65-baseline-stop'),
                         ('candidate', 'completed-ready-notify-v65-stop')):
        receipt = load(prefix + '/receipt.json')
        # Store only scalar safe operational assertions, not raw paths/PIDs/env.
        result['cleanup'][name] = {
            'stores_deleted': receipt['stores_deleted'],
            'rows': [{key: row[key] for key in
                      ('case', 'post_measurement_only', 'status', 'error', 'timed_out')}
                     for row in receipt['rows']],
        }
    result['restoration'] = {'daemon_sha256': digest(read('ready-notify-v65/daemon.rs.original')),
                             'cmp_original': True, 'head_diff_empty': True,
                             'witness': 'worker and coordinator independent scoped comparisons',
                             'time_utc': '2026-10-31T10:50:22Z'}
    result['privacy'] = {'environment_allowlist': ENV, 'control_allowlist': list(CONTROLS),
        'excluded': ['ambient environment', 'raw state JSON', 'private homes and caches',
                     'raw daemon and worker logs', 'toolchain path listings', 'binaries and targets',
                     'telemetry and credentials'],
        'method': 'fixed-field selection and exact full-line diagnostics, not environment redaction'}
    result['private_originals'] = provenance
    for path, expected in read_paths.items():
        assert digest(path.read_bytes()) == expected, 'private original changed during export'
    (output / 'observations.json').write_text(json.dumps(result, indent=2, sort_keys=True) + '\n')


def manifest(output):
    inventory = {str(path.relative_to(output)): digest(path.read_bytes())
                 for path in output.rglob('*') if path.is_file()
                 and path.name != 'SHA256.json' and '__pycache__' not in path.parts}
    (output / 'SHA256.json').write_text(json.dumps(inventory, indent=2, sort_keys=True) + '\n')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('private_root', type=Path, nargs='?')
    parser.add_argument('--manifest-only', action='store_true')
    parser.add_argument('--output', type=Path, default=Path(__file__).resolve().parent)
    args = parser.parse_args()
    if not args.manifest_only:
        parser.error('private_root required') if args.private_root is None else collect(args.private_root, args.output)
    manifest(args.output)
