#!/usr/bin/env python3
"""Mocks only: never builds or launches a native compiler."""
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import MagicMock, patch

import stable_resident_typed as typed


def response(**changes):
    row = dict(status=0, pid=123, executed=True, stdout_b64='', stderr_b64='',
               owned=dict(retirement_reason=None),
               live=dict.fromkeys(typed.COUNTERS, 0))
    row.update(changes)
    return row


def ir(width=8, value=7):
    return ('define i%d @typed_value() {\nstart:\n  ret i%d %d\n}\n'
            % (width, width, value)).encode()


class TypedValidationTests(unittest.TestCase):
    def test_environment_is_dedicated_and_strips_other_resident_gates(self):
        with patch.dict(os.environ, {'JUSTRUST_RUSTC_RESIDENT_VALUES': '1',
                        'JUSTRUST_RUSTC_RESIDENT_TYPED': 'bad'}):
            before = dict(os.environ)
            for mode in ('control', '1', 'leaf'):
                env = typed.environment(mode)
                self.assertEqual(env['JUSTRUST_RUSTC_RESIDENT_TYPED'], mode)
                self.assertNotIn('JUSTRUST_RUSTC_RESIDENT_VALUES', env)
            self.assertEqual(dict(os.environ), before)
        with self.assertRaises(ValueError):
            typed.environment('0')

    def test_constructor_passes_explicit_env_without_shared_mutation(self):
        with tempfile.TemporaryDirectory() as directory:
            process = MagicMock()
            process.returncode = 0
            with patch.object(typed.subprocess, 'Popen', return_value=process) as popen:
                worker = typed.TypedResident(Path('/frozen'), Path(directory), '1')
                try:
                    call = popen.call_args
                    self.assertEqual(call.args[0], ['/frozen', '--jr-resident-owned'])
                    self.assertEqual(call.kwargs['env']['JUSTRUST_RUSTC_RESIDENT_TYPED'], '1')
                    self.assertEqual(int(call.kwargs['env']['JUSTRUST_RUSTC_RESIDENT_FD']),
                                     call.kwargs['pass_fds'][0])
                    self.assertNotIn('JUSTRUST_RUSTC_RESIDENT_TYPED', typed.owned.environment())
                finally:
                    worker.close()

    def test_only_actual_live_hit_counts_accept_unchanged(self):
        previous = response()['live']
        for key in typed.HITS:
            row = response()
            row['live'].update({key: 1, 'retained_entries': 1})
            self.assertEqual(typed.validate_live(row, previous, '1', True), row['live'])
        row = response(resident_hits=100, frontend={'typeck_loads': 100})
        with self.assertRaisesRegex(AssertionError, 'actual typed hits'):
            typed.validate_live(row, previous, '1', True)

    def test_control_requires_all_live_fields_zero(self):
        typed.validate_live(response(), None, 'control', False)
        for key in typed.COUNTERS:
            row = response()
            row['live'][key] = 1
            with self.subTest(key=key), self.assertRaises(AssertionError):
                typed.validate_live(row, None, 'control', False)

    def test_changed_error_and_recovery_sources_cannot_hit(self):
        previous = response()['live']
        previous['typeck_hits'] = 3
        for status in (0, 1):
            row = response(status=status)
            row['live']['typeck_hits'] = 3
            typed.validate_live(row, previous, '1', False)
            row['live']['typeck_hits'] += 1
            with self.assertRaisesRegex(AssertionError, 'authorized typed reuse'):
                typed.validate_live(row, previous, '1', False)

    def test_invalid_telemetry_and_retirement_fail_closed(self):
        for value in (None, {}, {'typeck_hits': True}):
            with self.assertRaises(AssertionError):
                typed.validate_live(response(live=value), None, '1', False)
        for change in (dict(executed=False), dict(owned={'retirement_reason': 'rss_budget'}),
                       dict(status=1)):
            with self.assertRaises(AssertionError):
                typed.validate_live(response(**change), None, '1', True)
        row = response()
        row['live']['typeck_hits'] = 1
        with self.assertRaisesRegex(AssertionError, 'retained entries'):
            typed.validate_live(row, None, '1', True)
        old = response()['live']
        old['mir_misses'] = 2
        with self.assertRaisesRegex(AssertionError, 'decreased'):
            typed.validate_live(response(), old, '1', False)

    def test_exact_diagnostics_pid_and_status(self):
        row = response(status=1, stderr_b64='ZXJyb3IK')
        typed.validate_pair(row, row, None, 123)
        for change in (dict(pid=124), dict(stderr_b64='ZGlmZmVyZW50'),
                       dict(timed_out=True), dict(status=0)):
            with self.assertRaises(AssertionError):
                typed.validate_pair(row, dict(row, **change), None, 123)

    def test_artifact_semantic_oracle(self):
        self.assertEqual(typed.artifact_oracle(ir(), (8, 7))['value'], 7)
        for data in (ir(8, 9), ir(16, 7), b'', ir().replace(b'ret', b'call void @bad()\n ret')):
            with self.assertRaises(AssertionError):
                typed.artifact_oracle(data, (8, 7))

    def test_fixture_and_revision_authorization(self):
        workload = typed.cases()
        self.assertLess(len(workload), 64)
        self.assertNotIn('extern crate', typed.fixture())
        self.assertIn('#![no_core]', typed.fixture())
        previous = None
        for name, source, expected, reusable in workload:
            with self.subTest(name=name):
                if reusable:
                    self.assertEqual(source, previous)
                    self.assertIsNotNone(expected)
                previous = source
        self.assertNotEqual(workload[7][1], workload[9][1])
        argv = typed.arguments(Path('/private'))
        self.assertTrue(any(a.startswith('-Cincremental=') for a in argv))
        self.assertNotIn('--extern', argv)
        self.assertIn('--emit=llvm-ir=/private/fixture.ll,mir=/private/fixture.mir', argv)
        self.assertNotIn('-o', argv)


