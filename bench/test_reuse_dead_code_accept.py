import base64
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch


spec = importlib.util.spec_from_file_location('dead_code_gate', Path(__file__).with_name('reuse-dead-code-accept.py'))
gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate)


def record(status=0, stdout=b'', stderr=b''):
    return dict(status=status, stdout_b64=base64.b64encode(stdout).decode('ascii'),
                stderr_b64=base64.b64encode(stderr).decode('ascii'), elapsed_seconds=0)


class DeadCodeAcceptanceTests(unittest.TestCase):
    def test_matrix_is_nontrivial_and_edits_real_source(self):
        cases = gate.cases()
        self.assertEqual(len(cases), 32)
        self.assertEqual(len({case[0] for case in cases}), len(cases))
        unchanged = {'initial', 'unchanged', 'body-revert', 'deny-recovery',
                     'expectation-recovery', 'type-error-recovery', 'final-recovery'}
        for name, source, expected in cases:
            if name in unchanged:
                self.assertEqual(source, gate.BASE, name)
            else:
                self.assertNotEqual(source, gate.BASE, name)
            self.assertIn('fn main()', source)
        self.assertEqual([name for name, _, expected in cases if expected is None],
                         ['deny-dead-code', 'deny-unfulfilled', 'type-error'])
        self.assertNotIn('#![allow(dead_code)]', gate.BASE)

    def test_transition_reuses_one_cache_and_fresh_never_reuses(self):
        first, second = gate.schedule(0), gate.schedule(1)
        self.assertEqual([enabled for _, enabled, _ in first[-3:]], [False, True, False])
        self.assertEqual({cache for _, _, cache in first[-3:]}, {'transition'})
        self.assertEqual(first[2:], second[2:])
        self.assertTrue({x[2] for x in first[:2]}.isdisjoint(x[2] for x in second[:2]))
        self.assertEqual(len({x[0] for x in first}), 7)

    def test_comparison_does_not_normalize_any_bytes_or_status(self):
        original = record(stderr=b'warning: /one/fixture.rs:1\n\xff')
        gate.compare(original, dict(original, elapsed_seconds=99), 'same')
        for changed in (record(stderr=b'warning: /two/fixture.rs:1\n\xff'),
                        record(stderr=b'warning: /one/fixture.rs:1\r\n\xff'),
                        record(status=1, stderr=b'warning: /one/fixture.rs:1\n\xff'),
                        record(stdout=b'x', stderr=b'warning: /one/fixture.rs:1\n\xff')):
            with self.assertRaises(AssertionError):
                gate.compare(original, changed, 'different')

    def test_expected_errors_reject_crashes_and_success(self):
        gate.validate_expected(dict(compile=record(status=1)), None, 'expected-error')
        for status in (0, 2, 101, -11):
            with self.assertRaises(AssertionError):
                gate.validate_expected(dict(compile=record(status=status)), None, 'not-ordinary-error')

    def test_success_checks_runtime_and_stderr(self):
        good = dict(compile=record(), runtime=record(stdout=b'1\n'))
        gate.validate_expected(good, '1\n', 'correct')
        for runtime in (record(stdout=b'2\n'), record(status=1, stdout=b'1\n'),
                        record(stdout=b'1\n', stderr=b'warning')):
            with self.assertRaises(AssertionError):
                gate.validate_expected(dict(good, runtime=runtime), '1\n', 'wrong')

    def test_runner_uses_raw_bytes(self):
        completed = subprocess.CompletedProcess(['compiler'], 1, b'\xff\r\n', b'\x80\n')
        with patch.object(gate.subprocess, 'run', return_value=completed) as run:
            result = gate.run(['compiler'])
        self.assertEqual(base64.b64decode(result['stdout_b64']), completed.stdout)
        self.assertEqual(base64.b64decode(result['stderr_b64']), completed.stderr)
        self.assertNotIn('text', run.call_args.kwargs)

    def test_timeouts_and_spawn_failures_are_not_ordinary_errors(self):
        for error in (subprocess.TimeoutExpired(['compiler'], 180), FileNotFoundError('missing compiler')):
            with patch.object(gate.subprocess, 'run', side_effect=error):
                with self.assertRaises(type(error)):
                    gate.run(['compiler'])

    def test_compile_paths_identical_and_no_stale_binary(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / 'fixture').write_bytes(b'stale')
            with patch.object(gate, 'run', return_value=record(status=1)) as run:
                off = gate.compile_case(Path('/compiler'), root, gate.BASE, False, 'off')
                on = gate.compile_case(Path('/compiler'), root, gate.BASE, True, 'on')
            self.assertFalse((root / 'fixture').exists())
            self.assertNotIn('runtime', off)
            self.assertNotIn('runtime', on)
            commands = [call.args[0] for call in run.call_args_list]
            self.assertEqual(commands[0][1], commands[1][1])
            self.assertEqual(commands[0][-1], commands[1][-1])
            self.assertIn('-Zincremental-verify-ich', commands[0])
            self.assertIn(gate.FLAG + '=no', commands[0])
            self.assertIn(gate.FLAG + '=yes', commands[1])

    def test_capture_writes_failed_evidence_on_timeout(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / 'gate'
            with patch.object(gate, 'digest', return_value='hash'), patch.object(
                    gate, 'run', side_effect=subprocess.TimeoutExpired(['compiler'], 180)):
                with self.assertRaises(subprocess.TimeoutExpired):
                    gate.capture(Path('/compiler'), root)
            evidence = json.loads((root / 'acceptance.json').read_text())
            self.assertEqual(evidence['verdict'], 'failed')
            self.assertEqual(evidence['completed_cases'], 0)
            self.assertEqual(evidence['diagnostics'], 'exact-bytes-no-normalization')

    def test_complete_capture_exercises_every_mode(self):
        with tempfile.TemporaryDirectory() as temporary:
            parent = Path(temporary)
            (parent / 'lib').mkdir()
            (parent / 'lib' / 'librustc_driver.so').write_bytes(b'fake driver')
            fake_sysroot = record(stdout=str(parent).encode() + b'\n')
            fake_success = dict(compile=record(stderr=b'exact warning\n'), runtime=record(stdout=b'1\n'))
            with patch.object(gate, 'digest', return_value='hash'), patch.object(
                    gate, 'run', side_effect=[record(), fake_sysroot]), patch.object(
                    gate, 'cases', return_value=[('one', gate.BASE, '1\n')]), patch.object(
                    gate, 'compile_case', return_value=fake_success) as compile_case:
                report = gate.capture(Path('/compiler'), parent / 'gate')
            self.assertEqual(report['verdict'], 'passed')
            self.assertEqual(compile_case.call_count, 7)
            self.assertEqual(report['completed_cases'], 1)
            self.assertEqual(set(report['observations'][0]['modes']), {s[0] for s in gate.schedule(0)})


if __name__ == '__main__':
    unittest.main()
