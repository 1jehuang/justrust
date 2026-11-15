"""CLI-only synthetic gate tests. Never compile or measure a workload."""
import contextlib
import io
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

import versus

sys.path.insert(0, str(Path(__file__).resolve().parent / 'synth'))
import run as synth_run


class SyntheticGateCliTests(unittest.TestCase):
    def invoke(self, *args):
        with mock.patch.object(sys, 'argv', ['versus.py', 'run', *args]):
            return versus.main()

    def test_help_describes_synthetic_controls(self):
        output = io.StringIO()
        with contextlib.redirect_stdout(output), self.assertRaises(SystemExit) as exit:
            self.invoke('--help')
        self.assertEqual(exit.exception.code, 0)
        self.assertIn('--stop-on-invalid', output.getvalue())
        self.assertIn('--retry-noisy N', output.getvalue())
        self.assertIn('incompatible with --resume', output.getvalue())

    def test_named_workloads_reject_controls_before_preflight(self):
        for flags in [('--stop-on-invalid',), ('--retry-noisy', '2')]:
            with self.subTest(flags=flags), mock.patch.object(versus.suite, 'preflight_runtime') as preflight:
                with self.assertRaisesRegex(SystemExit, 'require --synth'):
                    self.invoke('reference-loop', *flags)
                preflight.assert_not_called()

    def test_negative_retry_rejected_before_preflight(self):
        with mock.patch.object(versus.suite, 'preflight_runtime') as preflight:
            with self.assertRaisesRegex(SystemExit, 'must be nonnegative'):
                self.invoke('--synth', '1', '--retry-noisy', '-1')
            preflight.assert_not_called()

    def run_mocked(self, flags, result, blocked, expect_failure=False):
        with tempfile.TemporaryDirectory() as directory, contextlib.ExitStack() as ctx:
            ctx.enter_context(mock.patch.object(versus.suite, 'RESULTS', Path(directory)))
            ctx.enter_context(mock.patch.object(versus.suite, 'preflight_runtime'))
            ctx.enter_context(mock.patch.object(versus.suite, 'env_info', return_value={
                'justrust_rev': 'test', 'justrust_dirty': False}))
            ctx.enter_context(mock.patch.object(versus.suite, 'sh', return_value='mock cargo'))
            ctx.enter_context(mock.patch.object(versus.suite, 'verify_binary_identity'))
            ctx.enter_context(mock.patch.object(versus.suite, 'synthetic_partial_path', return_value=Path(directory) / 'partial.json'))
            ctx.enter_context(mock.patch.object(versus, 'open_instruction_counters', return_value=False))
            engine = ctx.enter_context(mock.patch.object(synth_run, 'run', return_value=result))
            ctx.enter_context(mock.patch.object(synth_run, 'table', return_value='synthetic table'))
            validity = ctx.enter_context(mock.patch.object(synth_run, '_blocked',
                **({'wraps': synth_run._blocked} if blocked is None else {'return_value': blocked})))
            uploaded = []
            upload = ctx.enter_context(mock.patch.object(versus.suite, 'upload_result',
                side_effect=lambda path: uploaded.append(json.loads(path.read_text()))))
            report = ctx.enter_context(mock.patch.object(versus, 'write_report'))
            output = ctx.enter_context(contextlib.redirect_stdout(io.StringIO()))
            if expect_failure:
                with self.assertRaisesRegex(SystemExit, 'synthetic gate invalid'):
                    self.invoke('--synth', '1', '-n', '1', '--no-instructions', *flags)
            else:
                self.invoke('--synth', '1', '-n', '1', '--no-instructions', *flags)
            call = engine.call_args
            self.assertIn('stock', [stack.name for stack in call.args[0]])
            if '--remote' in flags:
                self.assertIn(versus.REMOTE, call.args[0])
            self.assertEqual(call.args[1:], (1, 40, 1, 0))
            self.assertEqual(call.kwargs['resume'], False)
            self.assertEqual(call.kwargs['only'], None)
            self.assertEqual(call.kwargs['partial'], Path(directory) / 'partial.json')
            upload.assert_called_once()
            report.assert_called_once_with(upload.call_args.args[0])
            self.assertEqual(uploaded[0]['synth'], result)
            self.assertIn('synthetic table', output.getvalue())
            self.assertIn('saved ', output.getvalue())
            return call.kwargs, validity.call_count

    def test_defaults_do_not_override_engine_or_change_invalid_exit(self):
        kwargs, calls = self.run_mocked([], {'stopped_early': 'noisy'}, ['noisy'])
        self.assertNotIn('stop_on_invalid', kwargs)
        self.assertNotIn('retry_noisy', kwargs)
        self.assertEqual(calls, 0)

    def test_controls_forwarded_and_invalid_evidence_saved_uploaded_reported(self):
        kwargs, calls = self.run_mocked(['--stop-on-invalid', '--retry-noisy', '2'],
            {'stopped_early': 'point 0: noisy samples'}, ['incomplete coverage', 'noisy samples'], True)
        self.assertIs(kwargs['stop_on_invalid'], True)
        self.assertEqual(kwargs['retry_noisy'], 2)
        self.assertEqual(calls, 1)

    def test_invalid_without_stopped_marker_also_fails(self):
        self.run_mocked(['--stop-on-invalid'], {'results': []}, None, True)

    def test_valid_gate_succeeds(self):
        self.run_mocked(['--stop-on-invalid'], {'results': []}, [])

    def test_retry_without_failfast_preserves_exit_and_remote_stack(self):
        with mock.patch.dict(versus.os.environ, {}, clear=True):
            kwargs, calls = self.run_mocked(['--only', 'stock', '--remote', '--retry-noisy', '3'],
                {'results': []}, ['incomplete coverage'])
        self.assertEqual(kwargs['retry_noisy'], 3)
        self.assertNotIn('stop_on_invalid', kwargs)
        self.assertEqual(calls, 0)

    def test_resume_incompatibility_reaches_engine_with_actual_cli_args(self):
        original = synth_run.run
        with contextlib.ExitStack() as ctx:
            ctx.enter_context(mock.patch.object(versus.suite, 'preflight_runtime'))
            ctx.enter_context(mock.patch.object(versus.suite, 'env_info', return_value={}))
            ctx.enter_context(mock.patch.object(versus.suite, 'sh', return_value='mock cargo'))
            ctx.enter_context(mock.patch.object(versus.suite, 'synthetic_partial_path', return_value=None))
            ctx.enter_context(mock.patch.object(versus, 'open_instruction_counters', return_value=False))
            engine = ctx.enter_context(mock.patch.object(synth_run, 'run', wraps=original))
            with self.assertRaisesRegex(ValueError, 'stop_on_invalid cannot be combined with resume'):
                self.invoke('--synth', '1', '--stop-on-invalid', '--resume', '--no-instructions')
            self.assertTrue(engine.call_args.kwargs['resume'])
            self.assertTrue(engine.call_args.kwargs['stop_on_invalid'])


if __name__ == '__main__':
    unittest.main()
