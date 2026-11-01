#!/usr/bin/env python3
"""Mocked-only tests. No Rust, Cargo, Blaze, locks, installs or network calls."""
import argparse
import itertools
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

import three_stack as h


def libtest(failing=False, nonce=None, names=None):
    names = h.resident.NAMES if names is None else names
    lines = ['test ' + n + ' ... ' + ('FAILED' if failing and n == h.resident.EDIT_TEST else 'ok') for n in names]
    if nonce is not None:
        lines.append('JR_RUNTIME_EXECUTED_' + str(nonce))
    if failing:
        lines.append('JR_RUNTIME_FAILURE')
    lines.append('test result: ' + ('FAILED' if failing else 'ok') + '. '
                 + str(len(names) - int(failing)) + ' passed; ' + str(int(failing))
                 + ' failed; 0 ignored; 0 measured; 999 filtered out')
    return '\n'.join(lines) + '\n'


class Fixture(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix='ts87-')
        self.addCleanup(self.tmp.cleanup)
        root = Path(self.tmp.name)
        self.args = argparse.Namespace(source=root / 'source', output=root / 'out',
            toolchain=root / h.resident.TOOLCHAIN_NAME, cargo_seed=root / 'seed', samples=6, timeout=1)
        a = self.args
        (a.source / 'src').mkdir(parents=True)
        (a.source / 'src/summary.rs').write_bytes(b'fn shares_overlapping_time() {\nassert_eq!((1, 2), (1, 2));\n}\n')
        for n in range(62):
            (a.source / f'f{n}').write_text('pristine ' + str(n))
        for tool in ('cargo', 'rustc', 'justrust', 'blaze'):
            path = a.toolchain / 'bin' / tool if tool in ('cargo', 'rustc') else root / tool
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(b'\x7fELF' + tool.encode())
            path.chmod(0o700)
            setattr(a, tool, path)
        seed = a.cargo_seed / 'registry/src/index.crates.io-1234/a-1.0.0/src/lib.rs'
        seed.parent.mkdir(parents=True)
        seed.write_text('public sources')
        (seed.parent.parent / 'Cargo.toml').write_text('[package]\nname="a"\nversion="1.0.0"\n')
        self.original = h.resident.source_hashes(a.source)
        self.mtimes = {p: p.stat().st_mtime_ns for p in a.source.rglob('*') if p.is_file()}
        self.calls = []
        self.damage = None
        self.patches = [mock.patch.object(h.resident, 'pinned_source'),
                        mock.patch.object(h, 'identities', side_effect=self.identity),
                        mock.patch.object(h.runtime, 'run_process', side_effect=self.runner)]
        for patch in self.patches:
            patch.start()
            self.addCleanup(patch.stop)

    def identity(self, args):
        return {'tools': {k: {'path': str(getattr(args, k)), 'sha256': h.digest(getattr(args, k))}
                          for k in ('cargo', 'rustc', 'justrust', 'blaze')},
                'toolchain': {'path': str(args.toolchain), 'files': {'driver': 'fake-driver-hash'}},
                'harness': {'three_stack.py': 'mock-harness'}}

    def runner(self, command, cwd, env, prefix, timeout):
        self.calls.append((command, dict(env), prefix))
        phase = prefix.name
        source = (cwd / 'src/summary.rs').read_text()
        nonce = re.search(r'JR_RUNTIME_EXECUTED_(\d+)', source)
        nonce = int(nonce[1]) if nonce else None
        status = 101 if phase in ('type-error', 'assert-failure') else 0
        text = ('error[E0308]: mismatched types JR_RUNTIME_TYPE_ERROR\n' if phase == 'type-error'
                else libtest(phase == 'assert-failure', nonce))
        if phase == 'stop':
            text = 'stopped private daemon\n'
        if self.damage:
            status, text = self.damage(phase, command, status, text)
        prefix.with_suffix('.stdout').write_text(text)
        prefix.with_suffix('.stderr').write_text('')
        if command[0] == str(self.args.justrust):
            runs = Path(env['JUSTRUST_HOME']) / 'runs'
            runs.mkdir(exist_ok=True)
            private = runs / str(len(list(runs.iterdir())))
            private.mkdir()
            (private / 'output.jsonl').write_text(''.join(json.dumps({'s': 'o', 'l': line}) + '\n' for line in text.splitlines()))
            (private / 'meta.json').write_text(json.dumps({'real_cargo': str(self.args.cargo),
                'real_rustc': str(self.args.rustc), 'place': 'local', 'toolchain': None}))
        return {'command': command, 'status': status, 'error': None, 'timed_out': False,
                'elapsed_seconds': 0.125 + len(self.calls) / 10000,
                'noise': {'contamination': 'unknown'}, 'environment': h.runtime.logged_environment(env)}

    def run_ok(self):
        self.assertEqual(h.execute(self.args), 0)
        return h.replay(self.args.output)

    def entries(self):
        return [json.loads(line) for line in (self.args.output / 'journal.jsonl').read_text().splitlines()]

    def rewrite(self, entries):
        path = self.args.output / 'journal.jsonl'
        path.unlink()
        j = h.Journal(self.args.output)
        for e in entries:
            j.add(e['kind'], e['data'])

    def assert_restored(self):
        self.assertEqual(h.resident.source_hashes(self.args.source), self.original)
        for p, mtime in self.mtimes.items():
            self.assertEqual(p.stat().st_mtime_ns, mtime)

    def test_complete_and_offline_replay(self):
        result = self.run_ok()
        self.assertEqual(result['status'], 'complete')
        self.assertEqual(set(result['statistics']), set(h.PHASES))
        for phase in h.PHASES:
            for stack in h.STACKS:
                self.assertEqual(len(result['statistics'][phase][stack]['samples_seconds']), 6)
        self.assertEqual(len(self.calls), 6 * 3 * 7 + 6)
        self.assert_restored()
        with mock.patch.object(subprocess, 'Popen', side_effect=AssertionError('native process forbidden')):
            h.replay(self.args.output)

    def test_environment_isolation_and_shipping_defaults(self):
        with mock.patch.dict(os.environ, {'LD_PRELOAD': '/evil', 'RUSTFLAGS': '-Cbad', 'TOKEN': 'secret',
                                          'JUSTRUST_HOTPATCH': '0', 'CARGO_HOME': '/ambient'}):
            self.run_ok()
        homes = set()
        for _, env, _ in self.calls:
            self.assertNotIn('TOKEN', env)
            self.assertNotIn('LD_PRELOAD', env)
            self.assertNotIn('RUSTFLAGS', env)
            self.assertNotIn('JUSTRUST_HOTPATCH', env)
            self.assertNotIn('JUSTRUST_DEPCACHE', env)
            self.assertEqual(env['JUSTRUST_TOOLCHAIN'], 'system')
            self.assertEqual(env['JUSTRUST_NO_TELEMETRY'], '1')
            self.assertEqual(env['JUSTRUST_BUILDD'], '0')
            self.assertEqual(env['JUSTRUST_BUILDD_PREBUILD'], '0')
            self.assertEqual(env['JUSTRUST_REAL_CARGO'], str(self.args.cargo))
            self.assertEqual(env['RUSTC'], str(self.args.rustc))
            self.assertEqual(env['BLAZE_PROC_MACRO_SERVER'], '0')
            self.assertEqual(env['RA_JIT_PROC_MACRO_SERVER'], '0')
            homes.add(env['CARGO_TARGET_DIR'])
        self.assertEqual(len(homes), 18)

    def test_expensive_identity_checks_precede_reachable_edit(self):
        events = []
        original_write = Path.write_bytes
        pinned = self.args.source / 'src/summary.rs'

        def identity(args):
            events.append('identity')
            return self.identity(args)

        def write(path, data):
            if path == pinned:
                events.append('write')
            return original_write(path, data)

        def run(command, cwd, env, prefix, timeout):
            if prefix.name not in ('cold', 'warm-noop', 'stop'):
                self.assertEqual(events[-2:], ['identity', 'write'])
            events.append('run')
            return self.runner(command, cwd, env, prefix, timeout)

        with mock.patch.object(h, 'identities', side_effect=identity), \
                mock.patch.object(Path, 'write_bytes', write), \
                mock.patch.object(h.runtime, 'run_process', side_effect=run):
            self.run_ok()

    def test_semantic_oracles_preserved(self):
        original = (self.args.source / 'src/summary.rs').read_bytes()
        for phase in h.PHASES:
            changed = h.mutation(original, phase, 234)
            self.assertIn(b'assert_eq!((1, 2), (1, 2));', changed)
        with self.assertRaises(ValueError):
            h.mutation(b'no anchor', 'body-edit', 1)

    def test_invalid_paths(self):
        for path in ('relative', '/home/../tmp/file'):
            with self.subTest(path=path), self.assertRaises(argparse.ArgumentTypeError):
                h.absolute(path)
        link = self.args.source.parent / 'link'
        link.symlink_to(self.args.source, target_is_directory=True)
        with self.assertRaises(argparse.ArgumentTypeError):
            h.absolute(str(link / 'src/summary.rs'))

    def test_output_collision(self):
        self.args.output = self.args.source / 'out'
        with self.assertRaises(ValueError):
            h.execute(self.args)
        self.assertFalse(self.calls)

    def test_existing_output(self):
        self.args.output.mkdir()
        with self.assertRaises(ValueError):
            h.execute(self.args)
        self.assertFalse(self.calls)

    def test_wrappers_refused(self):
        self.args.blaze.write_bytes(b'#!/bin/sh\n')
        with self.assertRaises(ValueError):
            h.execute(self.args)
        self.assertFalse(self.calls)

    def test_samples_and_timeout(self):
        for n, timeout in ((5, 1), (0, 1), (6, float('nan')), (6, 0)):
            self.args.samples, self.args.timeout = n, timeout
            with self.subTest(n=n, timeout=timeout), self.assertRaises(ValueError):
                h.execute(self.args)
        self.assertFalse(self.calls)

    def test_seed_refuses_secrets_and_artifacts(self):
        (self.args.cargo_seed / 'credentials.toml').write_text('secret')
        with self.assertRaises(ValueError):
            h.public_seed(self.args.cargo_seed)

    def test_seed_refuses_symlink(self):
        (self.args.cargo_seed / 'registry/link').symlink_to(self.args.source)
        with self.assertRaises(ValueError):
            h.public_seed(self.args.cargo_seed)

    def test_seed_accepts_actual_public_module_directories(self):
        sources = self.args.cargo_seed / 'registry/src/index.crates.io-1949cf8c6b5b557f'
        for crate, relative in (
                ('windows-sys-0.61.2', 'src/Windows/Win32/Security/Credentials/mod.rs'),
                ('cc-1.6.0', 'src/target/generated.rs')):
            path = sources / crate / relative
            path.parent.mkdir(parents=True)
            path.write_text('pub fn public_module() {}')
            (sources / crate / 'Cargo.toml').write_text('[package]\n')
        manifest = h.public_seed(self.args.cargo_seed)
        self.assertTrue(any('Security/Credentials/mod.rs' in p for p in manifest))
        self.assertTrue(any('cc-1.6.0/src/target/generated.rs' in p for p in manifest))

    def test_seed_refuses_root_secret_and_build_artifact_extras(self):
        for relative in ('credentials', 'credentials.toml', 'target/debug/liba.rlib',
                         'registry/target/debug/liba.rlib',
                         'registry/src/index.crates.io-1234/target/debug/liba.rlib',
                         'registry/src/index.crates.io-1234/credentials.toml'):
            path = self.args.cargo_seed / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text('not public registry source layout')
            with self.subTest(path=relative), self.assertRaises(ValueError):
                h.public_seed(self.args.cargo_seed)
            path.unlink()

    def test_seed_refuses_manifestless_source_artifact_directory(self):
        path = self.args.cargo_seed / 'registry/src/index.crates.io-1234/fake-1.0.0/libfake.rlib'
        path.parent.mkdir(parents=True)
        path.write_text('artifact, not source package')
        with self.assertRaises(ValueError):
            h.public_seed(self.args.cargo_seed)

    def test_source_finally_after_bad_exit(self):
        self.damage = lambda phase, command, code, text: (0 if phase == 'type-error' else code, text)
        self.assertEqual(h.execute(self.args), 1)
        self.assert_restored()
        entries = self.entries()
        self.assertEqual(entries[-1]['data']['status'], 'failed/incomplete')
        self.assertTrue(any(e['kind'] == 'process' and e['data']['phase'] == 'type-error' for e in entries))
        with self.assertRaises(ValueError):
            h.replay(self.args.output)

    def test_missing_named_test_rejected_at_cold(self):
        self.damage = lambda phase, command, code, text: (code, libtest(names=h.resident.NAMES[:-1]))
        self.assertEqual(h.execute(self.args), 1)
        self.assertEqual(len(self.calls), 1)
        self.assert_restored()

    def test_stale_nonce_rejected(self):
        self.damage = lambda phase, command, code, text: (code, re.sub(r'JR_RUNTIME_EXECUTED_\d+', 'JR_RUNTIME_EXECUTED_1', text))
        self.assertEqual(h.execute(self.args), 1)
        self.assert_restored()

    def test_tool_swap_detected_before_next_invocation(self):
        def damage(phase, command, code, text):
            if phase == 'cold':
                self.args.justrust.write_bytes(b'\x7fELFchanged')
            return code, text
        self.damage = damage
        self.assertEqual(h.execute(self.args), 1)
        self.assertEqual(len(self.calls), 1)
        self.assert_restored()

    def test_raw_tamper(self):
        self.run_ok()
        (self.args.output / 'r0/cargo/cold.stdout').write_text('fabricated success')
        with self.assertRaises(ValueError):
            h.replay(self.args.output)

    def test_journal_tamper(self):
        self.run_ok()
        p = self.args.output / 'journal.jsonl'
        p.write_text(p.read_text().replace('999', '998', 1))
        with self.assertRaises(ValueError):
            h.replay(self.args.output)

    def test_reordered_failure_phases(self):
        self.run_ok()
        entries = self.entries()
        outcomes = [e for e in entries if e['kind'] == 'outcome']
        outcomes[3]['data'], outcomes[5]['data'] = outcomes[5]['data'], outcomes[3]['data']
        self.rewrite(entries)
        with self.assertRaises(ValueError):
            h.replay(self.args.output)

    def test_partial_results_are_not_statistics(self):
        self.run_ok()
        entries = self.entries()
        self.rewrite(entries[:-1])
        with self.assertRaises(ValueError):
            h.replay(self.args.output)

    def test_replay_detects_shared_caches(self):
        self.run_ok()
        entries = self.entries()
        states = [e['data'] for e in entries if e['kind'] == 'state']
        states[1]['environment']['CARGO_TARGET_DIR'] = states[0]['environment']['CARGO_TARGET_DIR']
        self.rewrite(entries)
        with self.assertRaises(ValueError):
            h.replay(self.args.output)

    def test_replay_rejects_status_even_with_resealed_chain(self):
        self.run_ok()
        entries = self.entries()
        for e in entries:
            if e['kind'] in ('process', 'outcome') and e['data']['phase'] == 'type-error':
                e['data']['status'] = 0
        self.rewrite(entries)
        with self.assertRaises(ValueError):
            h.replay(self.args.output)

    def test_route_missing_or_remote_rejected(self):
        args = self.args
        for meta in ({}, {'real_cargo': str(args.cargo), 'real_rustc': str(args.rustc), 'place': 'hosted'},
                     {'real_cargo': '/wrong', 'real_rustc': str(args.rustc), 'place': 'local'}):
            self.assertFalse(h.route_valid(meta, args))

    def test_cleanup_exact_private_home(self):
        self.run_ok()
        cleanup = [(cmd, env) for cmd, env, prefix in self.calls if prefix.name == 'stop']
        self.assertEqual(len(cleanup), 6)
        for cmd, env in cleanup:
            self.assertEqual(cmd, [str(self.args.blaze), 'cargo', 'stop'])
            self.assertTrue(Path(env['BLAZE_HOME']).is_relative_to(self.args.output))

    def test_timeout_and_spawn_failure_rejected(self):
        row = {'phase': 'cold', 'nonce': None, 'status': 0, 'timed_out': True, 'error': None}
        self.assertFalse(h.accepted(row, libtest(), None))
        row.update(timed_out=False, error='spawn failed')
        self.assertFalse(h.accepted(row, libtest(), None))

    def test_process_exception_keeps_intent_and_restores(self):
        with mock.patch.object(h.runtime, 'run_process', side_effect=OSError('injected spawn fault')):
            self.assertEqual(h.execute(self.args), 1)
        self.assert_restored()
        entries = self.entries()
        self.assertTrue(any(e['kind'] == 'intent' for e in entries))
        self.assertTrue(any(e['kind'] == 'failure' for e in entries))
        self.assertEqual(entries[-1]['data']['status'], 'failed/incomplete')

    def test_original_atime_and_mtime_restored(self):
        path = self.args.source / 'src/summary.rs'
        before = (1600000000123456789, 1600000000987654321)
        os.utime(path, ns=before)
        self.run_ok()
        self.assertEqual((path.stat().st_atime_ns, path.stat().st_mtime_ns), before)

    def test_missing_justrust_metadata_is_honest_incomplete(self):
        with mock.patch.object(h.runtime, 'recorded_output', side_effect=ValueError('missing private run')):
            self.assertEqual(h.execute(self.args), 1)
        self.assert_restored()
        self.assertIn('missing private run', self.entries()[-1]['data']['error'])

    def test_replay_rejects_reordered_event_kinds(self):
        self.run_ok()
        entries = self.entries()
        i = next(i for i, e in enumerate(entries) if e['kind'] == 'process')
        entries[i], entries[i + 1] = entries[i + 1], entries[i]
        self.rewrite(entries)
        with self.assertRaises(ValueError):
            h.replay(self.args.output)

    def test_replay_rejects_changed_identity_even_resealed(self):
        self.run_ok()
        entries = self.entries()
        entries[-1]['data']['identities_after']['tools']['blaze']['sha256'] = 'changed'
        self.rewrite(entries)
        with self.assertRaises(ValueError):
            h.replay(self.args.output)

    def test_replay_rejects_changed_environment_even_resealed(self):
        self.run_ok()
        entries = self.entries()
        state = next(e['data'] for e in entries if e['kind'] == 'state')
        state['environment']['JUSTRUST_REMOTE'] = 'on'
        self.rewrite(entries)
        with self.assertRaises(ValueError):
            h.replay(self.args.output)

    def test_progress_after_durable_outcome(self):
        def observe(*args, **kwargs):
            self.assertTrue(str(args[0]).startswith('three-stack: sample='))
            self.assertEqual(self.entries()[-1]['kind'], 'outcome')
            self.assertTrue(kwargs['flush'])
        with mock.patch('builtins.print', side_effect=observe) as progress:
            self.run_ok()
        self.assertEqual(progress.call_count, 126)

    def test_replay_after_relocation_without_original_directory(self):
        result = self.run_ok()
        moved = self.args.output.parent / 'relocated'
        self.args.output.rename(moved)
        with mock.patch.object(subprocess, 'Popen', side_effect=AssertionError('native process forbidden')):
            self.assertEqual(h.replay(moved), result)

    def test_replay_rejects_proc_macro_server_change(self):
        self.run_ok()
        entries = self.entries()
        state = next(e['data'] for e in entries if e['kind'] == 'state')
        state['environment']['BLAZE_PROC_MACRO_SERVER'] = '1'
        self.rewrite(entries)
        with self.assertRaises(ValueError):
            h.replay(self.args.output)


