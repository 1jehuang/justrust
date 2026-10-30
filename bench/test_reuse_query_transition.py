#!/usr/bin/env python3
"""Harness regression tests. No real compiler is launched by these tests."""
import importlib.util
import json
from pathlib import Path
import stat
import subprocess
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location('transition', Path(__file__).with_name('reuse-query-transition.py'))
transition = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(transition)


class TransitionTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name)
        self.rustc = self.base / 'rustc'
        self.rustc.write_bytes(b'frozen compiler')
        (self.base / 'lib').mkdir()
        (self.base / 'lib' / 'librustc_driver.so').write_bytes(b'frozen driver')
        self.output = self.base / 'evidence'
        self.commands = []
        self.mismatch = False
        self.runtime_bad = False
        self.ice = False

    def fake(self, command, **kwargs):
        self.commands.append(command)
        stdout, stderr, status = b'', b'', 0
        if '-vV' in command:
            stdout = b'rustc frozen native\n'
        elif '--print' in command:
            stdout = str(self.base).encode() + b'\n'
        elif '-o' in command:
            artifact = Path(command[command.index('-o') + 1])
            source = Path(command[1]).read_text()
            is_shared = any('incremental=' in x and 'shared-cache' in x for x in command)
            if source == 'error':
                status, stderr = 1, b'error[E0308]: wrong type\n'
            else:
                artifact.write_bytes(b'executable')
            if self.mismatch and is_shared:
                stderr += b'warning: distinct\n'
            if self.ice and is_shared:
                status, stderr = 101, b'internal compiler error\n'
        else:
            stdout = b'wrong\n' if self.runtime_bad else b'15\n'
        return subprocess.CompletedProcess(command, status, stdout, stderr)

    def capture(self, matrix=None):
        if matrix is None:
            matrix = [('initial', 'ok', 'dep', '15\n'), ('error', 'error', 'dep', None),
                      ('recovery', 'ok', 'dep', '15\n')]
        with patch.object(transition, 'cases', return_value=matrix), patch.object(transition.subprocess, 'run', side_effect=self.fake):
            return transition.capture(self.rustc, self.output)

    def report(self):
        return json.loads((self.output / 'acceptance.json').read_text())

    def test_same_cache_no_yes_no_and_fresh_references(self):
        report = self.capture()
        self.assertEqual(report['verdict'], 'passed')
        self.assertEqual(report['completed'], 9)
        self.assertEqual(stat.S_IMODE(self.output.stat().st_mode), 0o700)
        commands = [c for c in self.commands if '--crate-name=retention_fixture' in c]
        shared = [c for c in commands if any('shared-cache' in x for x in c)]
        fresh = [c for c in commands if c not in shared]
        self.assertEqual(len(shared), 9)
        self.assertEqual(len(fresh), 9)
        self.assertEqual(len({next(x for x in c if x.startswith('incremental=')) for c in shared}), 1)
        self.assertEqual(len({next(x for x in c if x.startswith('incremental=')) for c in fresh}), 9)
        self.assertEqual([next(x for x in c if x.startswith(transition.FLAG)) for c in shared],
                         [f'{transition.FLAG}={v}' for v in ('no', 'yes', 'no')] * 3)
        self.assertTrue(all('-Zincremental-verify-ich' in c for c in commands))
        self.assertEqual({c[1] for c in commands}, {str(self.output / 'work' / 'main.rs')})
        self.assertEqual(report['compiler']['sha256'], transition.digest(self.rustc))
        self.assertTrue(report['compiler']['drivers'])

    def test_exact_diagnostic_mismatch_fails_and_keeps_streams(self):
        self.mismatch = True
        with self.assertRaisesRegex(AssertionError, 'exact fresh/shared mismatch'):
            self.capture()
        self.assertEqual(self.report()['verdict'], 'failed')
        streams = list(self.output.rglob('shared-retention_fixture.stderr'))
        self.assertEqual(streams[0].read_bytes(), b'warning: distinct\n')

    def test_wrong_runtime_fails(self):
        self.runtime_bad = True
        with self.assertRaisesRegex(AssertionError, 'incorrect runtime'):
            self.capture()
        self.assertEqual(self.report()['verdict'], 'failed')

    def test_ice_not_ordinary_error(self):
        self.ice = True
        with self.assertRaisesRegex(AssertionError, 'expected status 0, got 101'):
            self.capture()
        self.assertEqual(self.report()['verdict'], 'failed')

    def test_existing_directory_never_reused(self):
        self.output.mkdir()
        sentinel = self.output / 'keep'
        sentinel.write_bytes(b'untouched')
        with self.assertRaises(FileExistsError):
            self.capture()
        self.assertEqual(sentinel.read_bytes(), b'untouched')
        self.assertEqual(self.commands, [])

    def test_empty_matrix_fails_closed(self):
        with self.assertRaisesRegex(AssertionError, 'Empty acceptance matrix'):
            self.capture([])
        self.assertEqual(self.report()['verdict'], 'failed')

    def test_missing_driver_fails_closed(self):
        (self.base / 'lib' / 'librustc_driver.so').unlink()
        with self.assertRaisesRegex(AssertionError, 'Missing native rustc_driver'):
            self.capture()
        self.assertEqual(self.report()['verdict'], 'failed')

    def test_compiler_mutation_fails(self):
        original = self.fake
        def mutate(command, **kwargs):
            result = original(command, **kwargs)
            if '-o' in command:
                self.rustc.write_bytes(b'changed')
            return result
        with patch.object(transition, 'cases', return_value=[('initial', 'ok', 'dep', '15\n')]), patch.object(transition.subprocess, 'run', side_effect=mutate):
            with self.assertRaisesRegex(AssertionError, 'changed during acceptance'):
                transition.capture(self.rustc, self.output)
        self.assertEqual(self.report()['verdict'], 'failed')

    def test_timeout_preserves_non_utf8_bytes(self):
        error = subprocess.TimeoutExpired(['compiler'], 180, output=b'\xffpartial', stderr=b'\xfeerror')
        prefix = self.base / 'timeout'
        with patch.object(transition.subprocess, 'run', side_effect=error):
            result = transition.run(['compiler'], prefix)
        self.assertIsNone(result['status'])
        self.assertEqual(result['error'], 'timeout')
        self.assertEqual(prefix.with_suffix('.stdout').read_bytes(), b'\xffpartial')
        self.assertEqual(prefix.with_suffix('.stderr').read_bytes(), b'\xfeerror')

    def test_launch_error_preserved(self):
        with patch.object(transition.subprocess, 'run', side_effect=FileNotFoundError('missing')):
            result = transition.run(['missing'], self.base / 'launch')
        self.assertIsNone(result['status'])
        self.assertIn('FileNotFoundError', result['error'])

    def test_matrix_includes_errors_const_async_and_recovery(self):
        names = {entry[0] for entry in transition.cases()}
        self.assertTrue({'nonconst', 'const-recovery', 'ordinary-from-async', 'async-recovery',
                         'async-closure', 'type-error', 'type-recovery', 'lint-error',
                         'dependency-mismatch', 'dependency-recovery', 'body-edit'} <= names)


if __name__ == '__main__':
    unittest.main()
