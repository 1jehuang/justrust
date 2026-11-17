#!/usr/bin/env python3
"""Pure Python mocks. No compiler, Cargo, client, worker, or benchmark execution."""
import copy
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import stable_resident_cargo_dependency_suite as subject
import test_stable_resident_cargo_dependency as fixtures


class DependencyCostTests(unittest.TestCase):
    setUp = fixtures.DependencyAcceptanceTests.setUp
    mock_runner = fixtures.DependencyAcceptanceTests.mock_runner

    def capture(self, **overrides):
        runner, self.calls = self.mock_runner(client=True, resident_root=True)
        options = dict(self.options, expected_client_sha256=subject.shared.native.digest(self.client))
        options.update(overrides)
        with patch.object(subject.uuid, 'uuid4') as nonce:
            nonce.return_value.hex = 'fixed-mock-nonce'
            return subject.capture(self.compiler, self.cargo, self.root / 'o', self.client,
                                   runner=runner, **options)

    def test_counterbalance_actual_client_originals_and_cold_targets(self):
        report = self.capture()
        self.assertEqual([p['order'] for p in report['repetitions']], [['off', 'on'], ['on', 'off']])
        self.assertEqual(len(self.calls), 40)
        self.assertEqual([self.calls[i]['mode'] for i in (0, 10, 20, 30)], ['off', 'on', 'on', 'off'])
        self.assertEqual((self.root / 'o/original.rs').read_bytes(), subject.shared.SOURCE.read_bytes())
        for index in range(2):
            for mode in ('off', 'on'):
                arm = self.root / 'o' / f'{index:03d}-{mode}'
                self.assertTrue((arm / 'target-evidence/libfixture_dep.rlib').is_file())
                self.assertTrue((arm / '00-root.rs').is_file())
        self.assertEqual(len(report['summary']['stages']), 10)
        self.assertEqual(len(report['summary']['stages']['root-only-edit']['paired_on_minus_off_seconds']), 2)
        self.assertEqual(subject.validate(report), report['summary'])

    def test_gate_and_counterbalance_reject_before_execution(self):
        for options in ({'root_native_frozen': False}, {'idle_authorized': False},
                        {'repetitions': 1}, {'repetitions': 3}, {'repetitions': True},
                        {'timeout': float('nan')}, {'timeout': 0}):
            with self.subTest(options=options), self.assertRaises(ValueError):
                self.capture(**options)
            self.assertFalse((self.root / 'o').exists())
            self.assertEqual(self.calls, [])

    def test_all_four_hashes_required(self):
        for component in ('compiler', 'driver', 'cargo', 'client'):
            for digest in (None, '0' * 64):
                with self.subTest(component=component, digest=digest), self.assertRaises(ValueError):
                    self.capture(**{'expected_' + component + '_sha256': digest})
                self.assertEqual(self.calls, [])
                self.assertFalse((self.root / 'o').exists())

    def test_receipt_tampering_rejected(self):
        report = self.capture()
        def row(receipt):
            return receipt['repetitions'][0]['arms']['on']['observations'][1]
        changes = [lambda r: r['repetitions'][1].update(order=['off', 'on']),
                   lambda r: r['provenance']['expected'].update(client_sha256='0' * 64),
                   lambda r: r['repetitions'][0]['workload'][1].update(root_b64='bad'),
                   lambda r: row(r).update(root_sha256='0' * 64),
                   lambda r: row(r)['cargo'].update(invocation_wall_seconds=float('nan')),
                   lambda r: row(r)['cargo']['invocation'].update(command=['cargo']),
                   lambda r: row(r)['cargo']['invocation']['environment'].update(API_KEY='secret'),
                   lambda r: row(r)['trace'][0]['result']['owned'].update(revisions=0)]
        for change in changes:
            receipt = copy.deepcopy(report)
            change(receipt)
            with self.assertRaises((ValueError, AssertionError)):
                subject.validate(receipt)

    def test_failure_keeps_original_output_and_receipts(self):
        with patch.object(subject.acceptance, 'capture_arm', side_effect=RuntimeError('mock failed')):
            with self.assertRaisesRegex(RuntimeError, 'mock failed'):
                self.capture()
        receipt = json.loads((self.root / 'o/receipt.json').read_text())
        self.assertEqual(receipt['verdict'], 'failed')
        self.assertTrue((self.root / 'o/original.rs').exists())

    def test_actual_client_mandatory(self):
        with self.assertRaisesRegex(ValueError, 'Actual justrust'):
            subject.capture(None, None, self.root / 'o', None,
                            root_native_frozen=True, idle_authorized=True)


class WorkerRssTests(unittest.TestCase):
    def test_local_proc_units_and_missing_worker(self):
        with tempfile.TemporaryDirectory() as directory:
            proc = Path(directory)
            worker = proc / '42'
            worker.mkdir()
            (worker / 'stat').write_text('42 (worker name) ' + ' '.join(['S'] + ['0'] * 18 + ['1234']))
            (worker / 'status').write_text('Name:\tworker\nVmRSS:\t17 kB\n')
            result = subject.worker_rss(42, proc)
            self.assertTrue(result['available'])
            self.assertEqual(result['rss_bytes'], 17 * 1024)
            self.assertEqual(result['starttime_ticks'], 1234)
            self.assertFalse(subject.worker_rss(43, proc)['available'])
            self.assertFalse(subject.worker_rss(True, proc)['available'])
            (worker / 'status').write_text('VmRSS: 17 MB\n')
            self.assertFalse(subject.worker_rss(42, proc)['available'])

    def test_sampler_includes_earlier_resident_generations_and_deduplicates(self):
        with tempfile.TemporaryDirectory() as directory:
            trace = Path(directory) / 'trace'
            trace.write_text(json.dumps(dict(event='complete', result=dict(pid=1))) + '\n')
            def runner(command, timeout, cwd, env):
                with trace.open('a') as stream:
                    for pid in (42, 42, 43):
                        stream.write(json.dumps(dict(event='complete', result=dict(pid=pid))) + '\n')
                return dict(status=0)
            with patch.object(subject.safety, 'timed_runner', side_effect=lambda r: r):
                invoke = subject.observed_runner(runner, sampler=lambda pid: dict(pid=pid))
                record = invoke(['mock'], 1, Path(directory), {'JUSTRUST_CARGO_RESIDENT_TRACE': str(trace)})
            self.assertEqual(record['resident_worker_rss'], [dict(pid=1), dict(pid=42), dict(pid=43)])


if __name__ == '__main__':
    unittest.main()