class PureTests(unittest.TestCase):
    def test_six_balanced_permutations(self):
        self.assertEqual({h.permutations(i) for i in range(6)}, set(itertools.permutations(h.STACKS)))
        self.assertEqual(h.permutations(6), h.permutations(0))
        for slot in range(3):
            self.assertEqual(sorted(h.permutations(i)[slot] for i in range(6)), sorted(h.STACKS * 2))

    def test_import_never_spawns_native(self):
        # A separate Python process installs a subprocess guard before import.
        code = "import subprocess; subprocess.Popen=lambda *a, **k: (_ for _ in ()).throw(AssertionError('native on import')); import three_stack"
        result = subprocess.run([sys.executable, '-c', code], cwd=Path(h.__file__).parent,
                                capture_output=True, text=True, check=False)
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_commands_no_wrappers_or_blaze_unsupported_flags(self):
        args = argparse.Namespace(cargo=Path('/cargo'), justrust=Path('/justrust'), blaze=Path('/ra-jit'))
        self.assertEqual(h.command(args, 'cargo')[:4], ['/cargo', 'test', '--locked', '--offline'])
        self.assertEqual(h.command(args, 'justrust')[:4], ['/justrust', 'test', '--locked', '--offline'])
        self.assertEqual(h.command(args, 'blaze')[:3], ['/ra-jit', 'cargo', 'test'])
        self.assertNotIn('--offline', h.command(args, 'blaze'))

    def test_libtest_parser_is_tuple_semantic(self):
        parsed = h.runtime.test_results(libtest())
        self.assertTrue(all(isinstance(v, tuple) for v in parsed['tests']))
        self.assertTrue(h.resident.valid_tests(parsed))
        self.assertTrue(h.resident.valid_tests(h.runtime.test_results(libtest(True)), parsed, failing=True))


if __name__ == '__main__':
    unittest.main()
