"""Mock-only tests, not native compiler acceptance results."""
import copy
import json
from types import SimpleNamespace

import pytest

import stable_resident_owned as owned


def execution(status=0, stdout='', stderr=''):
    return dict(status=status, stdout_b64=owned.shared.encoded(stdout.encode()),
                stderr_b64=owned.shared.encoded(stderr.encode()))


def pair(expected='true:false', status=0):
    result = {}
    for mode in ('fresh', 'owned'):
        row = result[mode] = dict(compile=execution(status))
        if expected is not None:
            row['runtime'] = dict(nonce=mode,
                execution=execution(stdout=mode + ':' + expected + '\n'))
    result['owned']['compile']['pid'] = 42
    return result


def test_valid_pair_and_error():
    owned.validate_pair(pair(), 'true:false', 42)
    owned.validate_pair(pair(None, 1), None, 42)


@pytest.mark.parametrize('change', ['pid', 'diagnostic', 'status', 'timeout', 'runtime',
                                   'runtime-stderr', 'runtime-status', 'nonce'])
def test_reject_mismatch(change):
    rows = pair()
    row = rows['owned']
    if change == 'pid':
        row['compile']['pid'] = 43
    elif change == 'diagnostic':
        row['compile']['stderr_b64'] = owned.shared.encoded(b'different diagnostic')
    elif change == 'status':
        row['compile']['status'] = 101
    elif change == 'timeout':
        row['compile']['timed_out'] = True
    elif change == 'runtime':
        row['runtime']['execution']['stdout_b64'] = owned.shared.encoded(b'stale output\n')
    elif change == 'runtime-stderr':
        row['runtime']['execution']['stderr_b64'] = owned.shared.encoded(b'unexpected')
    elif change == 'runtime-status':
        row['runtime']['execution']['status'] = 1
    else:
        row['runtime'] = copy.deepcopy(rows['fresh']['runtime'])
    with pytest.raises(AssertionError):
        owned.validate_pair(rows, 'true:false', 42)


@pytest.mark.parametrize('status', [0, 101, -11])
def test_error_must_be_ordinary(status):
    with pytest.raises(AssertionError, match='ordinary signature error'):
        owned.validate_pair(pair(None, status), None, 42)


def test_owned_counters_not_typed_hits():
    assert owned.validate_owned(dict(revisions=0, contexts=0))
    assert owned.validate_owned(dict(revisions=2, contexts=1), dict(revisions=1, contexts=1))
    for stats in [None, {}, dict(revisions=True, contexts=0), dict(revisions=1, contexts=-1)]:
        with pytest.raises(AssertionError):
            owned.validate_owned(stats)
    with pytest.raises(AssertionError, match='decreased'):
        owned.validate_owned(dict(revisions=0, contexts=1), dict(revisions=1, contexts=1))


def test_workload_contract(tmp_path, monkeypatch):
    rows = owned.cases()
    assert len(rows) <= 12
    assert rows[2][1] != rows[1][1]
    assert rows[3][2] != rows[2][2]
    assert rows[2][3] != rows[1][3]
    error = next(i for i, row in enumerate(rows) if row[0] == 'signature-error')
    assert rows[error][3] is None
    assert rows[error + 1][0] == 'same-pid-recovery'
    assert not any('incremental' in arg for arg in owned.arguments(tmp_path))
    for key in ('FD', 'FRONTEND', 'VALUES'):
        monkeypatch.setenv('JUSTRUST_RUSTC_RESIDENT_' + key, '1')
    assert not any(key.startswith('JUSTRUST_RUSTC_RESIDENT_') for key in owned.environment())


