#!/usr/bin/env python3
"""Acceptance only: frozen v4 opt-in modes, status identities, and command exits."""
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import time

ROOT = Path(__file__).resolve().parent
BASE = Path('/home/jeremy/rsa01')
SOURCE = Path('/home/jeremy/.jcode/scratch/ba30/source/src/summary.rs')
EXPECTED = 'deafad0c76f20059db6e1a07f4ed9665013ee61600db0089aec3667c38069df8'
FILTER = ['test', '--lib', '--', 'summary::tests::', '--test-threads=1', '--nocapture']


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def identities():
    result = {}
    for path in (BASE / 'engine').rglob('*'):
        if path.is_symlink() or not path.is_file() or not path.stat().st_mode & 0o111:
            continue
        with path.open('rb') as stream:
            if stream.read(4) != b'\x7fELF':
                continue
        st = path.stat()
        result[str(path.relative_to(BASE / 'engine'))] = {
            'sha256': digest(path), 'dev': st.st_dev, 'inode': st.st_ino}
    if not result:
        raise ValueError('no actual executable artifacts found')
    return result


def worker_identity(text):
    daemon = re.findall(r'^daemon: pid (\d+), version (.*?), exits ', text, re.M)
    workers = re.findall(r'^  worker: pid (\d+), (.*?), (?:holds .*?, )?log (.*)$', text, re.M)
    if len(daemon) != 1 or not workers:
        raise ValueError('status did not identify a live daemon and worker')
    return {'daemon': sorted(daemon), 'workers': sorted(workers)}


def run(tool, env, name, args):
    start = time.monotonic()
    done = subprocess.run([str(tool), *args], cwd=SOURCE.parent.parent, env=env,
                          stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=60)
    (ROOT / f'{name}.stdout').write_bytes(done.stdout)
    (ROOT / f'{name}.stderr').write_bytes(done.stderr)
    return {'name': name, 'command': [str(tool), *args], 'status': done.returncode,
            'elapsed_seconds': time.monotonic() - start}, done.stdout.decode(errors='replace'), done.stderr.decode(errors='replace')