class CaptureTests(unittest.TestCase):
    def setup_mock(self, root, bad_hits=False, close_error=False, mir_data=b'mock MIR\n'):
        class Worker:
            def __init__(self, compiler, cwd, mode, timeout):
                self.process = type('Process', (), {'pid': 123})()
                self.command = [str(compiler), '--jr-resident-owned']
                self.root, self.mode = cwd, mode
                self.index = 0
                self.hits = 0
                self.closed = False

            def compile(self, argv):
                name, source, expected, reusable = typed.cases()[self.index]
                self.index += 1
                row = compile_result(self.root, expected)
                row.update(pid=123, executed=True, owned={'retirement_reason': None})
                row['live'] = dict.fromkeys(typed.COUNTERS, 0)
                if self.mode == '1':
                    self.hits += int(reusable and not bad_hits)
                    row['live'].update(typeck_hits=self.hits, retained_entries=2,
                                       retained_estimated_bytes=80)
                return row

            def close(self):
                self.closed = True
                if close_error:
                    raise RuntimeError('mock shutdown failure')

        def compile_result(cwd, expected):
            (cwd / 'incremental').mkdir(exist_ok=True)
            if expected is not None:
                (cwd / 'fixture.ll').write_bytes(ir(*expected))
                (cwd / 'fixture.mir').write_bytes(mir_data)
            return dict(status=0 if expected is not None else 1,
                        stdout_b64='', stderr_b64='' if expected is not None else 'ZXJyb3IK')

        def fresh(command, timeout, cwd, env):
            source = (cwd / 'fixture.rs').read_text()
            expected = next(expected for _, case_source, expected, _ in typed.cases()
                            if source == case_source)
            return compile_result(cwd, expected)

        return Worker, fresh

    def test_full_mock_capture_and_receipts(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            worker, fresh = self.setup_mock(root)
            identity = dict(path='frozen', sha256='abc', drivers=['driver'])
            with patch.object(typed, 'TypedResident', worker), \
                    patch.object(typed.shared, 'run', fresh), \
                    patch.object(typed.shared, 'identity', return_value=identity):
                report = typed.capture(root / 'compiler', root)
            self.assertEqual(report['verdict'], 'passed')
            self.assertEqual(len(report['modes']), 2)
            self.assertGreater(report['modes'][1]['live']['typeck_hits'], 0)
            self.assertEqual(report['modes'][0]['live']['typeck_hits'], 0)
            self.assertEqual(json.loads((root / 'receipt.json').read_text()), report)
            for name in ('control', 'typed'):
                receipt = json.loads((root / name / 'receipt.json').read_text())
                self.assertEqual(len(receipt['observations']), len(typed.cases()))
                self.assertTrue((root / name / '00-resident.ll').exists())
                self.assertTrue((root / name / '00-resident.mir').exists())
                self.assertTrue((root / name / 'fresh-incremental').exists())
                self.assertTrue((root / name / 'resident-incremental').exists())

    def test_missing_hits_retains_failed_mode_and_top_receipts(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            worker, fresh = self.setup_mock(root, bad_hits=True)
            with patch.object(typed, 'TypedResident', worker), \
                    patch.object(typed.shared, 'run', fresh), \
                    patch.object(typed.shared, 'identity', return_value={'frozen': True}):
                with self.assertRaisesRegex(AssertionError, 'actual typed hits'):
                    typed.capture(root / 'compiler', root)
            for path in (root / 'receipt.json', root / 'typed' / 'receipt.json'):
                self.assertEqual(json.loads(path.read_text())['verdict'], 'failed')
            failed = json.loads((root / 'typed' / 'receipt.json').read_text())
            self.assertEqual(failed['observations'][-1]['name'], 'unchanged')
            self.assertIn('resident', failed['observations'][-1])

    def test_explicit_mir_demand_requires_nonempty_artifact(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            worker, fresh = self.setup_mock(root, mir_data=b'')
            with patch.object(typed, 'TypedResident', worker), \
                    patch.object(typed.shared, 'run', fresh):
                with self.assertRaisesRegex(AssertionError, 'empty artifact'):
                    typed.capture_mode(root / 'compiler', root / 'mode', 'control', 1)
            report = json.loads((root / 'mode' / 'receipt.json').read_text())
            self.assertEqual(report['verdict'], 'failed')
            self.assertEqual((root / 'mode' / '00-fresh.mir').read_bytes(), b'')

    def test_shutdown_failure_is_failed_receipt(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            worker, fresh = self.setup_mock(root, close_error=True)
            with patch.object(typed, 'TypedResident', worker), \
                    patch.object(typed.shared, 'run', fresh):
                with self.assertRaisesRegex(RuntimeError, 'shutdown failure'):
                    typed.capture_mode(root / 'compiler', root / 'mode', 'control', 1)
            report = json.loads((root / 'mode' / 'receipt.json').read_text())
            self.assertEqual(report['verdict'], 'failed')

    def test_identity_change_fails_after_both_modes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            worker, fresh = self.setup_mock(root)
            with patch.object(typed, 'TypedResident', worker), \
                    patch.object(typed.shared, 'run', fresh), \
                    patch.object(typed.shared, 'identity', side_effect=[{'hash': 1}, {'hash': 2}]):
                with self.assertRaisesRegex(AssertionError, 'identity changed'):
                    typed.capture(root / 'compiler', root)
            report = json.loads((root / 'receipt.json').read_text())
            self.assertEqual(report['verdict'], 'failed')
            self.assertEqual(report['identity_after'], {'hash': 2})


if __name__ == '__main__':
    unittest.main()
