"""Synthetic fixtures derived from measureme 12.0.3 format 9 source."""
import contextlib
import io
import json
import os
from pathlib import Path
import struct
import tempfile
import unittest
from unittest import mock

import query_profile as qp

BASE = 100000003
META = 100000001
MARK = (1 << 48) - 1


def header(magic, version=9):
    return magic + struct.pack('<I', version)


def page(tag, data):
    return struct.pack('<BI', tag, len(data)) + data


def raw(kind, event, thread, start, end):
    return struct.pack('<QQIIII', kind, event, thread, start & 0xffffffff,
                       end & 0xffffffff, ((start >> 32) << 16) | (end >> 32))


def fixture(strings=None, index=None, events=None, split=False):
    if strings is None:
        strings = b'Query\xfflabel\x1earg\xff'
    if index is None:
        index = struct.pack('<QQ', 42, 14)
    if events is None:
        events = raw(BASE + 8, 42, 7, (1 << 40) + 3, (1 << 40) + 9)
    streams = [header(b'MMES') + events, header(b'MMSD') + strings,
               header(b'MMSI') + index]
    result = header(b'MMPD')
    for tag, stream in enumerate(streams):
        if split:
            result += page(tag, stream[:3]) + page(tag, stream[3:])
        else:
            result += page(tag, stream)
    return result


