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


class FrontendMappingTests(unittest.TestCase):
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
