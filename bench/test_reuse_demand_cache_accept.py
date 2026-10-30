#!/usr/bin/env python3
"""Mock-only demand-cache harness tests. Never execute a compiler or fixture."""
import base64
import importlib.util
import json
import struct
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location('demand_accept', Path(__file__).with_name('reuse-demand-cache-accept.py'))
D = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(D)


def record(status=0, stdout=b'', stderr=b''):
    return dict(status=status, stdout_b64=base64.b64encode(stdout).decode(),
                stderr_b64=base64.b64encode(stderr).decode(), command=['mock'])


def result(fixture):
    value = dict(compile=record(1 if fixture['expected'] is None else 0, stderr=b'raw\xff\r\n'))
    if fixture['expected'] is not None:
        value['runtime'] = record(stdout=fixture['expected'].encode())
    if fixture['dependency'] is not None:
        value['dependency'] = record()
    return value


class DemandTests(unittest.TestCase):
    def capture_mock(self, root, compile_fn=None, mechanism=False, profiles=None, identities=None):
        with patch.object(D.H, 'identity', side_effect=identities or [{'compiler': 'frozen', 'driver': 'frozen'}] * 2), \
             patch.object(D.H, 'probe', return_value={'no': record(), 'yes': record()}), \
             patch.object(D.H, 'compile_case', side_effect=compile_fn or (lambda r, p, f, e, c, profile=None: result(f))), \
             patch.object(D, 'profile_evidence', side_effect=profiles):
            return D.capture(Path('/mock/rustc'), root, require_mechanism=mechanism)

    def profiles(self):
        return [dict(markers={D.PROMOTION: 0 if enabled else 1,
                              D.SKIPPED: 1 if enabled else 0, D.GREEN: 0})
                for _ in range(5) for enabled in (False, True)]

    def test_adapted_scalar_and_demand_cases(self):
        fixtures = D.cases()
        self.assertEqual(len(fixtures), 32)
        self.assertEqual(len({f['name'] for f in fixtures}), 32)
        self.assertTrue(all('retention_dep' not in f['source'] for f in fixtures))
        self.assertEqual(fixtures[-10]['source'], D.DEMAND)
        self.assertEqual(fixtures[-8]['source'], D.NARROW)
        self.assertEqual(fixtures[-5]['source'], D.DEMAND)
        self.assertIsNone(fixtures[-3]['expected'])
        self.assertEqual(fixtures[-2]['source'], fixtures[-1]['source'])

    def test_same_flag_persistence_is_not_replaced_by_transitions(self):
        first, second = D.schedule(0), D.schedule(1)
        self.assertEqual(len(first), 9)
        self.assertEqual(first[2:], second[2:])
        for start, enabled in ((2, False), (4, True)):
            self.assertEqual([x[1] for x in first[start:start + 2]], [enabled, enabled])
            self.assertEqual(first[start][2], first[start + 1][2])
        self.assertNotEqual(first[2][2], first[4][2])
        self.assertEqual([x[1] for x in first[-3:]], [False, True, False])
        self.assertEqual(len({x[2] for x in first[-3:]}), 1)

    def test_full_semantics_without_mechanism_claim(self):
        with tempfile.TemporaryDirectory() as temp:
            seen = []
            def compile_fn(r, p, f, e, c):
                seen.append((f, e, c))
                return result(f)
            report = self.capture_mock(Path(temp) / 'accept', compile_fn)
            self.assertEqual(len(seen), 32 * 9)
            self.assertEqual(report['semantic_verdict'], 'passed')
            self.assertEqual(report['mechanism_verdict'], 'not-requested')
            self.assertEqual(report['verdict'], 'passed')

    def test_positive_mechanism_is_separate_and_green_optional(self):
        with tempfile.TemporaryDirectory() as temp:
            report = self.capture_mock(Path(temp) / 'accept', mechanism=True, profiles=self.profiles())
            self.assertEqual(report['mechanism_verdict'], 'passed')
            proof = report['mechanism_evidence']['proof']
            self.assertEqual(proof['green_recompute_after_demand_return'], 0)
            self.assertEqual(len(report['mechanism_evidence']['observations']), 10)

    def test_mechanism_failure_does_not_erase_semantic_pass(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp) / 'accept'
            profiles = self.profiles()
            profiles[1]['markers'][D.SKIPPED] = 0
            with self.assertRaisesRegex(AssertionError, 'positive promotion-skipped'):
                self.capture_mock(root, mechanism=True, profiles=profiles)
            report = json.loads((root / 'acceptance.json').read_text())
            self.assertEqual(report['semantic_verdict'], 'passed')
            self.assertEqual(report['mechanism_verdict'], 'failed')
            self.assertEqual(report['verdict'], 'failed')

    def test_mechanism_requires_explicit_off_control(self):
        profiles = self.profiles()
        names = ('cold', 'unchanged', 'demand-narrow', 'demand-narrow-unchanged', 'demand-wide-return')
        observations = [dict(name=names[i // 2], enabled=bool(i % 2), profile=p)
                        for i, p in enumerate(profiles)]
        for marker in (D.PROMOTION, D.SKIPPED):
            original = observations[0]['profile']['markers'][marker]
            observations[0]['profile']['markers'][marker] = 0 if marker == D.PROMOTION else 1
            with self.assertRaisesRegex(AssertionError, 'Off control'):
                D.validate_mechanism(observations)
            observations[0]['profile']['markers'][marker] = original

    def test_diagnostics_exact_and_failure_report_retained(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp) / 'accept'
            def corrupt(r, p, f, enabled, cache):
                value = result(f)
                if enabled and cache == 'persistent/yes':
                    value['compile']['stderr_b64'] = base64.b64encode(b'raw\xff\n').decode()
                return value
            with self.assertRaisesRegex(AssertionError, 'exact status'):
                self.capture_mock(root, corrupt)
            report = json.loads((root / 'acceptance.json').read_text())
            self.assertEqual(report['verdict'], 'failed')
            self.assertIn('identity_after', report)
            self.assertIn('persistent-yes', report['observations'][0]['modes'])

    def test_compiler_driver_change_invalidates_gate(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp) / 'accept'
            with self.assertRaisesRegex(AssertionError, 'identity changed'):
                self.capture_mock(root, identities=[{'driver': 'before'}, {'driver': 'after'}])
            self.assertEqual(json.loads((root / 'acceptance.json').read_text())['verdict'], 'failed')

    def test_errors_reject_ice_and_false_success(self):
        for status in (0, 101, -11):
            with self.assertRaises(AssertionError):
                D.H.validate_expected({'compile': record(status)}, None, 'error')

    def test_existing_output_rejected(self):
        with tempfile.TemporaryDirectory() as temp, self.assertRaises(FileExistsError):
            D.capture(Path('/never/run'), Path(temp))

    def test_probe_failure_retains_raw_diagnostics(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp) / 'accept'
            with patch.object(D.H, 'identity', return_value={'frozen': True}), \
                 patch.object(D.H, 'probe', return_value={'no': record(), 'yes': record(1, stderr=b'unknown option\xff')}), \
                 self.assertRaisesRegex(AssertionError, 'probe failed'):
                D.capture(Path('/mock/rustc'), root)
            report = json.loads((root / 'acceptance.json').read_text())
            self.assertEqual(report['completed_cases'], 0)
            self.assertEqual(report['verdict'], 'failed')
            self.assertEqual(base64.b64decode(report['feature_probe']['yes']['stderr_b64']), b'unknown option\xff')

    def test_mechanism_cannot_pass_missing_observations(self):
        with self.assertRaisesRegex(AssertionError, 'Incomplete'):
            D.validate_mechanism([])

    def test_synthetic_native_profile_counts_no_guessed_labels(self):
        def header(magic):
            return magic + struct.pack('<I', 9)
        strings = bytearray()
        def string(value):
            address = 100000003 + 8 + len(strings)
            strings.extend(value.encode() + b'\xff')
            return address
        activity = string('GenericActivity')
        query = string('Query')
        labels = [string(label) for label in D.MARKERS]
        metadata = string(json.dumps({'process_id': 7, 'start_time': 12, 'cmd': 'synthetic, never executed'}))
        events = b''.join(struct.pack('<QQIIII', activity, label, 0, 1, 2, 0) for label in labels)
        events += struct.pack('<QQIIII', query, 42, 0, 2, 3, 0)
        streams = [header(b'MMES') + events, header(b'MMSD') + strings,
                   header(b'MMSI') + struct.pack('<QQ', 100000001, metadata - 100000003)]
        raw = header(b'MMPD') + b''.join(struct.pack('<BI', tag, len(stream)) + stream
                                      for tag, stream in enumerate(streams))
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            with self.assertRaisesRegex(AssertionError, 'exactly one'):
                D.profile_evidence(root)
            profile = root / 'synthetic.mm_profdata'
            profile.write_bytes(raw)
            evidence = D.profile_evidence(root)
            self.assertEqual(evidence['markers'], dict.fromkeys(D.MARKERS, 1))
            self.assertEqual(evidence['unresolved_records'], 1)
            self.assertEqual(evidence['sha256'], D.H.digest(profile))
            profile.write_bytes(b'corrupt')
            with self.assertRaises(ValueError):
                D.profile_evidence(root)

    def test_native_commands_flag_paths_ich_and_profile(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            commands = []
            def fake_run(command):
                commands.append([str(v) for v in command])
                if '-o' in command:
                    Path(command[command.index('-o') + 1]).write_bytes(b'mock')
                return record()
            with patch.object(D.H, 'run', side_effect=fake_run):
                for name, enabled, cache in D.schedule(0):
                    D.H.compile_case(Path('/mock/rustc'), root, D.cases()[0], enabled, cache)
            mains = [c for c in commands if '--crate-name=declarative_macro_accept' in c]
            self.assertEqual(len(mains), 9)
            self.assertEqual(len({c[1] for c in mains}), 1)
            self.assertTrue(all('-Zincremental-verify-ich' in c for c in commands if '-o' in c))
            self.assertTrue(all(any(a.startswith(D.FLAG + '=') for a in c) for c in mains))

    def test_mechanism_sequences_have_separate_persistent_caches(self):
        with tempfile.TemporaryDirectory() as temp:
            seen = []
            def compile_fn(r, p, f, enabled, cache, profile):
                seen.append((enabled, cache, profile, f))
                return result(f)
            evidence = {'observations': []}
            with patch.object(D.H, 'compile_case', side_effect=compile_fn), \
                 patch.object(D, 'profile_evidence', side_effect=self.profiles()):
                D.mechanism_evidence(Path('/mock/rustc'), Path(temp), evidence)
            self.assertEqual(len(seen), 10)
            self.assertEqual(len({v[2] for v in seen}), 10)
            self.assertEqual({v[1] for v in seen if v[0]}, {'mechanism/yes'})
            self.assertEqual({v[1] for v in seen if not v[0]}, {'mechanism/no'})


if __name__ == '__main__':
    unittest.main()
