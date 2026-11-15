#!/usr/bin/env python3
"""Synthetic validation of the harness only. These are not engine measurements."""
import argparse
import json
import os
from pathlib import Path
import sys
import subprocess
import tempfile
import threading
import unittest
from unittest.mock import patch

import hybrid_workload as workload


FAKE = '''#!/usr/bin/env python3
import os, pathlib, re, sys
root = pathlib.Path.cwd()
text = (root / 'src/lib.rs').read_text()
def maybe_queue():
    if os.environ.get('JUSTRUST_HYBRID_PREWARM') == 'after-success':
        with (pathlib.Path(os.environ['JR_ENGINE_HOME']) / 'prewarm.log').open('a') as f:
            f.write('AUTOMATIC_SUCCESS\\n')
        print('justrust hybrid: queued background preparation')
if 'stop' in sys.argv:
    sys.exit(0)
if 'prewarm' in sys.argv:
    if os.environ.get('FAKE_BAD_PREWARM'):
        with open(os.environ['HYBRID_SIDE_EFFECT'], 'a') as f: f.write('bad\\n')
    sys.exit(0)
if 'build' in sys.argv or 'check' in sys.argv:
    maybe_queue()
    sys.exit(0)
if 'run' in sys.argv:
    with open(os.environ['HYBRID_PROGRAM_SIDE_EFFECT'], 'a') as f: f.write('program-nonce=13\\n')
    print('HYBRID_PROGRAM_EXECUTED_13')
    maybe_queue()
    sys.exit(0)
if 'intentionally invalid' in text:
    print('error: invalid Rust')
    sys.exit(101)
nonce = re.search(r'nonce=(\\d+)', text)[1]
dep = re.search(r'\\{ (\\d+) \\}', (root / 'dep/src/lib.rs').read_text())[1]
config = re.search(r'value="([^"]+)"', (root / '.cargo/config.toml').read_text())[1]
with open(os.environ['HYBRID_SIDE_EFFECT'], 'a') as f:
    f.write('nonce=' + nonce + '\\n')
    if os.environ.get('FAKE_DOUBLE') and 'hybrid' in sys.argv: f.write('nonce=' + nonce + '\\n')
print('HYBRID_OBSERVED nonce=' + nonce + ' dependency=' + dep + ' config=' + config)
failed = 'value(), 99' in text
print('test result: ' + ('FAILED. 0 passed; 1 failed; 0 ignored;' if failed else 'ok. 1 passed; 0 failed; 0 ignored;'))
print('route=' + ('cold' if nonce == '0' else 'warm'))
if not failed: maybe_queue()
sys.exit(101 if failed else 0)
'''


