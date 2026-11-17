#!/usr/bin/env python3
"""Focused Typeck acceptance unit tests. Mocks never invoke a compiler."""
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import stable_resident_typeck as harness
import stable_resident_typed as typed
from test_stable_resident_typed import response, ir


class TypeckOracleTests(unittest.TestCase):
    def test_mir_only_and_proxy_hits_are_not_typeck(self):
        for changes in ({}, {'mir_hits': 3}):
            row = response(resident_hits=50, frontend={'typeck_loads': 100})
            row['live'].update(changes, retained_entries=1)
            with self.assertRaisesRegex(AssertionError, 'actual (Typeck|typed) hits'):
                harness.validate_live(row, response()['live'], '1', True)

    def test_typeck_delta_not_absolute_count(self):
        old = response()['live']
        old['typeck_hits'] = 2
        row = response()
        row['live'].update(typeck_hits=2, mir_hits=5, retained_entries=2)
        with self.assertRaisesRegex(AssertionError, 'actual Typeck hits'):
            harness.validate_live(row, old, '1', True)
        row['live']['typeck_hits'] = 3
        harness.validate_live(row, old, '1', True)

    def test_all_disabled_counters_zero(self):
        harness.validate_live(response(), None, 'control', False)
        for key in typed.COUNTERS:
            row = response()
            row['live'][key] = 1
            with self.subTest(key=key), self.assertRaises(AssertionError):
                harness.validate_live(row, None, 'control', False)

    def test_leaf_paths_do_not_leak_into_controls(self):
        for key, value in (('mir_leaf_hits', 1), ('mir_leaf_candidate_misses', 1),
                           ('mir_leaf_last_rejection', 'changed')):
            row = response()
            row['live'][key] = value
            with self.assertRaisesRegex(AssertionError, 'separate leaf'):
                harness.validate_live(row, None, 'control', False)

    def test_changed_errors_cannot_authorize_hits(self):
        for status in (0, 1):
            for key in typed.HITS:
                row = response(status=status)
                row['live'][key] = 1
                with self.assertRaisesRegex(AssertionError, 'authorized typed reuse'):
                    harness.validate_live(row, response()['live'], '1', False)

    def test_identity_is_exact_not_just_version(self):
        identity = dict(sha256=harness.LAUNCHER, drivers=[dict(sha256=harness.DRIVER)])
        harness.check_identity(identity)
        for bad in (dict(identity, sha256='wrong'), dict(identity, drivers=[]),
                    dict(identity, drivers=[dict(sha256='wrong')])):
            with self.assertRaises(AssertionError):
                harness.check_identity(bad)

    def test_fixture_requires_nested_inference_and_preserves_revision_controls(self):
        self.assertIn('let _infer = ||', harness.fixture())
        self.assertIn('let primitive = 7u8', harness.fixture())
        self.assertIn('#![no_core]', harness.fixture())
        self.assertNotIn('extern crate', harness.fixture())
        previous = None
        for name, source, expected, reusable in harness.cases():
            if reusable:
                self.assertEqual(source, previous)
                self.assertIsNotNone(expected)
            previous = source
        self.assertNotEqual(harness.cases()[7][1], harness.cases()[9][1])

    def test_exact_diagnostics_and_primitive_output_not_general_equivalence(self):
        row = response(status=1, stderr_b64='ZXJyb3IK')
        typed.validate_pair(row, row, None, 123)
        with self.assertRaises(AssertionError):
            typed.validate_pair(row, dict(row, stderr_b64=''), None, 123)
        for data in (ir(16, 7), ir(8, 9), b'', ir().replace(b'ret', b'call void @bad()\n ret')):
            with self.assertRaises(AssertionError):
                typed.artifact_oracle(data, (8, 7))

    def test_ordinary_candidates_preserve_baseline_and_strict_repeats(self):
        rows = harness.cases()
        self.assertEqual([row[0] for row in rows[:11]],
                         [row[0] for row in typed.cases()])
        for row, original in zip(rows[:11], typed.cases()):
            self.assertEqual(row[1], harness.fixture(
                original[2][1] if original[2] else 13,
                original[2][0] if original[2] else 16,
                8 if original[2] is None else original[2][0]))
            self.assertEqual(row[2:], original[2:])
        for index in range(11, len(rows), 3):
            initial, repeat, again = rows[index:index + 3]
            self.assertEqual(initial[1:3], repeat[1:3])
            self.assertEqual(initial[1:3], again[1:3])
            self.assertEqual([initial[3], repeat[3], again[3]], [False, True, True])
            self.assertIn('#![no_core]', initial[1])
            self.assertNotIn('extern crate', initial[1])
        generic = next(row[1] for row in rows if row[0] == 'generic-const-initial')
        self.assertIn('identity(constant_value())', generic)
        self.assertIn('ConstConsumer::<{ constant_value() }>', generic)
        opaque = next(row[1] for row in rows if row[0] == 'opaque-codegen-initial')
        self.assertIn('-> impl Value', opaque)
        self.assertIn('consume(hidden_value())', opaque)


