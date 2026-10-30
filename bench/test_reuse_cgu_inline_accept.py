#!/usr/bin/env python3
"""Mock-only tests, never execute rustc or a native fixture."""
import base64
import importlib.util
import json
import struct
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location('cgu_accept', Path(__file__).with_name('reuse-cgu-inline-accept.py'))
C = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(C)


def record(status=0, stdout=b'', stderr=b''):
    return dict(command=['mock'], status=status, stdout_b64=base64.b64encode(stdout).decode(),
                stderr_b64=base64.b64encode(stderr).decode())


def result(fixture):
    value = dict(compile=record(1 if fixture['expected'] is None else 0, stderr=b'diagnostic\xff\r\n'))
    if fixture['expected'] is not None:
        value['runtime'] = record(stdout=fixture['expected'].encode())
    return value


class CguTests(unittest.TestCase):
    def capture_mock(self, root, compile_fn=None, identities=None, probes=None):
        with patch.object(C.H, 'identity', side_effect=identities or [{'frozen': True}] * 2), \
             patch.object(C.H, 'probe', return_value=probes or {'no': record(), 'yes': record()}), \
             patch.object(C, 'compile_case', side_effect=compile_fn or (lambda r, p, f, c, e, i: result(f))):
            return C.capture(Path('/mock/rustc'), root)

    def test_configuration_cross_product(self):
        configs = C.configurations()
        self.assertEqual(len(configs), 8)
        self.assertEqual({(c['codegen_units'], c['incremental'], c['opt_level']) for c in configs},
                         {(units, incremental, opt) for units in (1, 16) for incremental in (False, True) for opt in (0, 2)})
        self.assertEqual(len({c['name'] for c in configs}), 8)

    def test_fixture_graph_and_diagnostics_contract(self):
        self.assertEqual(len(C.cases()), 12)
        self.assertIn('fn left() -> u32 { leaf() + 1 }', C.BASE)
        self.assertIn('fn right() -> u32 { leaf() + 2 }', C.BASE)
        self.assertEqual(C.BASE.count('super::diamond()'), 3)
        initial_module = C.BASE.split('mod a {', 1)[1].split('\n}', 1)[0]
        self.assertIn('pub fn root_a()', initial_module)
        self.assertIn('pub fn root_b()', initial_module)
        self.assertIn('a::root_b()', C.BASE)
        self.assertIn('mod c {', C.BASE)
        for function in ('leaf', 'left', 'right', 'diamond', 'cycle_a', 'cycle_b'):
            self.assertIn(f'#[inline(always)] fn {function}(', C.BASE)
        self.assertIn('cycle_b(n - 1)', C.BASE)
        self.assertIn('cycle_a(n - 1)', C.BASE)
        self.assertEqual(C.BASE.count('if n == 0'), 2)
        self.assertIn('generic(7u32), generic(11u64)', C.BASE)
        self.assertIn('impl<T> Drop for Owned<T>', C.BASE)
        self.assertEqual(C.DUPLICATE.count('pub fn collision()'), 2)
        fixtures = {f['name']: f for f in C.cases()}
        self.assertIsNone(fixtures['duplicate-symbol-error']['expected'])
        self.assertIsNone(fixtures['type-error']['expected'])
        self.assertEqual(fixtures['duplicate-symbol-recovery']['source'], C.BASE)
        self.assertEqual(fixtures['recovery-unchanged']['source'], C.BASE)

    def test_schedule_persistence_transitions_and_configuration_isolation(self):
        config = C.configurations()[0]
        first, second = C.schedule(0, config), C.schedule(1, config)
        self.assertEqual(first[2:], second[2:])
        self.assertTrue({x[2] for x in first[:2]}.isdisjoint(x[2] for x in second[:2]))
        self.assertEqual([x[1] for x in first[-3:]], [False, True, False])
        self.assertEqual(len({x[2] for x in first[-3:]}), 1)
        self.assertTrue({x[2] for x in first}.isdisjoint(x[2] for x in C.schedule(0, C.configurations()[1])))

    def test_mocked_complete_matrix(self):
        with tempfile.TemporaryDirectory() as temp:
            seen = []
            def compile_fn(r, p, f, c, e, i):
                seen.append((f, c, e, i))
                return result(f)
            root = Path(temp) / 'accept'
            report = self.capture_mock(root, compile_fn)
            self.assertEqual(report['verdict'], 'passed')
            self.assertEqual(report['completed_cases'], 96)
            self.assertEqual(report['planned_main_compiles'], 672)
            self.assertEqual(len(seen), 672)
            self.assertEqual(json.loads((root / 'acceptance.json').read_text()), report)

    def test_exact_diagnostics_failure_retained(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp) / 'accept'
            def corrupt(r, p, f, c, enabled, i):
                value = result(f)
                if enabled:
                    value['compile']['stderr_b64'] = base64.b64encode(b'diagnostic\xff\n').decode()
                return value
            with self.assertRaisesRegex(AssertionError, 'exact status'):
                self.capture_mock(root, corrupt)
            report = json.loads((root / 'acceptance.json').read_text())
            self.assertEqual(report['verdict'], 'failed')
            self.assertIn('fresh-yes', report['observations'][0]['modes'])
            self.assertIn('identity_after', report)

    def test_expected_errors_do_not_accept_crash_or_success(self):
        for status in (0, 101, -11):
            with self.assertRaises(AssertionError):
                C.H.validate_expected({'compile': record(status)}, None, 'expected-error')

    def test_independent_runtime_expectation(self):
        with tempfile.TemporaryDirectory() as temp:
            def wrong(r, p, f, c, e, i):
                value = result(f)
                if 'runtime' in value:
                    value['runtime'] = record(stdout=b'wrong\n')
                return value
            with self.assertRaisesRegex(AssertionError, 'wrong runtime'):
                self.capture_mock(Path(temp) / 'accept', wrong)

    def test_probe_failure_is_failure_not_skip(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp) / 'accept'
            with self.assertRaisesRegex(AssertionError, 'probe failed'):
                self.capture_mock(root, probes={'no': record(), 'yes': record(1, stderr=b'unknown option\xff')})
            report = json.loads((root / 'acceptance.json').read_text())
            self.assertEqual(report['completed_cases'], 0)
            self.assertEqual(report['verdict'], 'failed')
            self.assertEqual(base64.b64decode(report['feature_probe']['yes']['stderr_b64']), b'unknown option\xff')

    def test_identity_change_invalidates_run(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp) / 'accept'
            with self.assertRaisesRegex(AssertionError, 'identity changed'):
                self.capture_mock(root, identities=[{'driver': 'before'}, {'driver': 'after'}])
            self.assertEqual(json.loads((root / 'acceptance.json').read_text())['verdict'], 'failed')

    def test_commands_paths_config_flags_ich_and_no_incremental_leak(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            commands = []
            def run(command):
                commands.append([str(v) for v in command])
                if '-o' in command:
                    Path(command[command.index('-o') + 1]).write_bytes(b'mock')
                return record()
            fixture = C.cases()[0]
            with patch.object(C.H, 'run', side_effect=run):
                for config in C.configurations():
                    for _, enabled, cache in C.schedule(0, config):
                        C.compile_case(Path('/mock/rustc'), root, fixture, config, enabled, cache)
            compiles = [c for c in commands if '-o' in c]
            self.assertEqual(len(compiles), 56)
            self.assertEqual({c[1] for c in compiles}, {str(root / 'fixture.rs')})
            self.assertEqual({c[c.index('-o') + 1] for c in compiles}, {str(root / 'fixture')})
            for config_index, config in enumerate(C.configurations()):
                for command in compiles[config_index * 7:(config_index + 1) * 7]:
                    self.assertIn('-Zincremental-verify-ich', command)
                    self.assertIn(f'-Ccodegen-units={config["codegen_units"]}', command)
                    self.assertIn(f'-Copt-level={config["opt_level"]}', command)
                    self.assertEqual(any(a.startswith('incremental=') for a in command), config['incremental'])
                    self.assertTrue(any(a.startswith(C.FLAG + '=') for a in command))

    def test_stale_binary_removed_on_failure(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / 'fixture').write_bytes(b'stale')
            with patch.object(C.H, 'run', return_value=record(1)) as run:
                value = C.compile_case(Path('/mock/rustc'), root, C.cases()[7], C.configurations()[0], True, 'cache')
            self.assertFalse((root / 'fixture').exists())
            self.assertNotIn('runtime', value)
            self.assertEqual(run.call_count, 1)

    def test_existing_output_rejected(self):
        with tempfile.TemporaryDirectory() as temp, self.assertRaises(FileExistsError):
            C.capture(Path('/never/run'), Path(temp))

    def positive_observations(self):
        return [dict(name=name, enabled=enabled, profile=dict(markers=dict(zip(C.MARKERS, counts))))
                for name, enabled, counts in [('cold', False, (0, 0)), ('cold', True, (1, 3)),
                                              ('warm', False, (0, 0)), ('warm', True, (0, 0))]]

    def test_mechanism_requires_actual_duplicate_avoidance_and_off_zero(self):
        proof = C.validate_mechanism(self.positive_observations())
        self.assertEqual(proof['marker_totals'][C.MARKERS[1]], 3)
        for index, counts in ((1, (0, 0)), (1, (1, 0)), (0, (1, 1)), (2, (0, 1))):
            observations = self.positive_observations()
            observations[index]['profile']['markers'] = dict(zip(C.MARKERS, counts))
            with self.subTest(index=index, counts=counts), self.assertRaises(AssertionError):
                C.validate_mechanism(observations)
        with self.assertRaisesRegex(AssertionError, 'Incomplete'):
            C.validate_mechanism([])

    def test_independent_mechanism_and_failure_verdicts(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp) / 'positive'
            profiles = [o['profile'] for o in self.positive_observations()]
            seen = []
            def compile_fn(r, p, f, c, e, cache, profile=None):
                if profile is not None:
                    seen.append((e, cache, profile))
                return result(f)
            with patch.object(C.H, 'identity', return_value={'frozen': True}), \
                 patch.object(C.H, 'probe', return_value={'no': record(), 'yes': record()}), \
                 patch.object(C, 'compile_case', side_effect=compile_fn), \
                 patch.object(C, 'profile_evidence', side_effect=profiles):
                report = C.capture(Path('/mock/rustc'), root, True)
            self.assertEqual(report['semantic_verdict'], 'passed')
            self.assertEqual(report['mechanism_verdict'], 'passed')
            self.assertEqual(len(seen), 4)
            self.assertEqual(len({s[2] for s in seen}), 4)
            self.assertEqual([s[1] for s in seen], ['mechanism/no', 'mechanism/yes'] * 2)
            root = Path(temp) / 'negative'
            profiles = [dict(markers=dict.fromkeys(C.MARKERS, 0)) for _ in range(4)]
            with patch.object(C.H, 'identity', return_value={'frozen': True}), \
                 patch.object(C.H, 'probe', return_value={'no': record(), 'yes': record()}), \
                 patch.object(C, 'compile_case', side_effect=compile_fn), \
                 patch.object(C, 'profile_evidence', side_effect=profiles), \
                 self.assertRaisesRegex(AssertionError, 'No positive native'):
                C.capture(Path('/mock/rustc'), root, True)
            report = json.loads((root / 'acceptance.json').read_text())
            self.assertEqual(report['semantic_verdict'], 'passed')
            self.assertEqual(report['mechanism_verdict'], 'failed')
            self.assertEqual(report['verdict'], 'failed')

    def test_explicit_generic_activity_profile_command(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            commands = []
            def run(command):
                commands.append([str(x) for x in command])
                if '-o' in command:
                    Path(command[command.index('-o') + 1]).write_bytes(b'mock')
                return record()
            with patch.object(C.H, 'run', side_effect=run):
                C.compile_case(Path('/mock/rustc'), root, C.cases()[0], C.configurations()[0], True,
                               'mechanism/yes', profile=root / 'profile')
            self.assertIn('-Zself-profile-events=generic-activity', commands[0])
            self.assertIn(f'-Zself-profile={root}/profile', commands[0])

    def test_synthetic_profile_unknown_labels_and_strict_kind(self):
        def capture(kind):
            def header(magic):
                return magic + struct.pack('<I', 9)
            strings = bytearray()
            def string(value):
                address = 100000003 + 8 + len(strings)
                strings.extend(value.encode() + b'\xff')
                return address
            event_kind = string(kind)
            query_kind = string('Query')
            label = string(C.MARKERS[0])
            metadata = string(json.dumps(dict(process_id=7, start_time=12, cmd='synthetic')))
            events = struct.pack('<QQIIII', event_kind, label, 0, 1, 2, 0)
            events += struct.pack('<QQIIII', query_kind, 42, 0, 2, 3, 0)
            streams = [header(b'MMES') + events, header(b'MMSD') + strings,
                       header(b'MMSI') + struct.pack('<QQ', 100000001, metadata - 100000003)]
            return header(b'MMPD') + b''.join(struct.pack('<BI', tag, len(stream)) + stream for tag, stream in enumerate(streams))
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            with self.assertRaisesRegex(AssertionError, 'exactly one'):
                C.profile_evidence(root)
            path = root / 'synthetic.mm_profdata'
            path.write_bytes(capture('GenericActivity'))
            evidence = C.profile_evidence(root)
            self.assertEqual(evidence['markers'], dict(zip(C.MARKERS, (1, 0))))
            self.assertEqual(evidence['unresolved_records'], 1)
            self.assertEqual(evidence['sha256'], C.H.digest(path))
            path.write_bytes(capture('Query'))
            with self.assertRaisesRegex(AssertionError, 'kind/type'):
                C.profile_evidence(root)


if __name__ == '__main__':
    unittest.main()
