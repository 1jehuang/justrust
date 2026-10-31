"""Python-only synthetic orchestration/replay tests, never native runtime acceptance.

Every native launch and PID inspection is mocked. Reachable Rust fixture execution
and real native ABI/runtime validity remain root's separate acceptance gates.
"""
import argparse
import contextlib
import copy
import io
import json
import os
from pathlib import Path
import unittest
from unittest.mock import patch

import resident_suite as suite
import test_resident_engine_compare as fixtures


class ManifestRetouchTests(unittest.TestCase):
    def setUp(self):
        self.base = fixtures.HarnessTests()
        self.base.setUp()
        self.addCleanup(self.base.doCleanups)
        self.args = self.base.args
        self.args.mode = 'manifest-retouch'
        for tool in (self.args.engine, self.args.reference_engine):
            tool.chmod(0o700)
        (self.args.toolchain / 'lib/rustlib/src/rust/library/lib.rs').write_text('// fixture source\n')
        (self.args.source / 'Cargo.toml').write_bytes(b'[package]\nname="retouch"\nversion="0.0.0"\n')
        (self.args.source / 'Cargo.lock').write_bytes(b'version = 4\n')
        for index in range(60):
            (self.args.source / f'pinned-{index:02}.txt').write_text(f'fixed {index}\n')
        self.source = suite.harness.source_hashes(self.args.source)
        self.assertEqual(len(self.source), 63)
        self.calls = []
        self.pids = {}
        self.current_pid = None
        self.counter = 1000
        self.fault = None
        self.host = {'host': 'mock-host', 'cpu': ['mock-cpu'], 'ncpu': 2,
                     'machine': 'x86_64', 'kernel': 'mock', 'execution': 'local native direct'}

    def fake_run(self, command, cwd, env, prefix, timeout):
        self.calls.append((list(command), dict(env), prefix.name))
        status = 0
        if command[-1] == 'stop':
            text = 'stopped private state\n'
        elif command[-1] == 'status':
            text = f'  worker: pid {self.current_pid}, native, log private\n'
        else:
            row = json.loads((self.args.output / 'records.json').read_text())[-1]
            group, phase = row['group'], row['phase']
            if self.fault:
                self.fault(row, command, env)
            if phase == 'version':
                text = (suite.harness.REFERENCE_VERSION if group == 'reference' else 'jr-engine 0.1.0 source-built') + '\n'
            elif phase == 'type-error':
                text, status = 'error[E0308]: JR_RUNTIME_TYPE_ERROR\n', 101
            else:
                text = fixtures.output(failing=phase == 'assert-failure')
                status = 101 if phase == 'assert-failure' else 0
                if phase in ('warmup', 'retouch'):
                    self.assertIn(b'std::env::var("JR_MANIFEST_RETOUCH_NONCE")', (cwd / 'src/summary.rs').read_bytes())
                    text += 'JR_MANIFEST_RETOUCH_' + env[suite.RETOUCH_NONCE] + '\n'
                    if phase == 'warmup' or env[suite.RETOUCH_FLAG] == '0':
                        self.counter += 1
                        self.pids[group] = self.counter
                    self.current_pid = self.pids[group]
            if env[suite.RETOUCH_FLAG] == '1' and phase != 'version':
                text += suite.RETOUCH_MARKER + '\n'
        prefix.with_suffix('.stdout').write_text(text)
        prefix.with_suffix('.stderr').write_text('')
        return {'command': command, 'status': status, 'error': None, 'timed_out': False,
                'elapsed_seconds': 0.01 if env[suite.RETOUCH_FLAG] == '0' else 0.005,
                'noise': {'contamination': 'unknown'}}

    def fake_proc(self, pid, tool):
        # /proc stat state begins field 3, starttime is field 22 (index 19 here).
        stat = str(pid) + ' (mock ) worker) ' + ' '.join(['S'] + ['0'] * 18 + [str(pid * 10)])
        row = json.loads((self.args.output / 'records.json').read_text())[-1]
        metadata = json.loads((self.args.output / 'metadata.json').read_text())
        home = metadata['environments'][row['group']]['RA_JIT_HOME']
        argv = os.fsencode(tool) + b'\0--socket\0' + os.fsencode(Path(home) / 'workers/worker-id/sock') + b'\0'
        return {'pid': pid, 'stat': stat, 'exe': str(tool), 'exe_sha256': suite.digest(tool),
                'cmdline_hex': argv.hex()}

    def run_harness(self):
        with patch.object(suite.harness, 'pinned_source'), \
             patch.object(suite, 'check_retouch_output_length'), \
             patch.object(suite, 'pinned_hashes', return_value=self.source), \
             patch.object(suite, 'host_identity', return_value=self.host), \
             patch.object(suite.harness.runtime, 'run_process', side_effect=self.fake_run), \
             patch.object(suite, 'proc_identity', side_effect=self.fake_proc), \
             patch('subprocess.Popen', side_effect=AssertionError('native process forbidden in Python test')), \
             contextlib.redirect_stdout(io.StringIO()):
            suite.cmd_resident(self.args)

    def result(self, arm='off'):
        return json.loads((self.args.output / (arm + '.json')).read_text())

    def replay(self):
        return suite.replay_retouch(self.args.output, self.source)

    def mutate_json(self, name, mutate):
        path = self.args.output / name
        obj = json.loads(path.read_text())
        mutate(obj)
        path.write_text(json.dumps(obj))

    def test_full_abba_real_file_touches_runtime_fixture_and_semantic_gates(self):
        original_times = {name: (self.args.source / name).stat().st_mtime_ns
                          for name in (*suite.RETOUCH_FILES, 'src/summary.rs')}
        self.run_harness()
        metadata, samples, receipts = self.replay()
        self.assertEqual(metadata['order'], ['off', 'on', 'on', 'off'])
        self.assertEqual(samples['off']['samples_seconds'], [0.01] * 10)
        self.assertEqual(samples['on']['samples_seconds'], [0.005] * 10)
        self.assertEqual(metadata['source_before'], metadata['source_after'])
        self.assertEqual(len(receipts), 172)
        rows = json.loads((self.args.output / 'records.json').read_text())
        self.assertEqual(len(rows), 54)
        measured = [row for row in rows if row['phase'] == 'retouch']
        self.assertEqual(len(measured), 20)
        self.assertEqual(len({r['nonce'] for r in rows if r['nonce']}), 24)
        for row in measured:
            self.assertEqual(row['source_before'], metadata['fixture_source'])
            self.assertEqual([r['file'] for r in row['touches']], list(suite.RETOUCH_FILES))
            for touch in row['touches']:
                self.assertEqual(touch['before']['sha256'], touch['after']['sha256'])
                self.assertGreater(touch['after']['mtime_ns'], touch['before']['mtime_ns'])
        for name, stamp in original_times.items():
            self.assertEqual((self.args.source / name).stat().st_mtime_ns, stamp)
        self.assertEqual(suite.harness.source_hashes(self.args.source), self.source)
        self.assertEqual(sum(cmd[-1] == 'stop' for cmd, _, _ in self.calls), 5)
        with patch.object(suite, 'pinned_hashes', return_value=self.source):
            suite.validate(self.result())

    def test_fixture_reachable_runtime_env_not_per_sample_source_or_env_macro(self):
        original = self.base.original
        overlay = suite.retouch_fixture(original)
        self.assertIn(b'fn shares_overlapping_time() {\nlet nonce = std::env::var(', overlay)
        self.assertIn(b'println!("JR_MANIFEST_RETOUCH_{}", nonce)', overlay)
        self.assertNotIn(b'env!', overlay)
        self.assertEqual(overlay.count(b'std::env::var('), 1)
        self.assertEqual(suite.retouch_fixture(original), overlay)
        for invalid in (b'', original + original):
            with self.assertRaisesRegex(ValueError, 'anchor'):
                suite.retouch_fixture(invalid)

    def test_raw_nonce_missing_stale_duplicate_and_suffix_rejected(self):
        self.run_harness()
        rows = json.loads((self.args.output / 'records.json').read_text())
        index = next(i for i, row in enumerate(rows) if row['phase'] == 'retouch')
        path = self.args.output / f'call-{index:03}.stdout'
        original = path.read_text()
        token = 'JR_MANIFEST_RETOUCH_' + rows[index]['nonce']
        for replacement in ('', 'JR_MANIFEST_RETOUCH_' + 'a' * 32, token + '\n' + token, token + 'suffix'):
            with self.subTest(replacement=replacement):
                path.write_text(original.replace(token, replacement))
                with self.assertRaisesRegex(ValueError, 'nonce'):
                    self.replay()
        path.write_text(original)

    def test_bad_rows_and_touch_metadata_fail_closed(self):
        self.run_harness()
        path = self.args.output / 'records.json'
        original = path.read_bytes()
        def measured(rows):
            return next(row for row in rows if row['phase'] == 'retouch')
        mutations = [lambda r: r.pop(), lambda r: r.append(copy.deepcopy(r[-1])),
                     lambda r: measured(r).update(command=['wrong']),
                     lambda r: measured(r).update(timed_out=True),
                     lambda r: measured(r).update(error='spawn failure'),
                     lambda r: measured(r).update(status=101),
                     lambda r: measured(r).update(elapsed_seconds=float('nan')),
                     lambda r: measured(r).update(elapsed_seconds=True),
                     lambda r: measured(r).update(flag='on'),
                     lambda r: measured(r)['source_after'].update({'extra': 'wrong'}),
                     lambda r: measured(r).update(nonce_environment={}),
                     lambda r: measured(r)['touches'].pop(),
                     lambda r: measured(r)['touches'][0]['after'].update(sha256='f' * 64),
                     lambda r: measured(r)['touches'][0]['after'].update(size=1),
                     lambda r: measured(r)['touches'][0]['after'].update(mtime_ns=measured(r)['touches'][0]['before']['mtime_ns']),
                     lambda r: measured(r)['worker'].update(exe='/wrong/engine'),
                     lambda r: measured(r)['worker'].update(exe_sha256='f' * 64),
                     lambda r: measured(r)['worker'].update(cmdline_hex=b'--socket\0/outside/sock\0'.hex()),
                     lambda r: measured(r)['worker'].update(cmdline_hex='not-hex'),
                     lambda r: measured(r)['worker_status'].update(command=['wrong', 'status']),
                     lambda r: measured(r)['worker'].update(stat='invalid')]
        for index, mutate in enumerate(mutations):
            with self.subTest(index=index):
                path.write_bytes(original)
                self.mutate_json('records.json', mutate)
                with self.assertRaises(ValueError):
                    self.replay()
        path.write_bytes(original)

    def test_raw_original_semantic_gate_failure_and_zero_tests_rejected(self):
        self.run_harness()
        rows = json.loads((self.args.output / 'records.json').read_text())
        for phase, old, new in [('type-error', 'E0308', 'E0000'),
                                ('assert-failure', 'JR_RUNTIME_FAILURE', 'no failure'),
                                ('prepare', '4 passed', '0 passed'),
                                ('assert-recovery', suite.harness.NAMES[0], 'wrong::test')]:
            index = next(i for i, row in enumerate(rows) if row['phase'] == phase)
            path = self.args.output / f'call-{index:03}.stdout'
            original = path.read_text()
            path.write_text(original.replace(old, new))
            with self.assertRaises(ValueError):
                self.replay()
            path.write_text(original)

    def test_raw_status_pid_entry_marker_and_pid_reuse_checks(self):
        self.run_harness()
        rows = json.loads((self.args.output / 'records.json').read_text())
        for arm in ('off', 'on'):
            index = next(i for i, row in enumerate(rows) if row['phase'] == 'retouch' and
                         row['flag'] == ('1' if arm == 'on' else '0'))
            status = self.args.output / f'call-{index:03}-status.stdout'
            original = status.read_bytes()
            for bad in (b'', original + original, b'worker: pid 99\n'):
                status.write_bytes(bad)
                with self.assertRaisesRegex(ValueError, 'PID'):
                    self.replay()
            status.write_bytes(original)
            path = self.args.output / f'call-{index:03}.stdout'
            original = path.read_text()
            path.write_text(original.replace(suite.RETOUCH_MARKER + '\n', '') if arm == 'on'
                            else original + suite.RETOUCH_MARKER + '\n')
            with self.assertRaisesRegex(ValueError, 'marker'):
                self.replay()
            path.write_text(original)
        records = self.args.output / 'records.json'
        original = records.read_bytes()
        # Keep the status PID and executable unchanged but mutate start ticks.
        on = next(row for row in rows if row['phase'] == 'retouch' and row['flag'] == '1')
        on['worker']['stat'] += '0'
        records.write_text(json.dumps(rows))
        with self.assertRaisesRegex(ValueError, 'PID/starttime'):
            self.replay()
        records.write_bytes(original)

    def test_identity_policy_flag_private_state_and_restore_rejected(self):
        self.run_harness()
        path = self.args.output / 'metadata.json'
        original = path.read_bytes()
        mutations = [lambda m: m.update(order=['on', 'off', 'off', 'on']),
                     lambda m: m.update(samples=4), lambda m: m.update(source_restored=False),
                     lambda m: m.update(store_scope='seeded'), lambda m: m.update(cleanup_error='failed stop'),
                     lambda m: m['tools_after']['engine'].update(sha256='f' * 64),
                     lambda m: m['host_after'].update(kernel='different'),
                     lambda m: m['fixture_source'].update({'src/summary.rs': 'f' * 64}),
                     lambda m: m['environments']['segment-1'].update({suite.RETOUCH_FLAG: 'true'}),
                     lambda m: m['environments']['segment-1'].update(BLAZE_OPT='1'),
                     lambda m: m['environments']['segment-1'].update(RA_JIT_REMOTE='on'),
                     lambda m: m['environments']['segment-1'].update(CARGO_TARGET_DIR=m['environments']['segment-0']['CARGO_TARGET_DIR']),
                     lambda m: m['environments']['segment-1'].update(RA_JIT_CACHE='/outside'),
                     lambda m: m['stops'].pop()]
        for index, mutate in enumerate(mutations):
            with self.subTest(index=index):
                path.write_bytes(original)
                self.mutate_json('metadata.json', mutate)
                with self.assertRaises(ValueError):
                    self.replay()
        path.write_bytes(original)

    def test_compare_distinct_schema_same_abba_only_and_descriptive(self):
        self.run_harness()
        off, on = self.result('off'), self.result('on')
        output = io.StringIO()
        with patch.object(suite, 'pinned_hashes', return_value=self.source), contextlib.redirect_stdout(output):
            suite.compare(off, on)
        self.assertIn('-50.00%', output.getvalue())
        self.assertIn('No significance or general speedup verdict', output.getvalue())
        self.assertIn('not separate manifest/lock attribution', output.getvalue())
        with self.assertRaisesRegex(ValueError, 'mix'):
            suite.compare(off, {'schema': suite.SCHEMA})
        with patch.object(suite, 'pinned_hashes', return_value=self.source):
            with self.assertRaisesRegex(ValueError, 'OFF then ON'):
                suite.compare(on, off)
            changed = copy.deepcopy(on)
            changed['measurements']['on']['median_seconds'] = 99
            with self.assertRaisesRegex(ValueError, 'sample/host'):
                suite.compare(off, changed)
            for floor in (-1, float('nan')):
                with self.assertRaisesRegex(ValueError, 'floor'):
                    suite.compare(off, on, floor=floor)
        # Different valid runs/identities must not be accepted as one ABBA pair.
        changed = copy.deepcopy(on)
        changed['artifacts_sha256']['records.json'] = 'f' * 64
        with patch.object(suite, 'validate_retouch', side_effect=[(off['metadata'], off['measurements']),
                                                                (changed['metadata'], changed['measurements'])]):
            with self.assertRaisesRegex(ValueError, 'same frozen'):
                suite.compare(off, changed)

    def test_failure_restores_originals_and_stops_private_state(self):
        def fail(row, command, env):
            if row['phase'] == 'retouch':
                raise RuntimeError('synthetic launch failure')
        self.fault = fail
        with self.assertRaisesRegex(RuntimeError, 'synthetic launch failure'):
            self.run_harness()
        self.assertEqual(suite.harness.source_hashes(self.args.source), self.source)
        self.assertEqual(sum(cmd[-1] == 'stop' for cmd, _, _ in self.calls), 2)
        metadata = json.loads((self.args.output / 'metadata.json').read_text())
        self.assertIn('synthetic launch failure', metadata['error'])
        self.assertFalse((self.args.output / 'off.json').exists())

    def test_touch_input_mutation_symlink_and_no_timestamp_change_rejected(self):
        originals = {name: (self.args.source / name).read_bytes() for name in suite.RETOUCH_FILES}
        path = self.args.source / 'Cargo.toml'
        original = path.read_bytes()
        path.write_bytes(original + b'# changed\n')
        with self.assertRaisesRegex(ValueError, 'differs from pin'):
            suite.retouch_inputs(self.args.source, originals)
        path.write_bytes(original)
        target = self.base.root / 'manifest-target'
        target.write_bytes(original)
        path.unlink()
        path.symlink_to(target)
        with self.assertRaisesRegex(ValueError, 'regular non-symlink'):
            suite.retouch_inputs(self.args.source, originals)
        path.unlink()
        path.write_bytes(original)
        snap = suite.retouch_snapshot(path)
        with patch.object(suite, 'retouch_snapshot', return_value=snap):
            with self.assertRaisesRegex(ValueError, 'retouch was not observed'):
                suite.retouch_inputs(self.args.source, originals)

    def test_cli_and_legacy_default_dispatch_preserved(self):
        parser = argparse.ArgumentParser()
        suite.add_parser(parser.add_subparsers())
        argv = ['resident']
        for name in ('reference-engine', 'engine', 'source', 'output', 'toolchain'):
            argv.extend(['--' + name, '/absolute/' + name])
        self.assertEqual(parser.parse_args(argv).mode, 'edit')
        args = parser.parse_args(argv + ['--mode', 'manifest-retouch'])
        with patch.object(suite, 'cmd_manifest_retouch') as call, \
             patch.object(suite.harness, 'execute', side_effect=AssertionError('legacy runner called')):
            suite.cmd_resident(args)
        call.assert_called_once_with(args)
        import suite as standard
        a = self.base.root / 'dispatch.json'
        a.write_text(json.dumps({'schema': suite.RETOUCH_SCHEMA}))
        with patch.object(suite, 'compare') as call, \
             patch('runtime_context.require_matching_context', side_effect=AssertionError('legacy context called')):
            standard.cmd_compare(argparse.Namespace(a=a, b=a, floor=7))
        self.assertEqual(call.call_args.args[2], 7)

    def test_native_pid_proof_reader_and_start_race_fail_closed(self):
        stat = '4321 (worker ) with space) ' + ' '.join(['S'] + ['0'] * 18 + ['555'])
        argv = b'/engine\0--socket\0/private/workers/w/sock\0'
        with patch.object(Path, 'read_text', side_effect=[stat, stat]), \
             patch.object(Path, 'read_bytes', return_value=argv), \
             patch.object(suite.os, 'readlink', return_value=str(self.args.engine)), \
             patch.object(suite, 'digest', return_value='a' * 64):
            receipt = suite.proc_identity(4321, self.args.engine)
        self.assertEqual(receipt['cmdline_hex'], argv.hex())
        self.assertEqual(suite.worker_key(receipt, {'path': str(self.args.engine), 'sha256': 'a' * 64}, '/private'), (4321, '555'))
        with patch.object(Path, 'read_text', side_effect=[stat, stat + '0']), \
             patch.object(Path, 'read_bytes', return_value=argv), \
             patch.object(suite.os, 'readlink', return_value=str(self.args.engine)), \
             patch.object(suite, 'digest', return_value='a' * 64):
            with self.assertRaisesRegex(ValueError, 'PID changed'):
                suite.proc_identity(4321, self.args.engine)
        receipt['stat'] = stat.replace(') S ', ') Z ')
        with self.assertRaisesRegex(ValueError, 'dead or unknown'):
            suite.worker_key(receipt, {'path': str(self.args.engine), 'sha256': 'a' * 64}, '/private')

    def test_output_native_socket_budget_is_explicit(self):
        suite.check_retouch_output_length(Path('/dev/shm/mr74'))
        with self.assertRaisesRegex(ValueError, 'short'):
            suite.check_retouch_output_length(Path('/long/' + 'a' * 40))

