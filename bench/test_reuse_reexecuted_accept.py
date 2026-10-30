import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('reexecuted_accept', Path(__file__).with_name('reuse-reexecuted-accept.py'))
accept = importlib.util.module_from_spec(spec)
spec.loader.exec_module(accept)


class ReexecutedAcceptanceTests(unittest.TestCase):
    def record(self, marker, **changes):
        return dict(dict(label=marker, kind='GenericActivity', record_type='interval',
                         event_id=1, thread=0, start_ns=1, end_ns=2), **changes)

    def test_explicit_filter_covers_generic_markers_without_query_intervals(self):
        self.assertEqual(accept.MARKER_FILTER, 'generic-activity,function-args')

    def test_six_labels_and_distinct_eval_always(self):
        self.assertEqual(len(accept.MARKERS), 6)
        for marker in accept.MARKERS:
            summary = accept.inventory([self.record(marker)], lambda _: marker + '\x1etype_of')
            self.assertEqual(summary[marker]['interval_records'], 1)
            self.assertEqual(summary[marker]['query_name_interval_records'], {'type_of': 1})
            for other in set(accept.MARKERS) - {marker}:
                self.assertIsNone(summary[other]['interval_records'])
                self.assertEqual(summary[other]['evidence'], 'unknown-marker-absent')

    def test_repeated_records_are_counts_not_distinct_keys(self):
        marker = accept.MARKERS[0]
        result = accept.inventory([self.record(marker)] * 3, lambda _: marker + '\x1etype_of')
        self.assertEqual(result[marker]['interval_records'], 3)
        self.assertEqual(result[marker]['query_name_interval_records'], {'type_of': 3})
        self.assertNotIn('provider_elapsed_ns', result[marker])

    def test_missing_and_malformed_names_fail_closed(self):
        marker = accept.MARKERS[0]
        for text in (marker, marker + '\x1e', 'wrong\x1etype_of', marker + '\x1etype_of\x1eextra', marker + '\x1ea key'):
            with self.subTest(text=text), self.assertRaisesRegex(AssertionError, 'argument'):
                accept.inventory([self.record(marker)], lambda _: text)

    def test_noninterval_and_wrong_kind_fail_closed(self):
        marker = accept.MARKERS[0]
        for changes in ({'record_type': 'instant'}, {'record_type': 'integer'}, {'kind': 'Query'}):
            with self.assertRaisesRegex(AssertionError, 'GenericActivity interval'):
                accept.inventory([self.record(marker, **changes)], lambda _: marker + '\x1etype_of')

    def test_unknown_markers_do_not_establish_coverage(self):
        result = accept.inventory([self.record('jr_unknown')], lambda _: 'unused')
        self.assertTrue(all(v['interval_records'] is None for v in result.values()))

    def observation(self, name, labels):
        records = [dict(self.record(label), event_id=i) for i, label in enumerate(labels)]
        markers = accept.inventory(records, lambda i: labels[i] + '\x1etype_of')
        return dict(name=name, modes=dict(on=dict(profile=dict(markers=markers))))

    def test_count_gate_requires_both_ordinary_outcomes(self):
        same, different = accept.MARKERS[:2]
        with self.assertRaisesRegex(AssertionError, 'different_value'):
            accept.validate_counts([self.observation('unchanged', [same])])
        result = accept.validate_counts([self.observation('body-edit', [same, different])])
        self.assertEqual(result, {same: 1, different: 1})
        with self.assertRaisesRegex(AssertionError, 'Fresh incremental'):
            accept.validate_counts([self.observation('initial', [same, different])])

    def test_matrix_has_real_edits_error_and_recovery(self):
        cases = accept.cases()
        self.assertEqual([c[0] for c in cases], ['initial', 'unchanged', 'body-edit', 'type-error', 'recovery'])
        self.assertEqual(cases[0][1], cases[1][1])
        self.assertNotEqual(cases[1][1], cases[2][1])
        self.assertIsNone(cases[3][2])
        self.assertEqual(cases[0][1:], cases[-1][1:])

    def test_existing_output_is_untouched(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            sentinel = root / 'acceptance.json'
            sentinel.write_text('keep')
            with self.assertRaises(FileExistsError):
                accept.capture(root / 'missing', root)
            self.assertEqual(sentinel.read_text(), 'keep')

    def test_failed_provenance_is_retained(self):
        with tempfile.TemporaryDirectory() as tmp:
            compiler = Path(tmp) / 'rustc'
            compiler.write_text('identity')
            root = Path(tmp) / 'capture'
            failure = dict(status=1, stdout='', stderr='complete provenance error')
            with patch.object(accept, 'run', return_value=failure):
                with self.assertRaisesRegex(AssertionError, 'provenance'):
                    accept.capture(compiler, root)
            result = json.loads((root / 'acceptance.json').read_text())
            self.assertEqual(result['verdict'], 'failed')
            self.assertEqual(result['compiler']['version']['stderr'], failure['stderr'])
            for field in ('provider_elapsed_ns', 'time_saved_ns', 'inputs_changed', 'provider_avoidable'):
                self.assertIsNone(result[field])


if __name__ == '__main__':
    unittest.main()
