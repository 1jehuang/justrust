#!/usr/bin/env python3
"""Mock contract tests only. Never invokes a native compiler or build."""
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import stable_resident_mir_leaf as leaf
import stable_resident_typed as typed


def response(status=0, **changes):
    result = dict(status=status, pid=123, executed=True, stdout_b64='', stderr_b64='',
                  owned={'retirement_reason': None}, live=dict.fromkeys(leaf.COUNTERS, 0))
    result.update(changes)
    return result


def ir(a=7, b=9):
    return (('define i8 @leaf_a() {\n  ret i8 %d\n}\n' % a)
            + ('define i8 @leaf_b() {\n  ret i8 %d\n}\n' % b)).encode()


class ValidationTests(unittest.TestCase):
    def test_real_native_file_loader_fixture_and_fixed_demand(self):
        files = leaf.sources()
        self.assertEqual(set(files), {'fixture.rs', 'module_a.rs', 'module_b.rs'})
        self.assertIn(b'#![no_core]', files['fixture.rs'])
        self.assertIn(b'mod module_a;\nmod module_b;', files['fixture.rs'])
        self.assertNotIn(b'extern crate', b''.join(files.values()))
        args = leaf.arguments(Path('/frozen'))
        self.assertIn('--emit=llvm-ir=/frozen/fixture.ll,mir=/frozen/fixture.mir', args)
        self.assertNotIn('--extern', args)
        old = None
        cross = []
        for name, files, expected, kind in leaf.cases():
            changed = leaf.validate_transition(old, files, kind)
            if kind == 'cross':
                cross.append(changed)
            if kind == 'error':
                self.assertIsNone(expected)
                self.assertIn(b'-> u16 { 11u8 }', files['module_b.rs'])
            old = files
        self.assertEqual(cross, [['module_b.rs'], ['module_a.rs']])

    def test_cross_requires_actual_leaf_counter_not_aggregate_or_frontend(self):
        previous = response()['live']
        row = response(frontend={'mir_loads': 100}, resident_hits=100)
        row['live'].update(mir_hits=2, retained_entries=1)
        with self.assertRaisesRegex(AssertionError, 'mir_leaf_hits'):
            leaf.validate_live(row, previous, 'leaf', 'cross')
        row['live']['mir_leaf_hits'] = 1
        self.assertEqual(leaf.validate_live(row, previous, 'leaf', 'cross'), row['live'])
        row['live']['typeck_hits'] = 1
        with self.assertRaisesRegex(AssertionError, 'Typeck'):
            leaf.validate_live(row, previous, 'leaf', 'cross')

    def test_strict_control_same_source_error_and_recovery(self):
        for key in leaf.COUNTERS:
            row = response()
            row['live'][key] = 1
            with self.subTest(key=key), self.assertRaises(AssertionError):
                leaf.validate_live(row, None, 'control', 'cold')
        for kind in ('cold', 'error', 'recovery'):
            row = response()
            row['live'].update(mir_leaf_hits=1, retained_entries=1)
            with self.subTest(kind=kind), self.assertRaises(AssertionError):
                leaf.validate_live(row, None, 'leaf', kind)
        with self.assertRaisesRegex(AssertionError, 'actual live hits'):
            leaf.validate_live(response(), None, 'leaf', 'same')
        row = response()
        row['live'].update(mir_leaf_hits=1, retained_entries=1)
        leaf.validate_live(row, None, 'leaf', 'same')
        # Existing default typed gates were not loosened for leaf.
        with self.assertRaises(ValueError):
            typed.validate_live(row, None, 'leaf', True)
        with self.assertRaises(AssertionError):
            typed.validate_live(dict(row, live=dict(row['live'], mir_hits=1)), None, '1', False)

    def test_telemetry_fail_closed(self):
        for value in (None, {}, dict.fromkeys(leaf.COUNTERS, True)):
            with self.assertRaises(AssertionError):
                leaf.validate_live(response(live=value), None, 'leaf', 'cold')
        row = response()
        previous = dict(row['live'], mir_leaf_hits=1)
        with self.assertRaisesRegex(AssertionError, 'decreased'):
            leaf.validate_live(row, previous, 'leaf', 'cold')
        for changes in ({'executed': False}, {'owned': {'retirement_reason': 'budget'}}):
            with self.assertRaises(AssertionError):
                leaf.validate_live(response(**changes), None, 'leaf', 'cold')

    def test_both_exports_and_changed_artifact_negative_controls(self):
        leaf.artifact_oracle(ir(7, 11), (7, 11))
        for data in (ir(), ir(8, 11), ir(7, 11).replace(b'i8', b'i16'),
                     ir(7, 11).replace(b'@leaf_b', b'@wrong'),
                     ir(7, 11) + ir(7, 11),
                     ir(7, 11).replace(b'ret i8 11', b'call void @bad()\n ret i8 11')):
            with self.subTest(data=data), self.assertRaises(AssertionError):
                leaf.artifact_oracle(data, (7, 11))

    def test_exact_diagnostics_pid_and_status(self):
        row = response(status=1, stderr_b64='ZXJyb3IK')
        typed.validate_pair(row, row, None, 123)
        for change in ({'pid': 999}, {'stderr_b64': ''}, {'status': 0}, {'timed_out': True}):
            with self.assertRaises(AssertionError):
                typed.validate_pair(row, dict(row, **change), None, 123)


