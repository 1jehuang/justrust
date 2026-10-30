"""Offline reuse inventory regression tests, including the public CLI."""
import contextlib
import io
import json
from pathlib import Path
import tempfile
import unittest

import reuse_ceiling as rc
from test_query_profile import fixture


def event(kind='Query', identity=1, typ='interval', **kwargs):
    return dict(kind=kind, event_id=identity, record_type=typ,
                label_resolved=True, start_ns=0, end_ns=100, **kwargs)


def graph(nodes, sha='a', command='same'):
    return dict(compiler_commit='pinned', command_hash_le=command,
                nodes=nodes, sha256=sha)


def node(key, value, kind=1):
    return dict(kind=kind, key_fingerprint=key, value_fingerprint=value)


class ReuseTests(unittest.TestCase):
    def test_overlapping_channels_not_added(self):
        rows = [event(), event('QueryCacheHit', typ='instant'),
                event('QueryCacheHit', typ='instant'),
                event('QueryCacheHitCount', typ='integer', value=2)]
        report = rc.summarize_records(rows)
        self.assertEqual(report['in_memory_hit_instant_records'], 2)
        self.assertEqual(report['in_memory_hit_counter_value_sum'], 2)
        self.assertEqual(report['hit_ids_with_channel_count_disagreement'], 0)
        self.assertIsNone(report['coverage']['request_count'])

    def test_candidates_are_records_not_time_or_semantic_savings(self):
        report = rc.summarize_records([event(), event(), event(identity=2)])
        self.assertEqual(report['hypothetical_repeat_provider_record_candidates'], 1)
        self.assertEqual(report['repeated_provider_ids'], 1)
        for key in ('time_saved_ns', 'cpu_work_ns', 'execution_span_ns', 'theoretical_maximum_reuse'):
            self.assertIsNone(report[key])
        self.assertIsNone(report['coverage']['semantic_avoidable_query_count'])

    def test_empty_capture_retains_unknowns(self):
        report = rc.summarize_records([])
        self.assertEqual(report['records'], 0)
        self.assertIsNone(report['coverage']['capture_complete'])

    def test_missing_provider_is_not_cross_session_reuse(self):
        report = rc.summarize_records([event('QueryCacheHit', typ='instant'),
                                      event('IncrementalLoadResult')])
        self.assertEqual(report['hit_ids_without_observed_provider'], 1)
        self.assertEqual(report['incremental_load_interval_records'], 1)
        self.assertIsNone(report['coverage']['cross_session_reused_query_count'])

    def test_channel_disagreement_and_unresolved_labels(self):
        row = event('QueryCacheHit', typ='instant')
        row['label_resolved'] = False
        report = rc.summarize_records([row, event('QueryCacheHitCount', typ='integer', value=9)])
        self.assertEqual(report['unresolved_label_records'], 1)
        self.assertEqual(report['hit_ids_with_channel_count_disagreement'], 1)

    def test_reject_bad_event_shape_and_duplicate_counter(self):
        for rows in ([event('QueryCacheHit')], [event(identity=100_000_003)],
                     [event('QueryCacheHitCount', typ='integer', value=1)] * 2):
            with self.assertRaises(ValueError):
                rc.summarize_records(rows)

    def test_graph_identity_uses_key_and_kind_not_index(self):
        before = graph({1: node('key', 'same'), 2: node('other', 'old'), 3: node('gone', 'x')})
        after = graph({9: node('key', 'same'), 1: node('other', 'new'), 8: node('added', 'x')}, 'b')
        result = rc.compare_graphs(before, after)
        self.assertEqual(result['shared_query_keys'], 2)
        self.assertEqual(result['shared_keys_matching_value_fingerprint'], 1)
        self.assertEqual(result['shared_keys_changed_value_fingerprint'], 1)
        self.assertEqual(result['before_only_keys'], 1)
        self.assertEqual(result['after_only_keys'], 1)
        self.assertIsNone(result['hypothetical_avoidable_recomputation'])
        self.assertIsNone(result['executed_matching_result_queries'])

    def test_graph_duplicate_key_and_command_mismatch_rejected(self):
        good = graph({1: node('k', 'v')})
        for bad in (graph({2: node('k', 'v'), 3: node('k', 'v')}),
                    graph({}, command='different')):
            with self.assertRaises(ValueError):
                rc.compare_graphs(good, bad)

    def test_identical_snapshot_is_not_asserted_session_reuse(self):
        value = graph({1: node('k', 'v')})
        result = rc.compare_graphs(value, value)
        self.assertTrue(result['identical_graph_bytes'])
        self.assertIsNone(result['observed_cross_session_reuse'])

    def test_public_cli_real_format_and_invalid_input(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'profile'
            path.write_bytes(fixture())
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                rc.main(['--profile', str(path)])
            report = json.loads(output.getvalue())
            self.assertEqual(report['profile']['observed_provider_interval_records'], 1)
            self.assertEqual(len(report['profile_sha256']), 64)
            path.write_bytes(b'invalid')
            with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as err:
                rc.main(['--profile', str(path)])
            self.assertEqual(err.exception.code, 2)

    def test_requires_both_graphs(self):
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as err:
            rc.main(['--profile', 'unused', '--before-graph', 'unused'])
        self.assertEqual(err.exception.code, 2)


if __name__ == '__main__':
    unittest.main()
