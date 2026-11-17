#!/usr/bin/env python3
"""Pure mocked decoder acceptance tests. Never execute a native compiler."""
import copy
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import Mock, patch

import stable_resident_decoder as harness


def response(mode='1', kind='cold', previous=None):
    stats = dict.fromkeys(harness.COUNTERS, 0)
    stats['retired'] = False
    if mode == '1':
        stats.update(previous or {})
        if kind == 'cold':
            stats.update(misses=3, publications=3, retained_entries=3)
        elif kind == 'hit':
            stats['hits'] += 3
        else:
            stats.update(retained_entries=3, retired=True, retirement_requests=1)
    return dict(status=1 if kind == 'failure' else 0, pid=12, executed=True,
                stdout_b64='', stderr_b64='', decoder=stats,
                owned=dict(retirement_reason='compile-error' if kind == 'failure' and mode == '1' else None),
                ast=dict.fromkeys(harness.ast.COUNTERS, 0),
                lexer=dict.fromkeys(harness.ast.COUNTERS, 0),
                live=dict.fromkeys(harness.ast.LIVE_COUNTERS, 0))


class DecoderTests(unittest.TestCase):
    def test_explicit_clean_environment(self):
        with patch.dict('os.environ', {'JUSTRUST_RUSTC_RESIDENT_TYPED': '1',
                'JUSTRUST_RUSTC_RESIDENT_DECODERS': '1'}):
            for mode in ('0', '1'):
                env = harness.environment(mode)
                self.assertEqual(env['JUSTRUST_RUSTC_RESIDENT_DECODERS'], mode)
                self.assertEqual(env['JUSTRUST_RUSTC_RESIDENT_TYPED'], '0')
                self.assertEqual(env['JUSTRUST_RUSTC_RESIDENT_AST'], '0')
            with self.assertRaises(ValueError):
                harness.environment(None)

    def test_positive_hit_deltas_and_controls(self):
        for mode in ('0', '1'):
            previous = None
            for kind in ('cold', 'hit', 'hit', 'failure'):
                record = response(mode, kind, previous)
                previous = harness.validate_stats(record, previous, mode, kind)
        record = response()
        with self.assertRaisesRegex(AssertionError, 'positive decoder hit'):
            harness.validate_stats(record, record['decoder'], '1', 'hit')
        with self.assertRaisesRegex(AssertionError, 'Disabled decoder'):
            harness.validate_stats(record, None, '0', 'cold')

    def test_optional_native_phase_timing_is_strict_and_monotonic(self):
        first = response()
        first['decoder'].update(selection_nanos=10, publication_nanos=20)
        previous = harness.validate_stats(first, None, '1', 'cold')
        later = response(kind='hit', previous=previous)
        later['decoder'].update(selection_nanos=11, publication_nanos=21)
        harness.validate_stats(later, previous, '1', 'hit')
        for key in ('selection_nanos', 'publication_nanos'):
            for invalid in (True, -1, 1.5, None, 2**64):
                bad = copy.deepcopy(first)
                bad['decoder'][key] = invalid
                with self.assertRaisesRegex(AssertionError, 'phase timing'):
                    harness.validate_stats(bad, None, '1', 'cold')
            bad = copy.deepcopy(later)
            bad['decoder'][key] = previous[key] - 1
            with self.assertRaisesRegex(AssertionError, 'phase timing'):
                harness.validate_stats(bad, previous, '1', 'hit')
            del bad['decoder'][key]
            with self.assertRaisesRegex(AssertionError, 'phase timing'):
                harness.validate_stats(bad, previous, '1', 'hit')
        absent = response(kind='hit', previous=previous)
        absent['decoder'].pop('selection_nanos')
        absent['decoder'].pop('publication_nanos')
        with self.assertRaisesRegex(AssertionError, 'phase timing disappeared'):
            harness.validate_stats(absent, previous, '1', 'hit')
        disabled = response(mode='0')
        disabled['decoder'].update(selection_nanos=0, publication_nanos=0)
        harness.validate_stats(disabled, None, '0', 'cold')
        disabled['decoder']['selection_nanos'] = 1
        with self.assertRaisesRegex(AssertionError, 'Disabled decoder'):
            harness.validate_stats(disabled, None, '0', 'cold')

    def test_bad_telemetry(self):
        for key in harness.COUNTERS:
            for bad in (True, -1, 1.5, None):
                record = response()
                record['decoder'][key] = bad
                with self.assertRaises(AssertionError):
                    harness.validate_stats(record, None, '1', 'cold')
        for bad in (0, 1, None, 'true'):
            record = response()
            record['decoder']['retired'] = bad
            with self.assertRaises(AssertionError):
                harness.validate_stats(record, None, '1', 'cold')
        for field in ('ast', 'lexer', 'live'):
            record = response()
            record[field]['unexpected_typed_reuse'] = 1
            with self.assertRaises(AssertionError):
                harness.validate_stats(record, None, '1', 'cold')
        record = response('1', 'hit', response()['decoder'])
        record['decoder']['misses'] = 0
        with self.assertRaisesRegex(AssertionError, 'decreased'):
            harness.validate_stats(record, response()['decoder'], '1', 'hit')

    def test_failure_requires_retirement(self):
        prior = response()['decoder']
        for key in ('retired', 'retirement_requests'):
            record = response('1', 'failure', prior)
            record['decoder'][key] = False if key == 'retired' else 0
            with self.assertRaises(AssertionError):
                harness.validate_stats(record, prior, '1', 'failure')
        record = response('1', 'failure', prior)
        record['owned']['retirement_reason'] = None
        with self.assertRaisesRegex(AssertionError, 'retire owner'):
            harness.validate_stats(record, prior, '1', 'failure')

    def test_eof_observed_before_stdin_close(self):
        worker = object.__new__(harness.DecoderResident)
        worker.buffer, worker.timeout, worker.response_fd = b'', 1, 9
        worker.selector = Mock()
        worker.selector.select.return_value = [True]
        worker.process = Mock(returncode=0)
        with patch.object(harness.os, 'read', return_value=b''):
            self.assertEqual(worker.retirement_eof(), dict(eof=True, returncode=0))
        worker.process.stdin.close.assert_not_called()
        with patch.object(harness.os, 'read', return_value=b'junk'):
            with self.assertRaisesRegex(AssertionError, 'extra response'):
                worker.retirement_eof()
        worker.selector.select.return_value = []
        with self.assertRaisesRegex(AssertionError, 'did not close'):
            worker.retirement_eof()
        worker.buffer = b'junk'
        with self.assertRaisesRegex(AssertionError, 'trailing'):
            worker.retirement_eof()

    def test_exact_diagnostics_runtime_and_pid(self):
        records = {}
        for transport, nonce in (('fresh', 'abc'), ('resident', 'def')):
            records[transport] = dict(compile=response(), artifact=dict(sha256='digest'),
                runtime=dict(nonce=nonce, execution=dict(status=0, stderr_b64='',
                    stdout_b64=harness.shared.encoded((nonce + ':true:linux:11\n').encode()))))
        harness.validate_pair(records, 12, 11)
        for mutation in ('pid', 'diagnostics', 'runtime', 'artifact', 'timeout'):
            changed = copy.deepcopy(records)
            row = changed['resident']
            if mutation == 'pid':
                row['compile']['pid'] = 99
            elif mutation == 'diagnostics':
                row['compile']['stderr_b64'] = 'bad'
            elif mutation == 'runtime':
                row['runtime']['execution']['stdout_b64'] = harness.shared.encoded(b'def:true:macos:11\n')
            elif mutation == 'artifact':
                del row['artifact']
            else:
                row['compile']['timed_out'] = True
            with self.assertRaises(AssertionError):
                harness.validate_pair(changed, 12, 11)

    def test_mocked_complete_capture_and_recovery(self):
        workers = []
        def create(compiler, root, mode, timeout, evidence=None):
            worker = Mock()
            worker.process.pid = 100 + len(workers)
            worker.command = [str(compiler), '--jr-resident-owned']
            worker.stats = None
            worker.calls = 0
            def compile(argv):
                worker.calls += 1
                kind = 'failure' if worker.calls == 4 else ('cold' if worker.calls == 1 else 'hit')
                record = response(mode, kind, worker.stats)
                worker.stats = record['decoder']
                record['pid'] = worker.process.pid
                if record['status'] == 0:
                    Path(argv[-1]).write_bytes(b'ordinary artifact')
                else:
                    record['stderr_b64'] = 'error'
                return record
            worker.compile.side_effect = compile
            worker.retirement_eof.return_value = dict(eof=True, returncode=0)
            workers.append(worker)
            return worker
        def run(argv, timeout, cwd, env):
            failure = 'false' in (cwd / 'helper.rs').read_text()
            result = dict(status=int(failure), stdout_b64='', stderr_b64='error' if failure else '')
            if not failure:
                Path(argv[-1]).write_bytes(b'fresh artifact')
            return result
        def runtime(binary, root, env, timeout):
            value = 23 if '23' in (root / 'helper.rs').read_text() else 11
            nonce = str(runtime.counter)
            runtime.counter += 1
            return dict(nonce=nonce, execution=dict(status=0, stderr_b64='',
                stdout_b64=harness.shared.encoded(f'{nonce}:true:linux:{value}\n'.encode())))
        runtime.counter = 0
        with tempfile.TemporaryDirectory() as directory, \
                patch.object(harness, 'DecoderResident', side_effect=create), \
                patch.object(harness.shared, 'run', side_effect=run), \
                patch.object(harness, 'runtime', side_effect=runtime), \
                patch.object(harness.shared, 'identity', return_value={'sha256': 'frozen'}):
            out = Path(directory) / 'evidence'
            report = harness.capture('/frozen/rustc', out)
            self.assertEqual(report['verdict'], 'passed', report)
            self.assertEqual(len(workers), 4)
            self.assertEqual([w.calls for w in workers], [4, 1, 4, 1])
            self.assertEqual(json.loads((out / 'receipt.json').read_text()), report)
            for worker in workers:
                worker.close.assert_called_once()
            workers[0].retirement_eof.assert_not_called()
            workers[2].retirement_eof.assert_called_once()
            with self.assertRaises(FileExistsError):
                harness.capture('/frozen/rustc', out)

    def test_identity_failure_retains_receipt(self):
        with tempfile.TemporaryDirectory() as directory, \
                patch.object(harness.shared, 'identity', side_effect=[{'sha256': 'a'}, {'sha256': 'b'}]), \
                patch.object(harness, 'capture_mode', return_value={'verdict': 'passed'}):
            out = Path(directory) / 'evidence'
            report = harness.capture('/compiler', out)
            self.assertEqual(report['verdict'], 'failed')
            self.assertIn('identity_error', report)
            self.assertTrue((out / 'receipt.json').exists())

    def test_failed_mode_preserves_original_response(self):
        with tempfile.TemporaryDirectory() as directory, \
                patch.object(harness, 'DecoderResident') as factory, \
                patch.object(harness.shared, 'run', return_value=dict(status=1, stdout_b64='', stderr_b64='original fresh')):
            root = Path(directory) / 'mode'
            worker = factory.return_value
            worker.process.pid = 12
            worker.command = ['/compiler', '--jr-resident-owned']
            original = response()
            original.update(status=1, stderr_b64='original resident')
            worker.compile.return_value = original
            report = harness.capture_mode('/compiler', root, '1', 1)
            self.assertEqual(report['verdict'], 'failed')
            saved = json.loads((root / 'receipt.json').read_text())
            self.assertEqual(saved['observations'][0]['records']['resident']['compile'], original)
            worker.close.assert_called_once()

    def test_launch_uses_owned_protocol_and_fixed_cwd(self):
        with tempfile.TemporaryDirectory() as directory, \
                patch.object(harness.subprocess, 'Popen') as launch:
            root = Path(directory)
            evidence = root / 'owner'
            evidence.mkdir()
            worker = harness.DecoderResident('/compiler', root, '1', evidence=evidence)
            try:
                args, kwargs = launch.call_args
                self.assertEqual(args[0], ['/compiler', '--jr-resident-owned'])
                self.assertEqual(kwargs['cwd'], root)
                self.assertEqual(kwargs['env']['JUSTRUST_RUSTC_RESIDENT_DECODERS'], '1')
                self.assertEqual(kwargs['env']['JUSTRUST_RUSTC_RESIDENT_TYPED'], '0')
                self.assertEqual(kwargs['pass_fds'], (int(kwargs['env']['JUSTRUST_RUSTC_RESIDENT_FD']),))
            finally:
                worker.selector.close()
                harness.os.close(worker.response_fd)
                worker.stdout.close()
                worker.stderr.close()
                worker.transcript.close()

    def test_no_private_flags_or_incremental(self):
        argv = harness.arguments(Path('/fixture'))
        self.assertFalse(any(x.startswith('-Z') or 'incremental' in x for x in argv))
        self.assertIn('std::matches!', harness.SOURCE)
        self.assertIn('std::env::consts::OS', harness.SOURCE)


if __name__ == '__main__':
    unittest.main()
