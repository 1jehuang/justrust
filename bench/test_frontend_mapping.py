import contextlib
import copy
import hashlib
import io
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import frontend_mapping as mapping


def fixture():
    query = dict(schema=4, producer='rustc-query-admission', capture_id='test',
                 expected_waits=2, query_reads={'events': []}, wait_events=[])
    for wait, times in [(1, [0, 4, 6, 10]), (2, [3, 5, 8, 12])]:
        for stage, at in zip(mapping.STAGES, times):
            event = dict(kind=stage, wait_id=wait, at_ns=at)
            if stage == 'notified':
                event['reason'] = 'completed'
            query['wait_events'].append(event)
    assessment = dict(schema=4, producer=query['producer'], capture_id='test',
                      issues=[],
                      structural_complete=True, wait_structural_complete=True,
                      query_reads={'records_complete': True, 'semantic_complete': False},
                      task_bindings={'graph_binding_complete': False})
    return query, assessment


def timing_fixture():
    # These fixtures model assessment consumption, not strict native schemas.
    query, assessment = fixture()
    bridge = dict(query_capture_id='test', pool_capture_id='pool',
                  pool_observer=7, pool_epoch_in_query_ns=0)
    query.update(schema=5, execution_clock=dict(bridge),
                 task_bindings={'pool': {'capture_id': 'pool', 'observer': 7}})
    assessment.update(schema=5, clock_alignment={'aligned': True, 'issues': []})
    pool = dict(schema=3, producer='rustc-thread-pool-join', capture_id='pool',
                observer=7, query_capture_id='test',
                timing={'clock_bridge': dict(bridge), 'observed_cpu_ns': 999999})
    cpu = dict(scope='observed-exclusive-thread-segments', cpu_clock='linux-thread-cpu',
               records_complete=True, observed_cpu_ns=100, task_cpu_ns=80,
               opaque_cpu_ns=20, measured_segments=2, failed_segments=0,
               observed_threads=1, expected_segments=2, lost_segments=0,
               active_segments=0, clock_failures=0, clock_bridge=dict(bridge),
               query_clock_aligned=False, issues=[])
    pool_assessment = dict(schema=3, producer=pool['producer'], capture_id='pool',
                           structural_complete=True, issues=[], observed_cpu=cpu)
    return query, assessment, pool, pool_assessment


def cache_call_fixture():
    query, assessment = fixture()
    reasons = {key: 0 for key in mapping.CACHE_CALL_REASONS}
    reasons['no_cache_hit'] = 1
    assessment['cache_call_provenance'] = dict(
        scope=mapping.CACHE_CALL_SCOPE, records_complete=True,
        observed_requests=2, observed_cache_hits=1, orphan_cache_hits=0,
        resolved_requests=1, unresolved_requests=1,
        distinct_publishing_job_relations=1, lost_events=0,
        unresolved_reasons=reasons,
        relations=[dict(request_id=2, context_id=1, consumer_job_id=2,
                        result_id=1, producer_job_id=1, dep_node_index=42,
                        request_start_event_index=0, publication_event_index=1,
                        cache_hit_event_index=2, request_terminal_event_index=3)])
    return query, assessment


def dispatch_result_fixture():
    query, assessment = fixture()
    query.update(schema=6, execution_clock=None, task_bindings={'pool': None})
    assessment.update(schema=6, clock_alignment={
        'aligned': False, 'issues': ['query execution clock unavailable']})
    reasons = {key: 0 for key in mapping.DISPATCH_RESULT_REASONS}
    reasons['null_dep_node_index'] = 1
    assessment['dispatch_result_provenance'] = dict(
        scope=mapping.DISPATCH_RESULT_SCOPE, records_complete=True,
        observed_requests=2, observed_dispatch_resolutions=2,
        resolved_requests=1, unresolved_requests=1,
        distinct_publishing_job_relations=1, lost_events=0,
        unresolved_reasons=reasons,
        relations=[dict(request_id=2, context_id=1, consumer_job_id=2,
                        result_id=1, producer_job_id=1, dep_node_index=42,
                        request_start_event_index=0, publication_event_index=1,
                        request_terminal_event_index=3)])
    return query, assessment