@pytest.mark.parametrize('fail', [False, True])
def test_mock_capture_receipt_and_recovery(tmp_path, monkeypatch, fail):
    commands = []
    class MockWorker:
        def __init__(self, compiler, root, timeout):
            self.process = SimpleNamespace(pid=42)
            self.command = [str(compiler), '--jr-resident-owned']
            self.owned = None
            self.closed = False
            workers.append(self)
        def compile(self, argv):
            commands.append(list(argv))
            result = compile_result()
            result['pid'] = 43 if fail else 42
            return result
        def close(self):
            assert not self.closed
            self.closed = True
    workers = []
    def compile_result():
        status = 1 if '-> u32' in (tmp_path / 'helper.rs').read_text() else 0
        if not status:
            (tmp_path / 'fixture').write_bytes(b'mock executable')
        return execution(status, stderr='signature mismatch\n' if status else '')
    def run(command, timeout, cwd=None, env=None):
        assert cwd == tmp_path
        commands.append(list(command[1:]))
        return compile_result()
    def runtime(binary, timeout, root):
        source = (root / 'fixture.rs').read_text()
        helper = (root / 'helper.rs').read_text()
        value = ('false' if '-> bool { false }' in source else 'true') + ':'
        value += 'false' if '{ false }' in helper else 'true'
        nonce = str(len(commands))
        return dict(nonce=nonce, execution=execution(stdout=nonce + ':' + value + '\n'))
    monkeypatch.setattr(owned, 'OwnedResident', MockWorker)
    monkeypatch.setattr(owned.shared, 'identity', lambda compiler: dict(mock=True))
    monkeypatch.setattr(owned.shared, 'run', run)
    monkeypatch.setattr(owned, 'runtime', runtime)
    if fail:
        with pytest.raises(AssertionError, match='PID changed'):
            owned.capture(tmp_path / 'mock-rustc', tmp_path)
    else:
        report = owned.capture(tmp_path / 'mock-rustc', tmp_path)
        assert len(report['observations']) == 8
        assert report['session']['owned'] is None
        assert all(command == commands[0] for command in commands)
        assert 'Not evidence' in report['claim']
    assert workers[0].closed
    receipt = json.loads((tmp_path / 'receipt.json').read_text())
    assert receipt['verdict'] == ('failed' if fail else 'passed')
    assert receipt['observations']


def test_runtime_nonce_per_execution(monkeypatch, tmp_path):
    calls = []
    monkeypatch.setattr(owned.shared, 'run', lambda command, *args, **kwargs: calls.append(command))
    first = owned.runtime(tmp_path / 'fixture', 1, tmp_path)
    second = owned.runtime(tmp_path / 'fixture', 1, tmp_path)
    assert first['nonce'] != second['nonce']
    assert calls[0][1] == first['nonce']
    assert calls[1][1] == second['nonce']


@pytest.mark.parametrize('mutation', [None, 'pid', 'id', 'status', 'elapsed', 'owned'])
def test_mock_dedicated_fd_protocol(tmp_path, monkeypatch, mutation):
    """Mock Popen writes a real pipe, never invokes a compiler or fake native build."""
    import io
    import os

    class Input(io.BytesIO):
        def __init__(self, fd, stdout, stderr):
            super().__init__()
            self.fd, self.stdout, self.stderr = fd, stdout, stderr
        def write(self, payload):
            request = json.loads(payload)
            frame = dict(id=request['id'], pid=42, status=0, elapsed_seconds=0.001,
                         owned=dict(revisions=request['id'], contexts=1))
            if mutation in ('pid', 'id'):
                frame[mutation] = 999
            elif mutation == 'status':
                frame['status'] = True
            elif mutation == 'elapsed':
                frame['elapsed_seconds'] = float('nan')
            elif mutation == 'owned':
                frame['owned']['contexts'] = -1
            self.stdout.write(b'raw stdout\n')
            self.stderr.write(b'raw stderr\n')
            os.write(self.fd, json.dumps(frame).encode() + b'\n')
            return len(payload)
        def close(self):
            if not self.closed:
                os.close(self.fd)
            super().close()

    def popen(command, **kwargs):
        assert command == ['/mock/compiler', '--jr-resident-owned']
        fd = kwargs['pass_fds'][0]
        assert kwargs['env']['JUSTRUST_RUSTC_RESIDENT_FD'] == str(fd)
        assert kwargs['cwd'] == tmp_path
        return SimpleNamespace(pid=42, returncode=0,
            stdin=Input(os.dup(fd), kwargs['stdout'], kwargs['stderr']),
            wait=lambda timeout: 0)

    monkeypatch.setattr(owned.subprocess, 'Popen', popen)
    worker = owned.OwnedResident('/mock/compiler', tmp_path)
    try:
        if mutation:
            with pytest.raises(AssertionError):
                worker.compile(['fixture.rs'])
        else:
            for _ in range(2):
                record = worker.compile(['fixture.rs'])
                assert record['stdout_b64'] == owned.shared.encoded(b'raw stdout\n')
                assert record['stderr_b64'] == owned.shared.encoded(b'raw stderr\n')
            assert worker.owned == dict(revisions=2, contexts=1)
            with pytest.raises(ValueError, match='fixed'):
                worker.compile(['different.rs'])
    finally:
        worker.close()
    assert (tmp_path / 'resident.stdout').read_bytes().startswith(b'raw stdout\n')
    transcript = [json.loads(line) for line in (tmp_path / 'resident.jsonl').read_bytes().splitlines()]
    assert transcript[0]['request'] == dict(id=1, args=['fixture.rs'])
    assert 'response_b64' in transcript[1]
