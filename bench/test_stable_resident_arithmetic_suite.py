#!/usr/bin/env python3
"""Python-only suite protocol tests. Never invokes a compiler or consumer."""
import argparse
import contextlib
import copy
import io
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import stable_resident_arithmetic_suite as suite


class ArithmeticSuiteTests(unittest.TestCase):
    def test_fixture_exact_root_and_single_function_edits(self):
        for items in (1, 7, 128):
            previous, old_constants = suite.fixture(items, 0)
            self.assertEqual(previous['fixture.rs'], suite.arithmetic.ROOT)
            self.assertEqual(len(old_constants), items * 2)
            for revision in range(1, 64):
                files, constants = suite.fixture(items, revision)
                self.assertEqual([n for n in files if files[n] != previous[n]],
                                 ['module_b.rs' if revision % 2 else 'module_a.rs'])
                changed = [n for n in constants if constants[n] != old_constants[n]]
                self.assertEqual(changed, ['leaf_b_0000' if revision % 2 else 'leaf_a_0000'])
                self.assertEqual(constants[changed[0]], old_constants[changed[0]] + 1)
                previous, old_constants = files, constants

    def test_independent_u32_oracle(self):
        _, constants = suite.fixture(8, 63)
        actual = list(map(int, suite.expected_output(constants).splitlines()))
        expected = []
        for name, constant in constants.items():
            for x, y in suite.arithmetic.INPUTS:
                if name.startswith('leaf_a_'):
                    expected.append((((x + y) % 2**32 ^ constant) * 3) % 2**32)
                else:
                    expected.append((x - constant) % 2**32 & 65535)
        self.assertEqual(actual, expected)
        source = suite.consumer_source(constants)
        self.assertEqual(source.count(b'printf('), len(constants) * len(suite.arithmetic.INPUTS))
        for name in constants:
            self.assertEqual(source.count(('extern uint32_t ' + name + '(').encode()), 1)

    def test_bounds_and_rotations(self):
        for items in (0, 129, True):
            with self.assertRaises(ValueError):
                suite.fixture(items, 0)
        for revision in (-1, 64, True):
            with self.assertRaises(ValueError):
                suite.fixture(1, revision)
        for edits in (1, 64, True):
            with self.assertRaises(ValueError):
                suite.workload(1, edits, 1)
        self.assertEqual(suite.workload(1, 2, 3)['order'],
                         [['fresh', 'control', 'leaf'], ['control', 'leaf', 'fresh'], ['leaf', 'fresh', 'control']])

    def test_explicit_attestation_required_before_any_execution(self):
        with self.assertRaises(TypeError):
            suite.capture('/absent', '/absent')
        with tempfile.TemporaryDirectory() as directory:
            compiler = Path(directory) / 'bin' / 'rustc'
            compiler.parent.mkdir()
            compiler.write_bytes(b'compiler')
            lib = Path(directory) / 'lib'
            lib.mkdir()
            (lib / 'librustc_driver.so').write_bytes(b'driver')
            with patch.object(suite.shared, 'identity', side_effect=AssertionError('must not execute')):
                with self.assertRaisesRegex(ValueError, 'compiler before execution'):
                    suite.capture(compiler, Path(directory) / 'output', expected_driver_sha256='0' * 64,
                                  expected_compiler_sha256='1' * 64)
                with self.assertRaisesRegex(ValueError, 'driver before execution'):
                    suite.capture(compiler, Path(directory) / 'output', expected_driver_sha256='0' * 64,
                                  expected_compiler_sha256=suite.sha(b'compiler'))
            self.assertFalse((Path(directory) / 'output').exists())

    def test_parser_requires_both_hashes_and_defaults(self):
        parser = argparse.ArgumentParser()
        suite.add_parser(parser.add_subparsers(required=True))
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
            parser.parse_args(['stable-native-arithmetic', '--compiler', '/c', '--output', '/o'])
        args = parser.parse_args(['stable-native-arithmetic', '--compiler', '/c', '--output', '/o',
                                 '--expected-compiler-sha256', '1' * 64, '--expected-driver-sha256', '2' * 64])
        self.assertEqual((args.items, args.edits, args.repetitions), (128, 6, 3))
        self.assertIs(args.fn, suite.run)

    def synthetic(self, root):
        spec = suite.workload(2, 2, 1)
        identity = dict(path='/attested/bin/rustc', sha256='1' * 64, drivers=[dict(sha256='2' * 64)])
        report = dict(schema=suite.SCHEMA, verdict='passed', expected_identity=suite.attestation('2' * 64, '1' * 64),
                      identity_before=identity, identity_after=copy.deepcopy(identity), workspace=str(root),
                      host=dict(boot_id='boot', affinity=[0], execution='local-native-owned'),
                      workload=spec, workload_sha256=suite.sha(suite.canonical(spec)), source_blobs={}, rows=[],
                      execution=dict(argv=suite.arithmetic.arguments(Path('.')), cwd='private-workspace',
                                     environment_sha256={a: '3' * 64 for a in suite.ARMS},
                                     cc=dict(path='/usr/bin/cc', sha256='4' * 64)))
        report['actual_execution'] = dict(argv=report['execution']['argv'], cwd=str(root))
        def artifact(name, data):
            path = root / name
            path.write_bytes(data)
            return dict(path=str(path), sha256=suite.sha(data))
        def record(command):
            return dict(command=command, status=0, stdout_b64='', stderr_b64='', elapsed_seconds=0.01)
        for arm in suite.order(0):
            for rev in range(3):
                files, constants = suite.fixture(2, rev)
                report['source_blobs'].update({suite.sha(d): suite.shared.encoded(d) for d in files.values()})
                producer = record([identity['path'], *report['execution']['argv']])
                if arm != 'fresh':
                    counters = {key: 0 for key in suite.leaf.COUNTERS}
                    if arm == 'leaf':
                        counters.update(mir_leaf_hits=rev * 2, retained_entries=4)
                    producer.update(pid=10 if arm == 'control' else 11, executed=True,
                                    request=dict(id=rev + 1, args=report['execution']['argv']), id=rev + 1,
                                    owned=dict(revisions=rev + 1, retirement_reason=None), live=counters)
                prefix = arm + str(rev)
                run = record([str(root / 'consumer')])
                run['stdout_b64'] = suite.shared.encoded(suite.expected_output(constants))
                report['rows'].append(dict(repetition=0, arm=arm, revision=rev, warmup=rev == 0, validated=True,
                    sources={n: suite.sha(d) for n, d in files.items()}, record=producer,
                    artifacts={s: artifact(prefix + '.' + s, b'artifact ' + s.encode()) for s in ('o', 'll', 'mir')},
                    consumer_source=artifact(prefix + '.c', suite.consumer_source(constants)),
                    consumer_binary=artifact(prefix + '.bin', b'consumer'), consumer_run=run,
                    consumer_compile=record(suite.consumer_argv(root)), oracle_sha256=suite.sha(suite.expected_output(constants))))
        report['aggregates'] = suite.aggregate(report['rows'])
        return report

    def test_offline_validation_and_tamper_gates(self):
        with tempfile.TemporaryDirectory() as directory:
            report = self.synthetic(Path(directory))
            self.assertEqual(suite.validate(report), report['aggregates'])
            mutations = [
                lambda r: r['rows'][1].update(warmup=True),
                lambda r: r['rows'][1]['sources'].update({'fixture.rs': 'bad'}),
                lambda r: r['rows'][1].update(validated=False),
                lambda r: r['rows'][4]['record'].update(pid=99),
                lambda r: r['rows'][4]['record']['live'].update(mir_leaf_hits=1),
                lambda r: r['rows'][7]['record']['live'].update(mir_leaf_hits=1),
                lambda r: r['rows'][7]['record']['owned'].update(revisions=65),
                lambda r: r['rows'][1]['consumer_run'].update(stdout_b64='bad'),
                lambda r: r['aggregates']['fresh'].update(n=99),
                lambda r: r['execution']['argv'].append('--bad'),
                lambda r: r['identity_after'].update(sha256='0' * 64),
            ]
            for mutate in mutations:
                broken = copy.deepcopy(report)
                mutate(broken)
                with self.assertRaises((ValueError, AssertionError)):
                    suite.validate(broken)
            path = Path(report['rows'][0]['artifacts']['o']['path'])
            path.write_bytes(b'corrupt')
            with self.assertRaisesRegex(ValueError, 'artifact identity'):
                suite.validate(report)

    def test_comparison_invariants_and_explicit_driver_change(self):
        with tempfile.TemporaryDirectory() as directory:
            a = self.synthetic(Path(directory))
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                suite.compare(a, copy.deepcopy(a))
            for arm in suite.ARMS:
                self.assertIn(f'{arm}: 0.010000s -> 0.010000s', output.getvalue())
            self.assertIn('before leaf/control: 1.0000x', output.getvalue())
            self.assertIn('after leaf/control: 1.0000x', output.getvalue())
            for key in ('boot_id', 'affinity', 'execution'):
                b = copy.deepcopy(a)
                b['host'][key] = 'different'
                with self.assertRaises(ValueError):
                    suite.compare(a, b)
            b = copy.deepcopy(a)
            b['execution']['environment_sha256']['leaf'] = '9' * 64
            with self.assertRaisesRegex(ValueError, 'different execution'):
                suite.compare(a, b)
            with self.assertRaisesRegex(ValueError, 'distinct attested drivers'):
                suite.compare(a, a, allow_compiler_change=True)
            b = copy.deepcopy(a)
            b['expected_identity']['driver_sha256'] = '5' * 64
            for key in ('identity_before', 'identity_after'):
                b[key]['drivers'][0]['sha256'] = '5' * 64
            with self.assertRaisesRegex(ValueError, 'different identity_before'):
                suite.compare(a, b)
            with contextlib.redirect_stdout(io.StringIO()):
                suite.compare(a, b, allow_compiler_change=True)

    def test_aggregate_excludes_warmup_and_refuses_failure(self):
        with tempfile.TemporaryDirectory() as directory:
            report = self.synthetic(Path(directory))
            report['rows'][0]['record']['elapsed_seconds'] = 999
            self.assertEqual(suite.aggregate(report['rows'])['fresh']['median_seconds'], .01)
            report['rows'][1]['record']['status'] = 1
            with self.assertRaises(ValueError):
                suite.aggregate(report['rows'])


if __name__ == '__main__':
    unittest.main()