class FrontendMappingTests(unittest.TestCase):
    def analyze_dispatch_fixture(self, query, authority, cache=False, large=False,
                                 pool=None, pool_authority=None):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            capture = root / 'capture'
            capture.mkdir()
            (capture / 'query.json').write_text(json.dumps(query))
            if pool is not None:
                (capture / 'pool.json').write_text(json.dumps(pool))
            authorities = [authority] + ([pool_authority] if pool_authority else [])
            encoded = json.dumps(authorities, indent=2).replace('\n', '\r\n').encode()
            result = subprocess.CompletedProcess([], 0, encoded, b'')
            artifact = root / 'graph.json'
            with patch.object(mapping.subprocess, 'run', return_value=result) as validator:
                report = mapping.analyze(capture, 'fake-validator', large_capture=large,
                                         cache_call_provenance=cache,
                                         dispatch_result_provenance=True, graph_output=artifact)
            self.assertEqual(artifact.read_bytes(), encoded)
            self.assertEqual(artifact.stat().st_mode & 0o777, 0o600)
            self.assertEqual(report['graph_output']['sha256'], hashlib.sha256(encoded).hexdigest())
            self.assertEqual(report['graph_output']['bytes'], len(encoded))
            self.assertNotIn(str(root), json.dumps(report))
            return report, validator.call_args.args[0]

    def test_dispatch_flag_and_both_flags_store_exact_artifact_with_separate_counts(self):
        for cache in (False, True):
            query, authority = dispatch_result_fixture()
            if cache:
                authority['cache_call_provenance'] = cache_call_fixture()[1]['cache_call_provenance']
            report, command = self.analyze_dispatch_fixture(query, authority, cache=cache, large=True)
            self.assertIn('--dispatch-result-provenance', command)
            self.assertIn('--large-capture', command)
            self.assertEqual('--cache-call-provenance' in command, cache)
            capture, = report['captures']
            self.assertEqual('cache_call_provenance' in capture, cache)
            summary = capture['dispatch_result_provenance']
            self.assertEqual(summary['stored_relation_rows'], 1)
            self.assertEqual(summary['observed_dispatch_resolutions'], 2)
            self.assertNotIn('relations', summary)
            self.assertEqual(summary['unresolved_reasons']['null_dep_node_index'], 1)
            for obj in (report, capture):
                for key in ('cpu_work_ns', 'execution_span_ns', 'theoretical_parallelism'):
                    self.assertIsNone(obj[key])
                self.assertFalse(obj['full_pipeline_complete'])

    def test_root6_requires_dispatch_flag_before_any_validator_or_artifact(self):
        query, _ = dispatch_result_fixture()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            capture = root / 'capture'
            capture.mkdir()
            (capture / 'query.json').write_text(json.dumps(query))
            artifact = root / 'graph.json'
            for cache in (False, True):
                with patch.object(mapping.subprocess, 'run') as validator:
                    with self.assertRaisesRegex(ValueError, 'explicit --dispatch-result-provenance'):
                        mapping.analyze(capture, 'unused', cache_call_provenance=cache,
                                        graph_output=artifact if cache else None)
                    validator.assert_not_called()
                self.assertFalse(artifact.exists())

    def test_dispatch_requires_exact_native_schema6_and_missing_field_fails_closed(self):
        for change in ('source4', 'source5', 'authority4', 'authority5', 'authority-bool', 'missing'):
            query, authority = dispatch_result_fixture()
            if change.startswith('source'):
                query['schema'] = int(change[-1])
                authority['schema'] = query['schema']
            elif change.startswith('authority'):
                authority['schema'] = True if change == 'authority-bool' else int(change[-1])
            else:
                del authority['dispatch_result_provenance']
            with self.subTest(change=change), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                capture = root / 'capture'
                capture.mkdir()
                (capture / 'query.json').write_text(json.dumps(query))
                artifact = root / 'graph.json'
                encoded = json.dumps([authority]).encode()
                with patch.object(mapping.subprocess, 'run', return_value=
                                  subprocess.CompletedProcess([], 0, encoded, b'')):
                    with self.assertRaises(ValueError):
                        mapping.analyze(capture, 'unused', dispatch_result_provenance=True,
                                        graph_output=artifact)
                self.assertEqual(artifact.read_bytes(), encoded)

    def test_dispatch_null_clock_is_explicitly_unaligned_and_does_not_clear_relations(self):
        query, authority = dispatch_result_fixture()
        report, _ = self.analyze_dispatch_fixture(query, authority)
        capture, = report['captures']
        self.assertEqual(capture['clock_alignment'], authority['clock_alignment'])
        self.assertNotIn('observed_cpu', capture)
        self.assertEqual(capture['dispatch_result_provenance']['resolved_requests'], 1)
        for change in ('missing-clock', 'missing-alignment', 'null-alignment', 'forged-aligned'):
            query, authority = dispatch_result_fixture()
            if change == 'missing-clock':
                del query['execution_clock']
            elif change == 'missing-alignment':
                del authority['clock_alignment']
            elif change == 'null-alignment':
                authority['clock_alignment'] = None
            else:
                authority['clock_alignment'] = {'aligned': True, 'issues': []}
            with self.subTest(change=change), self.assertRaises(ValueError):
                self.analyze_dispatch_fixture(query, authority)

    def test_dispatch_clock_bridge_uses_same_exact_pool_authority(self):
        query, authority, pool, pool_authority = timing_fixture()
        query['schema'] = authority['schema'] = 6
        authority['dispatch_result_provenance'] = dispatch_result_fixture()[1]['dispatch_result_provenance']
        report, _ = self.analyze_dispatch_fixture(query, authority, pool=pool,
                                                 pool_authority=pool_authority)
        self.assertEqual(report['captures'][0]['observed_cpu'], pool_authority['observed_cpu'])
        self.assertTrue(report['captures'][0]['clock_alignment']['aligned'])
        pool_authority['observed_cpu']['clock_bridge']['pool_observer'] = 9
        with self.assertRaisesRegex(ValueError, 'qualified clock bridge mismatch'):
            self.analyze_dispatch_fixture(query, authority, pool=pool, pool_authority=pool_authority)

    def test_dispatch_counts_partition_and_typed_fields_fail_closed(self):
        for change in ('row-count', 'request-count', 'reason-count', 'bool-count', 'scope', 'missing-reason'):
            _, authority = dispatch_result_fixture()
            result = authority['dispatch_result_provenance']
            if change == 'row-count':
                result['relations'] = []
            elif change == 'request-count':
                result['observed_requests'] += 1
            elif change == 'reason-count':
                result['unresolved_reasons']['feed_request'] += 1
            elif change == 'bool-count':
                result['observed_dispatch_resolutions'] = True
            elif change == 'scope':
                result['scope'] = 'execution-dag'
            else:
                del result['unresolved_reasons']['cache_hit_request']
            with self.subTest(change=change), self.assertRaises(ValueError):
                mapping._dispatch_result_summary(authority)

    def test_dispatch_partial_orphan_resolution_observations_not_request_coverage(self):
        _, authority = dispatch_result_fixture()
        result = authority['dispatch_result_provenance']
        result.update(records_complete=False, observed_requests=0,
                      observed_dispatch_resolutions=1, resolved_requests=0,
                      unresolved_requests=0, distinct_publishing_job_relations=0,
                      lost_events=1, relations=[])
        result['unresolved_reasons'] = {key: 0 for key in mapping.DISPATCH_RESULT_REASONS}
        summary = mapping._dispatch_result_summary(authority)
        self.assertFalse(summary['records_complete'])
        self.assertEqual(summary['observed_dispatch_resolutions'], 1)
        self.assertEqual(summary['observed_requests'], 0)
        self.assertEqual(summary['stored_relation_rows'], 0)

    def test_dispatch_missing_companion_flags_and_cli_aliases_fail_before_side_effects(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            report, artifact = root / 'report.json', root / 'graph.json'
            for cache in (False, True):
                with patch.object(mapping.subprocess, 'run') as validator:
                    with self.assertRaisesRegex(ValueError, 'companion --graph-output'):
                        mapping.analyze(root, 'unused', dispatch_result_provenance=True,
                                        cache_call_provenance=cache)
                    validator.assert_not_called()
            for extra in ([], ['--graph-output', str(report)]):
                with patch('sys.argv', ['frontend_mapping', '--capture', str(root),
                                       '--dispatch-result-provenance', '--output', str(report), *extra]), \
                        patch.object(mapping, 'analyze') as analyze, \
                        contextlib.redirect_stderr(io.StringIO()):
                    with self.assertRaises(SystemExit) as error:
                        mapping.main()
                    self.assertEqual(error.exception.code, 2)
                    analyze.assert_not_called()
            self.assertFalse(report.exists())
            self.assertFalse(artifact.exists())

    def test_dispatch_cli_forwards_flag_and_preserves_independent_cache_opt_in(self):
        for cache in (False, True):
            query, authority = dispatch_result_fixture()
            if cache:
                authority['cache_call_provenance'] = cache_call_fixture()[1]['cache_call_provenance']
            with tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                capture = root / 'capture'
                capture.mkdir()
                (capture / 'query.json').write_text(json.dumps(query))
                report, artifact = root / 'report.json', root / 'graph.json'
                encoded = json.dumps([authority]).encode()
                extra = ['--cache-call-provenance'] if cache else []
                with patch('sys.argv', ['frontend_mapping', '--capture', str(capture),
                                       '--dispatch-result-provenance', '--graph-output', str(artifact),
                                       '--output', str(report), *extra]), \
                        patch.object(mapping.subprocess, 'run', return_value=
                                     subprocess.CompletedProcess([], 0, encoded, b'')) as validator:
                    mapping.main()
                self.assertIn('--dispatch-result-provenance', validator.call_args.args[0])
                compact = json.loads(report.read_bytes())
                self.assertEqual('cache_call_provenance' in compact['captures'][0], cache)
                self.assertEqual(artifact.read_bytes(), encoded)

    def test_dispatch_artifact_containment_and_no_overwrite_controls(self):
        query, _ = dispatch_result_fixture()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            capture = root / 'capture'
            capture.mkdir()
            source = capture / 'query.json'
            source.write_text(json.dumps(query))
            alias = root / 'alias'
            alias.symlink_to(capture, target_is_directory=True)
            original = root / 'keep.json'
            original.write_bytes(b'preserved')
            link = root / 'graph-link.json'
            link.symlink_to(original)
            for artifact in (source, capture / 'new.json', alias / 'new.json'):
                with patch.object(mapping.subprocess, 'run') as validator, \
                        patch.object(mapping, '_read_regular') as read:
                    with self.assertRaisesRegex(ValueError, 'outside the original capture'):
                        mapping.analyze(capture, 'unused', dispatch_result_provenance=True,
                                        graph_output=artifact)
                    validator.assert_not_called()
                    read.assert_not_called()
            for artifact in (original, link):
                with patch.object(mapping.subprocess, 'run') as validator:
                    with self.assertRaises(FileExistsError):
                        mapping.analyze(capture, 'unused', dispatch_result_provenance=True,
                                        graph_output=artifact)
                    validator.assert_not_called()
            self.assertEqual(original.read_bytes(), b'preserved')

    def test_dispatch_partial_root_issues_preserve_artifact_without_compact_claim(self):
        query, authority = dispatch_result_fixture()
        authority['issues'] = ['query read records are incomplete or invalid']
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            capture = root / 'capture'
            capture.mkdir()
            (capture / 'query.json').write_text(json.dumps(query))
            artifact = root / 'graph.json'
            encoded = json.dumps([authority]).encode()
            with patch.object(mapping.subprocess, 'run', return_value=
                              subprocess.CompletedProcess([], 0, encoded, b'')):
                with self.assertRaisesRegex(ValueError, 'native query assessment has issues'):
                    mapping.analyze(capture, 'unused', dispatch_result_provenance=True,
                                    graph_output=artifact)
            self.assertEqual(artifact.read_bytes(), encoded)

    def test_default_ignores_unsolicited_dispatch_authority_and_does_not_forward_flag(self):
        query, authority = fixture()
        authority['dispatch_result_provenance'] = dispatch_result_fixture()[1]['dispatch_result_provenance']
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / 'query.json').write_text(json.dumps(query))
            with patch.object(mapping.subprocess, 'run', return_value=
                              subprocess.CompletedProcess([], 0, json.dumps([authority]), '')) as validator:
                report = mapping.analyze(root, 'unused')
            self.assertNotIn('--dispatch-result-provenance', validator.call_args.args[0])
            self.assertNotIn('dispatch_result_provenance', report['captures'][0])
            self.assertNotIn('graph_output', report)

    def test_cache_calls_forward_flag_and_store_exact_full_native_stdout(self):
        query, authority = cache_call_fixture()
        # Preserve CRLF and whitespace exactly rather than text-mode newline
        # normalization or reserialization. Native rows live in this artifact.
        encoded = ('  ' + json.dumps([authority], indent=2) + '\n').replace('\n', '\r\n').encode()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            capture = root / 'capture'
            capture.mkdir()
            (capture / 'query.json').write_text(json.dumps(query))
            artifact = root / 'private-native-graph.json'
            result = subprocess.CompletedProcess([], 0, encoded, b'')
            with patch.object(mapping.subprocess, 'run', return_value=result) as validator:
                report = mapping.analyze(capture, 'fake-validator', large_capture=True,
                                         cache_call_provenance=True, graph_output=artifact)
            command = validator.call_args.args[0]
            self.assertIn('--cache-call-provenance', command)
            self.assertIn('--large-capture', command)
            self.assertFalse(validator.call_args.kwargs['text'])
            self.assertEqual(artifact.read_bytes(), encoded)
            self.assertEqual(artifact.stat().st_mode & 0o777, 0o600)
            self.assertEqual(report['graph_output'], dict(
                name=artifact.name, bytes=len(encoded), sha256=hashlib.sha256(encoded).hexdigest(),
                format='justrust-graph-json-stdout'))
            summary = report['captures'][0]['cache_call_provenance']
            self.assertNotIn('relations', summary)
            self.assertEqual(summary['stored_relation_rows'], 1)
            self.assertEqual(summary['resolved_requests'], 1)
            self.assertEqual(summary['unresolved_requests'], 1)
            self.assertEqual(json.loads(artifact.read_bytes())[0]['cache_call_provenance']['relations'],
                             authority['cache_call_provenance']['relations'])
            self.assertNotIn(str(root), json.dumps(report))
            for obj in (report, report['captures'][0]):
                for key in ('cpu_work_ns', 'execution_span_ns', 'theoretical_parallelism'):
                    self.assertIsNone(obj[key])

    def test_cli_cache_calls_write_compact_report_and_exclusive_full_artifact(self):
        query, authority = cache_call_fixture()
        encoded = json.dumps([authority], indent=2).encode()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            capture = root / 'capture'
            capture.mkdir()
            (capture / 'query.json').write_text(json.dumps(query))
            report = root / 'report.json'
            artifact = root / 'graph.json'
            result = subprocess.CompletedProcess([], 0, encoded, b'')
            with patch('sys.argv', ['frontend_mapping', '--capture', str(capture),
                                   '--cache-call-provenance', '--graph-output', str(artifact),
                                   '--output', str(report)]), \
                    patch.object(mapping.subprocess, 'run', return_value=result):
                mapping.main()
            compact = json.loads(report.read_bytes())
            self.assertEqual(artifact.read_bytes(), encoded)
            self.assertEqual(report.stat().st_mode & 0o777, 0o600)
            self.assertEqual(compact['graph_output']['sha256'], hashlib.sha256(encoded).hexdigest())
            self.assertNotIn('relations', compact['captures'][0]['cache_call_provenance'])

    def test_cache_call_flag_and_graph_output_must_be_paired(self):
        with tempfile.TemporaryDirectory() as temporary:
            artifact = Path(temporary) / 'graph.json'
            for flag, output in [(True, None), (False, artifact)]:
                with self.subTest(flag=flag), patch.object(mapping.subprocess, 'run') as validator:
                    with self.assertRaisesRegex(ValueError, 'companion --graph-output'):
                        mapping.analyze(temporary, 'unused', cache_call_provenance=flag,
                                        graph_output=output)
                    validator.assert_not_called()
            self.assertFalse(artifact.exists())

    def test_default_ignores_unsolicited_native_calls_without_artifact_side_effects(self):
        query, authority = cache_call_fixture()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / 'query.json').write_text(json.dumps(query))
            result = subprocess.CompletedProcess([], 0, json.dumps([authority]), '')
            with patch.object(mapping.subprocess, 'run', return_value=result) as validator:
                report = mapping.analyze(root, 'fake-validator')
            self.assertNotIn('--cache-call-provenance', validator.call_args.args[0])
            self.assertTrue(validator.call_args.kwargs['text'])
            self.assertNotIn('graph_output', report)
            self.assertNotIn('cache_call_provenance', report['captures'][0])
            self.assertEqual(sorted(p.name for p in root.iterdir()), ['query.json'])

    def test_missing_native_cache_call_opt_in_field_fails_closed_but_retains_artifact(self):
        query, authority = fixture()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            capture = root / 'capture'
            capture.mkdir()
            (capture / 'query.json').write_text(json.dumps(query))
            artifact = root / 'full-graph.json'
            encoded = json.dumps([authority]).encode()
            result = subprocess.CompletedProcess([], 0, encoded, b'')
            with patch.object(mapping.subprocess, 'run', return_value=result):
                with self.assertRaisesRegex(ValueError, 'native cache-call provenance authority'):
                    mapping.analyze(capture, 'fake-validator', cache_call_provenance=True,
                                    graph_output=artifact)
            self.assertEqual(artifact.read_bytes(), encoded)

    def test_cache_call_artifact_never_overwrites_files_or_symlinks(self):
        query, _ = cache_call_fixture()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / 'query.json').write_text(json.dumps(query))
            original = root / 'preserved.txt'
            original.write_bytes(b'keep')
            link = root / 'link.json'
            link.symlink_to(original)
            dangling = root / 'dangling.json'
            dangling.symlink_to(root / 'missing')
            for artifact in (original, link, dangling):
                with self.subTest(artifact=artifact.name), \
                        patch.object(mapping.subprocess, 'run') as validator:
                    # Keep these non-receipt artifacts out of the capture source.
                    capture = root / 'capture'
                    capture.mkdir(exist_ok=True)
                    (capture / 'query.json').write_text(json.dumps(query))
                    with self.assertRaises(FileExistsError):
                        mapping.analyze(capture, 'fake-validator', cache_call_provenance=True,
                                        graph_output=artifact)
                    validator.assert_not_called()
            self.assertEqual(original.read_bytes(), b'keep')
            self.assertFalse((root / 'missing').exists())

    def test_graph_output_must_not_contaminate_capture_or_resolve_to_source(self):
        query, _ = cache_call_fixture()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            capture = root / 'capture'
            capture.mkdir()
            original = json.dumps(query).encode()
            source = capture / 'query.json'
            source.write_bytes(original)
            alias = root / 'capture-alias'
            alias.symlink_to(capture, target_is_directory=True)
            source_alias = root / 'source-alias.json'
            source_alias.symlink_to(source)
            before = sorted(str(p.relative_to(root)) for p in root.rglob('*'))
            for artifact in (capture, source, capture / 'graph.json',
                             capture / 'new-subdirectory' / 'graph.json',
                             alias / 'graph.json', source_alias,
                             root / 'capture' / '..' / 'capture' / 'graph.json'):
                with self.subTest(artifact=str(artifact)), \
                        patch.object(mapping.subprocess, 'run') as validator, \
                        patch.object(mapping, '_read_regular') as read:
                    with self.assertRaisesRegex(ValueError, 'outside the original capture'):
                        mapping.analyze(capture, 'fake-validator', cache_call_provenance=True,
                                        graph_output=artifact)
                    validator.assert_not_called()
                    read.assert_not_called()
            self.assertEqual(sorted(str(p.relative_to(root)) for p in root.rglob('*')), before)
            self.assertEqual(source.read_bytes(), original)

    def test_failed_native_graph_stdout_is_retained_without_valid_summary(self):
        query, _ = fixture()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            capture = root / 'capture'
            capture.mkdir()
            (capture / 'query.json').write_text(json.dumps(query))
            artifact = root / 'graph.json'
            result = subprocess.CompletedProcess([], 1, b'partial native stdout\r\n',
                                                 b'strict native rejection')
            with patch.object(mapping.subprocess, 'run', return_value=result):
                with self.assertRaisesRegex(ValueError, 'strict native rejection'):
                    mapping.analyze(capture, 'fake-validator', cache_call_provenance=True,
                                    graph_output=artifact)
            self.assertEqual(artifact.read_bytes(), result.stdout)

    def test_partial_native_cache_call_counters_and_reasons_are_not_promoted(self):
        _, authority = cache_call_fixture()
        calls = authority['cache_call_provenance']
        calls['records_complete'] = False
        calls['lost_events'] = 4
        calls['unresolved_reasons']['no_cache_hit'] = 0
        calls['unresolved_reasons']['capture_incomplete'] = 1
        summary = mapping._cache_call_summary(authority)
        self.assertFalse(summary['records_complete'])
        self.assertEqual(summary['lost_events'], 4)
        self.assertEqual(summary['unresolved_reasons']['capture_incomplete'], 1)
        self.assertEqual(summary['resolved_requests'], 1)
        self.assertEqual(summary['stored_relation_rows'], 1)

    def test_partial_root_issues_reject_summary_but_preserve_full_native_report(self):
        query, authority = cache_call_fixture()
        authority['issues'] = ['query read records are incomplete or invalid']
        authority['structural_complete'] = False
        authority['cache_call_provenance']['records_complete'] = False
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            capture = root / 'capture'
            capture.mkdir()
            (capture / 'query.json').write_text(json.dumps(query))
            artifact = root / 'graph.json'
            encoded = json.dumps([authority]).encode()
            result = subprocess.CompletedProcess([], 0, encoded, b'')
            with patch.object(mapping.subprocess, 'run', return_value=result):
                with self.assertRaisesRegex(ValueError, 'native query assessment has issues'):
                    mapping.analyze(capture, 'fake-validator', cache_call_provenance=True,
                                    graph_output=artifact)
            self.assertEqual(artifact.read_bytes(), encoded)

    def test_cache_call_counter_partition_and_typed_fields_fail_closed(self):
        for change in ('row-count', 'request-count', 'reason-count', 'bool-count', 'scope'):
            with self.subTest(change=change):
                _, authority = cache_call_fixture()
                calls = authority['cache_call_provenance']
                if change == 'row-count':
                    calls['relations'] = []
                elif change == 'request-count':
                    calls['observed_requests'] += 1
                elif change == 'reason-count':
                    calls['unresolved_reasons']['feed_request'] += 1
                elif change == 'bool-count':
                    calls['resolved_requests'] = True
                else:
                    calls['scope'] = 'execution-dag'
                with self.assertRaises(ValueError):
                    mapping._cache_call_summary(authority)

    def test_cli_cache_artifact_report_aliases_and_existing_reports_fail_before_analysis(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            report = root / 'report.json'
            artifact = root / 'graph.json'
            for extra in (['--cache-call-provenance'], ['--graph-output', str(artifact)]):
                with patch('sys.argv', ['frontend_mapping', '--capture', temporary,
                                       '--output', str(report), *extra]), \
                        patch.object(mapping, 'analyze') as analyze, \
                        contextlib.redirect_stderr(io.StringIO()):
                    with self.assertRaises(SystemExit) as error:
                        mapping.main()
                    self.assertEqual(error.exception.code, 2)
                    analyze.assert_not_called()
            for args in [('--output', str(report), '--graph-output', str(report)),
                         ('--output', str(report), '--graph-output', str(root / './report.json'))]:
                with patch('sys.argv', ['frontend_mapping', '--capture', temporary,
                                       '--cache-call-provenance', *args]), \
                        patch.object(mapping, 'analyze') as analyze, \
                        contextlib.redirect_stderr(io.StringIO()):
                    with self.assertRaises(SystemExit) as error:
                        mapping.main()
                    self.assertEqual(error.exception.code, 2)
                    analyze.assert_not_called()
            report.write_text('keep')
            with patch('sys.argv', ['frontend_mapping', '--capture', temporary,
                                   '--cache-call-provenance', '--graph-output', str(artifact),
                                   '--output', str(report)]), \
                    patch.object(mapping, 'analyze') as analyze, \
                    contextlib.redirect_stderr(io.StringIO()):
                with self.assertRaises(SystemExit):
                    mapping.main()
                analyze.assert_not_called()
            self.assertEqual(report.read_text(), 'keep')
            self.assertFalse(artifact.exists())

    def analyze_timing_fixture(self, query, assessment, pool, pool_assessment):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / 'query.json').write_text(json.dumps(query))
            if pool is not None:
                (root / 'pool.json').write_text(json.dumps(pool))
            authorities = [assessment]
            if pool_assessment is not None:
                authorities.extend(pool_assessment if isinstance(pool_assessment, list)
                                   else [pool_assessment])
            result = subprocess.CompletedProcess([], 0, json.dumps(authorities), '')
            with patch.object(mapping.subprocess, 'run', return_value=result):
                return mapping.analyze(root, 'fake-validator')

    def test_query5_copies_only_public_timing_and_preserves_unknown_full_cost(self):
        query, authority, pool, pool_authority = timing_fixture()
        report = self.analyze_timing_fixture(query, authority, pool, pool_authority)
        capture, = report['captures']
        self.assertEqual(capture['clock_alignment'], authority['clock_alignment'])
        self.assertEqual(capture['observed_cpu'], pool_authority['observed_cpu'])
        self.assertEqual(capture['observed_cpu']['observed_cpu_ns'], 100)
        self.assertFalse(capture['observed_cpu']['query_clock_aligned'])
        for item in (report, capture):
            for key in ('cpu_work_ns', 'execution_span_ns', 'theoretical_parallelism'):
                self.assertIsNone(item[key])
            self.assertFalse(item['full_pipeline_complete'])
        expected_bytes = json.dumps(query).encode()
        original = next(item for item in report['inputs'] if item['name'] == 'query.json')
        self.assertEqual(original['sha256'], hashlib.sha256(expected_bytes).hexdigest())
        self.assertEqual(original['bytes'], len(expected_bytes))

    def test_schema5_default_report_keeps_pre_dispatch_golden_bytes(self):
        query, authority, pool, pool_authority = timing_fixture()
        report = self.analyze_timing_fixture(query, authority, pool, pool_authority)
        encoded = (json.dumps(report, sort_keys=True, indent=2) + '\n').encode()
        # Independently recomputed with committed pre-dispatch analyzer 92f2bd143.
        self.assertEqual(hashlib.sha256(encoded).hexdigest(),
                         'e4f1782434fb375ffc494804555772958800b91d27d740bbf41327058b1d4fba')

    def test_forged_raw_clock_descriptor_does_not_upgrade_native_alignment(self):
        query, authority, pool, pool_authority = timing_fixture()
        authority['clock_alignment'] = {'aligned': False, 'issues': ['clock unavailable']}
        query['execution_clock']['pool_observer'] = 999
        capture, = self.analyze_timing_fixture(query, authority, pool, pool_authority)['captures']
        self.assertEqual(capture['clock_alignment'], authority['clock_alignment'])
        self.assertFalse(capture['observed_cpu']['query_clock_aligned'])

    def test_query5_requires_native_schema5_and_alignment_assessment(self):
        for change in ('schema4', 'missing', 'null', 'truthy', 'contradictory'):
            with self.subTest(change=change):
                query, authority, pool, pool_authority = timing_fixture()
                if change == 'schema4':
                    authority['schema'] = 4
                elif change == 'missing':
                    del authority['clock_alignment']
                elif change == 'null':
                    authority['clock_alignment'] = None
                elif change == 'truthy':
                    authority['clock_alignment']['aligned'] = 1
                else:
                    authority['clock_alignment']['issues'] = ['not aligned']
                with self.assertRaises(ValueError):
                    self.analyze_timing_fixture(query, authority, pool, pool_authority)

    def test_unavailable_clock_and_pool_remain_unknown_without_invented_cpu(self):
        query, authority, _, _ = timing_fixture()
        query['execution_clock'] = None
        query['task_bindings']['pool'] = None
        authority['clock_alignment'] = {'aligned': False, 'issues': ['clock unavailable']}
        capture, = self.analyze_timing_fixture(query, authority, None, None)['captures']
        self.assertEqual(capture['clock_alignment'], authority['clock_alignment'])
        self.assertNotIn('observed_cpu', capture)
        authority['clock_alignment'] = {'aligned': True, 'issues': []}
        with self.assertRaisesRegex(ValueError, 'no matching CPU pool'):
            self.analyze_timing_fixture(query, authority, None, None)

    def test_partial_and_unmeasurable_cpu_stays_as_public_native_assessment(self):
        query, authority, pool, pool_authority = timing_fixture()
        cpu = pool_authority['observed_cpu']
        cpu.update(records_complete=False, observed_cpu_ns=None, task_cpu_ns=None,
                   opaque_cpu_ns=None, failed_segments=1, lost_segments=2,
                   clock_failures=1, issues=['CPU task/thread authority unavailable'])
        capture, = self.analyze_timing_fixture(query, authority, pool, pool_authority)['captures']
        self.assertEqual(capture['observed_cpu'], cpu)
        self.assertIsNone(capture['cpu_work_ns'])

    def test_foreign_cpu_clock_bridge_components_are_rejected(self):
        for field, value in [('query_capture_id', 'foreign'), ('pool_capture_id', 'foreign'),
                             ('pool_observer', 8), ('pool_epoch_in_query_ns', 1),
                             ('pool_epoch_in_query_ns', False)]:
            with self.subTest(field=field):
                query, authority, pool, pool_authority = timing_fixture()
                pool_authority['observed_cpu']['clock_bridge'][field] = value
                with self.assertRaisesRegex(ValueError, 'qualified clock bridge mismatch'):
                    self.analyze_timing_fixture(query, authority, pool, pool_authority)

    def test_query_pool_reference_requires_exact_query_and_observer(self):
        for field in ('query', 'observer'):
            with self.subTest(field=field):
                query, authority, pool, pool_authority = timing_fixture()
                if field == 'query':
                    pool['query_capture_id'] = 'foreign'
                    pool_authority['observed_cpu']['clock_bridge']['query_capture_id'] = 'foreign'
                else:
                    query['task_bindings']['pool']['observer'] = 8
                with self.assertRaisesRegex(ValueError, 'qualified identity mismatch'):
                    self.analyze_timing_fixture(query, authority, pool, pool_authority)

    def test_duplicate_missing_foreign_and_legacy_cpu_authorities_are_rejected(self):
        for change in ('duplicate', 'missing', 'foreign', 'schema2', 'missing-source',
                       'missing-cpu'):
            with self.subTest(change=change):
                query, authority, pool, pool_authority = timing_fixture()
                if change == 'duplicate':
                    pool_authority = [pool_authority, copy.deepcopy(pool_authority)]
                elif change == 'missing':
                    pool_authority = None
                elif change == 'foreign':
                    pool_authority['capture_id'] = 'foreign'
                elif change == 'schema2':
                    pool_authority['schema'] = 2
                elif change == 'missing-cpu':
                    del pool_authority['observed_cpu']
                else:
                    pool = None
                with self.assertRaises(ValueError):
                    self.analyze_timing_fixture(query, authority, pool, pool_authority)

    def test_skipped_pool3_source_is_validated_under_neutral_name(self):
        query, authority, pool, pool_authority = timing_fixture()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / 'query.json').write_text(json.dumps(query))
            original = json.dumps(pool).encode()
            (root / 'pool.manifest.json').write_bytes(original)
            def validate(command, **kwargs):
                snapshot = Path(command[2])
                self.assertFalse((snapshot / 'pool.manifest.json').exists())
                pools = [p for p in snapshot.glob('*.json')
                         if json.loads(p.read_bytes())['producer'] == pool['producer']]
                self.assertEqual(len(pools), 1)
                self.assertEqual(pools[0].read_bytes(), original)
                return subprocess.CompletedProcess(command, 0,
                                                   json.dumps([authority, pool_authority]), '')
            with patch.object(mapping.subprocess, 'run', side_effect=validate):
                report = mapping.analyze(root, 'fake-validator')
            self.assertEqual(report['captures'][0]['observed_cpu'], pool_authority['observed_cpu'])
            original_input = next(item for item in report['inputs']
                                  if item['name'] == 'pool.manifest.json')
            self.assertEqual(original_input['sha256'], hashlib.sha256(original).hexdigest())

    def test_duplicate_skipped_pool3_source_cannot_borrow_authority(self):
        query, _, pool, _ = timing_fixture()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / 'query.json').write_text(json.dumps(query))
            (root / 'pool.json').write_text(json.dumps(pool))
            pool['timing']['observed_cpu_ns'] = 123456789
            (root / 'pool.envelope.json').write_text(json.dumps(pool))
            with patch.object(mapping.subprocess, 'run') as validator:
                with self.assertRaisesRegex(ValueError, 'duplicate CPU pool source'):
                    mapping.analyze(root, 'fake-validator')
                validator.assert_not_called()

    def test_legacy_query_report_keeps_original_fields_without_timing(self):
        query, authority = fixture()
        # Optional foreign timing claims on a legacy assessment are not consumed.
        authority['clock_alignment'] = {'aligned': True, 'issues': []}
        report = self.analyze_timing_fixture(query, authority, None, None)
        capture, = report['captures']
        self.assertEqual(capture, mapping.summarize(query, authority))
        self.assertNotIn('clock_alignment', capture)
        self.assertNotIn('observed_cpu', capture)
        encoded = (json.dumps(report, sort_keys=True, indent=2) + '\n').encode()
        # Golden derived independently using committed pre-timing analyzer
        # 46128dfdd and the same fixture with mocked public schema4 validation.
        self.assertEqual(hashlib.sha256(encoded).hexdigest(),
                         'a21b3148b38491d16e4886dfc035e0a24d74c1cb713e617d8e05bf535bcc7302')

    def test_overlaps_are_not_added_as_wall_or_cpu_time(self):
        summary = mapping.intervals_summary([(0, 10), (3, 12), (12, 12)])
        self.assertEqual(summary['sum_elapsed_ns'], 19)
        self.assertEqual(summary['union_elapsed_ns'], 12)
        self.assertEqual(summary['peak_overlapping_intervals'], 2)
        self.assertEqual(summary['count'], 3)
        self.assertEqual(mapping.intervals_summary([])['max_elapsed_ns'], 0)
        with self.assertRaises(ValueError):
            mapping.intervals_summary([(3, 2)])

    def test_wait_stages_and_unknown_work_span(self):
        query, assessment = fixture()
        report = mapping.summarize(query, assessment)
        self.assertEqual(report['complete_observed_waits'], 2)
        self.assertEqual(report['query_clock_wait_intervals']['total']['union_elapsed_ns'], 12)
        self.assertEqual(report['query_clock_wait_intervals']['post_condvar']['sum_elapsed_ns'], 8)
        for name in ('cpu_work_ns', 'execution_span_ns', 'theoretical_parallelism'):
            self.assertIsNone(report[name])
        self.assertFalse(report['full_pipeline_complete'])
        self.assertFalse(report['task_binding_coverage']['graph_binding_complete'])
        assessment['capture_id'] = 'foreign'
        with self.assertRaises(ValueError):
            mapping.summarize(query, assessment)

    def test_partial_wait_is_not_a_complete_interval(self):
        query, assessment = fixture()
        query['wait_events'].pop()
        assessment['wait_structural_complete'] = False
        report = mapping.summarize(query, assessment)
        self.assertEqual(report['complete_observed_waits'], 1)
        self.assertFalse(report['waits_complete'])

    def test_invalid_wait_stage_order_is_rejected(self):
        query, assessment = fixture()
        query['wait_events'][1]['at_ns'] = 15
        with self.assertRaises(ValueError):
            mapping.summarize(query, assessment)

    def test_duplicate_wait_stage_never_overwrites_evidence(self):
        query, assessment = fixture()
        query['wait_events'].append(dict(query['wait_events'][0]))
        with self.assertRaisesRegex(ValueError, 'duplicate wait lifecycle'):
            mapping.summarize(query, assessment)

    def test_successful_native_assessment_with_issues_is_not_authority(self):
        query, assessment = fixture()
        # Native graph returns success for these rejected lifecycle properties.
        # Otherwise-monotonic timestamps must not bypass the native verdict.
        for issue in ('wait 1 has duplicate lifecycle stage 0',
                      'wait 1 has foreign or unadmitted endpoint',
                      'wait 1 registration precedes endpoint admission',
                      'wait 1 completion notification disagrees with terminal'):
            with self.subTest(issue=issue), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                (root / 'query.json').write_text(json.dumps(query))
                rejected = dict(assessment, issues=[issue], wait_structural_complete=False)
                result = subprocess.CompletedProcess([], 0, json.dumps([rejected]), '')
                with patch.object(mapping.subprocess, 'run', return_value=result):
                    with self.assertRaisesRegex(ValueError, 'native query assessment has issues'):
                        mapping.analyze(root, 'fake-validator')

    def test_missing_native_issue_authority_is_rejected(self):
        query, assessment = fixture()
        del assessment['issues']
        with self.assertRaisesRegex(ValueError, 'missing issue authority'):
            mapping.summarize(query, assessment)

    def test_snapshot_is_the_original_validated_bytes(self):
        # Mocked validator tests snapshot mechanics only, not schema correctness.
        # Real public CLI + actual compiler captures exercise schema acceptance.
        query, assessment = fixture()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / 'query.json'
            source.write_text(json.dumps(query))
            original = source.read_bytes()
            def validate(command, **kwargs):
                snapshot, = Path(command[2]).glob('*.json')
                self.assertEqual(snapshot.read_bytes(), original)
                self.assertEqual(snapshot.stat().st_mode & 0o777, 0o400)
                source.write_text('{"changed":true}')
                return subprocess.CompletedProcess(command, 0, json.dumps([assessment]), '')
            with patch.object(mapping.subprocess, 'run', side_effect=validate):
                report = mapping.analyze(root, 'fake-validator')
            self.assertEqual(report['captures'][0]['capture_id'], 'test')
            self.assertEqual(report['inputs'][0]['bytes'], len(original))

    def test_skipped_query_filenames_are_neutrally_routed(self):
        # Native graph skips these names before strict query parsing. The mock
        # models routing only, not native receipt schema validation.
        skipped = ['a.manifest.json', 'a.declaration.json', 'a.execute-entered.json',
                   'a.execute-result.json', 'compiler-owner-a.event-1.json',
                   'a.envelope.json']
        query, assessment = fixture()
        for name in skipped:
            with self.subTest(name=name), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                original = json.dumps(query).encode()
                (root / name).write_bytes(original)
                # Ensure canonical name allocation cannot collide with an
                # unrelated original document or change its native routing.
                auxiliary = root / 'frontend-query-0.json'
                auxiliary.write_text('{"producer":"auxiliary"}')
                def validate(command, **kwargs):
                    snapshot = Path(command[2])
                    self.assertFalse((snapshot / name).exists())
                    self.assertEqual((snapshot / auxiliary.name).read_bytes(),
                                     auxiliary.read_bytes())
                    candidates = [p for p in snapshot.glob('*.json')
                                  if p.name != auxiliary.name]
                    self.assertEqual(len(candidates), 1)
                    self.assertTrue(candidates[0].name.startswith('frontend-query-'))
                    self.assertEqual(candidates[0].read_bytes(), original)
                    return subprocess.CompletedProcess(command, 0, json.dumps([assessment]), '')
                with patch.object(mapping.subprocess, 'run', side_effect=validate):
                    report = mapping.analyze(root, 'fake-validator')
                self.assertEqual(len(report['captures']), 1)
                self.assertIn(name, [item['name'] for item in report['inputs']])

    def test_skipped_duplicate_query_cannot_borrow_validated_identity(self):
        query, _ = fixture()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / 'query.json').write_text(json.dumps(query))
            query['wait_events'][0]['at_ns'] = 999
            (root / 'forged.manifest.json').write_text(json.dumps(query))
            with patch.object(mapping.subprocess, 'run') as validator:
                with self.assertRaisesRegex(ValueError, 'duplicate query source'):
                    mapping.analyze(root, 'fake-validator')
                validator.assert_not_called()

    def test_query_sources_and_validated_authorities_must_match_exactly(self):
        query, assessment = fixture()
        foreign = dict(assessment, capture_id='foreign')
        for authorities in ([], [foreign], [assessment, foreign], [assessment, assessment]):
            with self.subTest(authorities=authorities), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                (root / 'query.json').write_text(json.dumps(query))
                result = subprocess.CompletedProcess([], 0, json.dumps(authorities), '')
                with patch.object(mapping.subprocess, 'run', return_value=result):
                    with self.assertRaisesRegex(ValueError, 'authority|mismatch'):
                        mapping.analyze(root, 'fake-validator')

    def test_large_capture_forwards_native_opt_in_and_preserves_unknowns(self):
        query, assessment = fixture()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / 'query.json').write_text(json.dumps(query))
            result = subprocess.CompletedProcess([], 0, json.dumps([assessment]), '')
            with patch.object(mapping, 'MAX_BYTES', 1), \
                    patch.object(mapping.subprocess, 'run', return_value=result) as validator:
                with self.assertRaises(ValueError):
                    mapping.analyze(root, 'fake-validator')
                validator.assert_not_called()
                report = mapping.analyze(root, 'fake-validator', large_capture=True)
                self.assertEqual(validator.call_args.args[0][-1], '--large-capture')
            self.assertTrue(report['large_capture'])
            self.assertFalse(report['full_pipeline_complete'])
            for key in ('cpu_work_ns', 'execution_span_ns', 'theoretical_parallelism'):
                self.assertIsNone(report[key])

    def test_invalid_validator_result_does_not_get_analyzed(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / 'bad.json').write_text('{}')
            failed = subprocess.CompletedProcess([], 1, 'bad record', 'strict rejection')
            with patch.object(mapping.subprocess, 'run', return_value=failed):
                with self.assertRaisesRegex(ValueError, 'bad record.*\nstrict rejection'):
                    mapping.analyze(root, 'fake-validator')

    def test_empty_symlink_and_nonregular_inputs_are_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            directory = root / 'capture'
            directory.mkdir()
            with self.assertRaises(ValueError):
                mapping.analyze(directory, 'unused')
            alias = root / 'alias'
            alias.symlink_to(directory, target_is_directory=True)
            with self.assertRaises(ValueError):
                mapping.analyze(alias, 'unused')
            data = root / 'source'
            data.write_text('{}')
            (directory / 'receipt.json').symlink_to(data)
            with self.assertRaises(OSError):
                mapping.analyze(directory, 'unused')
            (directory / 'receipt.json').unlink()
            os.mkfifo(directory / 'pipe.json')
            with self.assertRaises(ValueError):
                mapping.analyze(directory, 'unused')

    def test_file_and_total_byte_bounds(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / 'a.json').write_bytes(b'{}')
            (root / 'b.json').write_bytes(b'{}')
            with patch.object(mapping, 'MAX_BYTES', 3):
                with self.assertRaises(ValueError):
                    mapping.analyze(root, 'unused')
            with patch.object(mapping, 'MAX_FILES', 1):
                with self.assertRaises(ValueError):
                    mapping.analyze(root, 'unused')

    def test_non_json_entries_count_toward_directory_budget(self):
        query, assessment = fixture()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / 'query.json').write_text(json.dumps(query))
            (root / 'unrelated.txt').write_text('not receipt bytes')
            result = subprocess.CompletedProcess([], 0, json.dumps([assessment]), '')
            with patch.object(mapping, 'MAX_FILES', 1), \
                    patch.object(mapping.subprocess, 'run') as validator:
                with self.assertRaisesRegex(ValueError, 'directory entry budget'):
                    mapping.analyze(root, 'fake-validator')
                validator.assert_not_called()
            with patch.object(mapping, 'MAX_FILES', 2), \
                    patch.object(mapping.subprocess, 'run', return_value=result):
                report = mapping.analyze(root, 'fake-validator')
            self.assertEqual(len(report['inputs']), 1)

    def test_cli_never_overwrites_existing_output(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / 'report.json'
            output.write_text('keep')
            with patch('sys.argv', ['frontend_mapping', '--capture', temporary,
                                    '--output', str(output)]), \
                    patch.object(mapping, 'analyze', return_value={}), \
                    contextlib.redirect_stderr(io.StringIO()):
                with self.assertRaises(SystemExit) as error:
                    mapping.main()
            self.assertEqual(error.exception.code, 2)
            self.assertEqual(output.read_text(), 'keep')


if __name__ == '__main__':
    unittest.main()
