#!/usr/bin/env python3
"""Mock-only tests. These are not native compiler/Cargo acceptance evidence."""
import contextlib
import io
import json
import os
from pathlib import Path
import re
import tempfile
import unittest
from unittest import mock

import stable_resident_cargo as harness


def response(status=0, hits=1, pid=1234):
    frontend = {key: 0 for key in harness.native.FRONTEND_COUNTERS + harness.native.FRONTEND_OPTIONAL}
    frontend.update(graph_hits=hits, archive_hits=hits)
    return dict(event='complete', key='stable-key', elapsed_ns=100,
                result=dict(id=1, status=status, pid=pid, frontend=frontend))


def cargo_record(nonce, error=False):
    if error:
        diagnostic = dict(level='error', message='JR_CARGO_DELIBERATE_ERROR', spans=[])
        stdout = json.dumps(dict(reason='compiler-message', message=diagnostic)) + '\n'
        stderr = 'error: could not compile fixture\n'
    else:
        stdout = ''.join(f'test {name} ... ok\n' for name in (*harness.dogfood.ORIGINAL_TESTS, harness.PROBE))
        stdout += 'test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out;\n'
        stderr = f'JR_CARGO_NONCE={nonce}\n'
    return dict(status=101 if error else 0, timed_out=False,
                stdout_b64=harness.native.encoded(stdout.encode()),
                stderr_b64=harness.native.encoded(stderr.encode()), elapsed_seconds=0.01)


class FakeCargo:
    def __init__(self):
        self.calls = []
        self.environments = []
        self.index = 0

    def __call__(self, command, timeout, cwd, env):
        self.calls.append(list(map(str, command)))
        self.environments.append(dict(env))
        self.index += 1
        lib = (cwd / 'src/lib.rs').read_text()
        nonce = re.search(r'fn edited_value\(\) -> &\x27static str \{ "([^"]+)"', lib).group(1)
        error = 'compile_error!' in lib
        fresh = self.index == 2
        if env.get('JUSTRUST_CARGO_RESIDENT') == 'serial' and not fresh and 'RUSTC_WRAPPER' not in env:
            with Path(env['JUSTRUST_CARGO_RESIDENT_TRACE']).open('a') as trace:
                trace.write(json.dumps(response(int(error), self.index)) + '\n')
        return cargo_record(nonce, error)


