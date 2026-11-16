"""Synthetic parser/contract tests, not native resident performance evidence."""
import base64
import copy
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import stable_resident as native
import stable_resident_dogfood as dogfood


def compile_record(status=0, stderr=b''):
    return dict(status=status, stdout_b64='', stderr_b64=native.encoded(stderr),
                elapsed_seconds=.01, peak_rss_kib=1)


def runtime_record(nonce='new', fail=False, elapsed='0.32'):
    statuses = {name: 'ok' for name in (*dogfood.ORIGINAL_TESTS, dogfood.PROBE)}
    if fail:
        statuses[dogfood.ORIGINAL_TESTS[2]] = 'FAILED'
    passed, failed = (3, 1) if fail else (4, 0)
    outcome = 'FAILED' if fail else 'ok'
    output = '\nrunning 4 tests\n'
    output += ''.join(f'test {name} ... {status}\n' for name, status in sorted(statuses.items()))
    output += f'\ntest result: {outcome}. {passed} passed; {failed} failed; 0 ignored; 0 measured; 0 filtered out; finished in {elapsed}s\n'
    return dict(status=101 if fail else 0, stdout_b64=native.encoded(output.encode()),
                stderr_b64=native.encoded(f'JR_DOGFOOD_NONCE={nonce}\nsync_queue diagnostic: variable\n'.encode()),
                elapsed_seconds=.4, peak_rss_kib=1)


class DogfoodTests(unittest.TestCase):
    def test_reviewed_exact_shipping_source_edit_matrix(self):
        source = dogfood.DEFAULT_SOURCE.read_text()
        rows = dogfood.variants(source)
        self.assertEqual([r[0] for r in rows], ['baseline', 'unchanged', 'body-edit', 'revert',
                                              'assertion-failure', 'recovery', 'type-error', 'type-recovery'])
        for index in (0, 1, 3, 5, 7):
            self.assertEqual(rows[index][1], source)
        self.assertEqual(rows[2][1].replace('Duration::from_millis(2)', 'Duration::from_millis(1)'), source)
        self.assertEqual(rows[4][1].count('foreground_waiters.load(Ordering::SeqCst), 1);'), 1)
        self.assertEqual(rows[4][1].count('foreground_waiters.load(Ordering::SeqCst), 0);'), 1)
        self.assertIn('inner: "deliberate type error"', rows[6][1])
        with self.assertRaises(ValueError):
            dogfood.variants(source.replace('from_millis(1)', 'from_millis(3)'))

    def test_fixed_arguments_only_cache_root_differs(self):
        root = Path('/private/workload')
        first, second = [dogfood.arguments(root, Path('/cache') / mode) for mode in native.MODES[:2]]
        self.assertEqual([(a, b) for a, b in zip(first, second) if a != b],
                         [('incremental=/cache/fresh', 'incremental=/cache/resident-values-off')])
        self.assertIn('--test', first)
        self.assertIn('-Zincremental-verify-ich', first)
        self.assertIn('-Zcodegen-backend=llvm', first)
        self.assertIn('std::env::var', dogfood.WRAPPER)
        self.assertNotIn('env!(', dogfood.WRAPPER)

    def test_named_statuses_ignore_variable_timing_but_require_nonce(self):
        first = dogfood.statuses(runtime_record(elapsed='.31'), 'new', 'pass')
        second = dogfood.statuses(runtime_record(elapsed='99.2'), 'new', 'pass')
        self.assertEqual(first, second)
        self.assertEqual(len(first), 4)
        for nonce in ('stale', ''):
            with self.assertRaisesRegex(AssertionError, 'nonce'):
                dogfood.statuses(runtime_record(nonce=nonce), 'new', 'pass')

    def test_missing_duplicate_unknown_or_ignored_original_test_rejected(self):
        for transform in (
                lambda text: text.replace('test ' + dogfood.ORIGINAL_TESTS[0] + ' ... ok\n', ''),
                lambda text: text + 'test fresh_nonce_probe ... ok\n',
                lambda text: text.replace(dogfood.ORIGINAL_TESTS[0], 'unknown_test'),
                lambda text: text.replace('... ok', '... ignored', 1)):
            row = runtime_record()
            row['stdout_b64'] = native.encoded(transform(base64.b64decode(row['stdout_b64']).decode()).encode())
            with self.assertRaisesRegex(AssertionError, 'names/statuses'):
                dogfood.statuses(row, 'new', 'pass')

    def test_deliberate_assertion_failure_must_be_exact_test_and_exit(self):
        row = runtime_record(fail=True)
        actual = dogfood.statuses(row, 'new', 'test-fail')
        self.assertEqual(actual[dogfood.ORIGINAL_TESTS[2]], 'FAILED')
        with self.assertRaises(AssertionError):
            dogfood.statuses(row, 'new', 'pass')
        row['status'] = -11
        with self.assertRaisesRegex(AssertionError, 'exit'):
            dogfood.statuses(row, 'new', 'test-fail')

    def test_exact_compile_diagnostics_and_type_error_no_stale_runtime(self):
        modes = {mode: dict(compile=compile_record(1, b'error: expected mutex\n')) for mode in native.MODES}
        dogfood.validate_pair(modes, 'compile-fail')
        modes['resident-values-on']['compile']['stderr_b64'] = native.encoded(b'different')
        with self.assertRaisesRegex(AssertionError, 'Exact compiler'):
            dogfood.validate_pair(modes, 'compile-fail')
        modes['resident-values-on'] = copy.deepcopy(modes['fresh'])
        modes['resident-values-on']['runtime'] = runtime_record()
        with self.assertRaisesRegex(AssertionError, 'stale'):
            dogfood.validate_pair(modes, 'compile-fail')

    def test_fresh_reference_independently_validated(self):
        modes = {mode: dict(compile=compile_record(), runtime=runtime_record(), nonce='new')
                 for mode in native.MODES}
        dogfood.validate_pair(modes, 'pass')
        for row in modes.values():
            row['runtime']['status'] = 7
        with self.assertRaisesRegex(AssertionError, 'exit'):
            dogfood.validate_pair(modes, 'pass')

    def test_failure_keeps_receipt_and_repository_source_intact(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / 'source.rs'
            source.write_bytes(dogfood.DEFAULT_SOURCE.read_bytes())
            before = native.digest(source)
            with patch.object(native, 'identity', side_effect=RuntimeError('compiler unavailable')):
                with self.assertRaisesRegex(RuntimeError, 'unavailable'):
                    dogfood.capture('/missing/rustc', source, root)
            import json
            report = json.loads((root / 'receipt.json').read_text())
            self.assertEqual(report['verdict'], 'failed')
            self.assertTrue(report['source_unchanged'])
            self.assertEqual(before, native.digest(source))
            self.assertIn('compiler unavailable', report['error'])


if __name__ == '__main__':
    unittest.main()
