#!/usr/bin/env python3
"""Mock-only contract tests. Never execute rustc, a native build, or fixture binaries."""
import base64
import importlib.util
import json
from pathlib import Path
import subprocess
import struct
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
        for name in ('edition-2021-expr', 'edition-2024-expr'):
            self.assertIn('($e) + private', fixtures[name]['source'])
            self.assertIn('value!(const { 2 })', fixtures[name]['source'])
        self.assertIn('(never) => { 99 }; ($e:expr)', H.BASE)
        self.assertIn('$crate::helper()', H.DEPENDENCY)
        self.assertIn('forward!(3)', H.BASE)
        self.assertEqual(H.cases()[-1]['source'], H.BASE)
        self.assertEqual(fixtures['literal-eligible']['source'], H.LITERAL)
        self.assertNotIn('$', H.LITERAL)
        self.assertIn('() => { 42 }', H.LITERAL)
        self.assertEqual(fixtures['literal-eligible-body-edit']['expected'], '')

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

    def capture_mock(self, root, compile_fn=None, probe_records=None, trace_record=None, identities=None,
                     require_native_hit=False, evidence_fn=None):
        identities = identities or [dict(sha256='compiler', drivers=['driver'])] * 2
        with patch.object(H, 'identity', side_effect=identities), \
             patch.object(H, 'probe', return_value=probe_records or {'no': record(), 'yes': record()}), \
             patch.object(H, 'run', return_value=trace_record or record(stderr=b'note: trace_macro\n')), \
             patch.object(H, 'compile_case', side_effect=compile_fn or (lambda r, p, f, e, c: result(f))), \
             patch.object(H, 'native_evidence', side_effect=evidence_fn):
            return H.capture(Path('/mock/rustc'), root, require_native_hit=require_native_hit)

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

    def test_marker_counts_never_guess_unresolved_virtual_labels(self):
        records = [dict(label_resolved=True, label=label, kind='GenericActivity', record_type='interval')
                   for label in H.MARKERS]
        records += [dict(label_resolved=False, label=None, kind='Query', record_type='interval'),
                    dict(label_resolved=True, label='irrelevant', kind='GenericActivity', record_type='interval')]
        counted = H.marker_counts(records)
        self.assertEqual(counted['markers'], dict.fromkeys(H.MARKERS, 1))
        self.assertEqual(counted['unresolved_records'], 1)
        with self.assertRaises(AssertionError):
            H.marker_counts([dict(label_resolved=True, label=H.MARKERS[0], kind='Query', record_type='interval')])

    def positive_observations(self):
        return [dict(name=name, profile=dict(markers=dict(zip(H.MARKERS, counts))))
                for name, counts in [('off-before', [0, 0, 0, 0, 0]),
                                     ('cold-yes', [1, 0, 1, 1, 1]),
                                     ('warm-yes', [1, 0, 1, 0, 1]),
                                     ('body-edit-yes', [1, 0, 1, 0, 1]),
                                     ('off-after', [0, 0, 0, 0, 0])]]

    def test_positive_native_proof_requires_cold_provider_then_warm_avoidance(self):
        proof = H.validate_native_markers(self.positive_observations())
        self.assertEqual(proof['inferred_provider_avoided'], {'warm-yes': 1, 'body-edit-yes': 1})
        for name, counts in [('cold-yes', [0, 0, 0, 0, 0]),
                             ('cold-yes', [1, 0, 1, 0, 1]),
                             ('warm-yes', [0, 0, 0, 0, 0]),
                             ('warm-yes', [1, 0, 1, 1, 1]),
                             ('body-edit-yes', [1, 0, 1, 1, 1]),
                             ('body-edit-yes', [1, 1, 1, 0, 1]),
                             ('off-after', [1, 0, 1, 0, 1])]:
            observations = self.positive_observations()
            next(o for o in observations if o['name'] == name)['profile']['markers'] = dict(zip(H.MARKERS, counts))
            with self.subTest(name=name, counts=counts), self.assertRaises(AssertionError):
                H.validate_native_markers(observations)

    def test_evidence_profiles_must_exist_unambiguously(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            with self.assertRaisesRegex(AssertionError, 'exactly one'):
                H.profile_evidence(root)
            for name in ('one', 'two'):
                (root / f'{name}.mm_profdata').write_bytes(b'not a native execution')
            with self.assertRaisesRegex(AssertionError, 'exactly one'):
                H.profile_evidence(root)

    def test_profile_parser_synthetic_capture_keeps_unknown_query_ids_unknown(self):
        # Pure synthetic measureme v9 bytes, not a compiler invocation.
        def header(magic):
            return magic + struct.pack('<I', 9)
        strings = bytearray()
        def add_string(value):
            offset = 8 + len(strings)
            strings.extend(value.encode() + b'\xff')
            return 100000003 + offset
        kind = add_string('GenericActivity')
        query = add_string('Query')
        label = add_string(H.MARKERS[0])
        metadata = add_string(json.dumps(dict(process_id=7, start_time=12, cmd='synthetic compiler command')))
        events = struct.pack('<QQIIII', kind, label, 0, 1, 2, 0)
        events += struct.pack('<QQIIII', query, 42, 0, 2, 3, 0)
        streams = [header(b'MMES') + events, header(b'MMSD') + strings,
                   header(b'MMSI') + struct.pack('<QQ', 100000001, metadata - 100000003)]
        data = header(b'MMPD')
        for tag, stream in enumerate(streams):
            data += struct.pack('<BI', tag, len(stream)) + stream
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            path = root / 'synthetic.mm_profdata'
            path.write_bytes(data)
            evidence = H.profile_evidence(root)
            self.assertEqual(evidence['markers'][H.MARKERS[0]], 1)
            self.assertEqual(evidence['unresolved_records'], 1)
            self.assertEqual(evidence['records'], 2)
            self.assertEqual(evidence['sha256'], H.digest(path))
            self.assertEqual(evidence['metadata']['cmd'], 'synthetic compiler command')
            path.write_bytes(b'corrupt')
            with self.assertRaises(ValueError):
                H.profile_evidence(root)

    def test_native_evidence_commands_and_results_are_separate(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            seen = []
            def compile_fn(r, p, f, enabled, cache, profile=None):
                seen.append((f, enabled, cache, profile))
                return result(f)
            profiles = [o['profile'] for o in self.positive_observations()]
            evidence = dict(observations=[])
            with patch.object(H, 'compile_case', side_effect=compile_fn), \
                 patch.object(H, 'profile_evidence', side_effect=profiles):
                H.native_evidence(Path('/mock/rustc'), root, evidence)
            self.assertEqual(len(seen), 5)
            self.assertEqual([v[1] for v in seen], [False, True, True, True, False])
            self.assertEqual(len({v[2] for v in seen[1:]}), 1)
            self.assertEqual(len({v[3] for v in seen}), 5)
            self.assertEqual(seen[1][0]['source'], seen[2][0]['source'])
            self.assertNotEqual(seen[2][0]['source'], seen[3][0]['source'])
            self.assertIn('proof', evidence)

    def test_semantic_pass_is_not_native_hit_proof(self):
        with tempfile.TemporaryDirectory() as temp:
            report = self.capture_mock(Path(temp) / 'semantics')
            self.assertEqual(report['semantic_verdict'], 'passed')
            self.assertEqual(report['native_hit_verdict'], 'not-requested')
            def failed_evidence(*args):
                raise AssertionError('no native hit')
            root = Path(temp) / 'native'
            with self.assertRaisesRegex(AssertionError, 'no native hit'):
                self.capture_mock(root, require_native_hit=True, evidence_fn=failed_evidence)
            report = json.loads((root / 'acceptance.json').read_text())
            self.assertEqual(report['semantic_verdict'], 'passed')
            self.assertEqual(report['native_hit_verdict'], 'failed')
            self.assertEqual(report['verdict'], 'failed')
            report = self.capture_mock(Path(temp) / 'positive', require_native_hit=True)
            self.assertEqual(report['native_hit_verdict'], 'passed')

    def test_profile_flags_are_explicit_and_all_native_cases_verify_ich(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            commands = []
            def fake_run(command):
                commands.append([str(v) for v in command])
                if '-o' in command:
                    Path(command[command.index('-o') + 1]).write_bytes(b'mock')
                return record()
            with patch.object(H, 'run', side_effect=fake_run):
                H.compile_case(Path('/mock/rustc'), root, H.case('literal', H.LITERAL, ''),
                               True, 'proof/cache', profile=root / 'profiles/cold')
            command = commands[0]
            self.assertIn('-Zself-profile-events=default,function-args', command)
            self.assertIn(f'-Zself-profile={root}/profiles/cold', command)
            self.assertIn('-Zincremental-verify-ich', command)

    def test_existing_output_is_rejected(self):
        with tempfile.TemporaryDirectory() as temp:
            with self.assertRaises(FileExistsError):
                H.capture(Path('/never/executed'), Path(temp))

    def test_tt_matrix_preserves_literal_matrix(self):
        self.assertEqual(len(H.cases()), 32)
        fixtures = {f['name']: f for f in H.tt_cases()}
        self.assertEqual(len(fixtures), 19)
        self.assertIn('$($t:tt)*', H.TT)
        self.assertNotIn('literal!', H.TT)
        self.assertNotIn(':expr', H.TT)
        self.assertIn('tokens!(40 + 2)', H.TT)
        self.assertIsNone(fixtures['tt-count-mismatch']['expected'])
        self.assertEqual(fixtures['tt-count-recovery']['expected'], '')
        self.assertIn('tokens!($e)', fixtures['tt-expr-interpolation-fallback']['source'])
        self.assertIn('!= 9', fixtures['tt-expr-interpolation-fallback']['source'])

    def test_tt_proof_independent_of_literal_success(self):
        with tempfile.TemporaryDirectory() as temp:
            seen = []
            def compile_fn(r, p, f, enabled, cache, profile=None):
                seen.append((f, enabled, cache, profile))
                return result(f)
            evidence = dict(observations=[])
            profiles = [o['profile'] for o in self.positive_observations()]
            with patch.object(H, 'compile_case', side_effect=compile_fn), \
                 patch.object(H, 'profile_evidence', side_effect=profiles):
                H.native_evidence(Path('/mock/rustc'), Path(temp), evidence, tt=True)
            self.assertEqual(len(seen), 5)
            self.assertEqual([v[1] for v in seen], [False, True, True, True, False])
            self.assertEqual(seen[1][0]['source'], H.TT)
            self.assertEqual(seen[1][0], seen[2][0])
            self.assertNotEqual(seen[2][0]['source'], seen[3][0]['source'])
            self.assertEqual({v[2] for v in seen[1:]}, {'tt-proof/shared'})
            self.assertTrue(all('tt-profiles' in str(v[3]) for v in seen))
            self.assertEqual(evidence['marker_schema'], list(H.MARKERS))
            self.assertEqual(evidence['proof']['inferred_provider_avoided'],
                             {'warm-yes': 1, 'body-edit-yes': 1})

    def test_literal_only_compiler_cannot_pass_tt_gate(self):
        # Simulates literal-positive compiler with TT rejected. No compiler runs.
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp) / 'accept'
            literal_profiles = [o['profile'] for o in self.positive_observations()]
            rejected = dict(markers=dict(zip(H.MARKERS, [0, 1, 0, 0, 0])))
            tt_profiles = [dict(markers=dict.fromkeys(H.MARKERS, 0)), rejected,
                           rejected, rejected, dict(markers=dict.fromkeys(H.MARKERS, 0))]
            with patch.object(H, 'identity', return_value={'mock': True}), \
                 patch.object(H, 'probe', return_value={'no': record(), 'yes': record()}), \
                 patch.object(H, 'run', return_value=record(stderr=b'note: trace_macro\n')), \
                 patch.object(H, 'compile_case', side_effect=lambda r, p, f, e, c, profile=None: result(f)), \
                 patch.object(H, 'profile_evidence', side_effect=literal_profiles + tt_profiles), \
                 self.assertRaisesRegex(AssertionError, 'Cold'):
                H.capture(Path('/mock/rustc'), root, True, True)
            report = json.loads((root / 'acceptance.json').read_text())
            self.assertEqual(report['semantic_verdict'], 'passed')
            self.assertEqual(report['native_hit_verdict'], 'passed')
            self.assertEqual(report['tt_hit_verdict'], 'failed')
            self.assertEqual(report['verdict'], 'failed')
            self.assertEqual(report['completed_cases'], 51)

    def test_tt_gate_rejects_absent_success_and_off_markers(self):
        for name, counts in [('cold-yes', [0, 0, 0, 0, 0]),
                             ('cold-yes', [1, 0, 1, 1, 0]),
                             ('warm-yes', [1, 0, 1, 0, 0]),
                             ('off-before', [1, 0, 1, 1, 1])]:
            observations = self.positive_observations()
            next(o for o in observations if o['name'] == name)['profile']['markers'] = dict(zip(H.MARKERS, counts))
            with self.subTest(name=name), self.assertRaises(AssertionError):
                H.validate_native_markers(observations)

    def test_tt_only_option_runs_full_semantics_and_independent_proof(self):
        with tempfile.TemporaryDirectory() as temp:
            profiles = [o['profile'] for o in self.positive_observations()]
            with patch.object(H, 'identity', return_value={'mock': True}), \
                 patch.object(H, 'probe', return_value={'no': record(), 'yes': record()}), \
                 patch.object(H, 'run', return_value=record(stderr=b'note: trace_macro\n')), \
                 patch.object(H, 'compile_case', side_effect=lambda r, p, f, e, c, profile=None: result(f)) as compile_mock, \
                 patch.object(H, 'profile_evidence', side_effect=profiles):
                report = H.capture(Path('/mock/rustc'), Path(temp) / 'accept', require_tt_hit=True)
            self.assertEqual(report['verdict'], 'passed')
            self.assertEqual(report['tt_hit_verdict'], 'passed')
            self.assertEqual(report['native_hit_verdict'], 'not-requested')
            self.assertEqual(compile_mock.call_count, 51 * 7 + 5)
            self.assertNotIn('native_evidence', report)
            self.assertEqual(len(report['tt_evidence']['observations']), 5)


if __name__ == '__main__':
    unittest.main()
