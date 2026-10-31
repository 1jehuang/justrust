"""Python-only parser factor controls. All native launches and /proc are mocked."""
import argparse
import contextlib
import copy
from dataclasses import FrozenInstanceError
import io
import json
import unittest
from unittest.mock import patch

import resident_suite as suite
import test_resident_manifest_retouch as prior


class ParallelManifestTests(unittest.TestCase):
    def setUp(self):
        prior.ManifestRetouchTests.setUp(self)
        self.args.mode = suite.PARALLEL_POLICY.mode

    run_harness = prior.ManifestRetouchTests.run_harness
    result = prior.ManifestRetouchTests.result
    mutate_json = prior.ManifestRetouchTests.mutate_json
    fake_proc = prior.ManifestRetouchTests.fake_proc

    def replay(self):
        return suite.replay_retouch(self.args.output, self.source, suite.PARALLEL_POLICY)

    def fake_run(self, command, cwd, env, prefix, timeout):
        result = prior.ManifestRetouchTests.fake_run(self, command, cwd, env, prefix, timeout)
        result['elapsed_seconds'] = 0.005 if env[suite.PARALLEL_FLAG] == '1' else 0.01
        if command[-1] not in ('stop', 'status', '--version') and env[suite.PARALLEL_FLAG] == '1':
            prefix.with_suffix('.stderr').write_text(suite.PARALLEL_MARKER + '\n')
        return result

    test_full_abba = prior.ManifestRetouchTests.test_full_abba_real_file_touches_runtime_fixture_and_semantic_gates
    test_runtime_fixture = prior.ManifestRetouchTests.test_fixture_reachable_runtime_env_not_per_sample_source_or_env_macro
    test_raw_nonce = prior.ManifestRetouchTests.test_raw_nonce_missing_stale_duplicate_and_suffix_rejected
    test_bad_rows_and_touches = prior.ManifestRetouchTests.test_bad_rows_and_touch_metadata_fail_closed
    test_original_semantic_gates = prior.ManifestRetouchTests.test_raw_original_semantic_gate_failure_and_zero_tests_rejected
    test_identity_and_restoration = prior.ManifestRetouchTests.test_identity_policy_flag_private_state_and_restore_rejected
    test_compare = prior.ManifestRetouchTests.test_compare_distinct_schema_same_abba_only_and_descriptive
    test_failure_cleanup = prior.ManifestRetouchTests.test_failure_restores_originals_and_stops_private_state
    test_proc_reader = prior.ManifestRetouchTests.test_native_pid_proof_reader_and_start_race_fail_closed
    test_socket_budget = prior.ManifestRetouchTests.test_output_native_socket_budget_is_explicit

    def test_frozen_policy_and_old_flag_semantics(self):
        with self.assertRaises(FrozenInstanceError):
            suite.PARALLEL_POLICY.parallel = False
        self.assertEqual(suite.RETOUCH_POLICY.flags('off'), {suite.RETOUCH_FLAG: '0'})
        self.assertEqual(suite.RETOUCH_POLICY.flags('on'), {suite.RETOUCH_FLAG: '1'})
        self.assertFalse(suite.RETOUCH_POLICY.reuse_worker('off'))
        self.assertEqual(suite.WATCHED_POLICY.flags('off'),
                         {suite.RETOUCH_FLAG: '1', suite.WATCHED_FLAG: '0'})
        self.assertEqual(suite.WATCHED_POLICY.flags('on'),
                         {suite.RETOUCH_FLAG: '1', suite.WATCHED_FLAG: '1'})
        self.assertTrue(suite.WATCHED_POLICY.reuse_worker('off'))
        self.assertTrue(suite.PARALLEL_POLICY.reuse_worker('off'))

    def test_exact_three_flags_and_only_parser_changes(self):
        self.run_harness()
        metadata, _, _ = self.replay()
        states = metadata['environments']
        self.assertEqual({k: states['reference'][k] for k in suite.PARALLEL_POLICY.flags('off')},
                         {k: '0' for k in suite.PARALLEL_POLICY.flags('off')})
        rows = json.loads((self.args.output / 'records.json').read_text())
        for i, arm in enumerate(suite.RETOUCH_ORDER):
            expected = {suite.RETOUCH_FLAG: '1', suite.WATCHED_FLAG: '1',
                        suite.PARALLEL_FLAG: '1' if arm == 'on' else '0'}
            self.assertEqual(suite.PARALLEL_POLICY.flags(arm), expected)
            group = f'segment-{i}'
            self.assertEqual({k: states[group][k] for k in expected}, expected)
            selected = [r for r in rows if r['group'] == group]
            self.assertTrue(all(r['flags'] == expected for r in selected))
            measured = [r for r in selected if r['phase'] in ('warmup', 'retouch')]
            self.assertEqual(len({(r['worker']['pid'], r['worker']['stat'], r['worker']['cmdline_hex'])
                                  for r in measured}), 1)
        with self.assertRaisesRegex(ValueError, 'wrong retouch schema'):
            suite.replay_retouch(self.args.output, self.source, suite.WATCHED_POLICY)

    def test_wrong_flags_schema_run_rules_and_first_touch_omission(self):
        self.run_harness()
        for name in ('metadata.json', 'records.json'):
            path = self.args.output / name
            original = path.read_bytes()
            if name == 'metadata.json':
                mutations = [lambda m: m.update(schema=suite.WATCHED_SCHEMA),
                             lambda m: m.update(factor=suite.WATCHED_FLAG),
                             lambda m: m.update(rules={}), lambda m: m.update(run_id='bad')]
                for group in ('reference', 'segment-0', 'segment-1'):
                    for flag in suite.PARALLEL_POLICY.flags('off'):
                        for bad in ('true', '01', None):
                            mutations.append(lambda m, g=group, f=flag, b=bad:
                                             m['environments'][g].update({f: b}))
            else:
                def first(rows):
                    return next(r for r in rows if r['phase'] == 'retouch')
                mutations = [lambda r: first(r).update(schema=suite.WATCHED_SCHEMA),
                             lambda r: first(r).update(run_id='a' * 32),
                             lambda r: first(r).update(rules={}),
                             lambda r: r.remove(first(r))]
                for flag in suite.PARALLEL_POLICY.flags('off'):
                    mutations.append(lambda r, f=flag: first(r)['flags'].update({f: 'wrong'}))
            for i, mutate in enumerate(mutations):
                with self.subTest(name=name, mutation=i):
                    path.write_bytes(original)
                    self.mutate_json(name, mutate)
                    with self.assertRaises(ValueError):
                        self.replay()
            path.write_bytes(original)

    def test_actual_batch_raw_stderr_not_summary_boolean(self):
        self.run_harness()
        rows = json.loads((self.args.output / 'records.json').read_text())
        for arm in ('off', 'on'):
            index = next(i for i, r in enumerate(rows) if r['phase'] == 'retouch'
                         and r['flags'][suite.PARALLEL_FLAG] == ('1' if arm == 'on' else '0'))
            path = self.args.output / f'call-{index:03}.stderr'
            original = path.read_bytes()
            for bad in ('', suite.PARALLEL_MARKER + '\n' + suite.PARALLEL_MARKER + '\n',
                        suite.PARALLEL_MARKER + ' suffix\n') if arm == 'on' else (suite.PARALLEL_MARKER + '\n',):
                path.write_text(bad)
                with self.assertRaisesRegex(ValueError, 'parallel.*marker'):
                    self.replay()
            path.write_bytes(original)
            if arm == 'on':
                # Moving a genuine marker to stdout cannot prove stderr activation.
                stdout = self.args.output / f'call-{index:03}.stdout'
                saved = stdout.read_bytes()
                stdout.write_bytes(saved + original)
                path.write_text('')
                with self.assertRaisesRegex(ValueError, 'parallel.*marker'):
                    self.replay()
                stdout.write_bytes(saved)
                path.write_bytes(original)

    def test_marker_contract_versions_off_and_no_dynamic_fields(self):
        line = suite.PARALLEL_MARKER + '\n'
        suite.check_parallel_activation('', line, '1', 'retouch')
        suite.check_parallel_activation('', '', '1', 'version')
        suite.check_parallel_activation('', '', '1', 'warmup')
        for flag, phase, stderr in [('0', 'retouch', line), ('0', 'prepare', line),
                                    ('1', 'version', line), ('1', 'retouch', ''),
                                    ('1', 'retouch', line + line),
                                    ('1', 'retouch', suite.PARALLEL_MARKER),
                                    ('1', 'retouch', line.rstrip() + ' packages=5 workers=2\n'),
                                    ('1', 'retouch', line.replace('v81', 'v80'))]:
            with self.subTest(flag=flag, phase=phase, stderr=stderr), self.assertRaisesRegex(ValueError, 'marker'):
                suite.check_parallel_activation('', stderr, flag, phase)

    def test_missing_actual_batch_fails_runner_and_stops_private_states(self):
        original = self.fake_run
        def no_batch(command, cwd, env, prefix, timeout):
            receipt = original(command, cwd, env, prefix, timeout)
            if prefix.name.startswith('call-') and not prefix.name.endswith('-status'):
                rows = json.loads((self.args.output / 'records.json').read_text())
                if rows[-1]['phase'] == 'retouch' and env[suite.PARALLEL_FLAG] == '1':
                    prefix.with_suffix('.stderr').write_text('')
            return receipt
        with patch.object(self, 'fake_run', side_effect=no_batch), self.assertRaisesRegex(ValueError, 'marker'):
            self.run_harness()
        self.assertEqual(suite.harness.source_hashes(self.args.source), self.source)
        self.assertFalse((self.args.output / 'off.json').exists())
        metadata = json.loads((self.args.output / 'metadata.json').read_text())
        self.assertIn('marker', metadata['error'])
        self.assertEqual(len(metadata['stops']), 3)

    def test_same_pid_ticks_exe_and_socket_both_candidate_arms(self):
        self.run_harness()
        path = self.args.output / 'records.json'
        original = path.read_bytes()
        for arm in ('0', '1'):
            for field, value in (('pid', 99), ('stat', None), ('exe', '/wrong'),
                                 ('exe_sha256', 'f' * 64), ('cmdline_hex', None)):
                rows = json.loads(original)
                row = next(r for r in rows if r['phase'] == 'retouch' and r['flags'][suite.PARALLEL_FLAG] == arm)
                if field == 'stat':
                    value = row['worker']['stat'] + '0'
                if field == 'cmdline_hex':
                    value = bytes.fromhex(row['worker'][field]).replace(b'worker-id', b'other-id').hex()
                row['worker'][field] = value
                path.write_text(json.dumps(rows))
                with self.subTest(arm=arm, field=field), self.assertRaises(ValueError):
                    self.replay()
        path.write_bytes(original)

    def test_parser_dispatch_and_cross_mode_compare(self):
        parser = argparse.ArgumentParser()
        suite.add_parser(parser.add_subparsers())
        argv = ['resident', '--mode', suite.PARALLEL_POLICY.mode]
        for name in ('reference-engine', 'engine', 'source', 'output', 'toolchain'):
            argv += ['--' + name, '/absolute/' + name]
        args = parser.parse_args(argv)
        with patch.object(suite, 'cmd_manifest_retouch') as call:
            suite.cmd_resident(args)
        call.assert_called_once_with(args, suite.PARALLEL_POLICY)
        self.run_harness()
        off, on = self.result(), self.result('on')
        for schema in (suite.SCHEMA, suite.RETOUCH_SCHEMA, suite.WATCHED_SCHEMA):
            changed = copy.deepcopy(on)
            changed['schema'] = schema
            with self.assertRaisesRegex(ValueError, 'mix'):
                suite.compare(off, changed)
        text = io.StringIO()
        with patch.object(suite, 'pinned_hashes', return_value=self.source), contextlib.redirect_stdout(text):
            suite.compare(off, on)
        self.assertIn('noop/clientplanning', text.getvalue())
        self.assertIn('noise unknown', text.getvalue())


if __name__ == '__main__':
    unittest.main()
