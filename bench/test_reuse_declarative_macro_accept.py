#!/usr/bin/env python3
"""Mock-only contract tests. Never execute rustc, a native build, or fixture binaries."""
import base64
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location('macro_accept', Path(__file__).with_name('reuse-declarative-macro-accept.py'))
H = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(H)


def record(status=0, stdout=b'', stderr=b''):
    return dict(command=['mock'], status=status,
                stdout_b64=base64.b64encode(stdout).decode(),
                stderr_b64=base64.b64encode(stderr).decode(), elapsed_seconds=0)


def result(fixture):
    value = dict(compile=record(1 if fixture['expected'] is None else 0, stderr=b'raw diagnostics\xff\r\n'))
    if fixture['expected'] is not None:
        value['runtime'] = record(stdout=fixture['expected'].encode())
    if fixture['dependency'] is not None:
        value['dependency'] = record()
    return value


class HarnessTests(unittest.TestCase):
    def test_matrix_contract(self):
        fixtures = {f['name']: f for f in H.cases()}
        required = ('definition-edit', 'input-edit', 'body-edit', 'nested-local-shadowing',
                    'hygiene', 'dollar-crate', 'exported-dependency-definition-edit',
                    'exported-dependency-body-edit', 'span-relocation', 'unused-macro-diagnostic',
                    'unused-rule-diagnostic', 'macro-tracing', 'expansion-failure',
                    'expansion-recovery', 'interpolation-opaque', 'edition-2021-expr',
                    'edition-2024-expr', 'edition-2018-pattern', 'edition-2021-pattern')
        self.assertTrue(set(required) <= fixtures.keys())
        self.assertEqual(len(fixtures), len(H.cases()))
        self.assertIsNone(fixtures['edition-2021-expr']['expected'])
        self.assertIsNotNone(fixtures['edition-2024-expr']['expected'])
        self.assertIn('(never) => { 99 }; ($e:expr)', H.BASE)
        self.assertIn('$crate::helper()', H.DEPENDENCY)
        self.assertIn('forward!(3)', H.BASE)
        self.assertEqual(H.cases()[-1]['source'], H.BASE)

    def test_schedule_persistence_and_freshness(self):
        first, second = H.schedule(0), H.schedule(1)
        self.assertEqual([v[1] for v in first[-3:]], [False, True, False])
        self.assertEqual({v[2] for v in first[-3:]}, {'transition'})
        self.assertEqual(first[2:], second[2:])
        self.assertTrue({v[2] for v in first[:2]}.isdisjoint(v[2] for v in second[:2]))

    def test_exact_bytes_status_and_no_normalization(self):
        original = record(stderr=b'/same/fixture.rs:7\r\n\xff')
        H.compare(original, dict(original, elapsed_seconds=99), 'equal')
        for changed in (record(1, stderr=b'/same/fixture.rs:7\r\n\xff'),
                        record(stderr=b'/other/fixture.rs:7\r\n\xff'),
                        record(stderr=b'/same/fixture.rs:7\n\xff'),
                        record(stdout=b'x', stderr=b'/same/fixture.rs:7\r\n\xff')):
            with self.assertRaisesRegex(AssertionError, 'exact status'):
                H.compare(original, changed, 'different')

    def test_expected_errors_reject_ice_and_false_success(self):
        H.validate_expected(dict(compile=record(1)), None, 'error')
        for status in (0, 101, -11):
            with self.assertRaises(AssertionError):
                H.validate_expected(dict(compile=record(status)), None, 'error')
        with self.assertRaises(AssertionError):
            H.validate_expected(dict(compile=record(), runtime=record(stdout=b'wrong')), 'right', 'runtime')

    def test_run_preserves_raw_bytes(self):
        completed = subprocess.CompletedProcess(['mock'], 1, b'\x00\xff', b'\r\n\xfe')
        with patch.object(H.subprocess, 'run', return_value=completed) as call:
            actual = H.run([Path('/mock/compiler'), '--arg'])
        self.assertEqual(base64.b64decode(actual['stderr_b64']), completed.stderr)
        self.assertEqual(base64.b64decode(actual['stdout_b64']), completed.stdout)
        call.assert_called_once_with(['/mock/compiler', '--arg'], capture_output=True, timeout=180)

    def test_commands_identical_paths_dependency_and_verify_ich(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            commands = []
            def fake_run(command):
                commands.append([str(v) for v in command])
                if '-o' in command:
                    Path(command[command.index('-o') + 1]).write_bytes(b'fake artifact')
                return record()
            fixture = H.case('dependency', dependency=H.DEPENDENCY, trace=True)
            with patch.object(H, 'run', side_effect=fake_run):
                for _, enabled, cache in H.schedule(0):
                    H.compile_case(Path('/mock/rustc'), root, fixture, enabled, cache)
            compiles = [c for c in commands if '-o' in c]
            self.assertEqual(len(compiles), 14)
            for c in compiles:
                self.assertIn('-Zincremental-verify-ich', c)
                self.assertIn('--edition=2024', c)
                self.assertTrue(any(v.startswith(H.FLAG + '=') for v in c))
            mains = [c for c in compiles if '--crate-name=declarative_macro_accept' in c]
            self.assertEqual({c[1] for c in mains}, {str(root / 'fixture.rs')})
            self.assertEqual({c[c.index('-o') + 1] for c in mains}, {str(root / 'fixture')})
            self.assertTrue(all('-Ztrace-macros=yes' in c for c in mains))
            self.assertEqual((root / 'fixture.rs').read_text(), fixture['source'])
            self.assertEqual((root / 'macro_dep.rs').read_text(), H.DEPENDENCY)
            self.assertEqual([c[c.index('-C') + 1] for c in mains[-3:]],
                             [f'incremental={root}/incremental/transition/binary'] * 3)

    def test_identity_hashes_compiler_and_driver(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            compiler = root / 'rustc'
            compiler.write_bytes(b'compiler')
            (root / 'lib').mkdir()
            driver = root / 'lib/librustc_driver-test.so'
            driver.write_bytes(b'driver')
            with patch.object(H, 'run', side_effect=[record(stdout=b'version\n'), record(stdout=str(root).encode() + b'\n')]):
                identity = H.identity(compiler)
            self.assertEqual(identity['sha256'], H.digest(compiler))
            self.assertEqual(identity['drivers'][0]['sha256'], H.digest(driver))

    def capture_mock(self, root, compile_fn=None, probe_records=None, trace_record=None, identities=None):
        identities = identities or [dict(sha256='compiler', drivers=['driver'])] * 2
        with patch.object(H, 'identity', side_effect=identities), \
             patch.object(H, 'probe', return_value=probe_records or {'no': record(), 'yes': record()}), \
             patch.object(H, 'run', return_value=trace_record or record(stderr=b'note: trace_macro\n')), \
             patch.object(H, 'compile_case', side_effect=compile_fn or (lambda r, p, f, e, c: result(f))):
            return H.capture(Path('/mock/rustc'), root)

    def test_full_matrix_mocked(self):
        with tempfile.TemporaryDirectory() as temp:
            seen = []
            def compile_fn(r, p, f, enabled, cache):
                seen.append((f['name'], enabled, cache))
                return result(f)
            report = self.capture_mock(Path(temp) / 'accept', compile_fn)
            self.assertEqual(report['verdict'], 'passed')
            self.assertEqual(report['completed_cases'], len(H.cases()))
            self.assertEqual(len(seen), 7 * len(H.cases()))
            self.assertEqual(report['identity_before'], report['identity_after'])
            self.assertEqual(json.loads((Path(temp) / 'accept/acceptance.json').read_text()), report)

    def test_differential_failure_persisted(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp) / 'accept'
            def corrupt(r, p, f, enabled, cache):
                value = result(f)
                if enabled:
                    value['compile'] = record(stderr=b'changed diagnostics')
                return value
            with self.assertRaisesRegex(AssertionError, 'exact status'):
                self.capture_mock(root, corrupt)
            report = json.loads((root / 'acceptance.json').read_text())
            self.assertEqual(report['verdict'], 'failed')
            self.assertIn('fresh-yes', report['observations'][0]['modes'])
            self.assertIn('identity_after', report)

    def test_unsupported_required_flag_is_not_pass(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp) / 'accept'
            with self.assertRaises(H.UnsupportedFeature):
                self.capture_mock(root, probe_records={'no': record(1, stderr=b'error: unknown unstable option: jr-cache-declarative-macros'), 'yes': record()})
            report = json.loads((root / 'acceptance.json').read_text())
            self.assertEqual(report['verdict'], 'unsupported')
            self.assertEqual(report['completed_cases'], 0)
            self.assertIn('feature_probe', report)

    def test_probe_crash_is_failure_not_unsupported(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp) / 'accept'
            with self.assertRaisesRegex(AssertionError, 'unexpectedly'):
                self.capture_mock(root, probe_records={'no': record(101), 'yes': record()})
            self.assertEqual(json.loads((root / 'acceptance.json').read_text())['verdict'], 'failed')

    def test_optional_trace_only_skips_explicit_unsupported(self):
        with tempfile.TemporaryDirectory() as temp:
            report = self.capture_mock(Path(temp) / 'accept', trace_record=record(1, stderr=b'error: unknown unstable option: trace-macros'))
            self.assertEqual(report['completed_cases'], len(H.cases()) - 1)
            skipped = [v for v in report['observations'] if 'unsupported' in v]
            self.assertEqual(len(skipped), 1)
            self.assertEqual(skipped[0]['fixture']['name'], 'macro-tracing')
        for trace in (record(101), record(1, stderr=b'other error'), record()):
            with tempfile.TemporaryDirectory() as temp, self.assertRaises(AssertionError):
                self.capture_mock(Path(temp) / 'accept', trace_record=trace)

    def test_compiler_or_driver_replacement_invalidates_report(self):
        for changed in (dict(sha256='other', drivers=['driver']), dict(sha256='compiler', drivers=['other'])):
            with tempfile.TemporaryDirectory() as temp:
                root = Path(temp) / 'accept'
                with self.assertRaisesRegex(AssertionError, 'identity changed'):
                    self.capture_mock(root, identities=[dict(sha256='compiler', drivers=['driver']), changed])
                self.assertEqual(json.loads((root / 'acceptance.json').read_text())['verdict'], 'failed')

    def test_existing_output_is_rejected(self):
        with tempfile.TemporaryDirectory() as temp:
            with self.assertRaises(FileExistsError):
                H.capture(Path('/never/executed'), Path(temp))


if __name__ == '__main__':
    unittest.main()