class DecodeTests(unittest.TestCase):
    def test_large_capture_requires_opt_in_and_preserves_record_api(self):
        data = fixture()
        with mock.patch.object(qp, 'MAX_EVENTS', 0):
            with self.assertRaises(ValueError):
                qp.decode(data)
            records = qp.decode(data, large_capture=True)
            self.assertEqual(len(records), 1)
            self.assertEqual(records[0]['event_id'], 42)
        with mock.patch.object(qp, 'LARGE_MAX_EVENTS', 0), self.assertRaises(ValueError):
            qp.decode(data, large_capture=True)
        with self.assertRaises(ValueError):
            qp.decode(data, large_capture='yes')

    def test_large_string_index_is_opt_in_and_still_bounded(self):
        data = fixture()
        with mock.patch.object(qp, 'MAX_STRINGS', 0):
            with self.assertRaises(ValueError):
                qp.decode(data)
            self.assertEqual(len(qp.decode(data, large_capture=True)), 1)
        with mock.patch.object(qp, 'LARGE_MAX_STRINGS', 0), self.assertRaises(ValueError):
            qp.decode(data, large_capture=True)

    def test_raw_id_and_interval_label(self):
        self.assertEqual(qp.decode(fixture()), [dict(event_id=42, kind='Query',
            label='label', label_resolved=True, thread=7, record_type='interval',
            start_ns=(1 << 40) + 3, end_ns=(1 << 40) + 9)])

    def test_all_record_types_and_boundaries(self):
        records = b''.join([raw(BASE + 8, 42, 0, 0, 0),
                            raw(BASE + 8, 42, 1, MARK, MARK),
                            raw(BASE + 8, 42, 2, MARK, MARK - 1)])
        result = qp.decode(fixture(events=records))
        self.assertEqual([r['record_type'] for r in result], ['interval', 'instant', 'integer'])
        self.assertEqual(result[1]['start_ns'], MARK)
        self.assertEqual(result[2]['value'], MARK)
        self.assertNotIn('start_ns', result[2])

    def test_split_pages_and_regular_id(self):
        event = raw(BASE + 8, BASE + 14, 9, 2, 4)
        result = qp.decode(fixture(events=event, split=True))
        self.assertEqual(result[0]['event_id'], BASE + 14)
        self.assertEqual(result[0]['label'], 'label')

    def test_composite_little_endian_u64_ref(self):
        strings = b'Query\xffabc\xff' + b'pre' + b'\xfe' + struct.pack('<Q', BASE + 14) + b'post\xff'
        result = qp.decode(fixture(strings=strings, index=struct.pack('<QQ', 42, 18)))
        self.assertEqual(result[0]['label'], 'preabcpost')

    def test_headers_versions_missing_stream(self):
        for bad in [b'', b'MMPD', header(b'XXXX'), header(b'MMPD', 8),
                    fixture().replace(b'MMES', b'XXXX'),
                    fixture().replace(header(b'MMSI'), header(b'MMSI', 8))]:
            with self.subTest(bad=bad[:16]), self.assertRaises(ValueError):
                qp.decode(bad)

    def test_page_failures(self):
        for tail in [b'\x03' + struct.pack('<I', 1) + b'x',
                     b'\x00\x01', page(0, b''),
                     struct.pack('<BI', 0, 262145), struct.pack('<BI', 0, 0xffffffff)]:
            with self.subTest(tail=tail), self.assertRaises(ValueError):
                qp.decode(header(b'MMPD') + tail)

    def test_alignment_truncation_and_bad_interval(self):
        for data in [fixture(events=b'x'), fixture(index=b'x'), fixture()[:-1],
                     fixture(events=raw(BASE + 8, 42, 0, 9, 2))]:
            with self.assertRaises(ValueError):
                qp.decode(data)

    def test_unmapped_invalid_and_overflow_ids(self):
        for event in [0, 100000002, (1 << 64) - 1]:
            with self.assertRaises(ValueError):
                qp.decode(fixture(events=raw(BASE + 8, event, 0, 1, 2)))
        with self.assertRaises(ValueError):
            qp.decode(fixture(index=struct.pack('<QQ', 42, (1 << 64) - 1)))

    def test_refcycle_and_bad_reference(self):
        for strings in [b'Query\xff\xfe' + struct.pack('<Q', BASE + 14) + b'\xff',
                        b'Query\xff\xfe\x01\xff',
                        b'Query\xffunterminated', b'Query\xff\x80\xff']:
            with self.assertRaises(ValueError):
                qp.decode(fixture(strings=strings))

    def test_duplicate_and_midstring_index(self):
        for index in [struct.pack('<QQQQ', 42, 14, 42, 14), struct.pack('<QQ', 42, 15)]:
            with self.assertRaises(ValueError):
                qp.decode(fixture(index=index))

    def test_limits_without_large_allocations(self):
        for constant, value in [('MAX_FILE_BYTES', 8), ('MAX_EVENTS', 0),
                                ('MAX_STRING_BYTES', 2), ('MAX_EXPANDED_BYTES', 2)]:
            with mock.patch.object(qp, constant, value), self.assertRaises(ValueError):
                qp.decode(fixture())

    def test_ref_bytes_not_confused_with_terminators(self):
        strings = b'Query\xffabc\xff\xfe' + struct.pack('<Q', 255) + b'\xff'
        index = struct.pack('<QQQQ', 42, 18, 255, 14)
        self.assertEqual(qp.decode(fixture(strings=strings, index=index))[0]['label'], 'abc')

    def test_aliased_ids_preserved_with_shared_labels(self):
        index = struct.pack('<QQQQ', 42, 14, 100000000, 14)
        events = raw(BASE + 8, 42, 0, 1, 2) + raw(BASE + 8, 100000000, 0, 2, 3)
        records = qp.decode(fixture(index=index, events=events))
        self.assertEqual([r['event_id'] for r in records], [42, 100000000])
        self.assertIs(records[0]['label'], records[1]['label'])

    def test_label_grammar(self):
        for label in [b'', b'bad\x00label', b'label\x1e', b'label\x1e\x1earg']:
            with self.assertRaises(ValueError):
                qp.decode(fixture(strings=b'Query\xff' + label + b'\xff'))

    def test_unused_cycle_is_rejected(self):
        strings = b'Query\xfflabel\xff\xfe' + struct.pack('<Q', BASE + 20) + b'\xff'
        with self.assertRaisesRegex(ValueError, 'cycle'):
            qp.decode(fixture(strings=strings))

    def test_reference_depth_and_string_count_caps(self):
        strings = b'Query\xff\xfe' + struct.pack('<Q', BASE + 24) + b'\xfflabel\xff'
        with mock.patch.object(qp, 'MAX_REFERENCE_DEPTH', 1), self.assertRaises(ValueError):
            qp.decode(fixture(strings=strings))
        with mock.patch.object(qp, 'MAX_STRINGS', 1), self.assertRaises(ValueError):
            qp.decode(fixture())

    def test_every_truncated_prefix(self):
        data = fixture()
        for length in range(len(data)):
            with self.subTest(length=length), self.assertRaises(ValueError):
                qp.decode(data[:length])

    def test_opt_in_unmapped_event_labels_only(self):
        events = raw(BASE + 8, 12, 3, 1, 2) + raw(BASE + 8, 42, 3, 2, 3)
        data = fixture(events=events)
        with self.assertRaisesRegex(ValueError, 'unmapped'):
            qp.decode(data)
        records = qp.decode(data, allow_unmapped_event_labels=True)
        self.assertEqual(records[0], dict(event_id=12, kind='Query', label=None,
            label_resolved=False, thread=3, record_type='interval', start_ns=1, end_ns=2))
        self.assertEqual(records[1]['label'], 'label')
        self.assertTrue(records[1]['label_resolved'])
        for bad in [raw(13, 12, 3, 1, 2),
                    raw(BASE + 8, 100000002, 3, 1, 2),
                    raw(BASE + 8, META, 3, 1, 2),
                    raw(BASE + 8, (1 << 64) - 1, 3, 1, 2)]:
            with self.assertRaises(ValueError):
                qp.decode(fixture(events=bad), allow_unmapped_event_labels=True)

    def test_opt_in_does_not_allow_broken_string_references(self):
        strings = b'Query\xff\xfe' + struct.pack('<Q', 123) + b'\xff'
        with self.assertRaisesRegex(ValueError, 'unmapped'):
            qp.decode(fixture(strings=strings), allow_unmapped_event_labels=True)
        with self.assertRaises(ValueError):
            qp.decode(fixture(strings=b'Query\xffbad\x00label\xff'),
                      allow_unmapped_event_labels=True)

    def test_opt_in_all_payloads_preserved(self):
        events = raw(BASE + 8, 100000000, 0, 1, 2)
        events += raw(BASE + 8, 100000000, 0, 3, MARK)
        events += raw(BASE + 8, 100000000, 0, 4, MARK - 1)
        records = qp.decode(fixture(events=events), allow_unmapped_event_labels=True)
        self.assertEqual([r['record_type'] for r in records], ['interval', 'instant', 'integer'])
        self.assertEqual([r['event_id'] for r in records], [100000000] * 3)
        self.assertTrue(all(r['label'] is None and r['label_resolved'] is False for r in records))

    def test_four_million_cap_and_small_boundary(self):
        self.assertEqual(qp.MAX_EVENTS, 4_000_000)
        event = raw(BASE + 8, 42, 0, 1, 2)
        with mock.patch.object(qp, 'MAX_EVENTS', 2):
            self.assertEqual(len(qp.decode(fixture(events=event * 2))), 2)
            with self.assertRaisesRegex(ValueError, 'event bound'):
                qp.decode(fixture(events=event * 3))

    def test_metadata_and_summary_provenance(self):
        strings = b'Query\xfflabel\x1earg\xff'
        metadata = dict(process_id=321, start_time=123456789,
                        cmd='rustc --sysroot pinned -Zincremental-info')
        index = struct.pack('<QQQQ', 42, 14, META, 8 + len(strings))
        data = fixture(strings=strings + json.dumps(metadata).encode() + b'\xff', index=index)
        self.assertEqual(qp.profile_metadata(data), metadata)
        summary = qp.summarize(data)
        import hashlib
        self.assertEqual(summary['profile_sha256'], hashlib.sha256(data).hexdigest())
        self.assertEqual(summary['profile_bytes'], len(data))
        self.assertEqual(summary['metadata'], metadata)
        self.assertEqual(summary['records'], 1)
        self.assertEqual(summary['record_types'], dict(interval=1, instant=0, integer=0))
        self.assertEqual(summary['label_coverage'], dict(resolved_records=1,
            unresolved_records=0, unresolved_unique_event_ids=0))
        self.assertNotIn('events', summary)

    def test_summary_unresolved_coverage_and_metadata_validation(self):
        strings = b'Query\xfflabel\xff'
        meta = dict(process_id=1, start_time=0, cmd='rustc')
        index = struct.pack('<QQQQ', 42, 14, META, 8 + len(strings))
        data = fixture(strings=strings + json.dumps(meta).encode() + b'\xff', index=index,
                       events=raw(BASE + 8, 12, 0, 1, 2) * 2)
        with self.assertRaises(ValueError):
            qp.summarize(data)
        summary = qp.summarize(data, allow_unmapped_event_labels=True)
        self.assertEqual(summary['label_coverage']['unresolved_records'], 2)
        self.assertEqual(summary['label_coverage']['unresolved_unique_event_ids'], 1)
        for bad in [b'[]', b'{}', b'not-json', b'{"process_id": true, "start_time": -1, "cmd": 42}']:
            with self.assertRaises(ValueError):
                qp.profile_metadata(fixture(strings=strings + bad + b'\xff', index=index))

    def test_empty_events(self):
        self.assertEqual(qp.decode(fixture(events=b'')), [])


