"""Offline provider inventory validation including actual binary-format CLI input."""
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

import provider_cost as pc
import query_profile as qp
from test_query_profile import fixture


def event(label='type_of', thread=1, start=0, end=10, **changes):
    value = dict(kind='Query', record_type='interval', label=label,
                 label_resolved=True, thread=thread, start_ns=start, end_ns=end)
    value.update(changes)
    return value


class ProviderCostTests(unittest.TestCase):
    def test_overlap_nesting_and_threads_not_added(self):
        result = pc.summarize_records([event(), event(start=5, end=15),
                                      event('other', start=2, end=8),
                                      event(thread=2, end=20)])
        self.assertEqual([(x['query_kind'], x['thread'], x['interval_records'], x['interval_union_ns'])
                          for x in result['per_kind_per_thread']],
                         [('other', 1, 1, 6), ('type_of', 1, 2, 15), ('type_of', 2, 1, 20)])
        self.assertEqual(result['all_query_per_thread'], [
            dict(thread=1, interval_records=3, interval_union_ns=15),
            dict(thread=2, interval_records=1, interval_union_ns=20)])
        self.assertIsNone(result['cost_for_same_results_ns'])

    def test_unsafe_missing_and_unresolved_names_are_unknown(self):
        records = [event(label=name) for name in ('private::path', 'foo\x1earg', '', None, 'é')]
        records.append(event(label='looks_safe', label_resolved=False))
        result = pc.summarize_records(records)
        self.assertEqual(result['unknown_query_name_interval_records'], 6)
        self.assertEqual(result['unresolved_event_label_records'], 1)
        self.assertEqual(result['per_kind_per_thread'], [dict(
            query_kind=None, thread=1, interval_records=6, interval_union_ns=10)])
        self.assertNotIn('private', json.dumps(result))

    def test_shapes_fail_closed(self):
        for changes in [dict(record_type='instant'), dict(record_type='integer'),
                        dict(thread=True), dict(thread=-1), dict(thread=None),
                        dict(start_ns=True), dict(start_ns=-1), dict(end_ns=0, start_ns=1),
                        dict(end_ns=None), dict(label_resolved=1), dict(kind=None)]:
            with self.subTest(changes=changes), self.assertRaises(ValueError):
                pc.summarize_records([event(**changes)])
        with self.assertRaises(ValueError):
            pc.summarize_records([None])

    def test_empty_and_nonquery(self):
        result = pc.summarize_records([])
        self.assertEqual(pc.analyze(fixture(events=b''))['query_interval_records'], 0)
        self.assertEqual(result['query_interval_records'], 0)
        self.assertEqual(result['all_query_per_thread'], [])
        self.assertEqual(result['per_kind_per_thread'], [])
        self.assertEqual(pc.summarize_records([event(kind='GenericActivity')])['query_interval_records'], 0)
        self.assertEqual(pc.summarize_records([event(end=0)])['all_query_per_thread'][0]['interval_union_ns'], 0)

    def test_binary_decode_strips_args_and_hashes_raw_capture(self):
        data = fixture(strings=b'Query\xfftype_of\x1ePRIVATE_ARGUMENT\xff')
        result = pc.analyze(data)
        self.assertEqual(result['raw_capture_sha256'], hashlib.sha256(data).hexdigest())
        self.assertEqual(result['per_kind_per_thread'][0]['query_kind'], 'type_of')
        self.assertNotIn('PRIVATE', json.dumps(result))
        self.assertEqual(result['capture_flags'], 'unknown')

    def test_binary_unsafe_labels_and_invalid_argument_grammar(self):
        result = pc.analyze(fixture(strings=b'Query\xffprivate::path\x1eSECRET\xff'))
        self.assertEqual(result['unknown_query_name_interval_records'], 1)
        self.assertIsNone(result['per_kind_per_thread'][0]['query_kind'])
        self.assertNotIn('SECRET', json.dumps(result))
        self.assertNotIn('private::path', json.dumps(result))
        with self.assertRaises(ValueError):
            pc.analyze(fixture(strings=b'Query\xfftype_of\x1e\xff'))
        with self.assertRaises(ValueError):
            pc.analyze(b'not a profile', allow_unmapped_event_labels=True)

    def test_strict_default_explicit_unknown_and_limits(self):
        data = fixture(index=b'')
        with self.assertRaises(ValueError):
            pc.analyze(data)
        result = pc.analyze(data, allow_unmapped_event_labels=True)
        self.assertFalse(result['label_coverage_complete'])
        self.assertEqual(result['unknown_query_name_interval_records'], 1)
        self.assertEqual(result['all_query_per_thread'][0]['interval_union_ns'], 6)
        with mock.patch.object(qp, 'MAX_EVENTS', 0):
            with self.assertRaises(ValueError):
                pc.analyze(fixture())
            self.assertEqual(pc.analyze(fixture(), large_capture=True)['query_interval_records'], 1)

    def test_real_binary_cli(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'capture.mm_profdata'
            path.write_bytes(fixture())
            command = [sys.executable, str(Path(pc.__file__)), '--profile', str(path)]
            run = subprocess.run(command, capture_output=True, text=True)
            self.assertEqual(run.returncode, 0, run.stderr)
            self.assertEqual(json.loads(run.stdout)['query_interval_records'], 1)
            self.assertNotIn('arg', json.loads(run.stdout)['per_kind_per_thread'][0]['query_kind'])
            path.write_bytes(fixture(index=b''))
            run = subprocess.run(command, capture_output=True, text=True)
            self.assertEqual(run.returncode, 2)
            self.assertEqual(run.stdout, '')
            run = subprocess.run(command + ['--large-capture', '--allow-unmapped-event-labels'], capture_output=True, text=True)
            self.assertEqual(run.returncode, 0, run.stderr)
            self.assertEqual(json.loads(run.stdout)['unknown_query_name_interval_records'], 1)
            path.write_bytes(b'PRIVATE malformed input')
            run = subprocess.run(command, capture_output=True, text=True)
            self.assertEqual(run.returncode, 2)
            self.assertNotIn('PRIVATE', run.stderr)


if __name__ == '__main__':
    unittest.main()
