"""Mock-only orchestration tests, not native acceptance or speed evidence."""
import copy
import json
from pathlib import Path
from types import SimpleNamespace

import pytest

import stable_resident_owned_dogfood as harness
from test_stable_resident_dogfood import compile_record, runtime_record


@pytest.mark.parametrize('failure', [None, 'pid', 'diagnostic', 'nonce', 'identity', 'shutdown'])
def test_mock_capture(tmp_path, monkeypatch, failure):
    source = tmp_path / 'original.rs'
    original = harness.shared.DEFAULT_SOURCE.read_bytes()
    source.write_bytes(original)
    workers, nonces, identities = [], [], []

    def identity(compiler):
        identities.append(compiler)
        return dict(mock=True, value=len(identities) if failure == 'identity' else 1)

    def compile_mock(work):
        text = (work / 'remote_sync_queue.rs').read_text()
        if 'deliberate type error' in text:
            return compile_record(1, b'error: expected mutex\n')
        (work / 'tests').write_bytes(b'mock executable')
        return compile_record()

    class Worker:
        def __init__(self, compiler, root, timeout):
            self.root = root
            self.process = SimpleNamespace(pid=42, returncode=None)
            self.command = [str(compiler), '--jr-resident-owned']
            self.argv = None
            self.owned = dict(revisions=0, contexts=0)
            self.closed = False
            workers.append(self)
        def compile(self, argv):
            assert not any('incremental' in arg for arg in argv)
            if self.argv is not None:
                assert argv == self.argv
            self.argv = list(argv)
            self.owned = {key: value + 1 for key, value in self.owned.items()}
            result = compile_mock(self.root)
            result['pid'] = 43 if failure == 'pid' else 42
            if failure == 'diagnostic':
                result['stderr_b64'] = harness.native.encoded(b'different')
            return result
        def close(self):
            assert not self.closed
            self.closed = True
            self.process.returncode = 1 if failure == 'shutdown' else 0
            if failure == 'shutdown':
                raise RuntimeError('mock shutdown failure')

    def run(command, timeout, cwd, env):
        assert not any(key.startswith('JUSTRUST_RUSTC_RESIDENT_') for key in env)
        if Path(command[0]).name != 'tests':
            return compile_mock(cwd)
        nonce = env[harness.shared.NONCE_KEY]
        nonces.append(nonce)
        text = (cwd / 'remote_sync_queue.rs').read_text()
        return runtime_record(nonce='stale' if failure == 'nonce' else nonce,
                              fail='foreground_waiters.load(Ordering::SeqCst), 1);' in text)

    monkeypatch.setattr(harness.native, 'identity', identity)
    monkeypatch.setattr(harness.native, 'run', run)
    monkeypatch.setattr(harness.owned, 'OwnedResident', Worker)
    monkeypatch.setenv('JUSTRUST_RUSTC_RESIDENT_FRONTEND', '1')
    monkeypatch.setenv('JUSTRUST_RUSTC_RESIDENT_VALUES', '1')
    if failure:
        with pytest.raises((AssertionError, RuntimeError)):
            harness.capture('/mock/rustc', source, tmp_path, repetitions=1)
    else:
        report = harness.capture('/mock/rustc', source, tmp_path, repetitions=2)
        assert len(report['observations']) == 20
        assert all(worker.owned == dict(revisions=10, contexts=10) for worker in workers)
        assert len(nonces) == len(set(nonces)) == 36
        assert report['modes'] == ['fresh', 'owned']
        assert all(session['command'][-1] == '--jr-resident-owned' for session in report['sessions'])
        assert report['source_unchanged']
        assert 'No typed-query-hit' in report['claim']
    assert all(worker.closed for worker in workers)
    assert source.read_bytes() == original
    receipt = json.loads((tmp_path / 'receipt.json').read_text())
    assert receipt['verdict'] == ('failed' if failure else 'passed')
    assert receipt['source_unchanged']
    assert 'identity_after' in receipt


@pytest.mark.parametrize('mutation', ['stale', 'nonce', 'status', 'timeout'])
def test_pair_rejects_invalid_observations(mutation):
    modes = {mode: dict(compile=compile_record(), nonce=mode,
                       runtime=runtime_record(nonce=mode)) for mode in harness.MODES}
    modes['owned']['compile']['pid'] = 42
    if mutation == 'stale':
        for row in modes.values():
            row['compile'] = compile_record(1)
        modes['owned']['compile']['pid'] = 42
        expectation = 'compile-fail'
    else:
        expectation = 'pass'
        if mutation == 'nonce':
            modes['owned'] = copy.deepcopy(modes['fresh'])
            modes['owned']['compile']['pid'] = 42
        elif mutation == 'status':
            modes['owned']['runtime']['status'] = 101
        else:
            modes['owned']['compile']['timed_out'] = True
    with pytest.raises(AssertionError):
        harness.validate_pair(modes, expectation, 42)


@pytest.mark.parametrize('timeout', [0, -1, float('nan'), float('inf')])
def test_capture_rejects_invalid_timeout(tmp_path, timeout):
    with pytest.raises(ValueError):
        harness.capture('/not-executed', '/not-read', tmp_path, timeout=timeout)
