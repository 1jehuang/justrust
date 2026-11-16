"""Mock-only tests. These do not establish native acceptance or performance."""
import base64
import io
import json
import os
import subprocess
from types import SimpleNamespace

import pytest

import stable_resident_owned_retirement as harness


def frame(index=64):
    return dict(id=index, pid=42, status=0, stdout_b64='', stderr_b64='',
                executed=True, owned=dict(contexts=index, revisions=index,
                    rss_bytes=1024, retirement_reason='context_capacity' if index == 64 else None))


@pytest.mark.parametrize('index', [1, 63, 64])
def test_valid_frame(index):
    harness.validate_frame(frame(index), index, 42, frame())


@pytest.mark.parametrize('mutation', ['early', 'missing', 'reason', 'contexts', 'revisions',
                                      'failed', 'boolean-status', 'pid', 'timeout',
                                      'missing-reason', 'missing-rss', 'rss-budget', 'refused'])
def test_reject_frame(mutation):
    row = frame(1 if mutation == 'early' else 64)
    if mutation == 'early':
        row['owned']['retirement_reason'] = 'context_capacity'
    elif mutation == 'missing':
        row.pop('executed')
    elif mutation == 'reason':
        row['owned']['retirement_reason'] = 'request_limit'
    elif mutation == 'missing-reason':
        row['owned'].pop('retirement_reason')
    elif mutation == 'missing-rss':
        row['owned'].pop('rss_bytes')
    elif mutation == 'rss-budget':
        row['owned']['rss_bytes'] = 2 * 1024 * 1024 * 1024
    elif mutation == 'refused':
        row['executed'] = False
    elif mutation in ('contexts', 'revisions'):
        row['owned'][mutation] -= 1
    elif mutation == 'failed':
        row['status'] = 1
    elif mutation == 'boolean-status':
        row['status'] = False
    elif mutation == 'pid':
        row['pid'] = 43
    else:
        row['timed_out'] = True
    with pytest.raises(AssertionError):
        harness.validate_frame(row, row['id'], 42, frame())


@pytest.mark.parametrize('failure', [None, 'early', 'failed', 'identity', 'shutdown'])
def test_capture_receipt(tmp_path, monkeypatch, failure):
    commands = []
    workers = []
    identities = []
    def identity(compiler):
        identities.append(str(compiler))
        return dict(identity=len(identities) if failure == 'identity' else 1)
    def compile_fresh(command, timeout, cwd, env):
        commands.append(command[1:])
        (cwd / 'fixture.rmeta').write_bytes(b'valid mock metadata')
        return {k: frame()[k] for k in ('status', 'stdout_b64', 'stderr_b64')}
    class Worker:
        def __init__(self, compiler, root, timeout):
            self.process = SimpleNamespace(pid=42)
            self.command = [str(compiler), '--jr-resident-owned']
            self.index = 0
            self.closed = False
            workers.append(self)
        def compile(self, argv):
            assert argv == commands[0]
            self.index += 1
            (tmp_path / 'fixture.rmeta').write_bytes(b'valid mock metadata')
            result = frame(self.index)
            if failure == 'early':
                result['owned']['retirement_reason'] = 'context_capacity'
            if failure == 'failed':
                result['status'] = 1
            return result
        def close(self):
            self.closed = True
    def retire(worker, argv, report, timeout):
        assert worker.index == 64
        if failure == 'shutdown':
            raise AssertionError('shutdown timeout')
        report['shutdown'] = dict(status=0)
    monkeypatch.setattr(harness.owned.shared, 'identity', identity)
    monkeypatch.setattr(harness.owned.shared, 'run', compile_fresh)
    monkeypatch.setattr(harness.owned, 'OwnedResident', Worker)
    monkeypatch.setattr(harness, 'retire', retire)
    if failure:
        with pytest.raises(AssertionError):
            harness.capture(tmp_path / 'compiler', tmp_path)
    else:
        report = harness.capture(tmp_path / 'compiler', tmp_path)
        assert len(report['observations']) == 64
        assert commands[0] == commands[1]
    receipt = json.loads((tmp_path / 'receipt.json').read_text())
    assert receipt['verdict'] == ('failed' if failure else 'passed')
    assert receipt['observations'][-1]['compile']
    assert receipt['identity_after']
    assert len(identities) == 2
    assert workers[0].closed


@pytest.mark.parametrize('mutation', [None, 'shutdown', 'extra', 'nonzero'])
def test_retirement_real_pipe_final_frame_preserved(tmp_path, monkeypatch, mutation):
    """Shared transport parses a real framed pipe, but Popen never runs a compiler."""
    import stable_resident_owned as owned
    class Input(io.BytesIO):
        def __init__(self, fd):
            super().__init__()
            self.fd = fd
        def write(self, payload):
            request = json.loads(payload)
            if request['id'] <= 64:
                result = frame(request['id'])
                result['elapsed_seconds'] = .001
                os.write(self.fd, json.dumps(result).encode() + b'\n')
            elif mutation == 'extra':
                os.write(self.fd, b'{"id":65}\n')
            if request['id'] == 65:
                os.close(self.fd)
                self.fd = None
            return len(payload)
        def close(self):
            if self.fd is not None:
                os.close(self.fd)
                self.fd = None
            super().close()
    def popen(command, **kwargs):
        process = SimpleNamespace(pid=42, returncode=None,
            stdin=Input(os.dup(kwargs['pass_fds'][0])))
        def wait(timeout=None):
            if mutation == 'shutdown' and timeout != 5 and process.returncode is None:
                raise subprocess.TimeoutExpired(command, timeout)
            process.returncode = 1 if mutation == 'nonzero' else 0
            return process.returncode
        process.wait = wait
        process.kill = lambda: setattr(process, 'returncode', -9)
        return process
    monkeypatch.setattr(owned.subprocess, 'Popen', popen)
    worker = owned.OwnedResident('/mock/compiler', tmp_path)
    report = {}
    try:
        for index in range(1, 65):
            result = worker.compile(['fixture.rs'])
            harness.validate_frame(result, index, 42, frame())
        if mutation:
            with pytest.raises(AssertionError):
                harness.retire(worker, ['fixture.rs'], report, 1)
        else:
            harness.retire(worker, ['fixture.rs'], report, 1)
            assert report['shutdown'] == dict(status=0, timed_out=False)
            assert report['trailing_response_b64'] == ''
    finally:
        if mutation == 'nonzero':
            with pytest.raises(RuntimeError):
                worker.close()
        else:
            worker.close()
    transcript = [json.loads(line) for line in (tmp_path / 'resident.jsonl').read_bytes().splitlines()]
    responses = [json.loads(base64.b64decode(row['response_b64']))
                 for row in transcript if 'response_b64' in row]
    assert len(responses) == 64
    assert responses[-1]['id'] == 64
    assert responses[-1]['executed'] is True
    assert responses[-1]['owned']['retirement_reason'] == 'context_capacity'
    assert responses[-1]['owned']['contexts'] == 64
    assert (tmp_path / 'resident.stdout').exists()
    assert (tmp_path / 'resident.stderr').exists()


def test_cli_requires_explicit_paths_and_new_directory(tmp_path, monkeypatch):
    monkeypatch.setattr('sys.argv', ['retirement'])
    with pytest.raises(SystemExit):
        harness.main()
    monkeypatch.setattr('sys.argv', ['retirement', '--compiler', '/mock/compiler',
                                    '--output', str(tmp_path)])
    with pytest.raises(FileExistsError):
        harness.main()
    assert '--emit=metadata' in harness.arguments(tmp_path)
    assert '--crate-type=rlib' in harness.arguments(tmp_path)
