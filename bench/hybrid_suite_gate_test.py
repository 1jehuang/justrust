#!/usr/bin/env python3
"""Unit/adapter subprocess tests only. These are not benchmark evidence."""
import argparse
import contextlib
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import hybrid_suite_gate as gate


class HybridSuiteGateTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.tool = self.root / 'tool'
        self.tool.write_text(f'#!{sys.executable}\nimport os,sys,json\nprint(json.dumps([sys.argv[1:], os.environ.get("JUSTRUST_NATIVE_ENGINE")]))\n')
        self.tool.chmod(0o700)

    def prepare(self):
        output = self.root / 'gate'
        args = argparse.Namespace(output=output, baseline=self.tool,
                                  candidate=self.tool, engine=self.tool, seed=1)
        with contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(gate.prepare(args), 0)
        return output

    def test_candidate_adapter_real_subprocess_adds_hybrid_and_engine(self):
        output = self.prepare()
        child = subprocess.check_output([str(output / 'candidate-exec'), 'test', '--lib'], text=True)
        self.assertEqual(json.loads(child), [['hybrid', 'test', '--lib'], str(self.tool)])

    def test_candidate_adapter_control_commands_are_not_hybrid(self):
        output = self.prepare()
        env = {k: v for k, v in os.environ.items() if k != 'JUSTRUST_NATIVE_ENGINE'}
        for command in ('-V', 'cache', 'toolchain', 'early'):
            child = subprocess.check_output([str(output / 'candidate-exec'), command], env=env, text=True)
            self.assertEqual(json.loads(child), [[command], None])

    def test_baseline_adapter_never_adds_hybrid(self):
        output = self.prepare()
        env = {k: v for k, v in os.environ.items() if k != 'JUSTRUST_NATIVE_ENGINE'}
        child = subprocess.check_output([str(output / 'baseline-exec'), 'test', '--lib'], env=env, text=True)
        self.assertEqual(json.loads(child), [['test', '--lib'], None])

    def test_identity_change_refuses_run(self):
        output = self.prepare()
        self.tool.write_text(self.tool.read_text() + '# changed\n')
        with self.assertRaisesRegex(ValueError, 'input changed'):
            gate.load_verified(output)

    def test_adapter_change_refuses_run(self):
        output = self.prepare()
        adapter = output / 'candidate-exec'
        adapter.write_text(adapter.read_text() + '# changed\n')
        with self.assertRaisesRegex(ValueError, 'input changed'):
            gate.load_verified(output)

    def test_prepare_refuses_existing_evidence(self):
        output = self.prepare()
        with self.assertRaises(FileExistsError):
            gate.prepare(argparse.Namespace(output=output))

    def test_comprehensive_commands_identical_controls(self):
        output = self.prepare()
        report = gate.load_verified(output)
        before = report['commands']['baseline']
        after = report['commands']['candidate']
        self.assertEqual(before[:-1], after[:-1])
        self.assertIn('--synth', before)
        self.assertEqual(before[before.index('--synth') + 1], '1')
        self.assertEqual(before[before.index('--points') + 1], '40')
        self.assertEqual(before[before.index('-n') + 1], '1')
        self.assertIn('--require-valid', report['commands']['compare'])
        self.assertEqual(report['tools']['weights']['sha256'], gate.sha256(gate.HERE / 'synth/weights.json'))

    def test_incomplete_comparison_refused(self):
        output = self.prepare()
        with self.assertRaisesRegex(ValueError, 'both complete'):
            gate.compare(argparse.Namespace(output=output))

    def test_controls_bind_performance_environment_not_secrets(self):
        base = {'PATH': '/bin', 'JUSTRUST_LOCAL': '1', 'SECRET': 'a'}
        self.assertEqual(gate.control_identity(base), gate.control_identity(dict(base, SECRET='b')))
        self.assertNotEqual(gate.control_identity(base), gate.control_identity(dict(base, JUSTRUST_LOCAL='0')))

    def test_real_process_log_retains_failure_output(self):
        log = self.root / 'failure.log'
        command = [sys.executable, '-c', 'import sys; print("actual child failure", flush=True); sys.exit(7)']
        with contextlib.redirect_stdout(io.StringIO()):
            result = gate.invoke(command, dict(os.environ), log)
        self.assertEqual(result['returncode'], 7)
        self.assertIn('actual child failure', log.read_text())
        self.assertGreater(result['wall_seconds'], 0)
        self.assertEqual(result['log_sha256'], gate.sha256(log))

    def test_failed_run_prevents_following_stack(self):
        output = self.prepare()
        args = argparse.Namespace(output=output, stack='both')
        # Stub execution tests orchestration only, never emitted as measured data.
        with patch.object(gate, 'invoke', return_value={'returncode': 9}) as invoke:
            self.assertEqual(gate.run(args), 9)
        self.assertEqual(invoke.call_count, 1)
        report = gate.load_verified(output)
        self.assertEqual(report['status'], 'blocked')
        self.assertNotIn('candidate', report['runs'])


if __name__ == '__main__':
    unittest.main()