def main():
    metadata = json.loads((BASE / 'metadata.json').read_text())
    tool = Path(metadata['tools_before']['engine']['path'])
    if digest(tool) != EXPECTED:
        raise ValueError('not frozen v4')
    env = {k: v for k, v in os.environ.items() if not k.startswith(
        ('JUSTRUST_', 'JR_ENGINE_', 'RA_JIT_', 'BLAZE_', 'CARGO_', 'RUST', 'XDG_'))}
    env.update(metadata['environments']['engine'])
    # Match the root gate's client PATH. A different inherited PATH is explicitly
    # a WorkerConfig input and must not be mistaken for telemetry invalidation.
    env['PATH'] = '/home/jeremy/.local/bin:/home/jeremy/.cargo/bin:/home/jeremy/.local/share/swiftly/bin:/usr/local/sbin:/usr/local/bin:/usr/bin:/usr/bin/site_perl:/usr/bin/vendor_perl:/usr/bin/core_perl'
    for variable, directory in [('XDG_RUNTIME_DIR', 'runtime'), ('XDG_CACHE_HOME', 'cache'),
                                ('XDG_CONFIG_HOME', 'config'), ('XDG_DATA_HOME', 'data')]:
        env[variable] = str(BASE / 'engine' / directory)
    original = SOURCE.read_bytes()
    state = {'scope': 'frozen-v4-telemetry-acceptance-not-speedup', 'rows': [],
             'tool_sha256': EXPECTED, 'source_sha256_before': digest(SOURCE),
             'source_modified_by_gate': False, 'client_path': env['PATH']}
    try:
        settle, stdout, stderr = run(tool, env, 'settle', FILTER)
        assert settle['status'] == 0 and 'test result: ok.' in stdout, (settle, stderr)
        state['settle'] = settle
        before, stdout, stderr = run(tool, env, 'status-before', ['cargo', 'status'])
        assert before['status'] == 0, stderr
        worker = worker_identity(stdout)
        baseline = identities()
        state.update(worker_identity_before=worker, artifact_identity_before=baseline)
        modes = [
            ('off', {}, FILTER, 0, False, False),
            ('stats-only', {'RA_JIT_STATS': '1'}, FILTER, 0, True, False),
            ('trace-only', {'RA_JIT_TRACE': str(ROOT / 'trace-only.json')}, FILTER, 0, False, True),
            ('alias-trace-only', {'BLAZE_TRACE': str(ROOT / 'alias-trace-only.json')}, FILTER, 0, False, True),
            ('returned-error', {'RA_JIT_TRACE': str(ROOT / 'returned-error.json')}, ['cargo', 'settle', 'not-a-pid'], 1, False, True),
            ('refused-command', {'RA_JIT_TRACE': str(ROOT / 'refused-command.json')}, ['test', '--message-format=json'], 101, False, True),
            ('trace-write-error', {'RA_JIT_TRACE': '/dev/null/trace.json'}, FILTER, 0, False, False),
            ('off-again', {}, FILTER, 0, False, False),
        ]
        for name, extra, args, expected_status, stats, trace in modes:
            row, stdout, stderr = run(tool, env | extra, name, args)
            assert row['status'] == expected_status, (name, row['status'], stderr)
            assert ('blaze time:' in stderr) == stats, (name, stderr)
            if args == FILTER:
                assert 'test result: ok.' in stdout and '0 failed' in stdout, (name, stdout, stderr)
            if name == 'returned-error':
                assert '`settle` is told the cargo to wait for' in stderr, stderr
            if name == 'refused-command':
                assert 'does not build with `--message-format`' in stderr, stderr
            if name == 'trace-write-error':
                assert 'the trace is not written' in stderr, stderr
            if trace:
                path = ROOT / f'{name}.json'
                data = json.loads(path.read_text())
                names = {event['name'] for event in data['traceEvents'] if event['ph'] == 'X'}
                assert 'client command' in names, (name, names)
                if args == FILTER:
                    assert {'client metadata', 'client planning', 'client checkout', 'client request',
                            'client whole build', 'client test execution', 'client\u2194daemon'} <= names
                row.update(trace_sha256=digest(path), phase_names=sorted(names))
            else:
                assert 'trace written to' not in stderr, (name, stderr)
            after, status_out, status_err = run(tool, env, f'{name}-status', ['cargo', 'status'])
            assert after['status'] == 0, status_err
            current_worker = worker_identity(status_out)
            assert current_worker == worker, (name, current_worker, worker)
            current_artifacts = identities()
            assert current_artifacts == baseline, (name, 'artifact identity changed')
            row.update(worker_identity_unchanged=True, artifact_identity_unchanged=True)
            state['rows'].append(row)
        # This known limitation is observed, retained, and explicitly not counted as
        # successful error finalization. No manifest or source file is modified.
        row, stdout, stderr = run(tool, env | {'RA_JIT_TRACE': str(ROOT / 'metadata-hard-exit.json')},
                                  'metadata-hard-exit', ['test', '--manifest-path', '/nonexistent/resident-trace-missing.toml'])
        assert row['status'] == 101 and not (ROOT / 'metadata-hard-exit.json').exists(), (row, stderr)
        assert 'error' in stderr.lower(), stderr
        state['known_metadata_hard_exit'] = row | {'trace_missing': True, 'expected_limitation': True}
        state['status'] = 'passed'
    except BaseException as error:
        state.update(status='failed', error=repr(error))
        raise
    finally:
        state.update(source_sha256_after=digest(SOURCE), source_unchanged=SOURCE.read_bytes() == original,
                     tool_sha256_after=digest(tool))
        (ROOT / 'acceptance.json').write_text(json.dumps(state, indent=2) + '\n')
    assert state['source_unchanged'] and state['tool_sha256_after'] == EXPECTED
    print(json.dumps({'status': state['status'], 'rows': len(state['rows']),
                      'source_unchanged': state['source_unchanged'], 'metadata_hard_exit_retained': True}))


if __name__ == '__main__':
    main()
