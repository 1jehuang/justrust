"""Synthetic parser/contract tests, not native resident performance evidence."""
import base64
import copy
from pathlib import Path
import tempfile
from types import SimpleNamespace
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
    failed = int(fail)
    passed = len(statuses) - failed
    outcome = 'FAILED' if fail else 'ok'
    output = f'\nrunning {len(statuses)} tests\n'
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
        self.assertEqual(len(first), len(dogfood.ORIGINAL_TESTS) + 1)
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

    def mocked_capture(self, frontend=True, transform=None):
        """Exercise orchestration only, never launch a compiler or test executable."""
        created, nonces = [], []
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / 'source.rs'
            source.write_bytes(dogfood.DEFAULT_SOURCE.read_bytes())

            def compile_mock(cwd):
                text = (cwd / 'remote_sync_queue.rs').read_text()
                if 'deliberate type error' in text:
                    return compile_record(1, b'error: expected mutex\n')
                (cwd / 'tests').write_bytes(b'mock executable')
                return compile_record()

            class Worker:
                def __init__(self, compiler, cache, cwd, values, timeout, **options):
                    self.cwd, self.values, self.options = cwd, values, options
                    self.command, self.process = ['mock'], SimpleNamespace(pid=123)
                    self.counters = dict.fromkeys(native.COUNTERS, 0)
                    self.frontend_counters = dict.fromkeys(
                        native.FRONTEND_COUNTERS + native.FRONTEND_OPTIONAL, 0)
                    created.append(self)

                def compile(self, argv):
                    if self.options.get('frontend'):
                        self.frontend_counters = {key: value + 1
                                                  for key, value in self.frontend_counters.items()}
                    if transform:
                        transform(self)
                    return compile_mock(self.cwd)

                def close(self):
                    pass

            def run_mock(command, timeout, cwd, env):
                if Path(command[0]).name != 'tests':
                    self.assertNotIn('JUSTRUST_RUSTC_RESIDENT_FRONTEND', env)
                    self.assertNotIn('JUSTRUST_RUSTC_RESIDENT_VALUES', env)
                    return compile_mock(cwd)
                nonce = env[dogfood.NONCE_KEY]
                nonces.append(nonce)
                text = (cwd / 'remote_sync_queue.rs').read_text()
                return runtime_record(nonce=nonce,
                                      fail='foreground_waiters.load(Ordering::SeqCst), 1);' in text)

            with patch.object(native, 'identity', return_value={'mock': True}), \
                    patch.object(native, 'Resident', Worker), \
                    patch.object(native, 'run', side_effect=run_mock), \
                    patch.dict('os.environ', {'JUSTRUST_RUSTC_RESIDENT_FRONTEND': '1',
                                              'JUSTRUST_RUSTC_RESIDENT_VALUES': '1'}):
                report = dogfood.capture('/mock/rustc', source, root, repetitions=1,
                                         frontend=frontend)
            self.assertEqual(len(nonces), len(set(nonces)))
            return report, created

    def test_frontend_mock_capture_isolates_flags_and_preserves_recovery_cases(self):
        report, workers = self.mocked_capture()
        self.assertEqual(report['verdict'], 'passed')
        self.assertEqual([worker.values for worker in workers], [False, False])
        self.assertEqual([worker.options for worker in workers],
                         [{'frontend': False}, {'frontend': True}])
        self.assertIn('not a retained typed GlobalCtxt', report['retention_scope'])
        self.assertIn('encoded-frontend-archive', report['retention_verdict'])
        signature_source = dogfood.variants(dogfood.DEFAULT_SOURCE.read_text(), frontend=True)[-2][1]
        self.assertIn('prioritize_foreground: bool', signature_source)
        self.assertIn('if !prioritize_foreground ||', signature_source)
        self.assertNotIn('.background_lock()', signature_source)
        self.assertEqual(len(report['observations']), 10)
        rows = {row['name']: row for row in report['observations']}
        self.assertNotEqual(rows['signature-edit']['source_sha256'], rows['baseline']['source_sha256'])
        self.assertEqual(rows['signature-recovery']['source_sha256'], rows['baseline']['source_sha256'])
        self.assertEqual(set(report['sessions'][1]['frontend']),
                         set(native.FRONTEND_COUNTERS + native.FRONTEND_OPTIONAL))

    def test_cli_frontend_opt_in_defaults_false(self):
        for flag in ([], ['--frontend']):
            with self.subTest(flag=flag), tempfile.TemporaryDirectory() as tmp:
                argv = ['dogfood', '--compiler', '/mock/rustc', '--output', str(Path(tmp) / 'new'), *flag]
                result = dict(verdict='passed', retention_verdict='mock-only', source_unchanged=True)
                with patch('sys.argv', argv), patch.object(dogfood, 'capture', return_value=result) as capture, \
                        patch.object(dogfood, 'upload'), patch.object(dogfood.os, 'umask'), patch('builtins.print'):
                    dogfood.main()
                self.assertEqual(capture.call_args.kwargs, {'frontend': bool(flag)})

    def test_default_mock_capture_keeps_scalar_options_and_cases(self):
        report, workers = self.mocked_capture(frontend=False)
        self.assertEqual([worker.values for worker in workers], [False, True])
        self.assertEqual([worker.options for worker in workers], [{}, {}])
        self.assertEqual(len(report['observations']), 8)
        self.assertNotIn('retention_scope', report)
        self.assertEqual(report['retention_verdict'], 'no-live-hits-observed-no-retention-claim')

    def test_frontend_mock_capture_accepts_either_typeck_or_mir_loads(self):
        for absent in ('typeck_loads', 'mir_loads'):
            with self.subTest(absent=absent):
                def one_kind(worker):
                    worker.frontend_counters[absent] = 0
                report, _ = self.mocked_capture(transform=one_kind)
                self.assertEqual(report['verdict'], 'passed')

    def test_frontend_mock_capture_rejects_no_hits_missing_fields_and_off_leak(self):
        def no_typed_loads(worker):
            worker.frontend_counters['typeck_loads'] = 0
            worker.frontend_counters['mir_loads'] = 0

        def off_leak(worker):
            if not worker.options.get('frontend'):
                worker.frontend_counters['archive_hits'] = 1

        def scalar_leak(worker):
            if worker.options.get('frontend'):
                worker.counters['resident_hits'] = 1

        for key in ('graph_hits', 'archive_hits', 'value_loads'):
            with self.subTest(no_hits=key):
                def no_hits(worker, key=key):
                    if worker.options.get('frontend'):
                        worker.frontend_counters[key] = 0
                with self.assertRaises(AssertionError):
                    self.mocked_capture(transform=no_hits)
        for key in native.FRONTEND_COUNTERS + native.FRONTEND_OPTIONAL:
            with self.subTest(missing_field=key):
                def missing_field(worker, key=key):
                    if worker.options.get('frontend'):
                        worker.frontend_counters.pop(key, None)
                with self.assertRaises(AssertionError):
                    self.mocked_capture(transform=missing_field)
        def bool_counter(worker):
            if worker.options.get('frontend'):
                worker.frontend_counters['graph_hits'] = True

        for transform in (no_typed_loads, off_leak, scalar_leak, bool_counter):
            with self.subTest(transform=transform.__name__):
                with self.assertRaises(AssertionError):
                    self.mocked_capture(transform=transform)

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
