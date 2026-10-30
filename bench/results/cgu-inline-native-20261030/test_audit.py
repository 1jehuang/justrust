#!/usr/bin/env python3
"""Offline mutation and detached-replay checks, never invoke rustc or fixtures."""
import copy
import importlib.util
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location('cgu_evidence_audit', HERE / 'audit.py')
A = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(A)


class AuditTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temp = tempfile.TemporaryDirectory()
        cls.tree = Path(cls.temp.name)
        cls.original, cls.repaired = A.prepare(cls.tree)
        cls.report = json.loads((cls.tree / 'acceptance-v2/acceptance.json').read_text())

    @classmethod
    def tearDownClass(cls):
        cls.temp.cleanup()

    def test_complete_source_detached_replay(self):
        with tempfile.TemporaryDirectory() as temp:
            directory = Path(temp)
            for path in [HERE / 'audit.py', *HERE.glob('*.tar.gz')]:
                shutil.copyfile(path, directory / path.name)
            result = subprocess.run([sys.executable, '-I', str(directory / 'audit.py')],
                                    cwd=directory, capture_output=True, timeout=30)
            self.assertEqual(result.returncode, 0, result.stderr)
            summary = json.loads(result.stdout)
            self.assertEqual(summary['original_semantic_modes'], 672)
            self.assertEqual(summary['repaired_semantic_modes'], 672)
            self.assertEqual(summary['raw_profiles_reparsed'], 12)

    def test_changed_diagnostic_rejected(self):
        report = copy.deepcopy(self.report)
        report['observations'][0]['modes']['transition-yes']['compile']['stderr_b64'] = 'eA=='
        with self.assertRaisesRegex(AssertionError, 'exact status'):
            A.audit_semantics(report, self.repaired)

    def test_missing_transition_rejected(self):
        report = copy.deepcopy(self.report)
        del report['observations'][0]['modes']['transition-no-after']
        with self.assertRaises(AssertionError):
            A.audit_semantics(report, self.repaired)

    def test_persistent_cache_reset_rejected(self):
        report = copy.deepcopy(self.report)
        command = report['observations'][1]['modes']['persistent-yes']['compile']['command']
        index = next(i for i, a in enumerate(command) if a.startswith('incremental='))
        command[index] = command[index].replace('incremental=', 'incremental=/new-prefix')
        with self.assertRaises(AssertionError):
            A.audit_semantics(report, self.repaired)

    def test_nonincremental_cache_leak_rejected(self):
        report = copy.deepcopy(self.report)
        observation = next(o for o in report['observations'] if not o['config']['incremental'])
        observation['modes']['fresh-no']['compile']['command'].append('incremental=/unexpected')
        with self.assertRaises(AssertionError):
            A.audit_semantics(report, self.repaired)

    def test_driver_identity_drift_rejected(self):
        report = copy.deepcopy(self.report)
        report['identity_after']['drivers'][0]['sha256'] = 'changed'
        with self.assertRaises(AssertionError):
            A.audit_semantics(report, self.repaired)

    def test_missing_revisit_rejected(self):
        evidence = copy.deepcopy(self.report['mechanism_evidence'])
        for observation in evidence['observations']:
            observation['profile']['markers'][self.repaired.MARKERS[1]] = 0
        with self.assertRaises(AssertionError):
            A.audit_profiles(evidence, self.repaired, self.tree, 'acceptance-v2')

    def test_original_failure_cannot_be_retconned(self):
        with tempfile.TemporaryDirectory() as temp:
            tree = Path(temp)
            original, repaired = A.prepare(tree)
            path = tree / 'acceptance/acceptance.json'
            report = json.loads(path.read_text())
            report['verdict'] = report['mechanism_verdict'] = 'passed'
            path.write_text(json.dumps(report))
            with self.assertRaises(AssertionError):
                A.audit(tree, original, repaired)

    def test_frozen_native_source_mutation_rejected(self):
        with tempfile.TemporaryDirectory() as temp:
            tree = Path(temp)
            original, repaired = A.prepare(tree)
            path = tree / 'source' / next(iter(A.SOURCE_HASHES))
            path.write_bytes(path.read_bytes() + b'\n// changed\n')
            with self.assertRaises(AssertionError):
                A.audit(tree, original, repaired)

    def test_raw_profile_corruption_rejected(self):
        with tempfile.TemporaryDirectory() as temp:
            tree = Path(temp)
            _, repaired = A.prepare(tree)
            path = next((tree / 'acceptance-v2/mechanism-profiles/cold/yes').glob('*.mm_profdata'))
            path.write_bytes(b'corrupt')
            with self.assertRaises((AssertionError, ValueError)):
                A.audit_profiles(self.report['mechanism_evidence'], repaired, tree, 'acceptance-v2')


if __name__ == '__main__':
    unittest.main()
