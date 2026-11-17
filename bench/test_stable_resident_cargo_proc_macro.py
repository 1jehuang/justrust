#!/usr/bin/env python3
"""Mock-only tests. Coordinator runs these after the comprehensive baseline."""
import base64
import json
from pathlib import Path
import tempfile
import unittest
from unittest import mock

import stable_resident_cargo_proc_macro as subject


def record(stage, *, marker=None, errors=None, status=None):
    if errors is None:
        errors = ([f"JR_PM_ERROR={stage['token']}:{stage['error']}"] * 2
                  if stage['error'] else [])
    stdout = '\n'.join(json.dumps(dict(reason='compiler-message', message=dict(
        level='error', message=message))) for message in errors)
    stderr = ''
    if not stage['error']:
        stdout += '\ntest proc_macro_nonce_probe ... ok\ntest result: ok. 1 passed; 0 failed;\n'
        stderr = 'JR_PM_RUNTIME=' + (marker or stage['nonce']) + '\n'
    return dict(status=(101 if stage['error'] else 0) if status is None else status,
                stdout_b64=base64.b64encode(stdout.encode()).decode(),
                stderr_b64=base64.b64encode(stderr.encode()).decode(), timed_out=False)


def artifact(fresh=True, digest='a' * 64):
    return dict(target=dict(name='fixture_macro'), fresh=fresh,
                artifacts=[dict(path='/fixture/libfixture_macro.so', sha256=digest)])


def completion(pid=42, key='a' * 64, status=0):
    return dict(event='complete', crate_name='dependency_fallback_root', key=key,
                result=dict(pid=pid, id=0, status=status))


class ProcMacroOracleTests(unittest.TestCase):
    def test_workload_has_forced_edits_two_calls_and_recoveries(self):
        rows = subject.stages('unique')
        self.assertEqual(len({s['root'] for s in rows}), len(rows))
        self.assertEqual([s['error'] for s in rows if s['error']], ['env', 'file'])
        for stage in rows:
            self.assertIn(b'fixture_macro::probe!(A "', stage['root'])
            self.assertIn(b'fixture_macro::probe!(B "', stage['root'])
            self.assertIn(stage['token'].encode(), stage['root'])
            if not stage['error']:
                self.assertIn(b'assert_eq!(counts, [1, 2])', stage['root'])
        self.assertIn(b'static COUNTER: AtomicUsize', subject.MACRO)
        self.assertIn(b'fetch_add(1, Ordering::SeqCst)', subject.MACRO)
        self.assertIn(b'static THREAD_COUNTER: Cell<usize>', subject.MACRO)
        self.assertNotIn(b'JR_PM_NONCE', subject.MACRO)
        for stage in rows:
            if not stage['error']:
                self.assertIn(b'assert_eq!(thread_counts, [1, 2])', stage['root'])

    def test_exact_success_and_diagnostic_nonces(self):
        for stage in subject.stages('unique'):
            subject.semantics(record(stage), stage)
            if stage['error']:
                with self.assertRaises(AssertionError):
                    subject.semantics(record(stage, errors=['JR_PM_ERROR=stale:env'] * 2), stage)
                with self.assertRaises(AssertionError):
                    subject.semantics(record(stage, errors=[f"JR_PM_ERROR={stage['token']}:{stage['error']}"]), stage)
            else:
                with self.assertRaises(AssertionError):
                    subject.semantics(record(stage, marker='stale:3,4:env:file'), stage)

    def test_no_silently_fresh_root_or_rebuilt_macro(self):
        previous = subject.validate_artifacts([artifact(False)], None)
        subject.validate_artifacts([artifact()], previous)
        with self.assertRaises(AssertionError):
            subject.validate_artifacts([artifact(False)], previous)
        with self.assertRaises(AssertionError):
            subject.validate_artifacts([artifact(digest='b' * 64)], previous)
        with self.assertRaises(AssertionError):
            subject.validate_artifacts([artifact(), dict(target=dict(name='dependency_fallback_root'), fresh=True)], previous)

    def test_strict_root_identity_and_fallback_are_distinct(self):
        stage = subject.stages('unique')[1]
        dep = subject.dependency
        dep.validate_trace(stage, [], 'on', False)
        with self.assertRaises(AssertionError):
            dep.validate_trace(stage, [], 'on', True)
        residency = dep.validate_trace(stage, [completion()], 'on', True)
        previous = dep.validate_root_transition(stage, residency, None)
        dep.validate_root_transition(stage, residency, previous)
        replaced = dep.validate_trace(stage, [completion(pid=43)], 'on', True)
        with self.assertRaises(AssertionError):
            dep.validate_root_transition(stage, replaced, previous)
        with self.assertRaises(AssertionError):
            dep.validate_trace(stage, [completion()], 'off', True)

    def test_authorization_gate_runs_before_any_candidate_or_guest(self):
        with mock.patch.object(subject.safety, 'attest') as attest:
            for options in ({}, dict(root_native_frozen=True), dict(idle_authorized=True)):
                with self.assertRaises(ValueError):
                    subject.capture('/missing', '/missing', '/missing', justrust='/missing', **options)
            attest.assert_not_called()

    def test_arm_mock_preserves_raw_records_and_requires_all_requests(self):
        rows = subject.stages('unique')
        calls = []
        def invoke(argv, timeout, *, cwd, env):
            stage = rows[len(calls)]
            calls.append(dict(argv=argv, env=dict(env)))
            self.assertEqual((cwd / 'src/lib.rs').read_bytes(), stage['root'])
            self.assertEqual(env.get('JR_PM_VALUE'), stage['env'])
            self.assertNotIn('JR_PM_NONCE', env)
            path = Path(env['JR_PM_FILE'])
            self.assertEqual(path.read_text() if path.exists() else None, stage['file'])
            return record(stage)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            fixture = root / 'fixture'
            (fixture / 'src').mkdir(parents=True)
            with mock.patch.object(subject.shared, 'environment', return_value={}), \
                    mock.patch.object(subject.safety, 'safe_environment', side_effect=lambda env: env), \
                    mock.patch.object(subject.dependency, 'metadata', side_effect=lambda rec: [
                        artifact(fresh=len(calls) != 1),
                        dict(target=dict(name='dependency_fallback_root'), fresh=False)]):
                report = subject.capture_arm(root / 'compiler', root / 'cargo', fixture,
                                             root / 'arm', 'off', rows, 10, invoke)
            self.assertEqual(report['verdict'], 'passed')
            self.assertEqual(len(calls), len(rows))
            self.assertEqual(len(report['observations']), len(rows))
            self.assertEqual(json.loads((root / 'arm/receipt.json').read_text()), report)
            for row in report['observations']:
                self.assertIn('stdout_b64', row['cargo'])
                self.assertIn('stderr_b64', row['cargo'])


if __name__ == '__main__':
    unittest.main()
