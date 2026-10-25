#!/usr/bin/env python3
"""Synthetic runner checks only. No Rust builds or performance measurements.

Run: python3 -m unittest discover -s bench -p test_hosted_compiler.py -v
"""
import argparse
import contextlib
import io
import itertools
import json
import math
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import hosted_compiler as bench


class CommandTests(unittest.TestCase):
    def test_default_worker_matrix_and_cli_contract(self):
        args = bench.parser().parse_args(['--source', 'lib.rs', '--binary', 'jr', '--output', 'out'])
        self.assertEqual(args.workers, [1, 8, 24, 48, 96])
        self.assertEqual((args.edition, args.repeats, args.warmups), ('2021', 5, 1))
        matrix = bench.cells(args.workers)
        self.assertEqual(len(matrix), 12)
        self.assertEqual([(c['backend'], c['workers'], c['mode']) for c in matrix[:2]],
                         [('stock', None, 'check'), ('stock', None, 'build')])
        for worker in args.workers:
            for mode in ('check', 'build'):
                cell = dict(backend='native', workers=worker, mode=mode)
                cmd = bench.command(cell, '/jr', '/rustc', '/p/lib.rs', '/out', '2021',
                                    ['feature="std"'])
                self.assertEqual(cmd[:4], ['/jr', '/p/lib.rs', '--lib', '--edition'])
                self.assertIn('2021', cmd)
                self.assertEqual(cmd[cmd.index('--workers') + 1], str(worker))
                self.assertEqual(cmd[cmd.index('--opt') + 1], 'off')
                self.assertEqual(cmd[cmd.index('--debug-assertions') + 1], 'yes')
                self.assertEqual(cmd[-2:], ['--cfg', 'feature="std"'])
                self.assertEqual('--check' in cmd, mode == 'check')
                self.assertEqual('-o' in cmd, mode == 'build')
                self.assertNotIn('--profile', cmd)
                self.assertNotIn('ssh', cmd)

    def test_stock_emit_scopes_and_separate_native_profiles(self):
        for mode, emit in [('check', 'metadata'), ('build', 'link')]:
            cell = dict(backend='stock', workers=None, mode=mode)
            cmd = bench.command(cell, '/jr', '/rustc', '/lib.rs', '/output', '2021')
            self.assertIn('--emit=' + emit, cmd)
            self.assertIn('rlib', cmd)
            self.assertIn('--edition=2021', cmd)
            for flag in ('opt-level=0', 'debug-assertions=yes', 'overflow-checks=yes', 'panic=abort'):
                self.assertIn(flag, cmd)
            self.assertNotIn('--workers', cmd)
            self.assertFalse(any(word.startswith('-Z') for word in cmd))
            with self.assertRaises(ValueError):
                bench.command(cell, '/jr', '/rustc', '/lib.rs', '/output', '2021', profile='/profile')
            cell.update(backend='native', workers=8)
            cmd = bench.command(cell, '/jr', '/rustc', '/lib.rs', '/output', '2021', profile='/profile')
            self.assertEqual(cmd[-2:], ['--profile', '/profile'])

    def test_rotation_and_random_orders_visit_every_cell_once_per_round(self):
        expected = list(range(12))
        rows = list(bench.schedule(12, 12, 'rotate', 1))
        self.assertEqual(rows[0], expected)
        self.assertEqual(rows[1], expected[1:] + expected[:1])
        for slot in range(12):
            self.assertEqual(sorted(row[slot] for row in rows), expected)
        for row in bench.schedule(12, 7, 'random', 48):
            self.assertEqual(sorted(row), expected)
        self.assertEqual(list(bench.schedule(12, 7, 'random', 48)),
                         list(bench.schedule(12, 7, 'random', 48)))
        self.assertNotEqual(list(bench.schedule(12, 7, 'random', 48)),
                            list(bench.schedule(12, 7, 'random', 96)))

    def test_finite_positive_samples_and_arguments(self):
        self.assertEqual(bench.statistics_for([3, 1, 2], 3)['median_ns'], 2)
        for samples in ([], [0], [-1], [True], [1.5], [math.nan], [math.inf]):
            with self.subTest(samples=samples), self.assertRaises(ValueError):
                bench.statistics_for(samples, 1)
        for value in ('nan', 'inf', '-inf', '0', '-1'):
            with self.assertRaises(argparse.ArgumentTypeError):
                bench.positive_timeout(value)
        for value in ('0', '-1'):
            with self.assertRaises(argparse.ArgumentTypeError):
                bench.positive_int(value)
        for text in ('NaN', 'Infinity', '{"bad":-Infinity}', '{"bad":1e999}'):
            with self.assertRaises(ValueError):
                bench.strict_json(text)

    def test_timeout_oserror_and_zero_samples_preserve_full_output(self):
        error = subprocess.TimeoutExpired(['jr'], 1, output=b'partial stdout\xff', stderr=b'full stderr')
        with patch.object(bench.subprocess, 'run', side_effect=error):
            event = bench.invoke(['jr'], '/', {}, 1, timed=True)
        self.assertFalse(event['ok'])
        self.assertEqual(event['stdout'], 'partial stdout\\xff')
        self.assertEqual(event['stderr'], 'full stderr')
        self.assertEqual(event['error'], 'timeout')
        with patch.object(bench.subprocess, 'run', side_effect=FileNotFoundError('missing')):
            event = bench.invoke(['jr'], '/', {}, 1, timed=True)
        self.assertFalse(event['ok'])
        self.assertIn('missing', event['error'])
        with patch.object(bench.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0, b'', b'')), \
             patch.object(bench.time, 'perf_counter_ns', return_value=1):
            event = bench.invoke(['jr'], '/', {}, 1, timed=True)
        self.assertFalse(event['ok'])
        self.assertEqual(event['process_wall_ns'], 0)

    def test_resolution_preserves_rustup_dispatch_basename(self):
        with patch.object(bench.shutil, 'which', return_value='/tool/rustc'):
            self.assertEqual(bench.resolve_binary('rustc'), '/tool/rustc')


