#!/usr/bin/env python3
"""Offline mutation tests. No compiler or native fixture execution."""
import copy
import importlib.util
import json
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest.mock import patch

HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location('demand_retained_audit', HERE / 'audit.py')
A = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(A)


class AuditTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        with tarfile.open(HERE / 'native-evidence.tar.gz') as archive:
            cls.report = json.load(archive.extractfile('acceptance/acceptance.json'))

    def test_full_retained_report(self):
        with patch.object(A.D.H, 'run', side_effect=AssertionError('native execution forbidden')):
            self.assertEqual(A.audit_semantics(self.report), (288, 198))
            proof, counts = A.audit_mechanism(self.report)
        self.assertEqual(proof['green_recompute_after_demand_return'], 1376)
        self.assertEqual(len(counts), 10)

    def test_diagnostic_mutation_fails_exact_comparison(self):
        report = copy.deepcopy(self.report)
        report['observations'][0]['modes']['persistent-yes-replay']['compile']['stderr_b64'] = 'eA=='
        with self.assertRaisesRegex(AssertionError, 'exact status'):
            A.audit_semantics(report)

    def test_missing_persistent_replay_is_failure(self):
        report = copy.deepcopy(self.report)
        del report['observations'][0]['modes']['persistent-yes-replay']
        with self.assertRaises(AssertionError):
            A.audit_semantics(report)

    def test_per_revision_persistent_cache_reset_is_failure(self):
        report = copy.deepcopy(self.report)
        command = report['observations'][1]['modes']['persistent-yes']['compile']['command']
        index = next(i for i, arg in enumerate(command) if arg.startswith('incremental='))
        command[index] = 'incremental=/new-cache/incremental/persistent/yes/binary'
        with self.assertRaises(AssertionError):
            A.audit_semantics(report)

    def test_compiler_identity_change_is_failure(self):
        report = copy.deepcopy(self.report)
        report['identity_after']['drivers'][0]['sha256'] = 'changed'
        with self.assertRaises(AssertionError):
            A.audit_semantics(report)

    def test_absent_skipped_or_off_marker_is_failure(self):
        for index, marker in ((0, A.D.PROMOTION), (1, A.D.SKIPPED)):
            report = copy.deepcopy(self.report)
            report['mechanism_evidence']['observations'][index]['profile']['markers'][marker] = 0
            with self.subTest(index=index), self.assertRaises(AssertionError):
                A.audit_mechanism(report)

    def test_conflated_mechanism_caches_are_failure(self):
        report = copy.deepcopy(self.report)
        observations = report['mechanism_evidence']['observations']
        off_cache = A.cache_of(observations[0]['result']['compile']['command'])
        for observation in observations:
            if observation['enabled']:
                command = observation['result']['compile']['command']
                index = next(i for i, arg in enumerate(command) if arg.startswith('incremental='))
                command[index] = off_cache
        with self.assertRaises(AssertionError):
            A.audit_mechanism(report)

    def test_raw_profile_corruption_is_failure(self):
        with tempfile.TemporaryDirectory() as temp, tarfile.open(HERE / 'native-evidence.tar.gz') as archive:
            tree = Path(temp)
            for member in archive.getmembers():
                if member.isfile() and member.name.startswith('acceptance/mechanism-profiles/'):
                    target = tree / member.name
                    target.parent.mkdir(parents=True, exist_ok=True)
                    target.write_bytes(archive.extractfile(member).read())
            profile = next((tree / 'acceptance/mechanism-profiles/cold/no').glob('*.mm_profdata'))
            profile.write_bytes(b'corrupt')
            with self.assertRaises((AssertionError, ValueError)):
                A.audit_mechanism(self.report, tree)


if __name__ == '__main__':
    unittest.main()
