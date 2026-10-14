"""All-event elapsed accounting, without CPU or execution-span inference."""
import contextlib
import hashlib
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest import mock

import query_activities as qa
import query_profile as qp
from test_query_mapping import paired_fixture


def event(start=0, end=10, kind='Query', label='outer', thread=0):
    return dict(record_type='interval', start_ns=start, end_ns=end,
                kind=kind, label=label, thread=thread)


class ActivityTests(unittest.TestCase):
    def test_nested_cross_thread_adjacent_and_gaps(self):
        records = [event(), event(2, 8, 'Unknown', 'child'),
                   event(3, 5, 'LLVM', None), event(10, 12),
                   event(15, 16), event(1, 7, thread=1)]
        result = qa.summarize_events(records)
        self.assertEqual(result, qa.summarize_events(iter(records)))
        rows = {(r['kind'], r['label']): r for r in result['rows']}
        self.assertEqual(rows['Unknown', 'child']['exclusive_elapsed_ns'], 4)
        self.assertEqual(rows['LLVM', None]['exclusive_elapsed_ns'], 2)
        self.assertEqual(result['covered_elapsed_ns'], 19)
        self.assertEqual(sum(r['exclusive_elapsed_ns'] for r in result['rows']), 19)
        self.assertEqual(result['threads'][0]['gap_elapsed_ns'], 3)
        self.assertEqual(result['observation_window_ns'], 16)
        self.assertIsNone(result['cpu_work_ns'])
        self.assertIsNone(result['execution_span_ns'])

    def test_equal_and_zero_stable_raw_order(self):
        records = [event(0, 10, label='first'), event(0, 10, label='second'),
                   event(0, 0), event(10, 10)]
        result = qa.summarize_events(records)
        rows = {r['label']: r for r in result['rows']}
        self.assertEqual(rows['first']['exclusive_elapsed_ns'], 0)
        self.assertEqual(rows['second']['exclusive_elapsed_ns'], 10)
        self.assertEqual(result['equal_interval_records'], 1)
        self.assertTrue(any('arbitrary' in s for s in result['limitations']))

    def test_crossing_rejected_for_every_kind(self):
        with self.assertRaisesRegex(ValueError, 'crossing'):
            qa.summarize_events([event(0, 6), event(3, 9, 'Unknown')])

    def test_empty_and_noninterval(self):
        empty = qa.summarize_events([])
        self.assertEqual(empty['observation_window_ns'], 0)
        self.assertEqual(empty['rows'], [])
        records = [dict(record_type='integer', value=999, thread=2, kind='Count', label=None),
                   dict(record_type='instant', start_ns=7, thread=2, kind='Mark', label='x')]
        result = qa.summarize_events(records)
        self.assertEqual(result['record_types'], dict(interval=0, instant=1, integer=1))
        self.assertEqual(result['event_kind_counts'], dict(Count=1, Mark=1))
        self.assertEqual(result['covered_elapsed_ns'], 0)
        self.assertTrue(all(r['inclusive_elapsed_ns'] == 0 for r in result['rows']))

    def test_validation(self):
        for key, value in [('start_ns', -1), ('end_ns', True), ('thread', True),
                           ('thread', -1), ('kind', ''), ('kind', 1),
                           ('label', 3), ('label', 'é' * 2049),
                           ('record_type', 'wrong'), ('start_ns', 11)]:
            with self.subTest(key=key, value=str(value)[:30]), self.assertRaises(ValueError):
                qa.summarize_events([dict(event(), **{key: value})])
        for record in [dict(record_type='instant', start_ns=True, kind='x', label=None, thread=0),
                       dict(record_type='integer', value=-1, kind='x', label=None, thread=0)]:
            with self.assertRaises(ValueError):
                qa.summarize_events([record])

    def test_limits_and_consumption(self):
        for limit in [True, -1, 8000001]:
            with self.assertRaises(ValueError):
                qa.summarize_events([], max_records=limit)
        self.assertEqual(qa.summarize_events([event()], max_records=1)['records'], 1)
        with self.assertRaises(ValueError):
            qa.summarize_events([event(), event()], max_records=1)
        for constant, records in [('MAX_THREADS', [event(thread=0), event(thread=1)]),
                                  ('MAX_GROUPS', [event(), event(label=None)])]:
            with mock.patch.object(qa, constant, 1), self.assertRaises(ValueError):
                qa.summarize_events(records)
        with mock.patch.object(qa, 'MAX_RECORDS', 1), self.assertRaises(ValueError):
            qa.summarize_events([event(), event()])
        consumed = []
        def source():
            for i in range(3):
                consumed.append(i)
                yield event(i, i + 1)
        self.assertEqual(qa.summarize_events(source())['records'], 3)
        self.assertEqual(consumed, [0, 1, 2])


class CLITests(unittest.TestCase):
    def test_cli_privacy_forwarding_preservation_and_bounds(self):
        _, profile, metadata, _ = paired_fixture()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source, output = root / 'profile', root / 'output'
            source.write_bytes(profile)
            args = ['--profile', str(source), '--output', str(output)]
            with mock.patch.object(qp, 'prepare', wraps=qp.prepare) as prepare:
                qa.main(args + ['--large-capture', '--allow-unmapped-event-labels'])
                prepare.assert_called_once_with(profile, allow_unmapped_event_labels=True, large_capture=True)
            result = json.loads(output.read_bytes())
            self.assertEqual(result['profile_sha256'], hashlib.sha256(profile).hexdigest())
            self.assertEqual(result['metadata'], dict(process_id=42, start_time=100,
                command_sha256=hashlib.sha256(metadata['cmd'].encode()).hexdigest()))
            self.assertNotIn(metadata['cmd'], output.read_text())
            self.assertNotIn('/captured', output.read_text())
            self.assertEqual(output.stat().st_mode & 0o777, 0o400)
            before = output.read_bytes()
            with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as error:
                qa.main(args)
            self.assertEqual(error.exception.code, 2)
            self.assertEqual(output.read_bytes(), before)
            with mock.patch.object(qa, 'MAX_OUTPUT_BYTES', 1), contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
                qa.main(['--profile', str(source), '--output', str(root / 'oversize')])
            self.assertFalse((root / 'oversize').exists())
            with mock.patch.object(qp, 'MAX_EVENTS', 0), contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
                qa.main(['--profile', str(source), '--output', str(root / 'defaultcap')])

    def test_symlinks_and_invalid_input(self):
        _, profile, _, _ = paired_fixture()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'profile').write_bytes(profile)
            (root / 'link').symlink_to(root / 'profile')
            (root / 'dirlink').symlink_to(root, target_is_directory=True)
            (root / 'outlink').symlink_to(root / 'missing')
            for source, output in [('link', 'out'), ('profile', 'outlink'),
                                   ('dirlink/profile', 'out'), ('profile', 'dirlink/out')]:
                with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as error:
                    qa.main(['--profile', str(root / source), '--output', str(root / output)])
                self.assertEqual(error.exception.code, 2)
            self.assertFalse((root / 'missing').exists())
            (root / 'bad').write_bytes(b'bad')
            with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
                qa.main(['--profile', str(root / 'bad'), '--output', str(root / 'out')])
            self.assertFalse((root / 'out').exists())


if __name__ == '__main__':
    unittest.main()