class IOTests(unittest.TestCase):
    def test_summary_cli_writes_exclusive_object_without_full_event_list(self):
        strings = b'Query\xfflabel\xff'
        meta = dict(process_id=123, start_time=456, cmd='rustc --sysroot pinned')
        data = fixture(strings=strings + json.dumps(meta).encode() + b'\xff',
                       index=struct.pack('<QQQQ', 42, 14, META, 8 + len(strings)))
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source, output = root / 'profile', root / 'summary.json'
            source.write_bytes(data)
            with mock.patch.object(qp, 'decode', side_effect=AssertionError('materialized list')), contextlib.redirect_stdout(io.StringIO()):
                qp.main(['--input', str(source), '--output', str(output), '--summary-only'])
            result = json.loads(output.read_bytes())
            self.assertEqual(result['metadata'], meta)
            self.assertEqual(result['records'], 1)
            self.assertEqual(result['record_types']['interval'], 1)
            original = output.read_bytes()
            with self.assertRaises(FileExistsError):
                qp.write_output(output, result)
            self.assertEqual(output.read_bytes(), original)
            self.assertEqual(output.stat().st_mode & 0o777, 0o400)
            with mock.patch.object(qp, 'MAX_FILE_BYTES', 4), self.assertRaises(ValueError):
                qp.write_output(root / 'too-large.json', result)
            self.assertFalse((root / 'too-large.json').exists())

    def test_cli_requires_explicit_opt_in(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source, output = root / 'private.mm_profdata', root / 'decoded.json'
            source.write_bytes(fixture(events=raw(BASE + 8, 13, 0, 1, 2)))
            stderr = io.StringIO()
            with contextlib.redirect_stderr(stderr), self.assertRaises(SystemExit) as caught:
                qp.main(['--input', str(source), '--output', str(output)])
            self.assertEqual(caught.exception.code, 2)
            self.assertNotIn(str(root), stderr.getvalue())
            self.assertFalse(output.exists())
            stdout = io.StringIO()
            with contextlib.redirect_stdout(stdout):
                qp.main(['--input', str(source), '--output', str(output),
                         '--allow-unmapped-event-labels'])
            self.assertEqual(json.loads(stdout.getvalue())['unresolved_event_labels'], 1)
            records = json.loads(output.read_bytes())
            self.assertEqual(records[0]['event_id'], 13)
            self.assertIsNone(records[0]['label'])
            self.assertFalse(records[0]['label_resolved'])

    def test_bounded_regular_input_and_exclusive_output(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / 'profile'
            source.write_bytes(fixture())
            with mock.patch.object(qp, 'MAX_FILE_BYTES', 8), self.assertRaises(ValueError):
                qp.read_input(source)
            link = root / 'link'
            link.symlink_to(source)
            with self.assertRaises(ValueError):
                qp.read_input(link)
            fifo = root / 'fifo'
            os.mkfifo(fifo)
            with self.assertRaises(ValueError):
                qp.read_input(fifo)
            out = root / 'out.json'
            qp.write_output(out, qp.decode(source.read_bytes()))
            old = out.read_bytes()
            with self.assertRaises(FileExistsError):
                qp.write_output(out, [])
            self.assertEqual(out.read_bytes(), old)
            with mock.patch.object(qp, 'MAX_FILE_BYTES', 4), self.assertRaises(ValueError):
                qp.write_output(root / 'oversize.json', [{'x': 12345}])
            self.assertFalse((root / 'oversize.json').exists())
            with mock.patch.object(qp, 'MAX_FILE_BYTES', 8), mock.patch.object(
                    qp.os, 'fstat', return_value=type('Info', (), {
                        'st_mode': 0o100400, 'st_size': 0})()), self.assertRaises(ValueError):
                qp.read_input(source)


if __name__ == '__main__':
    unittest.main()
