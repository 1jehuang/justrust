#!/usr/bin/env python3
"""Pure mocks only. Never starts Cargo, rustc, a worker, or the real client."""
import base64
import hashlib
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import stable_resident_cargo_dependency as subject


def encoded(text):
    return base64.b64encode(text.encode()).decode()


def record(stage, diagnostics=None):
    stdout = ''
    stderr = ''
    if stage['expectation'] == 'error':
        message = diagnostics or dict(level='error', message=subject.ERROR, spans=[])
        stdout = json.dumps(dict(reason='compiler-message', message=message)) + '\n'
        status = 101
    else:
        stdout = ('test dependency_nonce_probe ... ok\n'
                  'test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n')
        stderr = 'JR_CARGO_NONCE=' + stage['nonce'] + '\n'
        status = 0
    return dict(status=status, timed_out=False, stdout_b64=encoded(stdout), stderr_b64=encoded(stderr))


class DependencyAcceptanceTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='jd-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / 'bin').mkdir()
        (self.root / 'lib').mkdir()
        self.compiler = self.root / 'bin/rustc'
        self.cargo = self.root / 'bin/cargo'
        self.client = self.root / 'bin/justrust'
        self.driver = self.root / 'lib/librustc_driver.so'
        for path in (self.compiler, self.cargo, self.client, self.driver):
            path.write_text(path.name + ' mock bytes, never executable')
        self.options = dict(root_native_frozen=True, idle_authorized=True,
                            expected_compiler_sha256=subject.shared.native.digest(self.compiler),
                            expected_driver_sha256=subject.shared.native.digest(self.driver),
                            expected_cargo_sha256=subject.shared.native.digest(self.cargo))
        self.native = patch.object(subject.shared.native, 'run', side_effect=AssertionError('NO NATIVE INVOCATIONS'))
        self.native.start()
        self.addCleanup(self.native.stop)

    def capture(self, runner, **options):
        return subject.capture(self.compiler, self.cargo, self.root / 'o', runner=runner,
                               **dict(self.options, **options))

    def mock_runner(self, *, wrong_nonce=False, forbidden_trace=False, mismatch=False, client=False,
                    resident_root=False, stale_identity=False, restart=False, stale_request=False):
        calls = []
        workload = None
        def invoke(command, timeout, cwd, env):
            nonlocal workload
            index = len(calls) % 10
            source = (cwd / 'src/lib.rs').read_text()
            nonce = source.split('JR_CARGO_NONCE=', 1)[1].split('-initial:', 1)[0] if workload is None else None
            if workload is None:
                workload = subject.stages(nonce)
            stage = workload[index]
            self.assertEqual((cwd / 'src/lib.rs').read_bytes(), stage['root'])
            self.assertEqual((cwd / 'dep/src/lib.rs').read_bytes(), stage['dependency'])
            self.assertEqual(command[1:3], ['cargo', 'test'] if client else ['test', '--offline'])
            self.assertIn('--offline', command)
            self.assertNotIn('MY_API_KEY', env)
            before = (cwd / 'dep/src/lib.rs').stat().st_mtime_ns
            mode = 'off' if env['JUSTRUST_CARGO_RESIDENT'] == 'off' else 'on'
            if forbidden_trace or (mode == 'on' and not stage['root_only']):
                event = dict(event='complete', key='dependency-worker', elapsed_ns=1,
                             result=dict(pid=123, id=len(calls), status=1 if stage['expectation'] == 'error' else 0))
                with Path(env['JUSTRUST_CARGO_RESIDENT_TRACE']).open('a') as trace:
                    trace.write(json.dumps(event) + '\n')
            if mode == 'on' and resident_root and stage['name'] != 'dependency-error':
                generation = 0 if stale_identity or index < 4 else min(index - 3, 3)
                event = dict(event='complete', key=hashlib.sha256(str(generation).encode()).hexdigest(),
                             crate_name='dependency_fallback_root', elapsed_ns=1,
                             result=dict(pid=200 + generation + (index if restart else 0),
                                         id=0 if stale_request else index,
                                         status=1 if stage['expectation'] == 'error' else 0))
                if env.get('JUSTRUST_CARGO_RESIDENT_OWNED') == '1':
                    event['result']['owned'] = dict(enabled=True, revisions=index + 1, contexts=index + 1)
                with Path(env['JUSTRUST_CARGO_RESIDENT_TRACE']).open('a') as trace:
                    trace.write(json.dumps(event) + '\n')
            result = record(stage)
            if wrong_nonce and index == 1:
                result['stderr_b64'] = encoded('JR_CARGO_NONCE=stale\n')
            if mismatch and mode == 'on' and stage['expectation'] == 'error':
                result = record(stage, dict(level='error', message=subject.ERROR, spans=[{'line_start': 999}]))
            artifact_path = Path(env['CARGO_TARGET_DIR']) / 'libfixture_dep.rlib'
            artifact_path.parent.mkdir(parents=True, exist_ok=True)
            if stage['name'] != 'dependency-error':
                artifact_path.write_bytes(stage['dependency'])
            artifact = dict(reason='compiler-artifact', package_id='fixture_dep',
                            target=dict(name='fixture_dep'), fresh=stage['root_only'],
                            filenames=[str(artifact_path)])
            result['stdout_b64'] = encoded(json.dumps(artifact) + '\n' + base64.b64decode(result['stdout_b64']).decode())
            calls.append(dict(command=command, env=env, cwd=cwd, mode=mode, stage=stage, mtime=before))
            return result
        return invoke, calls

    def test_workload_signature_const_body_and_error_recovery(self):
        rows = subject.stages('nonce')
        self.assertEqual(len(rows), 10)
        self.assertIn(b'value(n: u32) -> u64', rows[4]['dependency'])
        self.assertIn(b'VALUE: u64 = 11', rows[5]['dependency'])
        self.assertIn(b'19 + n', rows[6]['dependency'])
        self.assertIn(subject.ERROR.encode(), rows[7]['dependency'])
        self.assertEqual(rows[6]['dependency'], rows[8]['dependency'])
        for i in (1, 2, 3, 9):
            self.assertEqual(rows[i - 1]['dependency'], rows[i]['dependency'])
            self.assertTrue(rows[i]['root_only'])

    def test_capture_is_offline_separate_targets_stable_dependency_path(self):
        runner, calls = self.mock_runner()
        with patch.dict(os.environ, {'MY_API_KEY': 'never-record-me', 'RUSTC_WRAPPER': '/bad'}):
            report = self.capture(runner)
        self.assertEqual(report['verdict'], 'passed')
        self.assertEqual(len(calls), 20)
        self.assertEqual(len({str(call['cwd'] / 'dep/src/lib.rs') for call in calls}), 1)
        self.assertEqual(len({call['env']['CARGO_TARGET_DIR'] for call in calls}), 2)
        for start in (0, 10):
            self.assertEqual(len({c['mtime'] for c in calls[start:start + 4]}), 1)
        for arm in report['arms'].values():
            for row in arm['observations']:
                if row['root_only']:
                    self.assertEqual(row['trace'], [])
        text = (self.root / 'o/receipt.json').read_text()
        self.assertNotIn('never-record-me', text)
        self.assertNotIn('RUSTC_WRAPPER', text)
        self.assertTrue((self.root / 'o/000-on/07-dependency.rs').exists())
        self.assertIn('stable_resident_cargo.py', report['provenance']['helpers'])

    def test_optional_actual_client_uses_shared_strict_runner(self):
        runner, calls = self.mock_runner(client=True)
        report = self.capture(runner, justrust=self.client,
                              expected_client_sha256=subject.shared.native.digest(self.client))
        self.assertEqual(report['verdict'], 'passed')
        for call in calls:
            self.assertEqual(call['command'][0], self.client)
            self.assertEqual(call['env']['JUSTRUST_REAL_CARGO'], str(self.cargo))
            self.assertEqual(call['env']['JUSTRUST_TOOLCHAIN'], 'system')

    def test_authorization_required_before_any_execution_or_output(self):
        for name in ('root_native_frozen', 'idle_authorized'):
            with self.assertRaisesRegex(ValueError, 'freeze and idle'):
                self.capture(lambda *a, **k: self.fail('executed'), **{name: False})
            self.assertFalse((self.root / 'o').exists())

    def test_attestation_precedes_execution_and_output(self):
        for name in ('compiler', 'driver', 'cargo'):
            for digest in (None, '0' * 64):
                with self.assertRaises(ValueError):
                    self.capture(lambda *a, **k: self.fail('executed'), **{'expected_' + name + '_sha256': digest})
                self.assertFalse((self.root / 'o').exists())

    def test_client_requires_attestation(self):
        with self.assertRaisesRegex(ValueError, 'client SHA256'):
            self.capture(lambda *a, **k: self.fail('executed'), justrust=self.client)

    def test_changed_candidate_fails_post_execution(self):
        runner, calls = self.mock_runner()
        def changing(*args, **kwargs):
            result = runner(*args, **kwargs)
            if len(calls) == 20:
                self.cargo.write_text('changed')
            return result
        with self.assertRaisesRegex(ValueError, 'Unattested cargo'):
            self.capture(changing)
        self.assertEqual(json.loads((self.root / 'o/receipt.json').read_text())['verdict'], 'failed')

    def test_root_only_complete_rejected_and_dependency_event_allowed(self):
        rows = subject.stages('nonce')
        event = [dict(event='complete')]
        for mode in ('off', 'on'):
            with self.assertRaisesRegex(AssertionError, 'fallback|normal-off'):
                subject.validate_trace(rows[1], event, mode)
        with self.assertRaises(AssertionError):
            subject.validate_trace(rows[4], event, 'off')
        result = subject.validate_trace(rows[4], event, 'on')
        self.assertIn('not external reuse', result['interpretation'])

    def test_stale_nonce_preserves_full_failed_evidence(self):
        runner, _ = self.mock_runner(wrong_nonce=True)
        with self.assertRaisesRegex(AssertionError, 'nonce'):
            self.capture(runner)
        arm = json.loads((self.root / 'o/000-off/receipt.json').read_text())
        self.assertEqual(arm['verdict'], 'failed')
        self.assertIn('stdout_b64', arm['observations'][-1]['cargo'])
        self.assertIn('trace', arm['observations'][-1])
        self.assertEqual(json.loads((self.root / 'o/receipt.json').read_text())['verdict'], 'failed')

    def test_exact_diagnostics_not_just_error_presence(self):
        runner, _ = self.mock_runner(mismatch=True)
        with self.assertRaisesRegex(AssertionError, 'Exact off/on diagnostics'):
            self.capture(runner)

    def test_exception_retains_arm_receipt_and_trace(self):
        def broken(*args, **kwargs):
            raise RuntimeError('mock runner failed')
        with self.assertRaisesRegex(RuntimeError, 'mock runner failed'):
            self.capture(broken)
        arm = json.loads((self.root / 'o/000-off/receipt.json').read_text())
        self.assertEqual(arm['verdict'], 'failed')
        self.assertEqual(arm['observations'][0]['trace'], [])

    def test_error_never_executes_stale_test(self):
        stage = subject.stages('nonce')[2]
        result = record(stage)
        result['stderr_b64'] = encoded('JR_CARGO_NONCE=stale\n')
        with self.assertRaises(AssertionError):
            subject.semantics(result, stage)
        result = record(stage)
        result['stdout_b64'] = encoded(base64.b64decode(result['stdout_b64']).decode() + 'test dependency_nonce_probe ... ok\n')
        with self.assertRaisesRegex(AssertionError, 'stale root'):
            subject.semantics(result, stage)

    def test_metadata_observed_without_artifact_equality_contract(self):
        artifact = self.root / 'libfixture_dep.rlib'
        artifact.write_bytes(b'first')
        result = dict(stdout_b64=encoded(json.dumps(dict(reason='compiler-artifact', package_id='dep',
                      fresh=False, target={'name': 'fixture_dep'}, filenames=[str(artifact)]))), stderr_b64=encoded(''))
        first = subject.metadata(result)
        artifact.write_bytes(b'second')
        self.assertNotEqual(first[0]['artifacts'][0]['sha256'], subject.metadata(result)[0]['artifacts'][0]['sha256'])

    def test_root_only_requires_positive_fresh_dependency_metadata(self):
        stage = subject.stages('nonce')[1]
        for evidence in ([], [dict(target={'name': 'fixture_dep'}, fresh=False)]):
            with self.assertRaisesRegex(AssertionError, 'Cargo-fresh'):
                subject.validate_artifacts(stage, evidence)
        subject.validate_artifacts(stage, [dict(target={'name': 'fixture_dep'}, fresh=True)])

    def test_normal_off_complete_fails_with_trace_preserved(self):
        runner, _ = self.mock_runner(forbidden_trace=True)
        with self.assertRaisesRegex(AssertionError, 'normal-off'):
            self.capture(runner)
        report = json.loads((self.root / 'o/receipt.json').read_text())
        self.assertEqual(report['arms']['off']['observations'][0]['trace'][0]['event'], 'complete')
        self.assertEqual(report['verdict'], 'failed')

    def test_required_root_resident_identity_and_dependency_invalidation(self):
        runner, calls = self.mock_runner(resident_root=True)
        report = self.capture(runner, require_resident_root=True)
        self.assertTrue(report['provenance']['require_resident_root'])
        self.assertIn('identity invalidation', report['claim'])
        observations = report['arms']['on']['observations']
        roots = [row['residency'].get('root') for row in observations]
        self.assertEqual(len({(root['key'], root['pid']) for root in roots[:4]}), 1)
        self.assertEqual(len({roots[i]['key'] for i in (0, 4, 5, 6)}), 4)
        self.assertIsNone(roots[7])
        self.assertEqual(roots[8]['pid'], roots[9]['pid'])
        self.assertGreater(roots[3]['request'], roots[2]['request'])
        self.assertEqual(len(calls), 20)

    def test_required_root_rejects_fallback_with_failed_receipt(self):
        runner, _ = self.mock_runner()
        with self.assertRaisesRegex(AssertionError, 'Required resident root'):
            self.capture(runner, require_resident_root=True)
        report = json.loads((self.root / 'o/receipt.json').read_text())
        self.assertEqual(report['verdict'], 'failed')
        self.assertIn('trace', report['arms']['on']['observations'][-1])

    def test_required_root_rejects_stale_dependency_identity(self):
        runner, _ = self.mock_runner(resident_root=True, stale_identity=True)
        with self.assertRaisesRegex(AssertionError, 'did not invalidate'):
            self.capture(runner, require_resident_root=True)

    def test_required_root_rejects_restarted_worker(self):
        runner, _ = self.mock_runner(resident_root=True, restart=True)
        with self.assertRaisesRegex(AssertionError, 'replaced the resident worker'):
            self.capture(runner, require_resident_root=True)

    def test_owned_batch_requires_explicit_root_and_sets_both_arm_environments(self):
        with self.assertRaisesRegex(ValueError, 'explicit resident root'):
            self.capture(lambda *a, **k: self.fail('executed'), owned_batch=True)
        runner, calls = self.mock_runner(resident_root=True, stale_request=True)
        report = self.capture(runner, require_resident_root=True, owned_batch=True)
        self.assertEqual(report['verdict'], 'passed')
        self.assertTrue(report['provenance']['owned_batch'])
        for call in calls:
            self.assertEqual(call['env']['JUSTRUST_CARGO_RESIDENT_OWNED'], '1')
            self.assertEqual(call['env']['JUSTRUST_CARGO_RESIDENT_EXTERNAL'], '1')
            self.assertEqual(call['env']['JUSTRUST_RUSTC_RESIDENT_FRONTEND'], '0')

    def test_owned_batch_requires_server_revision_evidence(self):
        stage = subject.stages('nonce')[1]
        event = dict(event='complete', crate_name='dependency_fallback_root', result={})
        with self.assertRaisesRegex(AssertionError, 'telemetry missing'):
            subject.validate_owned_batch(stage, [event], None)
        event['result']['owned'] = dict(enabled=True, revisions=2, contexts=2)
        subject.validate_owned_batch(stage, [event], dict(owned_revisions=1))
        with self.assertRaisesRegex(AssertionError, 'did not advance'):
            subject.validate_owned_batch(stage, [event], dict(owned_revisions=2))
        for field in ('revisions', 'contexts'):
            invalid = dict(event, result=dict(owned={**event['result']['owned'], field: True}))
            with self.assertRaisesRegex(AssertionError, 'telemetry missing'):
                subject.validate_owned_batch(stage, [invalid], None)

    def test_required_root_allows_client_scoped_request_id_reset(self):
        runner, _ = self.mock_runner(resident_root=True, stale_request=True)
        self.assertEqual(self.capture(runner, require_resident_root=True)['verdict'], 'passed')

    def test_required_dependency_transition_requires_actual_same_path_bytes(self):
        rows = subject.stages('nonce')
        def observations(path='dep.rlib', digest='a' * 64, exists=True):
            return [dict(target={'name': 'fixture_dep'}, artifacts=[
                dict(path=path, exists=exists, sha256=digest)])]
        previous = subject.validate_dependency_transition(rows[0], observations(), None)
        for evidence in ([], observations(exists=False), observations(digest=None)):
            with self.assertRaisesRegex(AssertionError, 'identity'):
                subject.validate_dependency_transition(rows[1], evidence, previous)
        for evidence in (observations(), observations(path='different.rlib', digest='b' * 64)):
            with self.assertRaisesRegex(AssertionError, 'same-path'):
                subject.validate_dependency_transition(rows[4], evidence, previous)
        changed = subject.validate_dependency_transition(rows[4], observations(digest='b' * 64), previous)
        self.assertNotEqual(changed, previous)
        with self.assertRaisesRegex(AssertionError, 'Root-only edit changed'):
            subject.validate_dependency_transition(rows[1], observations(digest='b' * 64), previous)
        self.assertEqual(subject.validate_dependency_transition(rows[7], [], previous), previous)

    def test_required_root_trace_cannot_mistake_dependency_completion(self):
        rows = subject.stages('nonce')
        dependency = dict(event='complete', key='a' * 64, crate_name='fixture_dep',
                          result=dict(pid=99, id=1, status=0))
        for index in (0, 1, 4, 5, 6, 8, 9):
            with self.assertRaisesRegex(AssertionError, 'Required resident root'):
                subject.validate_trace(rows[index], [dependency], 'on', True)
        root = {**dependency, 'crate_name': 'dependency_fallback_root'}
        self.assertIsNotNone(subject.validate_trace(rows[4], [root, dependency], 'on', True)['root'])
        self.assertIsNone(subject.validate_trace(rows[7], [dependency], 'on', True)['root'])
        with self.assertRaisesRegex(AssertionError, 'unexpectedly compiled the root'):
            subject.validate_trace(rows[7], [root], 'on', True)
        with self.assertRaisesRegex(AssertionError, 'exactly one'):
            subject.validate_trace(rows[4], [root, root], 'on', True)

    def test_required_root_error_and_recovery_must_use_same_worker(self):
        rows = subject.stages('nonce')
        previous = dict(key='a' * 64, pid=101, request=2)
        for index in (1, 2, 3, 9):
            current = dict(key='a' * 64, pid=101, request=1)
            self.assertEqual(subject.validate_root_transition(rows[index], {'root': current}, previous), current)
            for changed in ({**current, 'key': 'b' * 64}, {**current, 'pid': 102}):
                with self.assertRaisesRegex(AssertionError, 'replaced the resident worker'):
                    subject.validate_root_transition(rows[index], {'root': changed}, previous)

    def test_required_root_malformed_identity_status_and_extra_completion(self):
        stage = subject.stages('nonce')[2]
        event = dict(event='complete', key='a' * 64,
                     crate_name='dependency_fallback_root', result=dict(pid=99, id=2, status=1))
        for field, value in [('pid', 0), ('pid', True), ('id', -1), ('id', '2'),
                             ('status', 0), ('status', True)]:
            malformed = {**event, 'result': {**event['result'], field: value}}
            with self.assertRaisesRegex(AssertionError, 'Malformed'):
                subject.validate_trace(stage, [malformed], 'on', True)
        with self.assertRaisesRegex(AssertionError, 'Malformed'):
            subject.validate_trace(stage, [{**event, 'key': 'not-a-hash'}], 'on', True)
        with self.assertRaisesRegex(AssertionError, 'exactly one'):
            subject.validate_trace(stage, [event, event], 'on', True)
        with self.assertRaisesRegex(AssertionError, 'normal-off'):
            subject.validate_trace(stage, [event], 'off', True)

    def test_timeout_and_long_socket_paths_rejected(self):
        for timeout in (0, float('inf'), float('nan')):
            with self.assertRaises(ValueError):
                self.capture(lambda *a, **k: self.fail('executed'), timeout=timeout)
        with self.assertRaisesRegex(ValueError, 'too long'):
            subject.capture(self.compiler, self.cargo, self.root / ('x' * 100), **self.options)


if __name__ == '__main__':
    unittest.main()