class CaptureTests(unittest.TestCase):
    def mock_contract(self, bad=None):
        workload = leaf.cases()
        workers = []
        fresh_calls = []

        def emit(root, index, engine, mode):
            _, files, expected, _ = workload[index]
            for name, data in files.items():
                self.assertEqual((root / name).read_bytes(), data)
            if expected is not None:
                values = (7, 9) if bad == 'stale' and mode == 'leaf' and index == 2 and engine == 'resident' else expected
                (root / 'fixture.ll').write_bytes(ir(*values))
                (root / 'fixture.mir').write_bytes(b'' if bad == 'empty-mir' else b'mock MIR\n')
            row = response(status=0 if expected else 1)
            if not expected:
                row['stderr_b64'] = 'ZXJyb3IK'
            return row

        class Worker:
            def __init__(self, compiler, root, mode, timeout):
                self.root, self.mode, self.index = root, mode, 0
                self.process = type('Process', (), {'pid': 123})()
                self.command = [str(compiler), '--jr-resident-owned']
                self.live = dict.fromkeys(leaf.COUNTERS, 0)
                self.argv = None
                self.closed = False
                workers.append(self)
                for name in ('resident.stdout', 'resident.stderr', 'resident.jsonl'):
                    (root / name).write_bytes((mode + ':' + name).encode())

            def compile(self, argv):
                if self.argv is not None:
                    assert self.argv == argv
                self.argv = list(argv)
                row = emit(self.root, self.index, 'resident', self.mode)
                kind = workload[self.index][3]
                if self.mode == 'leaf':
                    self.live['mir_misses'] += 1
                    self.live['retained_entries'] = 2
                    if kind == 'same':
                        self.live['mir_hits'] += 1
                    if kind == 'cross' and bad != 'no-leaf-hits':
                        self.live['mir_leaf_hits'] += 1
                row['live'] = dict(self.live)
                self.index += 1
                return row

            def close(self):
                self.closed = True
                if bad == 'shutdown':
                    raise RuntimeError('mock shutdown failure')

        def run(command, timeout, cwd, env):
            mode = env['JUSTRUST_RUSTC_RESIDENT_TYPED']
            index = sum(1 for item in fresh_calls if item[0] == mode)
            fresh_calls.append((mode, list(command), dict(env)))
            return emit(cwd, index, 'fresh', mode)

        return Worker, run, workers, fresh_calls

    def test_complete_contract_receipts_sources_streams_and_identity(self):
        worker, run, workers, calls = self.mock_contract()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with patch.object(leaf.typed, 'TypedResident', worker), patch.object(leaf.shared, 'run', run), \
                    patch.object(leaf.shared, 'identity', return_value={'sha256': 'frozen'}):
                result = leaf.capture(Path('/frozen'), root)
            self.assertEqual(result['verdict'], 'passed')
            self.assertEqual(result['identity_before'], result['identity_after'])
            self.assertTrue(all(w.closed for w in workers))
            self.assertEqual(workers[0].argv, workers[1].argv)
            for name, data in leaf.sources().items():
                self.assertEqual((root / 'workspace' / name).read_bytes(), data)
            for mode in ('control', 'leaf'):
                receipt = json.loads((root / mode / 'receipt.json').read_text())
                self.assertEqual(receipt['verdict'], 'passed')
                self.assertTrue(receipt['sources_restored'])
                self.assertEqual(len(receipt['observations']), len(leaf.cases()))
                self.assertEqual((root / mode / 'resident.jsonl').read_bytes(),
                                 (mode + ':resident.jsonl').encode())
                mode_calls = [call for call in calls if call[0] == mode]
                self.assertTrue(all(call[1:] == mode_calls[0][1:] for call in mode_calls))
            leaf_rows = result['modes'][1]['observations']
            self.assertGreater(leaf_rows[2]['resident']['live']['mir_leaf_hits'],
                               leaf_rows[1]['resident']['live']['mir_leaf_hits'])

    def test_failure_receipts_archive_raw_evidence_and_restore_sources(self):
        for bad in ('stale', 'no-leaf-hits', 'empty-mir', 'shutdown'):
            worker, run, workers, _ = self.mock_contract(bad)
            with self.subTest(bad=bad), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                with patch.object(leaf.typed, 'TypedResident', worker), patch.object(leaf.shared, 'run', run), \
                        patch.object(leaf.shared, 'identity', return_value={'sha256': 'frozen'}):
                    with self.assertRaises((AssertionError, RuntimeError)):
                        leaf.capture(Path('/frozen'), root)
                receipt = json.loads((root / 'receipt.json').read_text())
                self.assertEqual(receipt['verdict'], 'failed')
                mode = 'leaf' if bad in ('stale', 'no-leaf-hits') else 'control'
                mode_receipt = json.loads((root / mode / 'receipt.json').read_text())
                self.assertEqual(mode_receipt['verdict'], 'failed')
                self.assertTrue(mode_receipt['sources_restored'])
                self.assertTrue((root / mode / 'resident.jsonl').exists())
                if bad == 'stale':
                    self.assertEqual((root / mode / '02-resident.ll').read_bytes(), ir())
                self.assertTrue(all(w.closed for w in workers))
                for name, data in leaf.sources().items():
                    self.assertEqual((root / 'workspace' / name).read_bytes(), data)
                    self.assertEqual((root / mode / (name + '.rawbackup')).read_bytes(), data)

    def test_identity_drift_fails_final_receipt(self):
        worker, run, _, _ = self.mock_contract()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with patch.object(leaf.typed, 'TypedResident', worker), patch.object(leaf.shared, 'run', run), \
                    patch.object(leaf.shared, 'identity', side_effect=[{'sha256': 'a'}, {'sha256': 'b'}]):
                with self.assertRaisesRegex(AssertionError, 'identity changed'):
                    leaf.capture(Path('/frozen'), root)
            self.assertEqual(json.loads((root / 'receipt.json').read_text())['verdict'], 'failed')


if __name__ == '__main__':
    unittest.main()
