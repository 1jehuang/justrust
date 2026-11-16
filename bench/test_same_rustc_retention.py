"""Regression tests for the isolated frozen-compiler experiment."""
import base64
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

import same_rustc_retention as bench


def record(status=0, stdout=b'', stderr=b''):
    return dict(status=status, stdout_b64=base64.b64encode(stdout).decode(),
                stderr_b64=base64.b64encode(stderr).decode(), elapsed_seconds=0.1,
                peak_rss_kib=100)


class RetentionTests(unittest.TestCase):
    def test_only_retention_flags_and_cache_differ(self):
        root = Path('/private')
        off = list(map(str, bench.command('/rustc', root, root / 'off', 'off')))
        on = list(map(str, bench.command('/rustc', root, root / 'on', 'on')))
        differences = [(a, b) for a, b in zip(off, on) if a != b]
        self.assertEqual(len(differences), 3)
        self.assertIn('-Zcodegen-backend=llvm', off)
        self.assertIn('-Zincremental-verify-ich', off)
        for flag in bench.FLAGS:
            self.assertIn((flag + '=no', flag + '=yes'), differences)

    def test_revision_matrix(self):
        cases = bench.cases(32)
        self.assertEqual([c[0] for c in cases], ['warm', 'unchanged-forced-recompile',
                         'body-edit', 'revert', 'type-error', 'recovery'])
        self.assertEqual(cases[0][1], cases[1][1])
        self.assertEqual(cases[0][1], cases[-1][1])
        self.assertEqual(cases[0][2], '1095\n')
        self.assertIn('"wrong"', cases[4][1])
        self.assertIn('impl Dispatch', cases[0][1])
        self.assertEqual(cases[0][1].replace('body() -> u64 { 7 }', 'body() -> u64 { 8 }'), cases[2][1])
        self.assertEqual(cases[0][1].count('constant() -> u64 { 7 }'), 1)
        self.assertEqual(cases[2][1].count('constant() -> u64 { 7 }'), 1)

    def test_exact_diagnostic_equivalence(self):
        pair = {m: dict(compile=record(1, stderr=b'error\n')) for m in ('off', 'on')}
        bench.validate_pair(pair, None)
        pair['on']['compile']['stderr_b64'] = base64.b64encode(b'error \n').decode()
        with self.assertRaisesRegex(AssertionError, 'Exact'):
            bench.validate_pair(pair, None)

    def test_reject_ice_and_bad_runtime(self):
        pair = {m: dict(compile=record(101)) for m in ('off', 'on')}
        with self.assertRaisesRegex(AssertionError, 'ordinary'):
            bench.validate_pair(pair, None)
        pair = {m: dict(compile=record(), runtime=record(stdout=b'wrong')) for m in ('off', 'on')}
        with self.assertRaisesRegex(AssertionError, 'runtime'):
            bench.validate_pair(pair, 'correct\n')

    def test_wait4_status_output_and_rss(self):
        result = bench.run([sys.executable, '-c', 'import sys; print("hello"); sys.stderr.write("bad\\n"); sys.exit(1)'], Path('unused'))
        self.assertEqual(result['status'], 1)
        self.assertEqual(base64.b64decode(result['stdout_b64']), b'hello\n')
        self.assertEqual(base64.b64decode(result['stderr_b64']), b'bad\n')
        self.assertGreater(result['peak_rss_kib'], 0)

    def test_cache_sizes(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / 'one').write_bytes(b'123')
            size = bench.cache_size(root)
            self.assertEqual(size['logical_bytes'], 3)
            self.assertEqual(size['files'], 1)
            self.assertGreaterEqual(size['allocated_bytes'], 3)

    def test_capture_rotates_and_preserves_failure_identity(self):
        def fake_run(command, rss_file=None):
            if len(command) == 1:
                return record(stdout=b'10\n')
            root = Path(command[1]).parent
            source = (root / 'fixture.rs').read_text()
            if '"wrong"' in source:
                return record(1, stderr=b'type error\n')
            (root / 'fixture').write_bytes(b'executable')
            return record()

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            with patch.object(bench, 'identity', side_effect=[{'hash': 'a'}, {'hash': 'b'}]), \
                    patch.object(bench, 'run', side_effect=fake_run), \
                    patch.object(bench, 'cases', return_value=[('warm', 'fn main() {}', '10\n')]):
                with self.assertRaisesRegex(AssertionError, 'identity changed'):
                    bench.capture(Path('/rustc'), root, repetitions=2, items=1)
            receipt = json.loads((root / (root.name + '.json')).read_text())
            self.assertEqual(receipt['verdict'], 'failed')
            self.assertEqual([r['order'] for r in receipt['observations']], [['off', 'on'], ['on', 'off']])
            self.assertEqual(len(receipt['observations']), 2)

    def test_compile_failure_receipt_survives(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            with patch.object(bench, 'identity', return_value={'hash': 'a'}), \
                    patch.object(bench, 'run', return_value=record(101, stderr=b'ICE')):
                with self.assertRaises(AssertionError):
                    bench.capture(Path('/rustc'), root, repetitions=2, items=1)
            receipt = json.loads((root / (root.name + '.json')).read_text())
            self.assertEqual(receipt['verdict'], 'failed')
            self.assertIn('Unexpected compile failure', receipt['error'])
            self.assertEqual(receipt['identity_before'], receipt['identity_after'])


if __name__ == '__main__':
    unittest.main()
