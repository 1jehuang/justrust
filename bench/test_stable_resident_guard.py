"""Mock-only orchestration tests. These do not establish native guard acceptance."""
import base64
from contextlib import contextmanager
import json
import os
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import stable_resident_guard as guard


def record(status=0, stdout=b'', stderr=b'', pid=4321):
    return dict(status=status, stdout_b64=guard.native.encoded(stdout),
                stderr_b64=guard.native.encoded(stderr), pid=pid, elapsed_seconds=.01,
                resident_hits=0, resident_misses=0, resident_entries=0)


class GuardMockTests(unittest.TestCase):
    @contextmanager
    def mocked_compiler(self, fresh_count=1, execute_resident=False, diagnostic=True,
                        recovery_status=0, different_pid=False, unexpected_guard_response=False,
                        identity_changed=False):
        launches = []
        fresh_commands = []

        def fake_run(command, timeout=180, cwd=None, env=None):
            fresh_commands.append(command)
            if len(command) == 1:
                source = Path(command[0]).parent / 'consumer.rs'
                return record(stdout=b'41\n' if source.read_text() == guard.RECOVERY else b'37\n')
            output = Path(command[command.index('-o') + 1])
            output.write_bytes(b'MOCK artifact, not native evidence')
            if '--crate-type=proc-macro' not in command:
                if fresh_count:
                    Path(env[guard.MARKER_ENV]).write_bytes(b'executed\n' * fresh_count)
            return record()

        class FakeWorker:
            def __init__(self, compiler, root, cwd, values, timeout):
                self.root = root
                self.process = SimpleNamespace(pid=4321, returncode=None)
                self.command = [str(compiler), '--jr-resident']
                self.marker = os.environ.get(guard.MARKER_ENV)
                self.first_args = None
                launches.append(self)
                (root / 'resident.stdout').write_bytes(b'')
                (root / 'resident.stderr').write_bytes(b'')

            def compile(self, args):
                if self.root.name.startswith('reject-'):
                    if unexpected_guard_response:
                        return record()
                    reason = ('only the host target' if self.root.name == 'reject-target'
                              else 'only the owned LLVM backend')
                    (self.root / 'resident.stderr').write_text(reason)
                    raise RuntimeError('Resident exited or closed response FD')
                if self.first_args is None:
                    self.first_args = list(args)
                    if execute_resident:
                        Path(self.marker).write_bytes(b'executed\n')
                    return record(1, stderr=guard.PROC_DIAGNOSTIC.encode() if diagnostic else b'wrong error')
                if args != self.first_args:
                    raise AssertionError('mock observed changing argv')
                if Path(args[0]).read_text() != guard.RECOVERY:
                    raise AssertionError('mock observed wrong recovery source')
                if recovery_status == 0:
                    Path(args[-1]).write_bytes(b'MOCK recovered artifact')
                return record(recovery_status, pid=999 if different_pid else self.process.pid)

            def close(self):
                self.process.returncode = 1 if self.root.name.startswith('reject-') else 0
                if self.process.returncode:
                    raise RuntimeError('Resident shutdown status 1')

        ids = [{'sha256': 'mock-before'}, {'sha256': 'mock-after' if identity_changed else 'mock-before'}]
        with patch.object(guard.native, 'Resident', FakeWorker), \
                patch.object(guard.native, 'run', side_effect=fake_run), \
                patch.object(guard.native, 'identity', side_effect=ids):
            yield launches, fresh_commands

    def test_mock_success_has_positive_control_guard_recovery_and_request_guards(self):
        with tempfile.TemporaryDirectory() as tmp, self.mocked_compiler() as (workers, commands):
            root = Path(tmp)
            report = guard.capture('/mock/compiler', root)
            self.assertEqual(report['verdict'], 'passed')
            self.assertEqual(len(workers), 3)
            resident = report['observations']['resident']
            self.assertFalse(resident['marker_after_rejection']['exists'])
            self.assertFalse(resident['marker_after_recovery']['exists'])
            self.assertEqual(resident['rejected_compile']['pid'], resident['recovered_compile']['pid'])
            self.assertEqual(resident['exit_status'], 0)
            self.assertEqual(set(report['request_guards']), {'backend', 'target'})
            self.assertEqual(base64.b64decode(report['observations']['fresh']['marker']['bytes_b64']),
                             b'executed\n')
            self.assertEqual(workers[0].first_args, report['consumer_args'])
            self.assertIn('--extern', workers[0].first_args)
            self.assertNotIn('-Cincremental', ' '.join(workers[0].first_args))
            self.assertIn('--crate-type=proc-macro', commands[0])
            self.assertEqual(json.loads((root / 'receipt.json').read_text())['verdict'], 'passed')

    def test_mock_requires_exactly_one_fresh_execution(self):
        for count in (0, 2):
            with self.subTest(count=count), tempfile.TemporaryDirectory() as tmp, \
                    self.mocked_compiler(fresh_count=count):
                with self.assertRaisesRegex(AssertionError, 'exactly once'):
                    guard.capture('/mock/compiler', Path(tmp))

    def test_mock_rejects_any_resident_macro_execution_even_if_guard_reports_error(self):
        with tempfile.TemporaryDirectory() as tmp, self.mocked_compiler(execute_resident=True):
            root = Path(tmp)
            with self.assertRaisesRegex(AssertionError, 'executed proc macro'):
                guard.capture('/mock/compiler', root)
            receipt = json.loads((root / 'receipt.json').read_text())
            self.assertEqual(receipt['verdict'], 'failed')
            self.assertTrue(receipt['observations']['resident']['marker_after_rejection']['exists'])
            self.assertEqual(receipt['observations']['resident']['exit_status'], 0)

    def test_mock_requires_specific_guard_diagnostic(self):
        with tempfile.TemporaryDirectory() as tmp, self.mocked_compiler(diagnostic=False):
            with self.assertRaisesRegex(AssertionError, 'diagnostic missing'):
                guard.capture('/mock/compiler', Path(tmp))

    def test_mock_recovery_failure_and_pid_change_are_rejected(self):
        for settings, expected in (({'recovery_status': 1}, 'failed recovery'),
                                   ({'different_pid': True}, 'process identity')):
            with self.subTest(settings=settings), tempfile.TemporaryDirectory() as tmp, \
                    self.mocked_compiler(**settings):
                with self.assertRaisesRegex(AssertionError, expected):
                    guard.capture('/mock/compiler', Path(tmp))

    def test_mock_unsupported_request_must_not_be_acknowledged(self):
        with tempfile.TemporaryDirectory() as tmp, self.mocked_compiler(unexpected_guard_response=True):
            root = Path(tmp)
            with self.assertRaisesRegex(AssertionError, 'unexpectedly acknowledged'):
                guard.capture('/mock/compiler', root)
            row = json.loads((root / 'reject-backend' / 'lifecycle.json').read_text())
            self.assertIn('unexpected_response', row)
            self.assertEqual(row['exit_status'], 1)

    def test_mock_identity_change_is_rejected(self):
        with tempfile.TemporaryDirectory() as tmp, self.mocked_compiler(identity_changed=True):
            with self.assertRaisesRegex(AssertionError, 'identity changed'):
                guard.capture('/mock/compiler', Path(tmp))

    def test_marker_environment_restores_original_even_on_exception(self):
        with patch.dict(os.environ, {guard.MARKER_ENV: 'original'}):
            with self.assertRaises(RuntimeError):
                with guard.marker_environment(Path('/unique/mock/marker')):
                    self.assertEqual(os.environ[guard.MARKER_ENV], '/unique/mock/marker')
                    raise RuntimeError('test')
            self.assertEqual(os.environ[guard.MARKER_ENV], 'original')
        with patch.dict(os.environ):
            os.environ.pop(guard.MARKER_ENV, None)
            with guard.marker_environment(Path('/unique/mock/marker')):
                self.assertIn(guard.MARKER_ENV, os.environ)
            self.assertNotIn(guard.MARKER_ENV, os.environ)

    def test_macro_source_marker_is_runtime_side_effect(self):
        self.assertIn('#[proc_macro]', guard.MACRO)
        self.assertIn('std::env::var("' + guard.MARKER_ENV + '")', guard.MACRO)
        self.assertIn('.append(true)', guard.MACRO)
        self.assertIn('file.write_all(b"executed\\n")', guard.MACRO)


if __name__ == '__main__':
    unittest.main()