class CargoAcceptanceTests(unittest.TestCase):
    def rows(self):
        return [dict(name=name, expectation=expectation,
                     trace=[] if expectation == 'fresh' else [response(int(expectation == 'error'))])
                for name, _, _, _, expectation in harness.stages(harness.SOURCE.read_bytes(), 'nonce')]

    def test_stages_preserve_original_and_change_actual_bodies(self):
        original = harness.SOURCE.read_bytes()
        rows = harness.stages(original, 'nonce')
        self.assertEqual(rows[0][1], original)
        self.assertEqual(rows[-1][1], original)
        self.assertIn(b'Duration::from_millis(2)', rows[3][1])
        self.assertNotEqual(rows[0][2], rows[3][2])
        self.assertIn(b'nonce-edited', rows[3][2])
        self.assertIn(b'compile_error!', rows[4][2])
        self.assertEqual(rows[3][2], rows[5][2])

    def test_env_modes_share_flags_and_frontend(self):
        with mock.patch.dict(os.environ, {'RUSTC_WRAPPER': 'bad', 'CARGO_ENCODED_RUSTFLAGS': 'bad',
                                         'JUSTRUST_RUSTC_RESIDENT_FD': '42', 'MAKEFLAGS': 'bad'}):
            args = (Path('/compiler'), Path('/fixture'))
            off = harness.environment(*args, 'off', Path('/workers'), Path('/trace'))
            on = harness.environment(*args, 'on', Path('/workers'), Path('/trace'))
        for env in (off, on):
            self.assertEqual(env['RUSTFLAGS'], harness.FLAGS)
            self.assertIn('-Zunstable-options', env['RUSTFLAGS'].split())
            self.assertEqual(env['JUSTRUST_RUSTC_RESIDENT_FRONTEND'], '1')
            for key in ('RUSTC_WRAPPER', 'CARGO_ENCODED_RUSTFLAGS', 'JUSTRUST_RUSTC_RESIDENT_FD', 'MAKEFLAGS'):
                self.assertNotIn(key, env)
        self.assertEqual(off['JUSTRUST_CARGO_RESIDENT_DIR'], '/workers')
        self.assertEqual(on['JUSTRUST_CARGO_RESIDENT_DIR'], '/workers')
        self.assertEqual(off['JUSTRUST_CARGO_RESIDENT'], 'off')
        self.assertEqual(on['JUSTRUST_CARGO_RESIDENT'], 'serial')

    def test_socket_length_is_checked_before_any_compilation(self):
        harness.check_socket_path(Path('/short'), 1)
        with self.assertRaisesRegex(ValueError, 'too long'):
            harness.check_socket_path(Path('/') / ('x' * 90), 1)
        # Count encoded bytes, not Python Unicode codepoints.
        with self.assertRaisesRegex(ValueError, 'too long'):
            harness.check_socket_path(Path('/') / ('é' * 35), 1)

    def test_private_output_rejects_existing_and_symlink_parent(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            output = harness.private_directory(root / 'new')
            self.assertEqual(output.stat().st_mode & 0o777, 0o700)
            with self.assertRaises(FileExistsError):
                harness.private_directory(output)
            (root / 'link').symlink_to(output, target_is_directory=True)
            with self.assertRaises(ValueError):
                harness.private_directory(root / 'link/child')

    def test_same_worker_across_edit_error_recovery(self):
        result = harness.validate_residency(self.rows(), 'on')
        self.assertEqual(result['worker_pids'], {'stable-key': 1234})

    def test_reject_worker_restart_during_error(self):
        rows = self.rows()
        rows[4]['trace'][0]['result']['pid'] += 1
        with self.assertRaisesRegex(AssertionError, 'PID changed'):
            harness.validate_residency(rows, 'on')

    def test_reject_fresh_cache_as_residency(self):
        rows = self.rows()
        rows[3]['trace'] = []
        with self.assertRaisesRegex(AssertionError, 'No native'):
            harness.validate_residency(rows, 'on')
        rows = self.rows()
        rows[1]['trace'] = [response()]
        with self.assertRaisesRegex(AssertionError, 'fresh shortcut'):
            harness.validate_residency(rows, 'on')

    def test_off_must_not_complete_rpc(self):
        with self.assertRaisesRegex(AssertionError, 'Resident-off'):
            harness.validate_residency(self.rows(), 'off')

    def test_reject_key_change_and_missing_cache_hits(self):
        rows = self.rows()
        rows[4]['trace'][0]['key'] = 'new-worker-key'
        with self.assertRaisesRegex(AssertionError, 'single worker'):
            harness.validate_residency(rows, 'on')
        rows = self.rows()
        for row in rows:
            for event in row['trace']:
                event['result']['frontend']['archive_hits'] = 0
        with self.assertRaisesRegex(AssertionError, 'reuse'):
            harness.validate_residency(rows, 'on')

    def test_runtime_nonce_and_compiler_error_checked(self):
        harness.semantics(cargo_record('edited'), 'edited', 'pass')
        harness.semantics(cargo_record(None, True), None, 'error')
        with self.assertRaisesRegex(AssertionError, 'nonce'):
            harness.semantics(cargo_record('stale'), 'edited', 'pass')
        with self.assertRaisesRegex(AssertionError, 'compile_error'):
            harness.semantics(cargo_record('stale'), None, 'error')
        record = cargo_record('x')
        record['timed_out'] = True
        with self.assertRaisesRegex(AssertionError, 'timed out'):
            harness.semantics(record, 'x', 'pass')

    def test_mock_full_mode_ordinary_cargo_and_wrapper_fallback(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            fixture = root / 'fixture'
            (fixture / 'src').mkdir(parents=True)
            fake = FakeCargo()
            result = harness.capture_mode(Path('/candidate/rustc'), Path('/candidate/cargo'),
                                          fixture, root / 'on', 'on', harness.SOURCE.read_bytes(),
                                          'nonce', 5, runner=fake)
            self.assertEqual(result['verdict'], 'passed', result['failures'])
            self.assertEqual(len(fake.calls), 8)
            self.assertTrue(all(cmd[:3] == ['/candidate/cargo', 'test', '--offline'] for cmd in fake.calls))
            self.assertTrue(all('--extern' not in cmd for cmd in fake.calls))
            self.assertTrue(all(env == fake.environments[0] for env in fake.environments[:7]))
            self.assertIn('RUSTC_WRAPPER', fake.environments[-1])
            self.assertEqual(result['wrapper_fallback']['trace'], [])
            receipt = json.loads((root / 'on/receipt.json').read_text())
            self.assertIn('JR_CARGO_DELIBERATE_ERROR', harness.base64.b64decode(
                receipt['observations'][4]['cargo']['stdout_b64']).decode())
            self.assertEqual((root / 'on/00-source.rs').read_bytes(), harness.SOURCE.read_bytes())

    def test_failure_receipt_survives_launch_error(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            fixture = root / 'fixture'
            (fixture / 'src').mkdir(parents=True)
            with self.assertRaises(OSError):
                harness.capture_mode(Path('/rustc'), Path('/cargo'), fixture, root / 'off',
                                     'off', harness.SOURCE.read_bytes(), 'nonce', 5,
                                     runner=mock.Mock(side_effect=OSError('deliberate launch failure')))
            receipt = json.loads((root / 'off/receipt.json').read_text())
            self.assertEqual(receipt['verdict'], 'failed')
            self.assertIn('deliberate launch failure', receipt['error'])

    def test_mock_capture_matching_repetitions_and_exact_bytes_receipt(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            compiler, cargo = root / 'rustc', root / 'cargo'
            compiler.write_bytes(b'mock compiler, never executed')
            cargo.write_bytes(b'mock cargo, never executed')
            real_capture_mode = harness.capture_mode

            def capture_mode(*args):
                return real_capture_mode(*args, runner=FakeCargo())

            with mock.patch.object(harness.native, 'identity', return_value={'identity': 'frozen'}), \
                 mock.patch.object(harness.native, 'run', return_value=cargo_record('version')), \
                 mock.patch.object(harness, 'capture_mode', side_effect=capture_mode):
                result = harness.capture(compiler, cargo, root / 'output', repetitions=2)
            self.assertEqual(result['verdict'], 'passed', result['failures'])
            self.assertEqual(len(result['repetitions']), 2)
            self.assertEqual(harness.base64.b64decode(result['original_source']['bytes_b64']),
                             harness.SOURCE.read_bytes())
            for pair in result['repetitions']:
                self.assertEqual(len(pair['on']['observations']), len(pair['off']['observations']))
                for off, on in zip(pair['off']['observations'], pair['on']['observations']):
                    self.assertEqual(off['source_sha256'], on['source_sha256'])
                    self.assertEqual(off['wrapper_sha256'], on['wrapper_sha256'])
                    self.assertEqual(off['nonce'], on['nonce'])
            nonces = [pair['on']['observations'][0]['nonce'] for pair in result['repetitions']]
            self.assertEqual(len(set(nonces)), 2)

    def test_missing_telemetry_and_regressing_counters_fail(self):
        rows = self.rows()
        del rows[3]['trace'][0]['result']['frontend']['graph_hits']
        with self.assertRaisesRegex(AssertionError, 'counter'):
            harness.validate_residency(rows, 'on')
        rows = self.rows()
        rows[0]['trace'][0]['result']['frontend']['graph_hits'] = 5
        with self.assertRaisesRegex(AssertionError, 'decreased'):
            harness.validate_residency(rows, 'on')

    def test_cli_refuses_existing_output_without_upload(self):
        with tempfile.TemporaryDirectory() as temp:
            args = ['stable_resident_cargo.py', '--compiler', '/not-run/rustc',
                    '--cargo', '/not-run/cargo', '--output', temp]
            with mock.patch.object(harness.sys, 'argv', args), \
                 mock.patch.object(harness.dogfood, 'upload') as upload, \
                 contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
                harness.main()
            upload.assert_not_called()

    def test_trace_rejects_malformed_rows(self):
        with tempfile.TemporaryDirectory() as temp:
            trace = Path(temp) / 'trace'
            trace.write_text('[]\n')
            with self.assertRaises(AssertionError):
                harness.trace_events(trace)


if __name__ == '__main__':
    unittest.main()