class SyntheticRunnerTests(unittest.TestCase):
    """Mocked compiler subprocesses, never source compatibility or speed evidence."""

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.project = self.root / 'project'
        self.project.mkdir()
        self.source = self.project / 'lib.rs'
        self.source.write_text('mod part; // unchanged synthetic input\n')
        (self.project / 'part.rs').write_text('pub fn value() {}')
        (self.project / 'Cargo.toml').write_text('[package]\nname="synthetic"\n')
        self.binary = self.root / 'native'
        self.binary.write_bytes(b'fake native compiler, never executed')
        self.rustc = self.root / 'rustc'
        self.rustc.write_bytes(b'fake rustup dispatcher, never executed')
        self.sysroot = self.root / 'sysroot'
        (self.sysroot / 'bin').mkdir(parents=True)
        self.actual = self.sysroot / 'bin/rustc'
        self.actual.write_bytes(b'fake actual rustc, distinct from dispatcher')
        self.output = self.root / 'result.json'
        self.scratch = self.root / 'scratch'
        self.commands = []
        self.compiles = 0
        self.mutation = None

    def argv(self, extra=()):
        return ['--source', str(self.source), '--binary', str(self.binary),
                '--rustc', str(self.rustc), '--output', str(self.output),
                '--input-root', str(self.project), '--scratch', str(self.scratch),
                '--workers', '1', '8', '--repeats', '2', '--order', 'rotate', *extra]

    def fake(self, cmd, **kwargs):
        self.commands.append((cmd, kwargs))
        self.assertTrue(Path(kwargs['cwd']).is_dir())
        self.assertFalse(kwargs['check'])
        if '--version' in cmd:
            return subprocess.CompletedProcess(cmd, 0, b'rustc synthetic\nLLVM version: synthetic\n', b'')
        if '--print' in cmd:
            return subprocess.CompletedProcess(cmd, 0, str(self.sysroot).encode() + b'\n', b'')
        self.compiles += 1
        self.assertEqual(list(Path(kwargs['cwd']).iterdir()), [])
        native = cmd[0] == str(self.binary)
        mode = 'check' if '--check' in cmd or '--emit=metadata' in cmd else 'build'
        if '-o' in cmd:
            artifact = Path(cmd[cmd.index('-o') + 1])
            self.assertFalse(artifact.exists())
            artifact.write_bytes(b'native artifact' if native else b'rustc artifact')
        if '--profile' in cmd:
            profile = Path(cmd[cmd.index('--profile') + 1])
            profile.write_text(json.dumps(dict(schema=1, success=True, mode=mode,
                                              workers=int(cmd[cmd.index('--workers') + 1]),
                                              compiler_wall_ns=15, phases=[{'phase': 'resolve', 'summed_wall_ns': 4}])))
        payload = dict(schema=1, mode=mode,
                       workers=int(cmd[cmd.index('--workers') + 1]) if native else None,
                       failures=0, capture_complete=True, events_dropped=0, total_ns=20, engine_ns=10)
        stdout = json.dumps(payload).encode() if native else b''
        result = subprocess.CompletedProcess(cmd, 0, stdout, b'full diagnostic warning\n')
        if self.mutation:
            result = self.mutation(cmd, kwargs, result)
        return result

    def run_fake(self, extra=(), mutation=None):
        self.mutation = mutation
        stdout, stderr = io.StringIO(), io.StringIO()
        ticks = itertools.count(100, 50)
        with patch.object(bench.subprocess, 'run', side_effect=self.fake), \
             patch.object(bench.time, 'perf_counter_ns', side_effect=lambda: next(ticks)), \
             patch.object(bench, 'topology', return_value={'host': 'synthetic', 'logical_cpus': 96}), \
             patch.object(bench, 'observation', return_value={'affinity': list(range(96)), 'loadavg': [0.1, 0.2, 0.3]}), \
             contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
            status = bench.main(self.argv(extra))
        return status, json.loads(self.output.read_text()), stderr.getvalue()

    def test_success_metadata_artifact_scopes_fresh_dirs_and_untimed_profiles(self):
        status, report, _ = self.run_fake(['--profile', '--compiler-revision', 'test-revision'])
        self.assertEqual(status, 0)
        self.assertTrue(report['ok'])
        self.assertTrue(report['measurements_ok'])
        self.assertEqual(len(report['cells']), 6)
        self.assertEqual(len(report['events']), 2 + 6 * 3 + 4)
        for cell in report['cells']:
            self.assertEqual(len(cell['warmup_events']), 1)
            self.assertEqual(len(cell['measurement_events']), 2)
            self.assertEqual(cell['statistics']['median_ns'], 50)
            self.assertEqual(cell['statistics']['count'], 2)
        timed = [event for event in report['events'] if event['timed']]
        self.assertEqual(len(timed), 12)
        self.assertTrue(all('--profile' not in event['command'] for event in timed))
        profiles = [event for event in report['events'] if event['phase'] == 'profile']
        self.assertEqual(len(profiles), 4)
        self.assertTrue(all('process_wall_ns' not in event for event in profiles))
        self.assertTrue(all(event['profile']['phases'][0]['phase'] == 'resolve' for event in profiles))
        compile_events = [event for event in report['events'] if event['phase'] != 'metadata']
        self.assertEqual(len({event['cwd'] for event in compile_events}), len(compile_events))
        self.assertEqual(list(self.scratch.iterdir()), [])
        self.assertEqual(self.source.read_text(), 'mod part; // unchanged synthetic input\n')
        metadata = report['metadata']
        self.assertEqual(metadata['source_sha256'], bench.sha256(self.source))
        self.assertEqual(metadata['inputs'][str(self.project / 'part.rs')], bench.sha256(self.project / 'part.rs'))
        self.assertIn(str(self.project / 'Cargo.toml'), metadata['inputs'])
        for path in (self.binary, self.rustc, self.actual):
            self.assertEqual(metadata['binary_hashes'][str(path)], bench.sha256(path))
        self.assertIn('LLVM version: synthetic', metadata['rustc_verbose'])
        self.assertEqual(metadata['topology']['logical_cpus'], 96)
        self.assertEqual(metadata['edition'], '2021')
        self.assertEqual(metadata['compiler_revision'], 'test-revision')
        self.assertEqual(metadata['timed_rustc'], str(self.actual))
        self.assertTrue(all(event['command'][0] == str(self.actual)
                            for event in timed if report['cells'][event['cell']]['backend'] == 'stock'))
        self.assertIn('NOT a reusable stock rlib', report['backend_scopes']['native'])
        self.assertIn('rustc/LLVM', report['backend_scopes']['stock'])
        self.assertIn('includes exec/startup', report['measurement'])
        self.assertTrue(all(event['before']['affinity'] == list(range(96)) for event in compile_events))

    def test_failed_warmup_retains_complete_error_and_no_measurements(self):
        def fail(cmd, kwargs, result):
            if cmd[0] == str(self.binary):
                return subprocess.CompletedProcess(cmd, 9, b'complete failure stdout', b'complete failure stderr\xff')
            return result
        status, report, diagnostics = self.run_fake(mutation=fail)
        self.assertEqual(status, 1)
        self.assertFalse(report['ok'])
        self.assertIn('warmup failed', report['error'])
        self.assertFalse(any(event['timed'] for event in report['events']))
        failed = next(event for event in report['events'] if event['returncode'] == 9)
        self.assertEqual(failed['stdout'], 'complete failure stdout')
        self.assertEqual(failed['stderr'], 'complete failure stderr\\xff')
        self.assertIn(failed['stderr'], diagnostics)
        self.assertEqual(list(self.scratch.iterdir()), [])

    def test_measured_failure_does_not_make_partial_stats_look_successful(self):
        def fail(cmd, kwargs, result):
            if self.compiles == 7:  # first measured cell, after all six warmups
                return subprocess.CompletedProcess(cmd, 7, b'full measured error', b'detail')
            return result
        status, report, _ = self.run_fake(mutation=fail)
        self.assertEqual(status, 1)
        self.assertFalse(report['measurements_ok'])
        self.assertIsNone(report['cells'][0]['statistics'])
        self.assertTrue(all(len(cell['measurement_events']) == 2 for cell in report['cells']))
        self.assertTrue(all(cell['statistics']['count'] == 2 for cell in report['cells'][1:]))
        self.assertEqual(sum(event['timed'] for event in report['events']), 12)

    def test_identity_changes_are_fail_closed_with_saved_event(self):
        for which in ('source', 'binary', 'actual rustc'):
            with self.subTest(which=which):
                if self.output.exists():
                    self.output.unlink()
                self.compiles = 0
                def change(cmd, kwargs, result):
                    path = {'source': self.source, 'binary': self.binary, 'actual rustc': self.actual}[which]
                    path.write_bytes(path.read_bytes() + b'changed')
                    return result
                status, report, _ = self.run_fake(mutation=change)
                self.assertEqual(status, 1)
                self.assertIn('identity changed', report['error'])
                self.assertFalse(report['measurements_ok'])
                self.assertEqual(len(report['events']), 3)
                self.assertEqual(list(self.scratch.iterdir()), [])

    def test_native_summary_invalid_capture_or_nonfinite_data_is_not_a_sample(self):
        for mutation in ({'capture_complete': False}, {'events_dropped': 1}, {'total_ns': math.nan},
                         {'total_ns': True}, {'workers': 96}, {'failures': 1}):
            with self.subTest(mutation=mutation):
                if self.output.exists():
                    self.output.unlink()
                def invalid(cmd, kwargs, result):
                    if cmd[0] == str(self.binary):
                        data = json.loads(result.stdout)
                        data.update(mutation)
                        result.stdout = json.dumps(data).encode()
                    return result
                status, report, _ = self.run_fake(mutation=invalid)
                self.assertEqual(status, 1)
                self.assertIn('warmup failed', report['error'])
                self.assertTrue(any(not event['ok'] for event in report['events']))
                json.dumps(report, allow_nan=False)

    def test_missing_artifact_and_invalid_separate_profile_are_failures(self):
        def missing(cmd, kwargs, result):
            if '-o' in cmd:
                Path(cmd[cmd.index('-o') + 1]).unlink()
            return result
        status, report, _ = self.run_fake(mutation=missing)
        self.assertEqual(status, 1)
        self.assertIn('artifact', next(event['error'] for event in report['events'] if not event['ok']))
        self.output.unlink()
        self.compiles = 0
        def bad_profile(cmd, kwargs, result):
            if '--profile' in cmd:
                Path(cmd[cmd.index('--profile') + 1]).write_text('{"success": false}')
            return result
        status, report, _ = self.run_fake(['--profile'], mutation=bad_profile)
        self.assertEqual(status, 1)
        self.assertFalse(report['ok'])
        self.assertTrue(report['measurements_ok'])
        self.assertTrue(all(cell['statistics']['count'] == 2 for cell in report['cells']))
        self.assertTrue(all(not event['ok'] for event in report['events'] if event['phase'] == 'profile'))

    def test_existing_output_not_overwritten_and_duplicate_workers_rejected(self):
        self.output.write_text('preserve')
        with self.assertRaises(SystemExit), contextlib.redirect_stderr(io.StringIO()):
            bench.main(self.argv())
        self.assertEqual(self.output.read_text(), 'preserve')
        self.output.unlink()
        status, report, _ = self.run_fake(['--workers', '1', '1'])
        self.assertEqual(status, 1)
        self.assertIn('duplicate worker', report['error'])

    def test_inventory_rejects_symlinks_and_preserves_manifests(self):
        (self.project / 'link.rs').symlink_to(self.source)
        with self.assertRaisesRegex(ValueError, 'symlink'):
            bench.inventory([self.project])
        (self.project / 'link.rs').unlink()
        self.assertEqual(len(bench.inventory([self.project])), 3)
        with patch.object(bench.os, 'environ', {'PATH': '/usr/bin:/bin', 'RUSTFLAGS': 'injected'}):
            _, report, _ = self.run_fake()
        self.assertNotIn('RUSTFLAGS', report['metadata']['environment'])
        self.assertEqual(report['metadata']['environment']['JR_OPT'], 'off')

    def test_failed_toolchain_metadata_is_saved_before_any_compile(self):
        def fail(cmd, **kwargs):
            return subprocess.CompletedProcess(cmd, 17, b'complete metadata output', b'metadata failure')
        with patch.object(bench.subprocess, 'run', side_effect=fail), \
             patch.object(bench, 'topology', return_value={'host': 'synthetic'}), \
             contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            status = bench.main(self.argv())
        report = json.loads(self.output.read_text())
        self.assertEqual(status, 1)
        self.assertFalse(report['ok'])
        self.assertIn('cannot pin stock toolchain', report['error'])
        self.assertEqual(len(report['events']), 1)
        self.assertEqual(report['events'][0]['stdout'], 'complete metadata output')
        self.assertEqual(report['events'][0]['stderr'], 'metadata failure')
        self.assertFalse(report['events'][0]['timed'])

    def test_real_cli_dispatch_with_synthetic_executables_not_rust_compilers(self):
        program = f'''#!{sys.executable}
from pathlib import Path
import json
import sys
if "--version" in sys.argv:
    print("synthetic toolchain, not rustc")
    raise SystemExit(0)
if "--print" in sys.argv:
    print({str(self.sysroot)!r})
    raise SystemExit(0)
native = "--workers" in sys.argv
mode = "check" if "--check" in sys.argv or "--emit=metadata" in sys.argv else "build"
workers = int(sys.argv[sys.argv.index("--workers") + 1]) if native else None
if "-o" in sys.argv:
    Path(sys.argv[sys.argv.index("-o") + 1]).write_bytes(b"synthetic artifact, not native code")
if "--profile" in sys.argv:
    Path(sys.argv[sys.argv.index("--profile") + 1]).write_text(json.dumps(dict(
        schema=1, mode=mode, workers=workers, success=True, compiler_wall_ns=10)))
if native:
    print(json.dumps(dict(schema=1, mode=mode, workers=workers, failures=0,
        capture_complete=True, events_dropped=0, total_ns=20, engine_ns=10)))
'''
        for path in (self.binary, self.rustc, self.actual):
            path.write_text(program)
            path.chmod(0o700)
        completed = subprocess.run([sys.executable, str(Path(bench.__file__).absolute()),
                                    *self.argv(['--profile'])], capture_output=True, timeout=20)
        self.assertEqual(completed.returncode, 0, completed.stderr)
        report = json.loads(self.output.read_text())
        self.assertTrue(report['ok'])
        self.assertEqual(sum(event['timed'] for event in report['events']), 12)
        self.assertTrue(all(cell['statistics']['count'] == 2 for cell in report['cells']))
        self.assertEqual(list(self.scratch.iterdir()), [])


if __name__ == '__main__':
    unittest.main()
