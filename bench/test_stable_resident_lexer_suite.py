#!/usr/bin/env python3
"""Mock/offline-only lexer suite tests. Never invokes a native compiler."""
import argparse
import contextlib
import copy
import io
import json
import re
from types import SimpleNamespace
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import stable_resident_lexer_suite as suite


class LexerSuiteTests(unittest.TestCase):
    def test_fixture_edits_are_one_constant_in_one_module(self):
        for items in (1, 7, 128):
            previous, old = suite.fixture(items, 0)
            self.assertEqual(previous['fixture.rs'], b'pub mod module_a;\npub mod module_b;\n')
            self.assertNotEqual(previous['module_a.rs'], previous['module_b.rs'])
            self.assertEqual(len(old), 2 * items)
            for revision in range(1, 33):
                files, constants = suite.fixture(items, revision)
                module = 'b' if revision % 2 else 'a'
                self.assertEqual([n for n in files if files[n] != previous[n]], [f'module_{module}.rs'])
                self.assertEqual([n for n in constants if constants[n] != old[n]], [f'module_{module}::value_0000'])
                self.assertEqual(constants[f'module_{module}::value_0000'], old[f'module_{module}::value_0000'] + 1)
                self.assertTrue(all(d.isascii() for d in files.values()))
                self.assertTrue(all(b'#' not in d and b'unsafe' not in d for d in files.values()))
                previous, old = files, constants

    def test_independent_oracle_and_all_function_calls(self):
        _, constants = suite.fixture(7, 32)
        nonce = 'a' * 32
        expected = [nonce]
        for constant in constants.values():
            for left, right in suite.INPUTS:
                # Independent bitwise expansion of XOR for bounded inputs.
                summed = left + right
                xor = sum((2 ** bit) * (((summed // 2 ** bit) % 2 + (constant // 2 ** bit) % 2) % 2)
                          for bit in range(32))
                expected.append(str(xor + xor + xor))
        self.assertEqual(suite.expected_output(constants, nonce), ('\n'.join(expected) + '\n').encode())
        source = suite.consumer_source(constants, nonce)
        for name in constants:
            self.assertEqual(source.count(('lexer_fixture::' + name + '(').encode()), len(suite.INPUTS))
        self.assertIn(nonce.encode(), source)
        argv = suite.arguments(Path('.'))
        self.assertIn('--crate-type=rlib', argv)
        self.assertIn('--emit=link', argv)
        self.assertFalse(any('mir' in arg or 'llvm' in arg or 'metadata' in arg for arg in argv))
        self.assertIn('--extern', suite.consumer_argv('/compiler', Path('/root')))

    def test_bounds_rotation_and_gates(self):
        for items in (0, 129, True):
            with self.assertRaises(ValueError):
                suite.fixture(items, 0)
        for revision in (-1, 33, True):
            with self.assertRaises(ValueError):
                suite.fixture(1, revision)
        for edits in (1, 33, True):
            with self.assertRaises(ValueError):
                suite.workload(1, edits, 1)
        for repetitions in (0, True):
            with self.assertRaises(ValueError):
                suite.workload(1, 2, repetitions)
        self.assertEqual([suite.order(n) for n in range(3)],
                         [['fresh', 'control', 'lexer'], ['control', 'lexer', 'fresh'], ['lexer', 'fresh', 'control']])
        with patch.dict('os.environ', {'JUSTRUST_RUSTC_RESIDENT_TYPED': 'leaf', 'JUSTRUST_RUSTC_RESIDENT_VALUES': '1'}):
            environments = {arm: suite.environment(arm) for arm in suite.ARMS}
        for arm, env in environments.items():
            self.assertEqual(env['JUSTRUST_RUSTC_RESIDENT_TYPED'], 'control')
            self.assertNotIn('JUSTRUST_RUSTC_RESIDENT_VALUES', env)
            self.assertEqual(env['JUSTRUST_RUSTC_RESIDENT_LEXER'], '1' if arm == 'lexer' else '0')
        self.assertEqual(environments['fresh'], environments['control'])
        self.assertEqual({k: v for k, v in environments['lexer'].items() if k != 'JUSTRUST_RUSTC_RESIDENT_LEXER'},
                         {k: v for k, v in environments['control'].items() if k != 'JUSTRUST_RUSTC_RESIDENT_LEXER'})

    def test_cli_attestation_before_execution(self):
        parser = argparse.ArgumentParser()
        suite.add_parser(parser.add_subparsers(required=True))
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
            parser.parse_args(['stable-native-lexer', '--compiler', '/c', '--output', '/o'])
        args = parser.parse_args(['stable-native-lexer', '--compiler', '/c', '--output', '/o',
                                 '--expected-driver-sha256', '2' * 64, '--expected-compiler-sha256', '1' * 64])
        self.assertIs(args.fn, suite.run)
        self.assertEqual((args.items, args.edits, args.repetitions), (128, 6, 3))
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            compiler = root / 'bin/rustc'
            compiler.parent.mkdir()
            compiler.write_bytes(b'compiler')
            (root / 'lib').mkdir()
            (root / 'lib/librustc_driver.so').write_bytes(b'driver')
            with patch.object(suite.shared, 'run', side_effect=AssertionError('native forbidden')), \
                 patch.object(suite.shared, 'identity', side_effect=AssertionError('native forbidden')):
                for binary, driver in [('0' * 64, '2' * 64), (suite.sha(b'compiler'), '0' * 64), ('BAD', '0' * 64)]:
                    with self.assertRaises(ValueError):
                        suite.capture(compiler, root / 'out', expected_compiler_sha256=binary,
                                      expected_driver_sha256=driver)
            self.assertFalse((root / 'out').exists())

    def synthetic(self, root):
        spec = suite.workload(2, 2, 1)
        identity = dict(path='/attested/bin/rustc', sha256='1' * 64, drivers=[dict(sha256='2' * 64)])
        report = dict(schema=suite.SCHEMA, verdict='passed', expected_identity=suite.attestation('2' * 64, '1' * 64),
                      identity_before=identity, identity_after=copy.deepcopy(identity), workspace=str(root),
                      host=dict(boot_id='00000000-0000-0000-0000-000000000001', affinity=[0], execution='local-native-owned'),
                      workload=spec, workload_sha256=suite.sha(suite.canonical(spec)), source_blobs={}, rows=[], sessions=[],
                      execution=suite.execution_identity())
        report['actual_execution'] = dict(argv=report['execution']['argv'], cwd=str(root))
        def artifact(name, data):
            path = root / name
            path.write_bytes(data)
            return dict(path=str(path), sha256=suite.sha(data))
        def record(command):
            return dict(command=command, status=0, stdout_b64='', stderr_b64='', elapsed_seconds=0.01)
        for arm in suite.order(0):
            frames = []
            session = dict(arm=arm, repetition=0, streams={})
            report['sessions'].append(session)
            if arm != 'fresh':
                session.update(pid=10 if arm == 'control' else 11, command=[identity['path'], '--jr-resident-owned'],
                               environment_sha256=report['execution']['environment_sha256'][arm])
            for step in range(4):
                rev = max(0, step - 1)
                files, constants = suite.fixture(2, rev)
                report['source_blobs'].update({suite.sha(d): suite.shared.encoded(d) for d in files.values()})
                producer = record([identity['path'], *report['execution']['argv']])
                if arm != 'fresh':
                    live = {key: 0 for key in (*suite.typed.COUNTERS, 'mir_leaf_hits', 'mir_leaf_candidate_misses',
                            'identity_begin_rejections', 'identity_complete_rejections', 'published_revisions', 'aborted_revisions')}
                    live['mir_leaf_last_rejection'] = None
                    counters = {key: 0 for key in suite.lexer.COUNTERS}
                    if arm == 'lexer':
                        counters.update(hits=step * 2, misses=2 + rev, retained_entries=2, retained_estimated_bytes=100)
                    producer.update(pid=session['pid'], executed=True, request=dict(id=step + 1, args=report['execution']['argv']),
                                    id=step + 1, owned=dict(revisions=step + 1, retirement_reason=None), live=live, lexer=counters,
                                    worker_elapsed_seconds=0.005)
                    wire = {k: v for k, v in producer.items() if k not in ('request', 'command', 'worker_elapsed_seconds', 'stdout_b64', 'stderr_b64')}
                    wire['elapsed_seconds'] = 0.005
                    frames.extend([dict(request=producer['request']), dict(response_b64=suite.shared.encoded(json.dumps(wire).encode()))])
                prefix = arm + str(step)
                nonce = suite.sha(prefix.encode())[:32]
                runtime = record([str(root / 'consumer')])
                runtime['stdout_b64'] = suite.shared.encoded(suite.expected_output(constants, nonce))
                report['rows'].append(dict(repetition=0, arm=arm, step=step, revision=rev, kind='cold' if step == 0 else 'unchanged' if step == 1 else 'edit',
                    warmup=step < 2, validated=True, sources={n: suite.sha(d) for n, d in files.items()}, record=producer,
                    artifacts={'rlib': artifact(prefix + '.rlib', b'!<arch>\nmock Rust archive')}, nonce=nonce,
                    consumer_rlib_sha256=suite.sha(b'!<arch>\nmock Rust archive'),
                    consumer_source=artifact(prefix + '.rs', suite.consumer_source(constants, nonce)),
                    consumer_binary=artifact(prefix + '.bin', b'mock consumer'), consumer_run=runtime,
                    consumer_compile=record(suite.consumer_argv(identity['path'], root)), oracle_sha256=suite.sha(suite.expected_output(constants, nonce))))
            if arm != 'fresh':
                for stream in ('resident.stdout', 'resident.stderr', 'resident.jsonl'):
                    data = b''.join(json.dumps(frame).encode() + b'\n' for frame in frames) if stream.endswith('jsonl') else b''
                    session['streams'][stream] = artifact(arm + '.' + stream, data)
        report['aggregates'] = suite.aggregate(report['rows'])
        return report

    def test_offline_success_and_comparison(self):
        with tempfile.TemporaryDirectory() as directory:
            report = self.synthetic(Path(directory))
            self.assertEqual(suite.validate(report), report['aggregates'])
            self.assertTrue(all(v['n'] == 2 for v in report['aggregates'].values()))
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                suite.compare(report, copy.deepcopy(report))
            for arm in suite.ARMS:
                self.assertIn(arm + ': ', output.getvalue())
            self.assertEqual(output.getvalue().count('lexer/processcontrol:'), 2)
            with self.assertRaises(ValueError):
                suite.compare(report, report, allow_compiler_change=True)

    def test_unsafe_offline_mutations_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            report = self.synthetic(Path(directory))
            mutations = [
                lambda r: r.update(schema='other'),
                lambda r: r['identity_after'].update(sha256='0' * 64),
                lambda r: r['execution']['argv'].append('--emit=mir'),
                lambda r: r['execution']['gates']['lexer'].update(JUSTRUST_RUSTC_RESIDENT_TYPED='leaf'),
                lambda r: r['execution']['common_environment_sha256'].update(lexer='0' * 64),
                lambda r: r['host'].update(boot_id='boot'),
                lambda r: r['host'].update(affinity=[False]),
                lambda r: r['rows'][2]['sources'].update({'fixture.rs': '0' * 64}),
                lambda r: r['rows'][2].update(warmup=True),
                lambda r: r['rows'][2].update(nonce=r['rows'][1]['nonce']),
                lambda r: r['rows'][2]['consumer_run'].update(stdout_b64=''),
                lambda r: r['rows'][2]['consumer_compile']['command'].append('--emit=mir'),
                lambda r: r['rows'][2]['artifacts']['rlib'].update(sha256='0' * 64),
                lambda r: r['rows'][6]['record'].update(pid=12),
                lambda r: r['rows'][6]['record']['lexer'].update(hits=1),
                lambda r: r['rows'][10]['record']['lexer'].update(hits=2),
                lambda r: r['rows'][10]['record']['live'].update(mir_misses=1),
                lambda r: r['rows'][10]['record']['live'].update(mir_leaf_candidate_misses=1),
                lambda r: r['rows'][10]['record']['owned'].update(retirement_reason='capacity'),
                lambda r: r['rows'][10]['record']['request']['args'].append('--emit=mir'),
                lambda r: r['sessions'][1]['streams'].pop('resident.jsonl'),
                lambda r: r['sessions'][1].update(environment_sha256='0' * 64),
                lambda r: r['aggregates']['lexer'].update(n=3),
                lambda r: r['rows'][2]['record'].update(elapsed_seconds=float('nan')),
                lambda r: r['rows'][0]['record'].update(elapsed_seconds=-1),
                lambda r: r['rows'][2]['record'].update(status=False),
                lambda r: r['rows'][2].update(consumer_rlib_sha256='0' * 64),
            ]
            for index, mutate in enumerate(mutations):
                with self.subTest(index=index):
                    changed = copy.deepcopy(report)
                    mutate(changed)
                    with self.assertRaises((ValueError, KeyError)):
                        suite.validate(changed)

    def test_compare_mismatches_and_distinct_driver_opt_in(self):
        with tempfile.TemporaryDirectory() as directory:
            report = self.synthetic(Path(directory))
            for key, value in [('boot_id', '00000000-0000-0000-0000-000000000002'), ('affinity', [1])]:
                changed = copy.deepcopy(report)
                changed['host'][key] = value
                with self.assertRaisesRegex(ValueError, 'different host'):
                    suite.compare(report, changed)
            changed = copy.deepcopy(report)
            changed['expected_identity']['driver_sha256'] = '3' * 64
            changed['identity_before']['drivers'][0]['sha256'] = '3' * 64
            changed['identity_after']['drivers'][0]['sha256'] = '3' * 64
            with self.assertRaisesRegex(ValueError, 'different identity_before'):
                suite.compare(report, changed)
            with contextlib.redirect_stdout(io.StringIO()):
                suite.compare(report, changed, allow_compiler_change=True)
            for floor in (-1, float('nan'), True):
                with self.assertRaises(ValueError):
                    suite.compare(report, report, floor)

    def test_mock_full_capture_independent_banks_and_raw_transport(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            compiler = root / 'bin/rustc'
            compiler.parent.mkdir()
            compiler.write_bytes(b'compiler')
            (root / 'lib').mkdir()
            (root / 'lib/librustc_driver.so').write_bytes(b'driver')
            identity = dict(path=str(compiler), sha256=suite.sha(b'compiler'), drivers=[dict(sha256=suite.sha(b'driver'))])
            bank_counts = []
            def result(command):
                return dict(command=list(map(str, command)), status=0, elapsed_seconds=0.01, stdout_b64='', stderr_b64='')
            def producer(cwd):
                bank = cwd / 'incremental'
                bank.mkdir(exist_ok=True)
                marker = bank / 'count'
                count = int(marker.read_text()) + 1 if marker.exists() else 1
                marker.write_text(str(count))
                bank_counts.append(count)
                (cwd / 'libfixture.rlib').write_bytes(b'!<arch>\nmock produced archive ' + str(count).encode())
            def run(command, timeout, cwd, env):
                record = result(command)
                if '--crate-type=rlib' in command:
                    producer(cwd)
                elif '--extern' in command:
                    (cwd / 'consumer').write_bytes(b'mock linked binary')
                else:
                    source = (cwd / 'consumer.rs').read_text()
                    nonce = re.search(r'println!\("([0-9a-f]{32})"\)', source).group(1)
                    values = [nonce]
                    for module, name, left, right in re.findall(r'lexer_fixture::(module_[ab])::(value_\d+)\((\d+), (\d+)\)', source):
                        module_source = (cwd / (module + '.rs')).read_text()
                        constant = int(re.search(r'pub fn ' + name + r'\(.*?\^ (\d+)\)', module_source).group(1))
                        values.append(str(3 * ((int(left) + int(right)) ^ constant)))
                    record['stdout_b64'] = suite.shared.encoded(('\n'.join(values) + '\n').encode())
                return record
            class Worker:
                next_pid = 100
                def __init__(self, compiler, cwd, arm, timeout):
                    self.cwd, self.arm, self.sequence = cwd, arm, 0
                    self.command = [str(compiler), '--jr-resident-owned']
                    self.environment_sha256 = suite.sha(suite.canonical(suite.environment(arm)))
                    Worker.next_pid += 1
                    self.process = SimpleNamespace(pid=Worker.next_pid)
                    self.frames = []
                def compile(self, argv):
                    producer(self.cwd)
                    self.sequence += 1
                    step = self.sequence - 1
                    live = {key: 0 for key in (*suite.typed.COUNTERS, 'mir_leaf_hits', 'mir_leaf_candidate_misses',
                        'identity_begin_rejections', 'identity_complete_rejections', 'published_revisions', 'aborted_revisions')}
                    live['mir_leaf_last_rejection'] = None
                    counters = dict.fromkeys(suite.lexer.COUNTERS, 0)
                    if self.arm == 'lexer':
                        counters.update(hits=step * 2, misses=2 + max(0, step - 1), retained_entries=2, retained_estimated_bytes=100)
                    request = dict(id=self.sequence, args=argv)
                    wire = dict(id=self.sequence, pid=self.process.pid, status=0, executed=True, elapsed_seconds=0.005,
                        owned=dict(revisions=self.sequence, retirement_reason=None), live=live, lexer=counters)
                    self.frames.extend([dict(request=request), dict(response_b64=suite.shared.encoded(json.dumps(wire).encode()))])
                    return dict(wire, request=request, elapsed_seconds=0.01, worker_elapsed_seconds=0.005, stdout_b64='', stderr_b64='')
                def close(self):
                    (self.cwd / 'resident.stdout').write_bytes(b'')
                    (self.cwd / 'resident.stderr').write_bytes(b'')
                    (self.cwd / 'resident.jsonl').write_bytes(b''.join(json.dumps(f).encode() + b'\n' for f in self.frames))
            with patch.object(suite.shared, 'identity', return_value=identity), patch.object(suite.shared, 'run', side_effect=run), \
                 patch.object(suite, 'LexerResident', Worker):
                report = suite.capture(compiler, root / 'out', 2, 2, 2,
                    expected_compiler_sha256=identity['sha256'], expected_driver_sha256=identity['drivers'][0]['sha256'])
            self.assertEqual(bank_counts, [1, 2, 3, 4] * 6)
            self.assertEqual(len(report['rows']), 24)
            self.assertTrue(all(value['n'] == 4 for value in suite.validate(report).values()))
            self.assertEqual(json.loads((root / 'out/receipt.json').read_text())['verdict'], 'passed')

    def test_mock_producer_failure_preserves_originals(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            compiler = root / 'bin/rustc'
            compiler.parent.mkdir()
            compiler.write_bytes(b'compiler')
            (root / 'lib').mkdir()
            (root / 'lib/librustc_driver.so').write_bytes(b'driver')
            identity = dict(path=str(compiler), sha256=suite.sha(b'compiler'), drivers=[dict(sha256=suite.sha(b'driver'))])
            def fail(command, timeout, cwd, env):
                (cwd / 'libfixture.rlib').write_bytes(b'original failed artifact')
                return dict(command=list(map(str, command)), status=1, elapsed_seconds=0.01,
                            stdout_b64=suite.shared.encoded(b'full stdout'), stderr_b64=suite.shared.encoded(b'full original error'))
            with patch.object(suite.shared, 'identity', return_value=identity), patch.object(suite.shared, 'run', side_effect=fail):
                with self.assertRaisesRegex(ValueError, 'producer failed'):
                    suite.capture(compiler, root / 'out', 1, 2, 1,
                                  expected_compiler_sha256=identity['sha256'], expected_driver_sha256=identity['drivers'][0]['sha256'])
            receipt = json.loads((root / 'out/receipt.json').read_text())
            self.assertEqual(receipt['verdict'], 'failed')
            self.assertNotIn('aggregates', receipt)
            self.assertEqual(suite.artifact_bytes(receipt['rows'][0]['artifacts']['rlib']), b'original failed artifact')
            self.assertEqual(receipt['rows'][0]['record']['stderr_b64'], suite.shared.encoded(b'full original error'))
            self.assertEqual(len(receipt['source_blobs']), 3)


    def test_primary_compile_error_survives_shutdown_error(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            compiler = root / 'bin/rustc'
            compiler.parent.mkdir()
            compiler.write_bytes(b'compiler')
            (root / 'lib').mkdir()
            (root / 'lib/librustc_driver.so').write_bytes(b'driver')
            identity = dict(path=str(compiler), sha256=suite.sha(b'compiler'),
                            drivers=[dict(sha256=suite.sha(b'driver'))])

            class Worker:
                def __init__(self, compiler, cwd, arm, timeout):
                    self.command = [str(compiler), '--jr-resident-owned']
                    self.process = SimpleNamespace(pid=999)
                    self.environment_sha256 = suite.sha(suite.canonical(suite.environment(arm)))
                    for name in ('resident.stdout', 'resident.stderr', 'resident.jsonl'):
                        (cwd / name).write_bytes(b'original raw evidence')

                def compile(self, argv):
                    raise ValueError('primary compilation protocol failure')

                def close(self):
                    raise RuntimeError('secondary shutdown failure')

            with patch.object(suite.shared, 'identity', return_value=identity), \
                    patch.object(suite, 'order', return_value=['control']), \
                    patch.object(suite, 'LexerResident', Worker):
                with self.assertRaisesRegex(ValueError, 'primary compilation protocol failure'):
                    suite.capture(compiler, root / 'out', 1, 2, 1,
                                  expected_compiler_sha256=identity['sha256'],
                                  expected_driver_sha256=identity['drivers'][0]['sha256'])
            receipt = json.loads((root / 'out/receipt.json').read_text())
            self.assertIn('primary compilation protocol failure', receipt['error'])
            self.assertIn('secondary shutdown failure', receipt['sessions'][0]['close_error'])
            for artifact in receipt['sessions'][0]['streams'].values():
                self.assertEqual(suite.artifact_bytes(artifact), b'original raw evidence')


if __name__ == '__main__':
    unittest.main()