class WatchedManifestRetouchTests(ManifestRetouchTests):
    """The same raw controls must hold with whole-generation fixed ON."""

    def setUp(self):
        super().setUp()
        self.args.mode = suite.WATCHED_POLICY.mode

    def replay(self):
        return suite.replay_retouch(self.args.output, self.source, suite.WATCHED_POLICY)

    def fake_run(self, command, cwd, env, prefix, timeout):
        result = super().fake_run(command, cwd, env, prefix, timeout)
        result['elapsed_seconds'] = 0.01 if env[suite.WATCHED_FLAG] == '0' else 0.005
        return result

    def test_raw_status_pid_entry_marker_and_pid_reuse_checks(self):
        self.run_harness()
        records = self.args.output / 'records.json'
        original = records.read_bytes()
        rows = json.loads(original)
        for arm in ('0', '1'):
            index = next(i for i, row in enumerate(rows) if row['phase'] == 'retouch'
                         and row['flags'][suite.WATCHED_FLAG] == arm)
            with self.subTest(arm=arm):
                status = self.args.output / f'call-{index:03}-status.stdout'
                status_bytes = status.read_bytes()
                for bad in (b'', status_bytes + status_bytes, b'worker: pid 99\n'):
                    status.write_bytes(bad)
                    with self.assertRaisesRegex(ValueError, 'PID'):
                        self.replay()
                status.write_bytes(status_bytes)
                rows[index]['worker']['stat'] += '0'
                records.write_text(json.dumps(rows))
                with self.assertRaisesRegex(ValueError, 'PID/starttime'):
                    self.replay()
                rows = json.loads(original)
                records.write_bytes(original)
                path = self.args.output / f'call-{index:03}.stdout'
                text = path.read_text()
                self.assertEqual(text.splitlines().count(suite.RETOUCH_MARKER), 1)
                path.write_text(text.replace(suite.RETOUCH_MARKER + '\n', ''))
                with self.assertRaisesRegex(ValueError, 'marker'):
                    self.replay()
                path.write_text(text)

    def test_exact_watcher_flags_and_fixed_generation_per_state_and_call(self):
        self.run_harness()
        metadata, samples, _ = self.replay()
        rows = json.loads((self.args.output / 'records.json').read_text())
        for row in rows:
            group = row['group']
            reference = group == 'reference'
            arm = 'off' if reference else suite.RETOUCH_ORDER[int(group[-1])]
            wanted = suite.WATCHED_POLICY.flags(arm, reference)
            self.assertEqual(row['flags'], wanted)
            self.assertEqual({k: metadata['environments'][group][k] for k in wanted}, wanted)
        for segment in range(4):
            measured = [row for row in rows if row['group'] == f'segment-{segment}'
                        and row['phase'] in ('warmup', 'retouch')]
            self.assertEqual(len(measured), 6)
            self.assertEqual(len({(r['worker']['pid'], r['worker']['stat']) for r in measured}), 1)
        self.assertEqual([len(samples[a]['samples_seconds']) for a in ('off', 'on')], [10, 10])
        for filename in ('metadata.json', 'records.json'):
            path = self.args.output / filename
            original = path.read_bytes()
            for key in (suite.RETOUCH_FLAG, suite.WATCHED_FLAG):
                for value in (None, 'true', 'on', 1, '0' if key == suite.RETOUCH_FLAG else '1'):
                    obj = json.loads(original)
                    flags = obj['environments']['segment-0'] if filename == 'metadata.json' else obj[6]['flags']
                    flags[key] = value
                    path.write_text(json.dumps(obj))
                    with self.assertRaisesRegex(ValueError, 'flag'):
                        self.replay()
                    path.write_bytes(original)

    def test_compare_distinct_schema_same_abba_only_and_descriptive(self):
        self.run_harness()
        off, on = self.result('off'), self.result('on')
        output = io.StringIO()
        with patch.object(suite, 'pinned_hashes', return_value=self.source), contextlib.redirect_stdout(output):
            suite.compare(off, on)
        for phrase in ('WATCHED manifest+lock factor', 'whole-generation fixed ON',
                       'All first-touch samples retained', 'noise unknown',
                       'No significance or general speedup verdict', 'default promotion'):
            self.assertIn(phrase, output.getvalue())
        for schema in (suite.RETOUCH_SCHEMA, suite.SCHEMA):
            changed = dict(on, schema=schema)
            with self.assertRaisesRegex(ValueError, 'mix'):
                suite.compare(off, changed)
        with patch.object(suite, 'pinned_hashes', return_value=self.source):
            with self.assertRaisesRegex(ValueError, 'OFF then ON'):
                suite.compare(on, off)
            for floor in (-1, float('nan')):
                with self.assertRaisesRegex(ValueError, 'floor'):
                    suite.compare(off, on, floor)
            changed = copy.deepcopy(on)
            changed['measurements']['on']['median_seconds'] = 99
            with self.assertRaisesRegex(ValueError, 'sample/host'):
                suite.compare(off, changed)
        changed = copy.deepcopy(on)
        changed['artifacts_sha256']['records.json'] = 'f' * 64
        with patch.object(suite, 'validate_retouch', side_effect=[(off['metadata'], off['measurements']),
                                                                (on['metadata'], on['measurements'])]):
            with self.assertRaisesRegex(ValueError, 'same frozen'):
                suite.compare(off, changed)
        with self.assertRaisesRegex(ValueError, 'schema'):
            suite.replay_retouch(self.args.output, self.source)
        path = self.args.output / 'metadata.json'
        original = path.read_bytes()
        for mutation in ({'schema': suite.RETOUCH_SCHEMA}, {'mode': 'manifest-retouch'},
                         {'factor': suite.RETOUCH_FLAG}):
            obj = json.loads(original)
            obj.update(mutation)
            path.write_text(json.dumps(obj))
            with self.assertRaises(ValueError):
                self.replay()
        path.write_bytes(original)

    def test_watched_cli_dispatch_and_immutable_policy(self):
        parser = argparse.ArgumentParser()
        suite.add_parser(parser.add_subparsers())
        argv = ['resident', '--mode', 'watched-manifest-retouch']
        for name in ('reference-engine', 'engine', 'source', 'output', 'toolchain'):
            argv.extend(['--' + name, '/absolute/' + name])
        args = parser.parse_args(argv)
        with patch.object(suite, 'cmd_manifest_retouch') as call:
            suite.cmd_resident(args)
        call.assert_called_once_with(args, suite.WATCHED_POLICY)
        with self.assertRaises(AttributeError):
            suite.WATCHED_POLICY.watched = False
        self.assertEqual(suite.RETOUCH_POLICY.flags('off'), {suite.RETOUCH_FLAG: '0'})
        self.assertFalse(suite.RETOUCH_POLICY.reuse_worker('off'))
        self.assertTrue(suite.WATCHED_POLICY.reuse_worker('off'))


if __name__ == '__main__':
    unittest.main()
