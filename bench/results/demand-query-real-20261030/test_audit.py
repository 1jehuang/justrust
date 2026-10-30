#!/usr/bin/env python3
"""Fast mutation checks for the portable auditor, no native execution."""
import copy
import importlib.util
from pathlib import Path
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('demand_audit', Path(__file__).with_name('audit.py'))
audit = importlib.util.module_from_spec(spec)
spec.loader.exec_module(audit)


class MutationTests(unittest.TestCase):
    def rejects(self, mutate, reason):
        original_load = audit.load
        state = copy.deepcopy(original_load('state.json'))
        mutate(state)
        def load(name):
            return state if name == 'state.json' else original_load(name)
        with patch.object(audit, 'load', side_effect=load):
            with self.assertRaisesRegex(ValueError, reason):
                audit.audit()

    def test_incomplete(self):
        self.rejects(lambda s: s.update(status='running'), 'incomplete')

    def test_source_not_restored(self):
        self.rejects(lambda s: s['source_after'].update({'src/summary.rs': 'bad'}), 'restoration')

    def test_identity_changed(self):
        self.rejects(lambda s: s['identities_after']['rustc'].update(sha256='bad'), 'identity')

    def test_missing_row(self):
        self.rejects(lambda s: s['rows'].pop(), 'rows')

    def test_failed_command(self):
        self.rejects(lambda s: s['rows'][0].update(status=1), 'failed')

    def test_wrong_source(self):
        self.rejects(lambda s: s['rows'][0].update(source_sha256='bad'), 'source')

    def test_wrong_journal_hash(self):
        self.rejects(lambda s: s['rows'][0]['raw_output'].update(sha256='bad'), 'journal digest')

    def test_wrong_test_receipt(self):
        self.rejects(lambda s: s['rows'][0]['parsed'].update(passed=5), 'test mismatch')


if __name__ == '__main__':
    unittest.main()
