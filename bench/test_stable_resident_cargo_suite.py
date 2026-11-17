#!/usr/bin/env python3
"""Pure mock tests only, never native performance evidence."""
import copy
import json
from pathlib import Path
import tempfile
import unittest
from unittest import mock

import stable_resident_cargo_suite as suite
from test_stable_resident_cargo import FakeCargo


class SuiteTests(unittest.TestCase):
    def receipt(self, client=False):
        with tempfile.TemporaryDirectory(prefix='jr') as temp:
            root = Path(temp)
            compiler, cargo, binary = (root / name for name in ('rustc', 'cargo', 'justrust'))
            for path in (compiler, cargo, binary):
                path.write_bytes(path.name.encode())
            fake = FakeCargo()
            calls = []
            def runner(command, timeout, cwd, env):
                calls.append((list(map(str, command)), str(cwd), dict(env)))
                fake.index %= 8
                return fake(command, timeout, cwd, env)
            with mock.patch.object(suite.acceptance.native, 'run', side_effect=runner), \
                    mock.patch.object(suite.acceptance.native, 'identity', side_effect=suite.identity):
                result = suite.capture(compiler, cargo, root / 'o', justrust=binary if client else None)
            self.assertEqual(json.loads((root / 'o/receipt.json').read_text()), result)
            self.assertTrue((root / 'o/000-on/03-source.rs').exists())
            return result, calls

    def test_capture_counterbalanced_same_cwd_and_args_separate_targets(self):
        report, calls = self.receipt()
        self.assertEqual(len(calls), 32)
        self.assertEqual([p['order'] for p in report['repetitions']], [['off', 'on'], ['on', 'off']])
        self.assertEqual(len({str(c[:2]) for c in calls}), 1)
        self.assertEqual(len({c[2]['CARGO_TARGET_DIR'] for c in calls}), 2)
        self.assertEqual(set(report['summary']), set(suite.MEASURED))
        self.assertEqual(len(report['summary']['body-edit']['samples']['on']), 2)

    def test_real_client_boundary_is_timed_and_preserved(self):
        report, calls = self.receipt(client=True)
        self.assertTrue(all(c[0][1:3] == ['cargo', 'test'] for c in calls))
        self.assertTrue(all(c[2]['JUSTRUST_TOOLCHAIN'] == 'system' for c in calls))
        self.assertTrue(report['provenance']['candidates']['client'])

    def test_monotonic_actual_invocation_clock_not_reported_compiler_time(self):
        runner = suite.timed_runner(lambda *a, **kw: {'elapsed_seconds': 999}, clock=iter([100, 500000100]).__next__)
        record = runner(['/cargo', 'test'], 3, Path('/cwd'), {'A': 'B'})
        self.assertEqual(record['invocation_wall_seconds'], 0.5)
        self.assertEqual(record['elapsed_seconds'], 999)
        self.assertEqual(record['invocation']['environment'], {'A': 'B'})

    def test_compare_rejects_each_provenance_mismatch(self):
        report, _ = self.receipt()
        self.assertEqual(set(suite.compare(report, report)['stages']), set(suite.MEASURED))
        for key in report['provenance']:
            changed = copy.deepcopy(report)
            if key in ('source_b64', 'repetitions'):
                continue
            changed['provenance'][key] = 'mismatch'
            with self.subTest(key=key), self.assertRaises((ValueError, TypeError)):
                suite.compare(report, changed)

    def test_strict_evidence_rejection(self):
        report, _ = self.receipt()
        for mutation in (
                lambda r: r.update(verdict='failed'),
                lambda r: r['repetitions'][1].update(order=['off', 'on']),
                lambda r: r['repetitions'][0]['arms']['on']['observations'][3]['cargo'].update(invocation_wall_seconds=float('nan')),
                lambda r: r['repetitions'][0]['arms']['on']['observations'][3].update(wrapper_sha256='bad'),
                lambda r: r['repetitions'][0]['arms']['on']['observations'][3]['cargo']['invocation']['environment'].update(RUSTFLAGS='bad'),
                lambda r: r['repetitions'][0]['arms']['on']['observations'][3]['cargo']['invocation']['command'].append('--bad'),
                lambda r: r['repetitions'][0]['arms']['on']['observations'][3].update(trace=[])):
            changed = copy.deepcopy(report)
            mutation(changed)
            with self.assertRaises((ValueError, AssertionError)):
                suite.validate(changed)

    def test_reject_unbalanced_counts_before_execution(self):
        for count in (0, 1, 3, True):
            with self.assertRaises(ValueError):
                suite.capture('/missing', '/missing', '/missing', repetitions=count)


if __name__ == '__main__':
    unittest.main()