class Contracts(unittest.TestCase):
    def test_reject_false_success(self):
        row = {'returncode': 0, 'timeout': False}
        self.assertFalse(workload.contract(row, '', ['nonce=1'], 1, 7, 'one')['ok'])

    def test_reject_signal_as_expected_compile_error(self):
        self.assertFalse(workload.contract({'returncode': -9, 'timeout': False}, 'error', [], 0, 7, 'one', 'compile')['ok'])

    def test_reject_stale_nonce(self):
        text = 'HYBRID_OBSERVED nonce=0 dependency=7 config=one\ntest result: ok. 1 passed; 0 failed; 0 ignored;'
        self.assertFalse(workload.contract({'returncode': 0, 'timeout': False}, text, ['nonce=0'], 1, 7, 'one')['ok'])

    def run_matrix(self, override=(), evict=False, operations=False, automatic=False):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            fake = root / 'fake'
            fake.write_text(FAKE)
            fake.chmod(0o700)
            args = argparse.Namespace(output=root / 'out', justrust=fake, engine=fake,
                                      env=list(override), timeout=5, prewarm=not automatic, evict=evict, operations=operations,
                                      automatic=automatic, automatic_ready_regex='AUTOMATIC_SUCCESS' if automatic else None,
                                      cold_route_regex=None, warm_route_regex=None, toolchain=None)
            status = workload.execute(args)
            report = json.loads((args.output / 'report.json').read_text())
            self.assertTrue(report['cleanup_passed'], report['cleanup'])
            self.assertEqual(len(report['cleanup']), 2)
            expected = 26 + (3 if evict else 0) + (0 if automatic else 1)
            expected += (6 if automatic else 7) if operations else 0
            self.assertEqual(len(report['rows']), expected)
            self.assertTrue(all(Path(row['log']).is_file() for row in report['rows']))
            self.assertTrue(all(len(row['log_sha256']) == 64 for row in report['rows']))
            return status, report

    def test_full_synthetic_matrix(self):
        status, report = self.run_matrix()
        self.assertEqual(status, 0)
        self.assertTrue(report['correctness_passed'])
        self.assertFalse(report['routing_evidence_complete'])

    def test_double_execution_rejected(self):
        status, report = self.run_matrix(['FAKE_DOUBLE=1'])
        self.assertEqual(status, 1)
        self.assertFalse(report['correctness_passed'])

    def test_eviction_stop_does_not_execute_test(self):
        status, report = self.run_matrix(evict=True)
        self.assertEqual(status, 0)
        self.assertTrue(next(row for row in report['rows'] if row['name'] == 'stop')['ok'])

    def test_launch_failure_is_not_compile_error(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            row = workload.run([str(root / 'absent')], root, dict(os.environ), root / 'launch.log', 1)
            self.assertIsNone(row['returncode'])
            self.assertIsNotNone(row['error'])
            self.assertFalse(workload.contract(row, 'error', [], 1, 7, 'one', 'compile')['ok'])

    def test_build_check_run_matrix(self):
        status, report = self.run_matrix(operations=True)
        self.assertEqual(status, 0)
        self.assertTrue(next(row for row in report['rows'] if row['name'] == 'run-prewarm')['ok'])
        self.assertTrue(all(row['side_effect_delta'] == ['program-nonce=13'] for row in report['rows'] if row['name'] == 'run'))

    def test_automatic_prewarm_matrix(self):
        status, report = self.run_matrix(automatic=True, operations=True, evict=True)
        self.assertEqual(status, 0)
        self.assertTrue(any('automatic_preparation' in row for row in report['rows']))
        self.assertFalse(any('JUSTRUST_HYBRID_PREWARM' in row.get('environment', {}) for row in report['rows'] if row['stack'] == 'shipping'))

    def test_automatic_wait_rejects_stale_success_receipt(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / 'prewarm.log').write_text('AUTOMATIC_SUCCESS\n')
            offset = workload.background_offset(root)
            row = workload.wait_background(root, offset, root / 'side', [], 0.03,
                                           'AUTOMATIC_SUCCESS', root / 'evidence')
            self.assertFalse(row['ok'])
            self.assertEqual((root / 'evidence').read_bytes(), b'')

    def test_automatic_wait_rejects_execution_during_prewarm(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / 'prewarm.log').write_text('AUTOMATIC_SUCCESS\n')
            (root / 'side').write_text('bad execution\n')
            row = workload.wait_background(root, 0, root / 'side', [], 0.03,
                                           'AUTOMATIC_SUCCESS', root / 'evidence')
            self.assertFalse(row['ok'])
            self.assertFalse(row['side_effects_unchanged'])

    def test_exact_recording_replaces_compact_console_without_duplication(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            fake = root / 'fake'
            fake.write_text('#!/usr/bin/env python3\nimport sys\nassert sys.argv[1:] == ["log", "20261115-135300000-123"]\nprint("FULL_RECORDING")\n')
            fake.chmod(0o700)
            text, receipt, ok = workload.full_output(fake,
                'FULL_RECORDING\nmore: justrust log 20261115-135300000-123\n', root,
                dict(os.environ), root / 'recorded.log', 5)
            self.assertTrue(ok)
            self.assertEqual(text, 'FULL_RECORDING\n')
            self.assertEqual(receipt['run_id'], '20261115-135300000-123')

    def test_ambiguous_recordings_never_choose_latest(self):
        text, receipt, ok = workload.full_output('/must/not/run',
            'justrust log 20261115-135300000-123\njustrust log 20261115-135300001-124',
            Path('/'), {}, Path('/must/not/write'), 1)
        self.assertFalse(ok)
        self.assertEqual(text, '')
        self.assertEqual(len(receipt['run_ids']), 2)

    def test_cleanup_does_not_claim_success_when_namespace_process_remains(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            fake = root / 'fake'
            fake.write_text('#!/usr/bin/env python3\nimport sys\nassert sys.argv[1:] == ["hybrid", "stop"]\n')
            fake.chmod(0o700)
            snapshot = {'process_pss_kib': {'123': 2}, 'unavailable_pids': [], 'pss_kib': 2}
            with patch.object(workload, 'namespace_pss', return_value=snapshot):
                row = workload.cleanup_namespace(fake, root,
                    dict(os.environ, JR_ENGINE_HOME=str(root / 'home')),
                    root / 'stop.log', 0.01)
            self.assertEqual(row['returncode'], 0)
            self.assertFalse(row['ok'])
            self.assertFalse(row['process_disappearance_verified'])

    def test_cleanup_verifies_owned_synthetic_process_disappears(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            env = dict(os.environ, JR_ENGINE_HOME=str(root / 'private-home'))
            child = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(10)'], env=env)
            reaper = threading.Thread(target=child.wait)
            reaper.start()
            try:
                fake = root / 'fake'
                fake.write_text(f'#!/usr/bin/env python3\nimport os,signal\nos.kill({child.pid}, signal.SIGTERM)\n')
                fake.chmod(0o700)
                row = workload.cleanup_namespace(fake, root, env, root / 'stop.log', 2)
                self.assertTrue(row['ok'], row)
                self.assertIn(str(child.pid), row['namespace_before']['process_pss_kib'])
                self.assertEqual(row['namespace_before']['process_nice'][str(child.pid)], os.getpriority(os.PRIO_PROCESS, 0))
                self.assertEqual(row['namespace_after']['process_pss_kib'], {})
            finally:
                if child.poll() is None:
                    child.kill()
                reaper.join(timeout=5)

    def test_prewarm_side_effect_rejected(self):
        status, report = self.run_matrix(['FAKE_BAD_PREWARM=1'])
        self.assertEqual(status, 1)
        self.assertFalse(next(row for row in report['rows'] if row['name'] == 'prewarm')['ok'])

    def test_timeout_raw_log_preserved(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            result = workload.run([sys.executable, '-c', 'import time; print("started", flush=True); time.sleep(3)'], root, dict(os.environ), root / 'timeout.log', 0.05)
            self.assertTrue(result['timeout'])
            self.assertNotEqual(result['returncode'], 0)
            self.assertIn('started', (root / 'timeout.log').read_text())

    def test_fixture_has_all_external_inputs(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            workload.fixture(root)
            files = workload.input_hashes(root)
            self.assertTrue({'build.rs', 'macro/src/lib.rs', 'dep/src/lib.rs', '.cargo/config.toml'} <= files.keys())


if __name__ == '__main__':
    unittest.main()
