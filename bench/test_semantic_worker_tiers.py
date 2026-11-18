#!/usr/bin/env python3
"""Pure Python protocol/orchestration tests. Never execute a native compiler."""
import io
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import semantic_worker_tiers as tiers


def owned(tier='hot'):
    value = dict.fromkeys(tiers.OWNER_FIELDS, 0)
    if tier == 'hot':
        value.update(graph_bytes=10, archive_bytes=100, hot_bytes=100, hot_entries=1)
    elif tier == 'compressed':
        value.update(archive_bytes=40, compressed_bytes=40, compressed_entries=1)
    return value


def counters(value):
    return dict.fromkeys(tiers.native.FRONTEND_COUNTERS + tiers.native.FRONTEND_OPTIONAL, value)


class TierTests(unittest.TestCase):
    def test_live_memory(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / '42').mkdir()
            (root / '42/smaps_rollup').write_text('0000-ffff ---p\nRss: 80 kB\nPss: 61 kB\n'
                'Private_Clean: 3 kB\nPrivate_Dirty: 20 kB\nPrivate_Hugetlb: 4 kB\n'
                'Swap: 2 kB\nSwapPss: 1 kB\n')
            sample = tiers.memory(42, root)
            self.assertEqual(sample['private_kib'], 27)
            self.assertEqual(sample['pss_kib'], 61)
            self.assertEqual(sample['swap_kib'], 2)
            (root / '42/smaps_rollup').write_text('Rss: 4 kB\n')
            with self.assertRaises(AssertionError):
                tiers.memory(42, root)

    def test_counterbalanced_order(self):
        rows = [tiers.order(rep, 0) for rep in range(10)]
        for row in rows:
            self.assertEqual(set(row), set(tiers.MODES))
        for position in range(5):
            self.assertEqual(sorted(row[position] for row in rows), sorted(tiers.MODES * 2))
        self.assertEqual(rows[5], rows[0][::-1])

    def worker(self, response):
        return SimpleNamespace(sequence=8, timeout=1, buffer=response,
            process=SimpleNamespace(pid=42, stdin=io.BytesIO()), transcript=io.BytesIO(),
            selector=SimpleNamespace(select=lambda timeout: [True]), response_fd=99)

    def test_control_shared_sequence_and_transcript(self):
        response = dict(id=9, pid=42, status=0, tier='compressed', memory=owned('compressed'))
        worker = self.worker(json.dumps(response).encode() + b'\nnext')
        with patch.object(tiers, 'memory', return_value={'rss_kib': 80}):
            record = tiers.control(worker, 'compressed')
        self.assertEqual(worker.sequence, 9)
        self.assertEqual(worker.buffer, b'next')
        self.assertEqual(json.loads(worker.process.stdin.getvalue()), dict(id=9, tier='compressed'))
        self.assertEqual(record['response'], response)
        self.assertEqual(len(worker.transcript.getvalue().splitlines()), 2)

    def test_control_rejects_bad_ack(self):
        for change in ({'id': 8}, {'pid': 43}, {'status': True}, {'status': 1},
                       {'tier': 'cold'}, {'memory': None}):
            response = dict(id=9, pid=42, status=0, tier='hot', memory={})
            response.update(change)
            with self.subTest(change=change), patch.object(tiers, 'memory', return_value={}):
                with self.assertRaises(AssertionError):
                    tiers.control(self.worker(json.dumps(response).encode() + b'\n'), 'hot')

    def test_control_reads_fragmented_response(self):
        worker = self.worker(b'')
        chunks = [json.dumps(dict(id=9, pid=42, status=0, tier='cold', memory=owned('cold'))).encode(), b'\n']
        with patch.object(tiers, 'memory', return_value={}), patch.object(tiers.os, 'read', side_effect=chunks):
            self.assertEqual(tiers.control(worker, 'cold')['response']['tier'], 'cold')

    def test_control_eof_and_timeout(self):
        with patch.object(tiers, 'memory', return_value={}), patch.object(tiers.os, 'read', return_value=b''):
            with self.assertRaises(RuntimeError):
                tiers.control(self.worker(b''), 'hot')
        worker = self.worker(b'')
        worker.timeout = -1
        with patch.object(tiers, 'memory', return_value={}):
            with self.assertRaises(TimeoutError):
                tiers.control(worker, 'cold')

    def test_green_requires_both_real_load_kinds(self):
        tiers.green_loads(counters(0), counters(1), typed=True)
        for key in ('archive_hits', 'value_loads', 'typeck_loads', 'mir_loads'):
            after = counters(1)
            after[key] = 0
            with self.subTest(key=key), self.assertRaises(AssertionError):
                tiers.green_loads(counters(0), after, typed=True)

    def test_unchanged_green_validation_does_not_require_undemanded_typed_values(self):
        after = counters(1)
        after['typeck_loads'] = after['mir_loads'] = 0
        tiers.green_loads(counters(0), after)
        with self.assertRaises(AssertionError):
            tiers.green_loads(counters(0), after, typed=True)

    def test_compressed_tier_requires_typed_loads_but_not_evicted_graph_hits(self):
        current = counters(1)
        current['graph_hits'] = 0
        tiers.validate_tier_loads(counters(0), current, 'frontend-compressed', True)
        with self.assertRaises(AssertionError):
            tiers.validate_tier_loads(counters(0), current, 'frontend-hot', True)
        for key in ('typeck_loads', 'mir_loads'):
            missing = dict(current, **{key: 0})
            with self.subTest(key=key), self.assertRaises(AssertionError):
                tiers.validate_tier_loads(counters(0), missing, 'frontend-compressed', True)

    def test_no_local_native_execution(self):
        with patch.object(tiers.native, 'identity') as identity:
            with self.assertRaises(ValueError):
                tiers.capture('/compiler', '/output')
            identity.assert_not_called()

    def test_mock_capture_full_edit_error_recovery(self):
        workers = []
        current = {'expected': None}
        real_cases = tiers.native.frontend_cases
        workload = real_cases(2)
        sources = {source: expected for _, source, expected in workload}
        def observable(expected):
            return dict(status=int(expected is None), stdout_b64='',
                        stderr_b64='dHlwZSBlcnJvcg==' if expected is None else '', elapsed_seconds=.01)
        class Worker:
            def __init__(self, compiler, cache, root, values, timeout, frontend):
                self.root, self.frontend = root, frontend
                self.process = SimpleNamespace(pid=42)
                self.command = ['mock-only']
                self.counters = {'resident_hits': 0}
                self.frontend_counters = counters(0)
                self.closed = False
                workers.append(self)
            def compile(self, argv):
                expected = sources[(self.root / 'fixture.rs').read_text()]
                current['expected'] = expected
                if self.frontend:
                    self.frontend_counters = {key: value + 1 for key, value in self.frontend_counters.items()}
                (self.root / 'fixture').write_bytes(b'mock')
                return observable(expected)
            def close(self):
                self.closed = True
        def run(argv, *args, **kwargs):
            if len(argv) > 1:
                expected = sources[Path(argv[1]).read_text()]
                current['expected'] = expected
                (Path(argv[1]).parent / 'fixture').write_bytes(b'mock')
                return observable(expected)
            return dict(status=0, stdout_b64=tiers.native.encoded(current['expected'].encode()), stderr_b64='')
        def control(worker, tier):
            return dict(elapsed_seconds=.001, response={'memory': owned(tier) if worker.frontend else owned('cold')})
        with tempfile.TemporaryDirectory() as directory, \
                patch.object(tiers, 'hosted_identity', return_value={'mock': True}), \
                patch.object(tiers.native, 'Resident', Worker), \
                patch.object(tiers.native, 'identity', return_value={'mock': True}), \
                patch.object(tiers.native, 'run', side_effect=run), \
                patch.object(tiers, 'memory', return_value=dict(rss_kib=4, pss_kib=3, private_kib=2, swap_kib=0)), \
                patch.object(tiers, 'control', side_effect=control):
            report = tiers.capture('/mock-only', directory, 1, 2, hosted=True)
            self.assertEqual(report['verdict'], 'passed')
            names = [row['name'] for row in report['observations']]
            self.assertIn('type-error', names)
            self.assertIn('recovery', names)
            self.assertIn('significant-signature-edit', names)
            self.assertTrue(all(worker.closed for worker in workers))
            self.assertTrue((Path(directory) / 'receipt.json').exists())

    def test_owner_schema_and_actual_tiers(self):
        tiers.tier_gate('compressed', owned(), owned('compressed'))
        tiers.tier_gate('cold', owned(), owned('cold'))
        for key in tiers.OWNER_FIELDS:
            for invalid in (None, True, -1, '0', 1.0):
                value = owned()
                value[key] = invalid
                with self.subTest(key=key, invalid=invalid), self.assertRaises(AssertionError):
                    tiers.owner_memory(value)
        for tier, after in [('compressed', owned()), ('compressed', owned('cold')), ('cold', owned('compressed'))]:
            with self.subTest(tier=tier, after=after), self.assertRaises(AssertionError):
                tiers.tier_gate(tier, owned(), after)

    def test_hosted_marker_validation(self):
        marker = Path('/run/pool/share')
        metadata = SimpleNamespace(st_uid=0, st_mode=0o100644, st_dev=1, st_ino=2)
        with patch.object(Path, 'lstat', return_value=metadata), \
                patch.object(Path, 'read_text', return_value='16\n'), \
                patch.object(tiers.native, 'digest', return_value='mock'):
            self.assertEqual(tiers.hosted_identity(marker)['allocated_cpus'], 16)
            for uid, mode in ((1000, 0o100644), (0, 0o100666), (0, 0o120644)):
                metadata.st_uid, metadata.st_mode = uid, mode
                with self.assertRaises(ValueError):
                    tiers.hosted_identity(marker)
        metadata.st_uid, metadata.st_mode = 0, 0o100644
        for value in ('0', '-1', '1.5', '', 'sixteen'):
            with patch.object(Path, 'lstat', return_value=metadata), \
                    patch.object(Path, 'read_text', return_value=value), self.assertRaises(ValueError):
                tiers.hosted_identity(marker)
        with self.assertRaises(ValueError):
            tiers.hosted_identity('/tmp/not-hosted')
        with patch.object(tiers.native, 'identity') as identity, \
                patch.object(tiers, 'hosted_identity', side_effect=ValueError('missing marker')):
            with self.assertRaises(ValueError):
                tiers.capture('/mock', '/output', hosted=True)
            identity.assert_not_called()

    def test_mock_shipping_module_route(self):
        original = tiers.dogfood.DEFAULT_SOURCE.read_bytes()
        matrix = tiers.dogfood.variants(original.decode(), frontend=True)
        expectations = {source: expectation for _, source, expectation in matrix}
        clock = [0.0]
        workers = []
        def compile_result(work):
            expectation = expectations[(work / 'remote_sync_queue.rs').read_text()]
            (work / 'tests').write_bytes(b'mock-test-artifact')
            clock[0] += .01
            return dict(status=1 if expectation == 'compile-fail' else 0,
                        stdout_b64='', stderr_b64='ZXJyb3I=' if expectation == 'compile-fail' else '',
                        elapsed_seconds=.01)
        class Worker:
            def __init__(self, compiler, cache, root, values, timeout, frontend):
                self.root, self.frontend = root, frontend
                self.process = SimpleNamespace(pid=42)
                self.command = ['pure-python-mock']
                self.counters = {'resident_hits': 0}
                self.frontend_counters = counters(0)
                self.closed = False
                workers.append(self)
            def compile(self, argv):
                if self.frontend:
                    self.frontend_counters = {key: value + 1 for key, value in self.frontend_counters.items()}
                return compile_result(self.root)
            def close(self):
                self.closed = True
        def run(argv, *args, **kwargs):
            work = Path(kwargs['cwd'])
            if '--test' in argv:
                return compile_result(work)
            expectation = expectations[(work / 'remote_sync_queue.rs').read_text()]
            nonce = kwargs['env'][tiers.dogfood.NONCE_KEY]
            names = list(tiers.dogfood.ORIGINAL_TESTS) + [tiers.dogfood.PROBE]
            failed = int(expectation == 'test-fail')
            statuses = {name: ('FAILED' if failed and name == names[2] else 'ok') for name in names}
            stdout = '\n'.join(f'test {name} ... {status}' for name, status in statuses.items())
            stdout += f'\ntest result: {"FAILED" if failed else "ok"}. {len(names) - failed} passed; {failed} failed; 0 ignored; 0 measured; 0 filtered out;\n'
            clock[0] += 100
            return dict(status=101 if failed else 0, stdout_b64=tiers.native.encoded(stdout.encode()),
                        stderr_b64=tiers.native.encoded(f'JR_DOGFOOD_NONCE={nonce}\n'.encode()), elapsed_seconds=100)
        def control(worker, tier):
            return dict(elapsed_seconds=.001, response={'memory': owned(tier) if worker.frontend else owned('cold')})
        with tempfile.TemporaryDirectory() as directory, \
                patch.object(tiers, 'hosted_identity', return_value={'mock': True}), \
                patch.object(tiers.native, 'identity', return_value={'mock': True}), \
                patch.object(tiers.native, 'Resident', Worker), patch.object(tiers.native, 'run', side_effect=run), \
                patch.object(tiers, 'control', side_effect=control), \
                patch.object(tiers.time, 'monotonic', side_effect=lambda: clock[0]), \
                patch.object(tiers, 'memory', return_value=dict(rss_kib=8, pss_kib=6, private_kib=5, swap_kib=0)):
            root = Path(directory) / 'evidence'
            root.mkdir()
            source = Path(directory) / 'original-module.rs'
            source.write_bytes(original)
            report = tiers.capture('/pure-mock', root, repetitions=1, source=source, hosted=True)
            self.assertEqual(source.read_bytes(), original)
            self.assertEqual((root / 'source-original.rs').read_bytes(), original)
            self.assertEqual((root / 'private-workload/wrapper.rs').read_text(), tiers.dogfood.WRAPPER)
            self.assertTrue(report['source_unchanged'])
            self.assertTrue(all(worker.closed for worker in workers))
            nonces = []
            for row in report['observations']:
                for mode, result in row['modes'].items():
                    if mode != 'fresh':
                        self.assertLess(result['wake_wall_seconds'], .011)
                    if result['compile']['status'] == 0:
                        self.assertEqual(len(result['named_statuses']), len(tiers.dogfood.ORIGINAL_TESTS) + 1)
                        nonces.append(result['nonce'])
            self.assertEqual(len(nonces), len(set(nonces)))
            row = next(row for row in report['observations'] if row['name'] == 'unchanged')
            row['modes']['control']['compile']['stderr_b64'] = 'ZGlmZmVyZW50'
            with self.assertRaises(AssertionError):
                tiers.validate(row)
            row['modes']['control']['compile']['stderr_b64'] = ''
            row['modes']['control']['compile']['status'] = 1
            with self.assertRaises(AssertionError):
                tiers.validate(row)
            row['modes']['control']['compile']['status'] = 0
            runtime = row['modes']['control']['runtime']
            saved_stdout = runtime['stdout_b64']
            runtime['stdout_b64'] = tiers.native.encoded(b'test missing ... ok\n')
            with self.assertRaises(AssertionError):
                tiers.validate(row)
            runtime['stdout_b64'] = saved_stdout
            row['modes']['control']['nonce'] = 'stale'
            with self.assertRaises(AssertionError):
                tiers.validate(row)

    def test_failure_keeps_receipt(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(tiers, 'hosted_identity', return_value={'mock': True}), patch.object(tiers.native, 'identity', side_effect=RuntimeError('mock failure')):
            with self.assertRaises(RuntimeError):
                tiers.capture('/mock', directory, hosted=True)
            report = json.loads((Path(directory) / 'receipt.json').read_text())
            self.assertEqual(report['verdict'], 'failed')
            self.assertIn('mock failure', report['error'])


if __name__ == '__main__':
    unittest.main()
