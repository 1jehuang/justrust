"""Protocol fakes here are unit tests only, never compiler/performance evidence."""
import base64
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

import stable_resident as bench


def record(status=0, stdout=b'', stderr=b''):
    return dict(status=status, stdout_b64=bench.encoded(stdout), stderr_b64=bench.encoded(stderr),
                elapsed_seconds=.01, peak_rss_kib=1)


class StableResidentTests(unittest.TestCase):
    def test_edit_matrix(self):
        rows = bench.cases(3)
        self.assertEqual([r[0] for r in rows], ['warm', 'unchanged', 'body-edit',
                                              'signature-edit', 'type-error', 'recovery'])
        self.assertEqual(rows[0][1], rows[1][1])
        self.assertEqual(rows[0][1], rows[-1][1])
        self.assertEqual(rows[0][2], '13\n')
        self.assertEqual(rows[2][2], '14\n')
        self.assertIn('value() -> u32', rows[3][1])
        self.assertIsNone(rows[4][2])
        self.assertIn('&dyn Number', rows[0][1])
        self.assertIn('#[track_caller]', rows[0][1])

    def test_arguments_only_cache_differs(self):
        root = Path('/private')
        fresh = bench.arguments(root, root / 'fresh')
        resident = bench.arguments(root, root / 'resident')
        self.assertEqual([(a, b) for a, b in zip(fresh, resident) if a != b],
                         [('incremental=/private/fresh', 'incremental=/private/resident')])
        self.assertIn('-Zcodegen-backend=llvm', fresh)
        self.assertIn('-Zincremental-verify-ich', fresh)

    def test_environment_is_fixed_and_fresh_strips_protocol(self):
        with patch.dict(os.environ, JUSTRUST_RUSTC_RESIDENT_FD='999', JUSTRUST_RUSTC_RESIDENT_VALUES='1'):
            self.assertNotIn('JUSTRUST_RUSTC_RESIDENT_FD', bench.compiler_env())
            self.assertNotIn('JUSTRUST_RUSTC_RESIDENT_VALUES', bench.compiler_env())
            self.assertEqual(bench.compiler_env(False)['JUSTRUST_RUSTC_RESIDENT_VALUES'], '0')
            self.assertEqual(bench.compiler_env(True)['JUSTRUST_RUSTC_RESIDENT_VALUES'], '1')

    def test_exact_diagnostics_and_error_recovery_contract(self):
        pair = {mode: dict(compile=record(1, stderr=b'type error\n')) for mode in bench.MODES}
        bench.validate_pair(pair, None)
        pair['resident-values-on']['compile']['stderr_b64'] = bench.encoded(b'different\n')
        with self.assertRaisesRegex(AssertionError, 'Exact'):
            bench.validate_pair(pair, None)
        for row in pair.values():
            row['compile'] = record(101)
        with self.assertRaisesRegex(AssertionError, 'ordinary'):
            bench.validate_pair(pair, None)

    def test_runtime_must_match_expected_not_just_other_mode(self):
        pair = {mode: dict(compile=record(), runtime=record(stdout=b'wrong')) for mode in bench.MODES}
        with self.assertRaisesRegex(AssertionError, 'runtime'):
            bench.validate_pair(pair, '13\n')

    def test_real_child_capture_and_memory(self):
        row = bench.run([sys.executable, '-c', 'import sys; print("ok"); sys.stderr.write("bad"); sys.exit(1)'])
        self.assertEqual(row['status'], 1)
        self.assertEqual(base64.b64decode(row['stdout_b64']), b'ok\n')
        self.assertEqual(base64.b64decode(row['stderr_b64']), b'bad')
        self.assertGreater(row['peak_rss_kib'], 0)
        self.assertGreater(bench.memory(os.getpid())['rss_kib'], 0)

    def test_fresh_timeout_preserves_partial_output(self):
        row = bench.run([sys.executable, '-c',
                         'import time; print("partial", flush=True); time.sleep(5)'], timeout=.1)
        self.assertTrue(row['timed_out'])
        self.assertLess(row['status'], 0)
        self.assertEqual(base64.b64decode(row['stdout_b64']), b'partial\n')

    def server(self, root, mutate=''):
        compiler = root / 'protocol_fake'
        compiler.write_text(f'''#!{sys.executable}
import json, os, sys
pipe = os.fdopen(int(os.environ['JUSTRUST_RUSTC_RESIDENT_FD']), 'w')
for line in sys.stdin:
    request = json.loads(line)
    assert set(request) == {{'id', 'args'}}
    bad = open(request['args'][0]).read() == 'bad'
    response = dict(id=request['id'], pid=os.getpid(), status=1 if bad else 0,
                    elapsed_seconds=0.001, resident_hits=request['id'],
                    resident_misses=request['id'], resident_entries=request['id'])
    print('out-' + str(request['id']), flush=True)
    print('err-' + str(request['id']), file=sys.stderr, flush=True)
    {mutate or 'pass'}
    print(json.dumps(response), file=pipe, flush=True)
''')
        compiler.chmod(0o700)
        return compiler

    def test_one_actual_server_process_survives_error_and_slices_streams(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / 'source'
            source.write_text('ok')
            server = bench.Resident(self.server(root), root, root, True, timeout=2)
            try:
                first = server.compile([source])
                source.write_text('bad')
                error = server.compile([source])
                source.write_text('ok')
                recovery = server.compile([source])
                self.assertEqual([first['status'], error['status'], recovery['status']], [0, 1, 0])
                self.assertEqual({r['pid'] for r in (first, error, recovery)}, {server.process.pid})
                self.assertEqual(base64.b64decode(recovery['stdout_b64']), b'out-3\n')
                self.assertEqual(base64.b64decode(error['stderr_b64']), b'err-2\n')
                self.assertEqual(recovery['worker_elapsed_seconds'], .001)
                self.assertGreater(recovery['memory_after']['rss_kib'], 0)
                with self.assertRaisesRegex(ValueError, 'fixed'):
                    server.compile(['different'])
            finally:
                server.close()
            self.assertEqual(len((root / 'resident.jsonl').read_text().splitlines()), 6)

    def test_invalid_responses_are_rejected(self):
        for mutation, error in (("response['id'] += 1", 'id/PID'),
                                ("response['pid'] += 1", 'id/PID'),
                                ("response['resident_hits'] = -1", 'counters')):
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                source = root / 'source'
                source.write_text('ok')
                server = bench.Resident(self.server(root, mutation), root, root, True, timeout=2)
                try:
                    with self.assertRaisesRegex(AssertionError, error):
                        server.compile([source])
                finally:
                    server.close()

    def test_live_entries_can_decrease_after_eviction(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / 'source'
            source.write_text('ok')
            server = bench.Resident(self.server(root, "response['resident_entries'] = 10 - request['id']"),
                                    root, root, True, timeout=2)
            try:
                self.assertEqual(server.compile([source])['resident_entries'], 9)
                self.assertEqual(server.compile([source])['resident_entries'], 8)
            finally:
                server.close()

    def test_protocol_eof_is_not_silently_restarted(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / 'source'
            source.write_text('ok')
            server = bench.Resident(self.server(root, 'sys.exit(2)'), root, root, True, timeout=2)
            with self.assertRaisesRegex(RuntimeError, 'response FD'):
                server.compile([source])
            with self.assertRaisesRegex(RuntimeError, 'shutdown status 2'):
                server.close()

    def test_failure_receipt_is_preserved(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            with patch.object(bench, 'identity', side_effect=RuntimeError('missing compiler')):
                with self.assertRaisesRegex(RuntimeError, 'missing compiler'):
                    bench.capture('/missing', root)
            receipt = json.loads((root / 'receipt.json').read_text())
            self.assertEqual(receipt['verdict'], 'failed')
            self.assertIn('missing compiler', receipt['error'])
            self.assertNotIn('summary', receipt)

    def capture_fake(self, root, identities, hits=True):
        class FakeResident:
            def __init__(self, compiler, directory, cwd, values, timeout):
                self.command = ['unit-test-only']
                self.process = type('Process', (), {'pid': 123})()
                self.counters = {'resident_hits': int(values and hits)}

            def compile(self, argv):
                return fake_run(['/compiler', *argv])

            def close(self):
                pass

        def fake_run(command, timeout=180, **kwargs):
            if len(command) == 1:
                source = (Path(command[0]).parent / 'fixture.rs').read_text()
                return record(stdout=b'14\n' if 'u64 { 8 }' in source else b'13\n')
            source = Path(command[1]).read_text()
            if '"wrong"' in source:
                return record(1, stderr=b'type error')
            Path(command[-1]).write_bytes(b'fake artifact, not evidence')
            return record()

        with patch.object(bench, 'Resident', FakeResident), patch.object(bench, 'run', fake_run), \
                patch.object(bench, 'identity', side_effect=identities):
            return bench.capture('/compiler', root, repetitions=3, items=3)

    def test_capture_full_sequence_rotates_order_and_checks_identity(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            with self.assertRaisesRegex(AssertionError, 'identity changed'):
                self.capture_fake(root, [{'sha': 'a'}, {'sha': 'b'}])
            receipt = json.loads((root / 'receipt.json').read_text())
            self.assertEqual(receipt['verdict'], 'failed')
            self.assertEqual(len(receipt['observations']), 18)
            for rep in range(3):
                self.assertEqual(receipt['observations'][rep * 6]['order'][0], bench.MODES[rep])
            self.assertEqual(receipt['observations'][-1]['name'], 'recovery')

    def test_success_and_no_hit_rejection(self):
        with tempfile.TemporaryDirectory() as tmp:
            report = self.capture_fake(Path(tmp), [{'sha': 'a'}, {'sha': 'a'}])
            self.assertEqual(report['verdict'], 'passed')
            self.assertEqual(set(report['summary']['body-edit']), set(bench.MODES))
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaisesRegex(AssertionError, 'no resident query hits'):
                self.capture_fake(Path(tmp), [{'sha': 'a'}, {'sha': 'a'}], hits=False)


if __name__ == '__main__':
    unittest.main()
