#!/usr/bin/env python3
"""Mutation tests for the portable retained-evidence audit, no native execution."""
import copy
import importlib.util
import json
from pathlib import Path
import tarfile
import unittest

HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location('retained_tt_audit', HERE / 'audit.py')
A = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(A)


class AuditTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        with tarfile.open(HERE / 'native-evidence.tar.gz') as archive:
            cls.positive = json.load(archive.extractfile('acceptance-tt/acceptance.json'))
            cls.negative = json.load(archive.extractfile('tt-literal-negative/acceptance.json'))

    def test_retained_matrices_and_expected_negative(self):
        self.assertEqual(A.audit_semantics(self.positive), 357)
        self.assertEqual(A.audit_semantics(self.negative), 357)
        A.audit_proof(self.positive, 'tt_evidence')
        A.audit_proof(self.negative, 'tt_evidence', negative=True)

    def test_changed_diagnostic_is_not_normalized(self):
        report = copy.deepcopy(self.positive)
        report['observations'][0]['modes']['transition-yes']['compile']['stderr_b64'] = 'eA=='
        with self.assertRaisesRegex(AssertionError, 'exact status'):
            A.audit_semantics(report)

    def test_missing_mode_is_not_skipped(self):
        report = copy.deepcopy(self.positive)
        del report['observations'][0]['modes']['persistent-yes']
        with self.assertRaises(AssertionError):
            A.audit_semantics(report)

    def test_identity_change_is_failure(self):
        report = copy.deepcopy(self.positive)
        report['identity_after']['sha256'] = 'changed'
        with self.assertRaises(AssertionError):
            A.audit_semantics(report)

    def test_positive_counts_cannot_be_removed(self):
        for name in ('cold-yes', 'warm-yes', 'body-edit-yes'):
            report = copy.deepcopy(self.positive)
            observation = next(o for o in report['tt_evidence']['observations'] if o['name'] == name)
            observation['profile']['markers'] = dict.fromkeys(A.H.MARKERS, 0)
            with self.subTest(name=name), self.assertRaises(AssertionError):
                A.audit_proof(report, 'tt_evidence')

    def test_negative_wrong_failure_cannot_pass(self):
        report = copy.deepcopy(self.negative)
        report['tt_evidence']['observations'][0]['result']['compile']['stderr_b64'] = 'eA=='
        with self.assertRaises(AssertionError):
            A.audit_proof(report, 'tt_evidence', negative=True)


if __name__ == '__main__':
    unittest.main()
