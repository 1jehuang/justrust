#!/usr/bin/env python3
"""Offline measureme 12.0.3 MMPD format-9 raw-event decoder.

Primary layout sources: measureme 12.0.3 file_header.rs, serialization.rs,
stringtable.rs and raw_event.rs. decodeme 10.1.3 supplies label semantics only:
its older 32-bit string/index layout is NOT format 9 and is not used here.

Import decode(bytes) -> list[dict]. Records retain their original numeric
64-bit event_id, with kind/label resolved separately. Virtual IDs <=100000000
can be correlated with the recording invocation's DepNodeIndex, never across
invocations. Raw timestamps are relative counter values in nanoseconds, with
no epoch offset, DAG inference, exclusive CPU-work or span interpretation.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import struct

MAX_FILE_BYTES = 256 * 1024 * 1024
MAX_EVENTS = 4_000_000
LARGE_MAX_EVENTS = 8_000_000
MAX_PAGE_BYTES = 256 * 1024
MAX_STRINGS = 2_000_000
LARGE_MAX_STRINGS = 4_000_000
MAX_STRING_BYTES = 1024 * 1024
MAX_EXPANDED_BYTES = 64 * 1024 * 1024
MAX_REFERENCE_DEPTH = 128
MAX_COMPONENTS = 8_000_000
METADATA_ID = 100_000_001
INVALID_ID = 100_000_002
FIRST_REGULAR_ID = 100_000_003
INSTANT_MARKER = (1 << 48) - 1
INTEGER_MARKER = INSTANT_MARKER - 1
RAW_EVENT = struct.Struct('<QQIIII')
STRING_DELIMITER = re.compile(b'[\xfe\xff]')


def check_header(data, magic):
    if len(data) < 8 or data[:4] != magic:
        raise ValueError('invalid or truncated stream header')
    if struct.unpack_from('<I', data, 4)[0] != 9:
        raise ValueError('unsupported format version, expected 9')


def split_streams(data):
    check_header(data, b'MMPD')
    streams = [bytearray(), bytearray(), bytearray()]
    position = 8
    while position < len(data):
        if len(data) - position < 5:
            raise ValueError('truncated page header')
        tag, size = struct.unpack_from('<BI', data, position)
        position += 5
        if tag > 2 or not 0 < size <= MAX_PAGE_BYTES:
            raise ValueError('invalid page tag or size')
        if size > len(data) - position:
            raise ValueError('truncated or overflowing page')
        streams[tag].extend(data[position:position + size])
        position += size
    for stream, magic in zip(streams, (b'MMES', b'MMSD', b'MMSI')):
        check_header(stream, magic)
    return streams


class Strings:
    """Parse all entries and validate even unused references, strictly bounded."""
    def __init__(self, data, index, large_capture=False):
        max_strings = LARGE_MAX_STRINGS if large_capture else MAX_STRINGS
        if (len(index) - 8) % 16:
            raise ValueError('misaligned string index')
        if (len(index) - 8) // 16 > max_strings:
            raise ValueError('too many string mappings')
        self.entries, self.index, self.cache = {}, {}, {}
        self.active = set()
        self.expanded_bytes = 0
        position, components = 8, 0
        while position < len(data):
            address = position
            parts = []
            literal_bytes = 0
            while True:
                if position >= len(data):
                    raise ValueError('unterminated string')
                tag = data[position]
                if tag == 255:
                    position += 1
                    break
                components += 1
                if components > MAX_COMPONENTS:
                    raise ValueError('too many string components')
                if tag == 254:
                    if len(data) - position < 9:
                        raise ValueError('truncated string reference')
                    parts.append(struct.unpack_from('<Q', data, position + 1)[0])
                    position += 9
                else:
                    match = STRING_DELIMITER.search(data, position)
                    if match is None:
                        raise ValueError('unterminated string literal')
                    end = match.start()
                    literal_bytes += end - position
                    if literal_bytes > MAX_STRING_BYTES:
                        raise ValueError('string size bound exceeded')
                    try:
                        parts.append(bytes(data[position:end]).decode('utf-8', errors='strict'))
                    except UnicodeError as error:
                        raise ValueError('invalid UTF-8 string') from error
                    position = end
            self.entries[address] = parts
            if len(self.entries) > max_strings:
                raise ValueError('too many strings')
        for sid, address in struct.iter_unpack('<QQ', index[8:]):
            if sid > METADATA_ID or sid in self.index or address not in self.entries:
                raise ValueError('invalid, duplicate or out-of-bounds string mapping')
            self.index[sid] = address
        # Resolve every stored entry, including metadata and unused strings.
        for address in self.entries:
            self.resolve(FIRST_REGULAR_ID + address)

    def address(self, sid):
        if sid <= METADATA_ID:
            if sid not in self.index:
                raise ValueError('unmapped virtual string ID')
            address = self.index[sid]
        elif sid == INVALID_ID:
            raise ValueError('invalid string ID')
        else:
            address = sid - FIRST_REGULAR_ID
        if address not in self.entries:
            raise ValueError('string address overflow or not an entry boundary')
        return address

    def resolve(self, sid, depth=0):
        address = self.address(sid)
        if address in self.active:
            raise ValueError('string reference cycle')
        if address in self.cache:
            return self.cache[address]
        if depth >= MAX_REFERENCE_DEPTH:
            raise ValueError('string reference depth bound exceeded')
        self.active.add(address)
        parts, size = [], 0
        try:
            for part in self.entries[address]:
                text = self.resolve(part, depth + 1) if isinstance(part, int) else part
                size += len(text.encode('utf-8'))
                if size > MAX_STRING_BYTES:
                    raise ValueError('expanded string size bound exceeded')
                parts.append(text)
            self.expanded_bytes += size
            if self.expanded_bytes > MAX_EXPANDED_BYTES:
                raise ValueError('total expanded strings bound exceeded')
            result = ''.join(parts)
            self.cache[address] = result
            return result
        finally:
            self.active.remove(address)


def label_of(text):
    parts = text.split('\x1e')
    if any(not part or any((ord(c) < 32 and c not in '\t\n\r\v\f')
                           or ord(c) == 127 for c in part) for part in parts):
        raise ValueError('invalid event label or argument grammar')
    return parts[0]


def decode(data, allow_unmapped_event_labels=False, large_capture=False):
    """Return all raw records in stream order, raising ValueError on bad input.

    Common keys: event_id:int, kind:str, label:str|None, label_resolved:bool,
    thread:int, record_type:'interval'|'instant'|'integer'. Intervals add
    start_ns/end_ns, instants add start_ns, integers add value. No filtering.
    Explicit opt-in permits ONLY absent mappings for user-virtual event IDs
    <=100000000, represented as label=None, label_resolved=False. Kinds and
    all string references, headers, payloads and mapped label grammar stay strict.
    """
    events, strings = prepare(data, allow_unmapped_event_labels, large_capture)
    return list(iter_prepared_records(events, strings, allow_unmapped_event_labels))


def prepare(data, allow_unmapped_event_labels=False, large_capture=False):
    if not isinstance(data, bytes):
        raise ValueError('decode requires bytes')
    if type(allow_unmapped_event_labels) is not bool:
        raise ValueError('allow_unmapped_event_labels requires an explicit boolean')
    if type(large_capture) is not bool:
        raise ValueError('large_capture requires an explicit boolean')
    if len(data) > MAX_FILE_BYTES:
        raise ValueError('profile exceeds 256MiB bound')
    events, string_data, string_index = split_streams(data)
    if (len(events) - 8) % RAW_EVENT.size:
        raise ValueError('misaligned raw event stream')
    count = (len(events) - 8) // RAW_EVENT.size
    if count > (LARGE_MAX_EVENTS if large_capture else MAX_EVENTS):
        raise ValueError('profile exceeds selected event bound')
    return events, Strings(string_data, string_index, large_capture)


def iter_prepared_records(events, strings, allow_unmapped_event_labels=False):
    labels = {}
    for kind, event_id, thread, lower1, lower2, upper in struct.iter_unpack('<QQIIII', events[8:]):
        start = lower1 | ((upper & 0xffff0000) << 16)
        end = lower2 | ((upper & 0xffff) << 32)
        event_kind = strings.resolve(kind)
        label_resolved = True
        if (allow_unmapped_event_labels and event_id <= 100_000_000
                and event_id not in strings.index):
            label, label_resolved = None, False
        else:
            label_address = strings.address(event_id)
            if label_address not in labels:
                labels[label_address] = label_of(strings.resolve(event_id))
            label = labels[label_address]
        record = dict(event_id=event_id, kind=event_kind, label=label,
                      label_resolved=label_resolved, thread=thread)
        if end == INTEGER_MARKER:
            record.update(record_type='integer', value=start)
        elif end == INSTANT_MARKER:
            record.update(record_type='instant', start_ns=start)
        else:
            if start > end:
                raise ValueError('invalid interval start/end')
            record.update(record_type='interval', start_ns=start, end_ns=end)
        yield record


def metadata_from_strings(strings):
    try:
        value = json.loads(strings.resolve(METADATA_ID))
    except json.JSONDecodeError as error:
        raise ValueError('invalid profile metadata JSON') from error
    if (not isinstance(value, dict) or type(value.get('process_id')) is not int
            or not 0 <= value['process_id'] <= 0xffffffff
            or type(value.get('start_time')) is not int
            or not 0 <= value['start_time'] < 1 << 64
            or not isinstance(value.get('cmd'), str)):
        raise ValueError('invalid profile metadata schema')
    return {key: value[key] for key in ('process_id', 'start_time', 'cmd')}


def profile_metadata(data, large_capture=False):
    """Return exact process_id, start_time and cmd from the metadata string.

    No path redaction or inferred flags: cmd is original recording provenance.
    This validates the container/string table, not individual event payloads.
    """
    _, strings = prepare(data, large_capture=large_capture)
    return metadata_from_strings(strings)


def summarize(data, allow_unmapped_event_labels=False, large_capture=False):
    """Validate every raw record, returning compact counts and exact provenance.

    Does not filter events, alter the raw capture, or infer DAG/span/CPU work.
    Uses the same record decoder without retaining the entire record list.
    """
    events, strings = prepare(data, allow_unmapped_event_labels, large_capture)
    metadata = metadata_from_strings(strings)
    types = dict(interval=0, instant=0, integer=0)
    count = resolved = unresolved = 0
    missing_ids = set()
    for record in iter_prepared_records(events, strings, allow_unmapped_event_labels):
        count += 1
        types[record['record_type']] += 1
        if record['label_resolved']:
            resolved += 1
        else:
            unresolved += 1
            missing_ids.add(record['event_id'])
    return dict(schema=1, format_version=9,
                profile_sha256=hashlib.sha256(data).hexdigest(), profile_bytes=len(data),
                records=count, record_types=types,
                label_coverage=dict(resolved_records=resolved, unresolved_records=unresolved,
                                    unresolved_unique_event_ids=len(missing_ids)),
                metadata=metadata,
                limitations=['Raw recording counts and metadata only. No DAG span, CPU-work or causal interpretation.',
                             'Virtual numeric IDs are invocation-local. Unresolved event labels, if explicitly allowed, are not inferred.'])


def open_regular(path, flags):
    """Reject every symlink component and nonregular leaves via dirfd walking."""
    parts = Path(os.path.abspath(path)).parts
    directory = os.open(parts[0], os.O_RDONLY | os.O_DIRECTORY)
    try:
        for part in parts[1:-1]:
            child = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW,
                            dir_fd=directory)
            os.close(directory)
            directory = child
        fd = os.open(parts[-1], flags | os.O_NOFOLLOW | os.O_NONBLOCK,
                     0o400, dir_fd=directory)
        if not stat.S_ISREG(os.fstat(fd).st_mode):
            os.close(fd)
            raise ValueError('regular non-symlink file required')
        return fd
    finally:
        os.close(directory)


def read_input(path):
    try:
        with os.fdopen(open_regular(path, os.O_RDONLY), 'rb') as handle:
            if os.fstat(handle.fileno()).st_size > MAX_FILE_BYTES:
                raise ValueError('profile exceeds 256MiB bound')
            data = handle.read(MAX_FILE_BYTES + 1)
            if len(data) > MAX_FILE_BYTES:
                raise ValueError('profile exceeds 256MiB bound')
            return data
    except OSError as error:
        raise ValueError('cannot read regular non-symlink profile') from error


def record_json(record):
    return json.dumps(record, sort_keys=True, ensure_ascii=True, allow_nan=False).encode('utf-8')


def write_output(path, records):
    # Bound JSON size before reserving the exclusive output, without retaining
    # another giant serialization. Repeated labels count toward this bound.
    if isinstance(records, dict):
        raw = record_json(records) + b'\n'
        if len(raw) > MAX_FILE_BYTES:
            raise ValueError('decoded output exceeds 256MiB bound')
        with os.fdopen(open_regular(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL), 'wb') as handle:
            handle.write(raw)
        return
    size = 3 + sum(len(record_json(record)) + 2 for record in records)
    if size > MAX_FILE_BYTES:
        raise ValueError('decoded output exceeds 256MiB bound')
    with os.fdopen(open_regular(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL), 'wb') as handle:
        handle.write(b'[')
        for number, record in enumerate(records):
            if number:
                handle.write(b',\n')
            handle.write(record_json(record))
        handle.write(b']\n')


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--input', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--summary-only', action='store_true',
                        help='validate all raw records but write compact counts/hash/metadata instead of event JSON')
    parser.add_argument('--large-capture', action='store_true', help='opt in to 8M events/4M string mappings; 256MiB byte bound unchanged')
    parser.add_argument('--allow-unmapped-event-labels', action='store_true',
                        help='retain missing user-virtual event labels as null with label_resolved=false')
    args = parser.parse_args(argv)
    try:
        data = read_input(args.input)
        if args.summary_only:
            output = summarize(data, allow_unmapped_event_labels=args.allow_unmapped_event_labels, large_capture=args.large_capture)
            count = output['records']
            unresolved = output['label_coverage']['unresolved_records']
        else:
            output = decode(data, allow_unmapped_event_labels=args.allow_unmapped_event_labels, large_capture=args.large_capture)
            count = len(output)
            unresolved = sum(not r['label_resolved'] for r in output)
        write_output(args.output, output)
        print(json.dumps(dict(format_version=9, records=count,
                              unresolved_event_labels=unresolved), sort_keys=True))
    except (OSError, ValueError) as error:
        # Never echo OSError messages containing private paths or raw strings.
        message = str(error) if isinstance(error, ValueError) else 'exclusive output unavailable'
        parser.exit(2, 'query profile: ' + message + '\n')


if __name__ == '__main__':
    main()
