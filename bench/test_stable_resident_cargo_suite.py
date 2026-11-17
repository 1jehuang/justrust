#!/usr/bin/env python3
"""Pure mock tests only, never native performance evidence."""
import argparse
import contextlib
import io
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
            (root / 'bin').mkdir()
            (root / 'lib').mkdir()
            compiler, cargo, binary = root / 'bin/rustc', root / 'cargo', root / 'justrust'
            driver = root / 'lib/librustc_driver.so'
            for path in (compiler, cargo, binary, driver):
                path.write_bytes(path.name.encode())
            fake = FakeCargo()
            calls = []
            def runner(command, timeout, cwd, env):
                calls.append((list(map(str, command)), str(cwd), dict(env)))
                fake.index %= 8
                return fake(command, timeout, cwd, env)
            with mock.patch.object(suite.acceptance.native, 'run', side_effect=runner), \
                    mock.patch.object(suite.acceptance.native, 'identity',
                                      side_effect=lambda path: dict(suite.identity(path), drivers=[suite.identity(driver)])):
                result = suite.capture(compiler, cargo, root / 'o', justrust=binary if client else None,
                                       expected_compiler_sha256=suite.identity(compiler)['sha256'],
                                       expected_driver_sha256=suite.identity(driver)['sha256'],
                                       expected_cargo_sha256=suite.identity(cargo)['sha256'],
                                       expected_client_sha256=suite.identity(binary)['sha256'] if client else None)
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
        with contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(set(suite.compare(report, report, floor=10)['stages']), set(suite.MEASURED))
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

    def test_attestation_failure_precedes_any_candidate_execution(self):
        with tempfile.TemporaryDirectory(prefix='jr') as temp:
            root = Path(temp)
            (root / 'bin').mkdir()
            (root / 'lib').mkdir()
            compiler, cargo, driver = root / 'bin/rustc', root / 'cargo', root / 'lib/librustc_driver.so'
            for path in (compiler, cargo, driver):
                path.write_bytes(b'candidate')
            with mock.patch.object(suite.acceptance.native, 'run') as runner:
                for key in ('compiler', 'driver', 'cargo'):
                    hashes = {f'expected_{name}_sha256': suite.identity(path)['sha256']
                              for name, path in (('compiler', compiler), ('cargo', cargo), ('driver', driver))}
                    hashes[f'expected_{key}_sha256'] = '0' * 64
                    with self.assertRaisesRegex(ValueError, 'Unattested'):
                        suite.capture(compiler, cargo, root / 'o', **hashes)
                runner.assert_not_called()
            self.assertFalse((root / 'o').exists())

    def test_secrets_removed_before_invocation_and_receipt(self):
        seen = []
        invoke = suite.timed_runner(lambda *a, **kw: seen.append(kw['env']) or {},
                                    clock=iter([1, 100]).__next__)
        env = dict(RUSTFLAGS='keep', AWS_SECRET_ACCESS_KEY='private', GITHUB_TOKEN='private',
                   CLOUDFLARE_API_KEY='private', CARGO_REGISTRY_TOKEN='private', PATH='/bin')
        result = invoke(['/cargo'], 3, Path('/cwd'), env)
        self.assertEqual(seen, [dict(RUSTFLAGS='keep', PATH='/bin')])
        self.assertNotIn('private', json.dumps(result))
        with mock.patch.dict(suite.os.environ, {'GITHUB_TOKEN': 'private-leak-check'}):
            report, _ = self.receipt()
        self.assertNotIn('private-leak-check', json.dumps(report))

    def test_parser_contract(self):
        parser = argparse.ArgumentParser()
        suite.add_parser(parser.add_subparsers())
        args = parser.parse_args(['stable-native-cargo', '--compiler', '/c', '--cargo', '/g',
                                  '--output', '/o', '--expected-compiler-sha256', '1' * 64,
                                  '--expected-driver-sha256', '2' * 64,
                                  '--expected-cargo-sha256', '3' * 64])
        self.assertIs(args.fn, suite.run)
        self.assertEqual(args.repetitions, 2)
        self.assertEqual(suite.SCHEMA, 'stable_native_cargo/v1')

    def test_reject_unbalanced_counts_before_execution(self):
        for count in (0, 1, 3, True):
            with self.assertRaises(ValueError):
                suite.capture('/missing', '/missing', '/missing', repetitions=count)


if __name__ == '__main__':
    unittest.main()
