#!/usr/bin/env python3
"""Mock/offline-only decoder suite tests. Never invokes a native compiler."""
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

import stable_resident_decoder_suite as suite


class DecoderSuiteTests(unittest.TestCase):
    def test_strict_decoder_and_other_cache_telemetry(self):
        with tempfile.TemporaryDirectory() as directory:
            report = self.synthetic(Path(directory))
            rows = [r for r in report['rows'] if r['arm'] == 'decoder']
            prior = suite.validate_live(rows[0]['record'], None, 'decoder', 'cold')
            mutations = [('decoder', 'hits', 0), ('decoder', 'hits', True),
                         ('decoder', 'publications', 0), ('decoder', 'rejected', 1),
                         ('decoder', 'retirement_requests', 1), ('decoder', 'retired', True),
                         ('decoder', 'unknown', 0), ('ast', 'hits', 1),
                         ('lexer', 'hits', 1), ('live', 'mir_leaf_last_rejection', 'reason'),
                         ('live', 'unknown', False)]
            for field, key, value in mutations:
                with self.subTest(field=field, key=key):
                    record = copy.deepcopy(rows[1]['record'])
                    record[field][key] = value
                    with self.assertRaises((ValueError, AssertionError)):
                        suite.validate_live(record, prior, 'decoder', 'unchanged')

    def test_sanitized_environment_and_bounds(self):
        keys = ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'RUSTC_WRAPPER',
                'RUSTC_WORKSPACE_WRAPPER', 'RUSTC_BOOTSTRAP', 'LLVM_PROFILE_FILE',
                'JUSTRUST_RUSTC_PHASES_FILE', 'JUSTRUST_RUSTC_EARLY_REPLY')
        with patch.dict('os.environ', dict.fromkeys(keys, 'injected')):
            for arm in suite.ARMS:
                self.assertFalse(set(keys) & suite.environment(arm).keys())
        self.assertEqual(suite.workload(1, 1, 1)['edits'], 1)
        self.assertEqual(suite.workload(128, 32, 1)['items'], 128)
        source = suite.fixture(1, 0)[0]['fixture.rs']
        self.assertIn(b'std::matches!', source)
        self.assertIn(b'std::env::consts::OS', source)

    def test_worker_attested_environment(self):
        for arm in ('control', 'decoder'):
            with tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                with patch.object(suite.subprocess, 'Popen', return_value=SimpleNamespace(pid=123)) as launch:
                    worker = suite.DecoderResident('/compiler', root, arm, 1)
                try:
                    env = dict(launch.call_args.kwargs['env'])
                    fd = int(env.pop('JUSTRUST_RUSTC_RESIDENT_FD'))
                    self.assertEqual(launch.call_args.kwargs['pass_fds'], (fd,))
                    self.assertEqual(env, suite.environment(arm))
                    self.assertEqual(worker.environment_sha256, suite.sha(suite.canonical(env)))
                finally:
                    worker.selector.close()
                    suite.os.close(worker.response_fd)
                    for stream in (worker.stdout, worker.stderr, worker.transcript):
                        stream.close()

    def test_root_stable_helper_only_edits_and_oracle(self):
        for items in (1, 7, 128):
            original, _ = suite.fixture(items, 0)
            for revision in range(1, 33):
                files, constants = suite.fixture(items, revision)
                self.assertEqual(files['fixture.rs'], original['fixture.rs'])
                self.assertNotEqual(files['helper.rs'], original['helper.rs'])
                self.assertEqual(len(constants), items + 1)
                nonce = 'a' * 32
                expected = [nonce, "linux"]
                for value in constants.values():
                    for x, y in suite.INPUTS:
                        expected.append(str(sum((1 << bit) * (((x >> bit) + (y >> bit) + (value >> bit)) % 2) for bit in range(8))))
                self.assertEqual(suite.expected_output(constants, nonce), ('\n'.join(expected) + '\n').encode())
                for name in constants:
                    self.assertEqual(suite.consumer_source(constants, nonce).count(('decoder_fixture::' + name + '(').encode()), len(suite.INPUTS))
        argv = suite.arguments(Path('.'))
        self.assertIn('--emit=link', argv)
        self.assertFalse(any(any(k in arg for k in ('incremental', 'mir', 'llvm', 'metadata')) for arg in argv))

    def test_upload_on_failure_and_no_upload_preexisting(self):
        for existed in (False, True):
            with tempfile.TemporaryDirectory() as directory:
                output = Path(directory) / 'out'
                if existed:
                    output.mkdir()
                args = SimpleNamespace(compiler=Path('/compiler'), output=output, items=1, edits=2,
                    repetitions=1, timeout=1, expected_driver_sha256='2' * 64,
                    expected_compiler_sha256='1' * 64)
                with patch.object(suite, 'capture', side_effect=ValueError('failed capture')), \
                        patch.object(suite.base, 'upload_result') as upload:
                    with self.assertRaisesRegex(ValueError, 'failed capture'):
                        suite.run(args)
                self.assertEqual(upload.call_count, 0 if existed else 1)

    def test_bounds_rotation_and_gates(self):
        for items in (0, 129, True):
            with self.assertRaises(ValueError):
                suite.fixture(items, 0)
        for revision in (-1, 33, True):
            with self.assertRaises(ValueError):
                suite.fixture(1, revision)
        for edits in (0, 33, True):
            with self.assertRaises(ValueError):
                suite.workload(1, edits, 1)
        for repetitions in (0, True):
            with self.assertRaises(ValueError):
                suite.workload(1, 2, repetitions)
        self.assertEqual([suite.order(n) for n in range(3)],
                         [['fresh', 'control', 'decoder'], ['control', 'decoder', 'fresh'], ['decoder', 'fresh', 'control']])
        with patch.dict('os.environ', {'JUSTRUST_RUSTC_RESIDENT_TYPED': 'leaf', 'JUSTRUST_RUSTC_RESIDENT_VALUES': '1'}):
            environments = {arm: suite.environment(arm) for arm in suite.ARMS}
        for arm, env in environments.items():
            self.assertEqual(env['JUSTRUST_RUSTC_RESIDENT_TYPED'], '0')
            self.assertNotIn('JUSTRUST_RUSTC_RESIDENT_VALUES', env)
            self.assertEqual(env['JUSTRUST_RUSTC_RESIDENT_DECODERS'], '1' if arm == 'decoder' else '0')
        self.assertEqual(environments['fresh'], environments['control'])
        self.assertEqual({k: v for k, v in environments['decoder'].items() if k != 'JUSTRUST_RUSTC_RESIDENT_DECODERS'},
                         {k: v for k, v in environments['control'].items() if k != 'JUSTRUST_RUSTC_RESIDENT_DECODERS'})

    def test_cli_attestation_before_execution(self):
        parser = argparse.ArgumentParser()
        suite.add_parser(parser.add_subparsers(required=True))
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
            parser.parse_args(['stable-native-decoder', '--compiler', '/c', '--output', '/o'])
        args = parser.parse_args(['stable-native-decoder', '--compiler', '/c', '--output', '/o',
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
                      host=dict(kernel='test-kernel', machine='x86_64', host='test-host', boot_id='00000000-0000-0000-0000-000000000001', affinity=[0], execution='local-native-owned'),
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
                    live = {key: 0 for key in (*suite.ast.LIVE_COUNTERS, 'mir_leaf_hits', 'mir_leaf_candidate_misses',
                            'identity_begin_rejections', 'identity_complete_rejections', 'published_revisions', 'aborted_revisions')}
                    live['mir_leaf_last_rejection'] = None
                    counters = {key: 0 for key in suite.ast.COUNTERS}
                    decoders = dict.fromkeys(suite.decoder.COUNTERS, 0)
                    decoders['retired'] = False
                    if arm == 'decoder':
                        decoders.update(hits=step * 3, misses=3, publications=3, retained_entries=3)
                    counters = dict.fromkeys(suite.ast.COUNTERS, 0)
                    producer.update(decoder=decoders, pid=session['pid'], executed=True, request=dict(id=step + 1, args=report['execution']['argv']),
                                    id=step + 1, owned=dict(revisions=step + 1, retirement_reason=None), live=live, lexer=dict.fromkeys(suite.ast.COUNTERS, 0), ast=counters,
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
            self.assertEqual(output.getvalue().count('decoder/processcontrol:'), 2)
            with self.assertRaises(TypeError):
                suite.compare(report, report, allow_compiler_change=True)

    def test_unsafe_offline_mutations_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            report = self.synthetic(Path(directory))
            mutations = [
                lambda r: r['execution']['adapter_sha256'].update({'stable_resident_decoder_suite.py': 'bad'}),
                lambda r: r.update(schema='other'),
                lambda r: r['identity_after'].update(sha256='0' * 64),
                lambda r: r['execution']['argv'].append('--emit=mir'),
                lambda r: r['execution']['gates']['decoder'].update(JUSTRUST_RUSTC_RESIDENT_TYPED='leaf'),
                lambda r: r['execution']['common_environment_sha256'].update(decoder='0' * 64),
                lambda r: r['host'].update(boot_id='boot'),
                lambda r: r['host'].update(affinity=[False]),
                lambda r: r['rows'][2]['sources'].update({'fixture.rs': '0' * 64}),
                lambda r: r['rows'][2].update(warmup=True),
                lambda r: r['rows'][2].update(nonce=r['rows'][1]['nonce']),
                lambda r: r['rows'][2]['consumer_run'].update(stdout_b64=''),
                lambda r: r['rows'][2]['consumer_compile']['command'].append('--emit=mir'),
                lambda r: r['rows'][2]['artifacts']['rlib'].update(sha256='0' * 64),
                lambda r: r['rows'][6]['record'].update(pid=12),
                lambda r: r['rows'][6]['record']['decoder'].update(hits=1),
                lambda r: r['rows'][10]['record']['decoder'].update(hits=1),
                lambda r: r['rows'][10]['record']['live'].update(mir_misses=1),
                lambda r: r['rows'][10]['record']['live'].update(mir_leaf_candidate_misses=1),
                lambda r: r['rows'][10]['record']['owned'].update(retirement_reason='capacity'),
                lambda r: r['rows'][10]['record']['request']['args'].append('--emit=mir'),
                lambda r: r['sessions'][1]['streams'].pop('resident.jsonl'),
                lambda r: r['sessions'][1].update(environment_sha256='0' * 64),
                lambda r: r['aggregates']['decoder'].update(n=3),
                lambda r: r['rows'][2]['record'].update(elapsed_seconds=float('nan')),
                lambda r: r['rows'][0]['record'].update(elapsed_seconds=-1),
                lambda r: r['rows'][2]['record'].update(status=False),
                lambda r: r['rows'][2].update(consumer_rlib_sha256='0' * 64),
                lambda r: r['rows'][2]['artifacts'].update(rlib=r['rows'][1]['artifacts']['rlib']),
                lambda r: r['sessions'][1].update(close_error='shutdown failure'),
            ]
            for index, mutate in enumerate(mutations):
                with self.subTest(index=index):
                    changed = copy.deepcopy(report)
                    mutate(changed)
                    with self.assertRaises((ValueError, KeyError, AssertionError)):
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
            for floor in (-1, float('nan'), True):
                with self.assertRaises(ValueError):
                    suite.compare(report, report, floor)

    def test_mock_full_capture(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            template_dir = root / 'template'
            template_dir.mkdir()
            template = self.synthetic(template_dir)
            compiler = root / 'bin/rustc'
            compiler.parent.mkdir()
            compiler.write_bytes(b'compiler')
            (root / 'lib').mkdir()
            (root / 'lib/librustc_driver.so').write_bytes(b'driver')
            identity = dict(path=str(compiler), sha256=suite.sha(b'compiler'),
                            drivers=[dict(sha256=suite.sha(b'driver'))])
            calls = []
            def result(command):
                return dict(command=list(map(str, command)), status=0, elapsed_seconds=0.01,
                            stdout_b64='', stderr_b64='')
            def producer(cwd):
                self.assertFalse((cwd / 'incremental').exists())
                (cwd / 'libfixture.rlib').write_bytes(b'!<arch>\nmock')
            def run(command, timeout, cwd, env):
                calls.append((str(cwd), copy.deepcopy(env)))
                record = result(command)
                if '--crate-type=rlib' in command:
                    producer(cwd)
                elif '--extern' in command:
                    (cwd / 'consumer').write_bytes(b'mock executable')
                else:
                    source = (cwd / 'consumer.rs').read_text()
                    nonce = re.search(r'println!\("([0-9a-f]{32})"\)', source).group(1)
                    values = [nonce, "linux"]
                    for name, left, right in re.findall(r'decoder_fixture::([\w:]+)\((\d+), (\d+)\)', source):
                        filename = 'helper.rs' if name.startswith('helper::') else 'fixture.rs'
                        body = (cwd / filename).read_text()
                        constant = int(re.search(r'pub fn ' + name.split('::')[-1] + r'\(.*?\^ (\d+)\)', body).group(1))
                        values.append(str(((int(left) ^ int(right)) ^ constant) & 255))
                    record['stdout_b64'] = suite.shared.encoded(('\n'.join(values) + '\n').encode())
                return record
            class Worker:
                def __init__(self, compiler, cwd, arm, timeout):
                    self.cwd, self.arm, self.step = cwd, arm, 0
                    self.command = [str(compiler), '--jr-resident-owned']
                    self.environment_sha256 = suite.sha(suite.canonical(suite.environment(arm)))
                    self.process = SimpleNamespace(pid=10 if arm == 'control' else 11)
                def compile(self, argv):
                    producer(self.cwd)
                    record = copy.deepcopy(next(r['record'] for r in template['rows']
                                                if r['arm'] == self.arm and r['step'] == self.step))
                    self.step += 1
                    return record
                def close(self):
                    session = next(s for s in template['sessions'] if s['arm'] == self.arm)
                    for name, artifact in session['streams'].items():
                        (self.cwd / name).write_bytes(suite.artifact_bytes(artifact, True))
            with patch.object(suite.shared, 'identity', return_value=identity), \
                    patch.object(suite.shared, 'run', side_effect=run), \
                    patch.object(suite, 'DecoderResident', Worker):
                report = suite.capture(compiler, root / 'out', 2, 2, 1,
                    expected_compiler_sha256=identity['sha256'], expected_driver_sha256=identity['drivers'][0]['sha256'])
            self.assertEqual(suite.validate(report), report['aggregates'])
            self.assertEqual(len(report['rows']), 12)
            self.assertEqual({cwd for cwd, env in calls}, {str(root / 'out/workspace')})

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
            self.assertEqual(len(receipt['source_blobs']), 2)


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
                    patch.object(suite, 'DecoderResident', Worker):
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
