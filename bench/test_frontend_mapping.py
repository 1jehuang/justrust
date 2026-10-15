import contextlib
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
    assessment = dict(producer=query['producer'], capture_id='test',
                      issues=[],
                      structural_complete=True, wait_structural_complete=True,
                      query_reads={'records_complete': True, 'semantic_complete': False},
                      task_bindings={'graph_binding_complete': False})
    return query, assessment


class FrontendMappingTests(unittest.TestCase):
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
