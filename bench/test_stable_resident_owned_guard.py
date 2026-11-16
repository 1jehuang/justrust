"""Mock-only safety tests. These never execute a native compiler."""
import json
from types import SimpleNamespace

import pytest

import stable_resident_owned_guard as harness


def output(status=0, stdout=b'', stderr=b''):
    return dict(status=status, stdout_b64=harness.native.encoded(stdout),
                stderr_b64=harness.native.encoded(stderr))


@pytest.mark.parametrize('failure', [None, 'fresh-marker', 'macro-executed', 'pid',
                                    'diagnostic', 'recovery', 'identity'])
def test_mock_capture(tmp_path, monkeypatch, failure):
    workers = []
    identity_calls = []

    def identity(compiler):
        identity_calls.append(compiler)
        return dict(mock=True, revision=len(identity_calls) if failure == 'identity' else 1)

    def run(command, timeout, cwd=None, env=None):
        if '--crate-type=proc-macro' in command:
            (tmp_path / 'libguard_macro.so').write_bytes(b'mock macro')
            return output()
        if str(tmp_path / 'consumer.rs') in list(map(str, command)):
            if failure != 'fresh-marker':
                (tmp_path / 'fresh' / 'macro-executions').write_bytes(b'executed\n')
            (tmp_path / 'consumer').write_bytes(b'mock binary')
            return output()
        source = (tmp_path / 'consumer.rs').read_text()
        return output(stdout=b'41\n' if source == harness.guard.RECOVERY else b'37\n')

    class Worker:
        def __init__(self, compiler, root, timeout):
            self.root = root
            self.command = [str(compiler), '--jr-resident-owned']
            self.process = SimpleNamespace(pid=40 + len(workers), returncode=None)
            self.args = None
            self.count = 0
            self.marker = harness.os.environ.get(harness.guard.MARKER_ENV)
            self.closed = False
            for stream in ('stdout', 'stderr'):
                (root / ('resident.' + stream)).write_bytes(b'')
            workers.append(self)
        def compile(self, argv):
            if self.args is not None:
                assert argv == self.args
            self.args = list(argv)
            self.count += 1
            if self.root.name.startswith('reject-'):
                kind = self.root.name.removeprefix('reject-')
                diagnostic = dict(backend='only the owned LLVM backend', target='only the host target',
                    incremental='resident owned batch incremental persistence requires an active-revision source-file view')[kind]
                (self.root / 'resident.stderr').write_text(diagnostic)
                self.process.returncode = 1
                raise RuntimeError('Owned process closed response FD')
            if self.count == 1:
                if failure == 'macro-executed':
                    harness.Path(self.marker).write_bytes(b'executed\n')
                diagnostic = b'wrong' if failure == 'diagnostic' else harness.guard.PROC_DIAGNOSTIC.encode()
                result = output(1, stderr=diagnostic)
            else:
                result = output(1 if failure == 'recovery' else 0)
                (tmp_path / 'consumer').write_bytes(b'mock recovered binary')
            result['pid'] = self.process.pid + (1 if failure == 'pid' and self.count == 2 else 0)
            return result
        def close(self):
            assert not self.closed
            self.closed = True
            if self.process.returncode == 1:
                raise RuntimeError('Resident shutdown status 1')
            self.process.returncode = 0

    monkeypatch.setattr(harness.owned, 'OwnedResident', Worker)
    monkeypatch.setattr(harness.native, 'run', run)
    monkeypatch.setattr(harness.native, 'identity', identity)
    if failure:
        with pytest.raises(AssertionError):
            harness.capture(tmp_path / 'mock-compiler', tmp_path)
    else:
        report = harness.capture(tmp_path / 'mock-compiler', tmp_path)
        assert len(workers) == 4
        assert workers[0].count == 2
        assert all(worker.count == 1 for worker in workers[1:])
        assert set(report['request_guards']) == {'backend', 'target', 'incremental'}
        assert all(row['exit_status'] == 1 for row in report['request_guards'].values())
        assert report['observations']['owned']['command'][-1] == '--jr-resident-owned'
    assert all(worker.closed for worker in workers)
    receipt = json.loads((tmp_path / 'receipt.json').read_text())
    assert receipt['verdict'] == ('failed' if failure else 'passed')
    assert 'identity_after' in receipt
    assert receipt['observations']['fresh']['compile']['stdout_b64'] == ''


@pytest.mark.parametrize('behavior', ['acknowledged', 'wrong-status', 'wrong-diagnostic', 'timeout'])
def test_reject_request_fails_closed(tmp_path, monkeypatch, behavior):
    class Worker:
        def __init__(self, compiler, root, timeout):
            self.process = SimpleNamespace(pid=1, returncode=0 if behavior == 'wrong-status' else 1)
            self.command = [str(compiler), '--jr-resident-owned']
            (root / 'resident.stdout').write_bytes(b'')
            (root / 'resident.stderr').write_bytes(b'wrong' if behavior == 'wrong-diagnostic' else b'expected')
        def compile(self, args):
            if behavior == 'acknowledged':
                return output(1)
            if behavior == 'timeout':
                raise TimeoutError('mock timeout')
            raise RuntimeError('closed FD')
        def close(self):
            pass
    monkeypatch.setattr(harness.owned, 'OwnedResident', Worker)
    with pytest.raises((AssertionError, TimeoutError)):
        harness.reject_request('/mock/compiler', tmp_path, ['source.rs'], 'expected', 1)
    assert (tmp_path / 'lifecycle.json').exists()


@pytest.mark.parametrize('timeout', ['nan', 'inf', '0', '-1'])
def test_cli_rejects_invalid_timeout(monkeypatch, timeout):
    monkeypatch.setattr('sys.argv', ['guard', '--compiler', '/mock/compiler',
                                  '--output', '/not-created', '--timeout', timeout])
    with pytest.raises(SystemExit) as error:
        harness.main()
    assert error.value.code == 2
