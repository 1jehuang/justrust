"""Deterministic hosted-infrastructure retries. No builds, network or snapshots."""
import contextlib
import io
import json
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest import mock

from synth import run as runner
import versus


INFRA = 'justrust: hosted build did not run: admission unavailable\n'


def sample(exit=0, **extra):
    return dict(wall=1.25, cpu=1.0, max_rss_mb=4, instructions=100,
                noisy=False, exit=exit, compile_failed=False, tests_failed=0, **extra)


class InfraRetryTests(unittest.TestCase):
    def setUp(self):
        self.stack = runner.versus.Stack('remote', 'hosted', True)
        self.data = dict(i=0, cmd=['check'], pre=['check'], prev=None, edits=[],
                         expect_fail=False, snap='fixture', cwd='.',
                         shape={'command': 'check', 'change': 'body'})
        self.plan = SimpleNamespace(point=SimpleNamespace(i=0, shape=self.data['shape']),
                                    snap='fixture', cmd=['check'], pre=['check'], prev=None,
                                    expect_fail=False, edits=[], to_json=lambda: self.data)

    def execution(self, outputs, retries=2, *, full=False):
        with tempfile.TemporaryDirectory() as tmp, contextlib.ExitStack() as ctx:
            calls = []
            outputs = iter(outputs)

            def execute(st, plan, cmd, tag, **kwargs):
                raw, text = next(outputs)
                path = Path(tmp) / f'{len(calls)}.log'
                path.write_text(text)
                calls.append(tag)
                return {**raw, 'log': str(path)}

            for name in ('_locked', '_recover_journal'):
                ctx.enter_context(mock.patch.object(runner.gen, name))
            reset = ctx.enter_context(mock.patch.object(runner.gen, 'restore_all'))
            restore = ctx.enter_context(mock.patch.object(runner.gen, 'restore'))
            apply = ctx.enter_context(mock.patch.object(runner.gen, 'apply', return_value=[]))
            ctx.enter_context(mock.patch.object(runner, '_run', side_effect=execute))
            ctx.enter_context(mock.patch.object(runner.os, 'sync'))
            sleep = ctx.enter_context(mock.patch.object(runner.time, 'sleep'))
            ctx.enter_context(contextlib.redirect_stderr(io.StringIO()))
            if full:
                ctx.enter_context(mock.patch.object(runner.gen, 'sample', return_value=([0], {'calls': 1, 'coverage': 1})))
                ctx.enter_context(mock.patch.object(runner.gen, 'plan', return_value=self.plan))
                ctx.enter_context(mock.patch.object(runner.gen, 'fingerprint', return_value='fixture'))
                ctx.enter_context(mock.patch.object(runner, 'checkpoint_identity', return_value={'schema': 1}))
                partial = Path(tmp) / 'partial.json'
                value = runner.run([self.stack], 1, 1, 1, 0, retry_infra=retries,
                                   stop_on_invalid=True, partial=partial)
                saved = json.loads(partial.read_text())
                self.assertEqual(saved['infra_attempts'], value['infra_attempts'])
                result, history = value, value['infra_attempts'][0]
            else:
                result, history = runner.run_point_infra(self.stack, self.plan, 123, retries)
            for attempt in history['attempts']:
                for command in attempt['commands']:
                    self.assertTrue(Path(command['raw']['log']).exists())
                    self.assertEqual(Path(command['raw']['log']).read_text(), command['log_text'])
            self.assertEqual(reset.call_count, len(history['attempts']) + (2 if full else 0))
            self.assertEqual(restore.call_count, apply.call_count)
            return result, history, calls, apply.call_count, [c.args[0] for c in sleep.call_args_list]

    def test_bounded_retries_exhausted_preserve_every_error_and_invalid_gate(self):
        value, history, calls, _, sleeps = self.execution([(sample(1), INFRA)] * 4, 3, full=True)
        self.assertEqual(calls, ['pre'] * 4)
        self.assertEqual(sleeps, [30, 60, 120])
        self.assertEqual(history['backoff_seconds'], 210)
        self.assertEqual(len(history['attempts']), 4)
        self.assertTrue(all('pre-build failed' in a['result']['error'] for a in history['attempts']))
        self.assertTrue(runner._blocked(value))
        self.assertTrue(value['results'][0]['errors'])
        self.assertEqual(value['results'][0]['samples']['remote'], [])

    def test_measured_failure_restarts_full_point_without_reusing_prebuild(self):
        result, history, calls, applies, sleeps = self.execution([
            (sample(), ''), (sample(1), INFRA), (sample(), ''), (sample(), '')])
        self.assertEqual(calls, ['pre', 'run', 'pre', 'run'])
        self.assertEqual(applies, 2)
        self.assertEqual(sleeps, [30])
        self.assertNotIn('error', result)
        self.assertEqual(len(history['attempts']), 2)
        self.assertEqual(len(history['attempts'][0]['commands']), 2)

    def test_successful_sample_never_retried_or_selected_for_speed(self):
        result, history, calls, _, sleeps = self.execution([(sample(), ''), (sample(), '')], 3)
        self.assertEqual(calls, ['pre', 'run'])
        self.assertEqual(result['wall'], 1.25)
        self.assertEqual(len(history['attempts']), 1)
        self.assertEqual(sleeps, [])

    def test_fail_closed_on_mixed_compiler_test_unknown_and_noise(self):
        cases = [(sample(1), INFRA + 'error[E0308]: mismatched types\n'),
                 (sample(1), INFRA + 'test result: FAILED. 0 passed; 1 failed\n'),
                 (sample(1), INFRA + "thread 'main' panicked at src/lib.rs:2\n"),
                 (sample(1), 'unknown network error\n'),
                 (sample(1), 'prefix ' + INFRA),
                 ({**sample(1), 'compile_failed': True}, INFRA),
                 ({**sample(1), 'tests_failed': 1}, INFRA)]
        for raw, text in cases:
            with self.subTest(text=text):
                _, history, calls, _, sleeps = self.execution([(raw, text)], 3)
                self.assertEqual(calls, ['pre'])
                self.assertEqual(sleeps, [])
                self.assertFalse(history['attempts'][0]['infra_failure'])
        noisy = {**sample(), 'noisy': True}
        result, _, calls, _, sleeps = self.execution([(sample(), ''), (noisy, INFRA)], 3)
        self.assertTrue(result['noisy'])
        self.assertEqual(calls, ['pre', 'run'])
        self.assertEqual(sleeps, [])

    def test_specific_transport_diagnostics_and_stock_refused(self):
        for text in [INFRA,
                     'justrust: hosted build did not run: the hosted build service is unavailable right now. This is not a problem with your subscription. (server: Hosted builds are temporarily unavailable.)\n',
                     'justrust: hosted build did not run: rsync /fixture/project failed (exit status: 255): justrust native: native connection/authentication failed (request outcome may be unknown): peer closed connection without sending TLS close_notify: https://docs.rs/rustls/latest/rustls/manual/_03_howto/index.html#unexpected-eof\n',
                     'justrust: remote build failed: compiled tests remotely, but returning selected executables failed: justrust native: native connection/authentication failed (request outcome may be unknown): peer closed connection without sending TLS close_notify: the machine closed the artifact stream early\n',
                     'justrust: remote required, refusing local fallback: admission unavailable\n',
                     'justrust: remote required, refusing local fallback: cannot authenticate native hosted TLS at host:443: unexpected EOF\n',
                     'justrust: compiled tests remotely, but returning selected executables failed: could not reach https://build.jcode.sh: curl: (28) Operation timed out after 30001 milliseconds with 0 bytes received. Builds run locally.\n']:
            evidence = [{'raw': sample(1), 'log_text': text}]
            self.assertTrue(runner._infra_failure(self.stack, {'error': 'failed'}, evidence), text)
            self.assertFalse(runner._infra_failure(runner.versus.Stack('stock', 'stock', False),
                                                  {'error': 'failed'}, evidence))

    def test_hosted_no_run_prefix_does_not_make_permanent_failures_retryable(self):
        for reason in ['cannot create /fixture: Permission denied (os error 13)',
                       'cannot create /fixture: No space left on device (os error 28)',
                       'missing credentials, run justrust login',
                       'authentication required',
                       'unsupported target wasm32-unknown-unknown',
                       'failed to parse manifest at Cargo.toml',
                       'unknown remote error',
                       'could not reach https://unrelated.example: curl: (28) Operation timed out',
                       'could not reach https://build.jcode.sh.evil.example: curl: (28) Operation timed out']:
            with self.subTest(reason=reason):
                _, history, calls, _, sleeps = self.execution([
                    (sample(1), 'justrust: hosted build did not run: ' + reason + '\n')], 3)
                self.assertEqual(calls, ['pre'])
                self.assertEqual(sleeps, [])
                self.assertFalse(history['attempts'][0]['infra_failure'])

    def test_cli_bounds_resume_and_forwarding(self):
        for flags in [('--synth', '1', '--retry-infra', '-1'),
                      ('--synth', '1', '--retry-infra', '4'), ('--retry-infra', '1')]:
            with mock.patch.object(sys, 'argv', ['versus.py', 'run', *flags]), \
                    mock.patch.object(versus.suite, 'preflight_runtime') as preflight:
                with self.assertRaises(SystemExit):
                    versus.main()
                preflight.assert_not_called()
        with mock.patch.object(runner.gen, '_locked') as locked:
            with self.assertRaisesRegex(ValueError, 'retry_infra.*resume'):
                runner.run([], 1, 1, 1, 0, resume=True, retry_infra=1)
            locked.assert_not_called()

    def test_full_valid_gate_and_retry_mode_mismatch(self):
        value, history, _, _, _ = self.execution([(sample(1), INFRA), (sample(), ''), (sample(), '')], full=True)
        self.assertEqual(runner._blocked(value), [])
        self.assertEqual(len(history['attempts']), 2)
        other = {**value, 'retry_infra': 1}
        self.assertTrue(any('retry_infra' in x for x in runner.comparison_issues(value, other)))
        self.assertIn('not total agent wait', runner.table(value))
        self.assertGreaterEqual(value['execution_elapsed_seconds'], history['elapsed_seconds'])

    def test_cli_forwards_policy_and_uploads_all_raw_attempts(self):
        from test_versus_synth_cli import SyntheticGateCliTests
        value, _, _, _, _ = self.execution([(sample(1), INFRA)] * 2, 1, full=True)
        kwargs, validity_calls = SyntheticGateCliTests().run_mocked(
            ['--retry-infra', '1', '--stop-on-invalid'], value,
            ['measurement errors'], expect_failure=True)
        self.assertEqual(kwargs['retry_infra'], 1)
        self.assertEqual(validity_calls, 1)

    def test_previous_stage_failure_restores_and_restarts_whole_point(self):
        self.plan.prev = ['build']
        result, history, calls, applies, sleeps = self.execution([
            (sample(), ''), (sample(1), INFRA),
            (sample(), ''), (sample(), ''), (sample(), '')])
        self.assertEqual(calls, ['pre', 'prev', 'pre', 'prev', 'run'])
        self.assertEqual(applies, 2)
        self.assertEqual(sleeps, [30])
        self.assertNotIn('error', result)
        self.assertEqual(history['attempts'][0]['commands'][-1]['stage'], 'prev')

    def test_default_result_bytes_unchanged(self):
        with contextlib.ExitStack() as ctx:
            for name in ('_locked', 'restore_all'):
                ctx.enter_context(mock.patch.object(runner.gen, name))
            ctx.enter_context(mock.patch.object(runner.gen, 'sample', return_value=([0], {'calls': 1, 'coverage': 1})))
            ctx.enter_context(mock.patch.object(runner.gen, 'plan', return_value=self.plan))
            ctx.enter_context(mock.patch.object(runner.gen, 'fingerprint', return_value='fixture'))
            point = ctx.enter_context(mock.patch.object(runner, 'run_point', return_value=sample()))
            ctx.enter_context(contextlib.redirect_stderr(io.StringIO()))
            value = runner.run([self.stack], 1, 1, 1, 0)
            expected = {'seed': 1, 'points': 1, 'reps': 1, 'generator': 'fixture',
                        'sample_info': {'calls': 1, 'coverage': 1}, 'stack_names': ['remote'],
                        'results': [{**self.data, 'samples': {'remote': [sample()]}, 'errors': {},
                                     'stats': {'remote': runner.versus.stats([sample()])}}]}
            self.assertEqual(json.dumps(value), json.dumps(expected))
            self.assertEqual(point.call_count, 1)
            self.assertNotIn('evidence', point.call_args.kwargs)


if __name__ == '__main__':
    unittest.main()
