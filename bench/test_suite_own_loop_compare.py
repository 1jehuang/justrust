#!/usr/bin/env python3
"""Mock-only suite dispatch coverage. Execution belongs to the coordinator."""
import argparse
import contextlib
import copy
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest import mock

import suite
import stable_resident_own_loop as own_loop
import test_stable_resident_own_loop as fixtures


class OwnLoopCompareDispatchTests(unittest.TestCase):
    def dispatch(self, before, after, **options):
        with tempfile.TemporaryDirectory() as directory:
            paths = [Path(directory) / name for name in ('before.json', 'after.json')]
            for path, report in zip(paths, (before, after)):
                path.write_text(json.dumps(report))
            args = argparse.Namespace(a=paths[0], b=paths[1], floor=10,
                                      allow_compiler_change=False, require_valid=False)
            for key, value in options.items():
                setattr(args, key, value)
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                suite.cmd_compare(args)
            return json.loads(output.getvalue())

    def test_exact_schema_dispatch_and_boolean_expectations(self):
        before = dict(schema=own_loop.SCHEMA, untouched='before')
        after = dict(schema=own_loop.SCHEMA, untouched='after')
        original = copy.deepcopy((before, after))
        with mock.patch.object(own_loop, 'compare', return_value={'scoped': 1}) as compare:
            result = self.dispatch(before, after, own_loop_before_macro_shm='0',
                                   own_loop_after_macro_shm='1', own_loop_before_macro_spins=0,
                                   own_loop_after_macro_spins=192)
        compare.assert_called_once_with(before, after, 10,
                                        expected_before_macro_shm=False,
                                        expected_after_macro_shm=True,
                                        expected_before_macro_spins=0,
                                        expected_after_macro_spins=192)
        self.assertEqual(result['comparison'], {'scoped': 1})
        self.assertIn('No general performance or default-promotion claim', result['claim'])
        self.assertEqual((before, after), original)

    def test_cross_schema_and_diagnostics_refuse_without_dispatch(self):
        for schemas in ((own_loop.SCHEMA, 'stable_native_cargo/v1'),
                        ('synthetic/v1', own_loop.SCHEMA),
                        (own_loop.DIAGNOSTIC_SCHEMA, own_loop.DIAGNOSTIC_SCHEMA),
                        (own_loop.SCHEMA, own_loop.DIAGNOSTIC_SCHEMA),
                        ('stable-resident-own-loop/v2', own_loop.SCHEMA)):
            with self.subTest(schemas=schemas), mock.patch.object(own_loop, 'compare') as compare:
                with self.assertRaises(SystemExit):
                    self.dispatch(*(dict(schema=schema) for schema in schemas))
                compare.assert_not_called()

    def test_wrong_workload_flags_and_broad_authorization_refuse(self):
        for flag, value in (('own_loop_before_macro_spins', 0),
                            ('own_loop_after_macro_spins', 192),
                            ('own_loop_before_macro_shm', '0'),
                            ('own_loop_after_macro_shm', '1')):
            with self.subTest(flag=flag), self.assertRaises(SystemExit):
                self.dispatch({}, {}, **{flag: value})
        for options in (dict(allow_compiler_change=True), dict(require_valid=True),
                        dict(own_loop_before_macro_shm='true')):
            with mock.patch.object(own_loop, 'compare') as compare:
                with self.assertRaises(SystemExit):
                    self.dispatch(dict(schema=own_loop.SCHEMA), dict(schema=own_loop.SCHEMA), **options)
                compare.assert_not_called()

    def test_exact_compiler_driver_hash_forwarding_and_complete_requirement(self):
        options = {'own_loop_' + side + '_' + component + '_sha256': digit * 64
                   for side, component, digit in (('before', 'compiler', 'a'), ('after', 'compiler', 'b'),
                                                   ('before', 'driver', 'c'), ('after', 'driver', 'd'))}
        before, after = (dict(schema=own_loop.SCHEMA, side=side) for side in ('before', 'after'))
        with mock.patch.object(own_loop, 'compare', return_value={}) as compare:
            self.dispatch(before, after, **options)
        expected = {key.replace('own_loop_', 'expected_', 1): value for key, value in options.items()}
        compare.assert_called_once_with(before, after, 10, expected_before_macro_shm=None,
                                        expected_after_macro_shm=None, expected_before_macro_spins=None,
                                        expected_after_macro_spins=None, **expected)
        for missing in options:
            incomplete = dict(options)
            incomplete.pop(missing)
            with self.subTest(missing=missing), mock.patch.object(own_loop, 'compare') as compare:
                with self.assertRaisesRegex(SystemExit, 'all four'):
                    self.dispatch(before, after, **incomplete)
                compare.assert_not_called()
        for schemas in (('synthetic/v1', 'synthetic/v1'), ('stable_native_cargo/v1', 'stable_native_cargo/v1'),
                        (own_loop.SCHEMA, 'synthetic/v1'), (own_loop.DIAGNOSTIC_SCHEMA, own_loop.DIAGNOSTIC_SCHEMA)):
            for flag, value in options.items():
                with self.subTest(schemas=schemas, flag=flag), mock.patch.object(own_loop, 'compare') as compare:
                    with self.assertRaises(SystemExit):
                        self.dispatch(*(dict(schema=schema) for schema in schemas), **{flag: value})
                    compare.assert_not_called()
        with mock.patch.object(own_loop, 'compare') as compare:
            with self.assertRaisesRegex(SystemExit, 'broad'):
                self.dispatch(before, after, allow_compiler_change=True, **options)
            compare.assert_not_called()

    def test_spin_parser_is_bounded_and_canonical(self):
        for text in ('0', '192', '256'):
            self.assertEqual(suite.own_loop_macro_spins(text), int(text))
        for text in ('-1', '257', '0192', '+1', '1.0', ' 1', '', '\u0661'):
            with self.subTest(text=text), self.assertRaises(argparse.ArgumentTypeError):
                suite.own_loop_macro_spins(text)

    def test_cli_registers_scoped_options_and_rejects_bad_choices(self):
        argv = ['suite.py', 'compare', 'a.json', 'b.json',
                '--own-loop-before-macro-spins', '0',
                '--own-loop-after-macro-spins', '192',
                '--own-loop-before-macro-shm', '0',
                '--own-loop-after-macro-shm', '1']
        with mock.patch('sys.argv', argv), mock.patch.object(suite, 'cmd_compare') as compare:
            suite.main()
        args = compare.call_args.args[0]
        self.assertEqual(args.own_loop_before_macro_spins, 0)
        self.assertEqual(args.own_loop_after_macro_spins, 192)
        self.assertEqual(args.own_loop_before_macro_shm, '0')
        self.assertEqual(args.own_loop_after_macro_shm, '1')
        hashes = []
        for side in ('before', 'after'):
            for component in ('compiler', 'driver'):
                hashes.extend(['--own-loop-' + side + '-' + component + '-sha256', 'a' * 64])
        with mock.patch('sys.argv', argv + hashes), mock.patch.object(suite, 'cmd_compare') as compare:
            suite.main()
        for side in ('before', 'after'):
            for component in ('compiler', 'driver'):
                self.assertEqual(getattr(compare.call_args.args[0], 'own_loop_' + side + '_' + component + '_sha256'), 'a' * 64)
        for flag, value in (('--own-loop-before-macro-shm', 'true'),
                            ('--own-loop-after-macro-spins', '257')):
            with mock.patch('sys.argv', ['suite.py', 'compare', 'a', 'b', flag, value]), \
                    contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
                suite.main()

    def test_strict_validation_errors_are_not_bypassed(self):
        with self.assertRaises((ValueError, KeyError)):
            self.dispatch(dict(schema=own_loop.SCHEMA), dict(schema=own_loop.SCHEMA))

    def test_actual_strict_compare_with_mock_captured_valid_receipts(self):
        helper = fixtures.OwnLoopTests()
        with tempfile.TemporaryDirectory() as directory:
            report, _, _ = helper.capture(Path(directory), commands=('check',),
                                          macro_shm=True, macro_spins=192)
            after = copy.deepcopy(report)
            result = self.dispatch(report, after, own_loop_before_macro_shm='1',
                                   own_loop_after_macro_shm='1', own_loop_before_macro_spins=192,
                                   own_loop_after_macro_spins=192)
            self.assertEqual(result['comparison'], own_loop.compare(report, after))
            after['provenance']['macro_spins'] = 193
            with self.assertRaises((ValueError, AssertionError)):
                self.dispatch(report, after, own_loop_before_macro_spins=192,
                              own_loop_after_macro_spins=193)

    def test_exact_hash_authorization_uses_strict_compare_without_other_exceptions(self):
        helper = fixtures.OwnLoopTests()
        with tempfile.TemporaryDirectory() as directory:
            before, _, _ = helper.capture(Path(directory), commands=('check',))
            after = copy.deepcopy(before)
            flags = {}
            expected = {}
            for component, digest in (('compiler', 'a' * 64), ('driver', 'b' * 64)):
                after['provenance']['attestation'][component].update(sha256=digest, path='/new/' + component)
                after['provenance']['expected'][component + '_sha256'] = digest
                for side, report in (('before', before), ('after', after)):
                    value = report['provenance']['expected'][component + '_sha256']
                    flags['own_loop_' + side + '_' + component + '_sha256'] = value
                    expected['expected_' + side + '_' + component + '_sha256'] = value
            for pair in after['repetitions']:
                for arm in pair['arms'].values():
                    for row in arm['observations']:
                        row['cargo']['invocation']['environment']['RUSTC'] = '/new/compiler'
            self.assertEqual(self.dispatch(before, after, **flags)['comparison'],
                             own_loop.compare(before, after, **expected))
            with self.assertRaisesRegex(ValueError, 'Provenance mismatch'):
                self.dispatch(before, after)
            for key in flags:
                for invalid in ('c' * 64, 'a' * 63, 'A' * 64, 'g' * 64, ''):
                    bad_flags = dict(flags, **{key: invalid})
                    with self.subTest(key=key, invalid=invalid), self.assertRaisesRegex(ValueError, 'hash mismatch'):
                        self.dispatch(before, after, **bad_flags)
            for field in ('manifest_b64', 'host'):
                bad = copy.deepcopy(after)
                bad['provenance'][field] = 'different-workload-or-host'
                with self.subTest(field=field), self.assertRaisesRegex(ValueError, 'Provenance mismatch'):
                    self.dispatch(before, bad, **flags)
            bad = copy.deepcopy(after)
            bad['provenance']['attestation']['cargo']['sha256'] = 'c' * 64
            bad['provenance']['expected']['cargo_sha256'] = 'c' * 64
            with self.assertRaisesRegex(ValueError, 'Provenance mismatch'):
                self.dispatch(before, bad, **flags)


if __name__ == '__main__':
    unittest.main()