class CaptureTests(unittest.TestCase):
    def run_capture(self, root, hit_key='typeck_hits', identities=None, shutdown=False):
        def compile_result(cwd):
            source = (cwd / 'fixture.rs').read_text()
            expected = next(e for _, s, e, _ in harness.cases() if s == source)
            (cwd / 'incremental').mkdir(exist_ok=True)
            if expected is not None:
                (cwd / 'fixture.ll').write_bytes(ir(*expected))
                (cwd / 'fixture.mir').write_bytes(b'nonempty MIR\n')
            return dict(status=0 if expected else 1, stdout_b64='',
                        stderr_b64='' if expected else 'ZXJyb3IK')

        class Worker:
            def __init__(self, compiler, cwd, mode, timeout):
                self.process = type('Process', (), {'pid': 123})()
                self.command = [str(compiler), '--jr-resident-owned']
                self.cwd, self.mode, self.index, self.hits = cwd, mode, 0, 0

            def compile(self, argv):
                row = compile_result(self.cwd)
                reusable = harness.cases()[self.index][3]
                self.index += 1
                live = dict.fromkeys(typed.COUNTERS, 0)
                if self.mode == '1':
                    self.hits += int(reusable)
                    live.update({hit_key: self.hits, 'retained_entries': 2})
                row.update(pid=123, executed=True, owned={'retirement_reason': None}, live=live)
                return row

            def close(self):
                if shutdown:
                    raise RuntimeError('shutdown failure')

        identity = dict(sha256=harness.LAUNCHER, drivers=[dict(sha256=harness.DRIVER)])
        with patch.object(typed, 'TypedResident', Worker), \
                patch.object(typed.shared, 'run', side_effect=lambda cmd, timeout, cwd, env: compile_result(cwd)), \
                patch.object(typed.shared, 'identity', side_effect=identities or [identity, identity]):
            return harness.capture(root / 'compiler', root / 'evidence')

    def test_positive_typeck_and_zero_controls_full_recovery(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            report = self.run_capture(root)
            self.assertEqual(report['verdict'], 'passed')
            self.assertEqual(report['modes'][0]['live']['typeck_hits'], 0)
            self.assertGreater(report['modes'][1]['live']['typeck_hits'], 0)
            self.assertEqual(json.loads((root / 'evidence/receipt.json').read_text()), report)
            for mode in ('control', 'typeck'):
                receipt = json.loads((root / 'evidence' / mode / 'receipt.json').read_text())
                self.assertEqual(len(receipt['observations']), len(harness.cases()))
                self.assertTrue((root / 'evidence' / mode / '10-source.rs').exists())
                self.assertTrue((root / 'evidence' / mode / '10-resident.ll').exists())

    def test_mir_only_failure_keeps_all_raw_cases_including_error_recovery(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            report = self.run_capture(root, 'mir_hits')
            self.assertEqual(report['verdict'], 'failed')
            mode = report['modes'][1]
            expected_hits = sum(row[3] for row in harness.cases())
            self.assertEqual(len(mode['failures']), expected_hits)
            self.assertEqual(mode['observations'][10]['name'], 'recovery-unchanged')
            self.assertEqual(mode['observations'][-1]['name'], 'opaque-codegen-unchanged-again')
            self.assertEqual(mode['observations'][7]['resident']['status'], 1)
            self.assertEqual(mode['live']['typeck_hits'], 0)
            self.assertEqual(mode['live']['mir_hits'], expected_hits)
            self.assertIn('ensure_ok', mode['typeck_failure_analysis']['gate'])
            self.assertEqual(mode['observations'][1]['hit_deltas'],
                             {'typeck_hits': 0, 'mir_hits': 1})
            raw = (root / 'evidence/receipt.json').read_bytes()
            with self.assertRaises(FileExistsError):
                harness.capture(root / 'compiler', root / 'evidence')
            self.assertEqual((root / 'evidence/receipt.json').read_bytes(), raw)

    def test_identity_after_is_checked_and_failure_preserved(self):
        good = dict(sha256=harness.LAUNCHER, drivers=[dict(sha256=harness.DRIVER)])
        with tempfile.TemporaryDirectory() as directory:
            report = self.run_capture(Path(directory), identities=[good, dict(good, path='changed')])
            self.assertEqual(report['verdict'], 'failed')
            self.assertIn('identity changed', report['identity_error'])

    def test_shutdown_failure_preserves_receipts_and_checks_identity_after(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with self.assertRaisesRegex(RuntimeError, 'shutdown failure'):
                self.run_capture(root, shutdown=True)
            report = json.loads((root / 'evidence/receipt.json').read_text())
            self.assertEqual(report['verdict'], 'failed')
            self.assertIn('identity_after', report)
            mode = json.loads((root / 'evidence/control/receipt.json').read_text())
            self.assertIn('shutdown_error', mode)


if __name__ == '__main__':
    unittest.main()
