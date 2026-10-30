#!/usr/bin/env python3
"""Synthetic harness tests. Never load Blaze or a native compiler."""
import argparse
import json
import os
from pathlib import Path
import signal
import sys
import tempfile
import unittest
from unittest.mock import patch

import blaze_runtime_compare as h

NAME = 'summary::tests::shares_overlapping_time'
ORIGINAL = b'fn shares_overlapping_time() {\n}\n'


def output(names=(NAME,), marker='', failed=False):
    lines = [f'test {name} ... ' + (marker + '\n' if marker else '') +
             ('FAILED' if failed and name == NAME else 'ok') for name in names]
    return '\n'.join(lines) + (f'\ntest result: {"FAILED" if failed else "ok"}. '
        f'{len(names) - int(failed)} passed; {int(failed)} failed; 0 ignored; 0 measured; 2 filtered out\n')


class HarnessTests(unittest.TestCase):
    def test_declarative_only_stack(self):
        with tempfile.TemporaryDirectory() as temp:
            args = argparse.Namespace(output=Path(temp), cargo=Path('/mock/cargo'),
                                      rustc=Path('/mock/rustc'), compiler_nightly=True)
            with patch.dict(os.environ, {'JUSTRUST_CACHE_PROC_MACROS': '1',
                                        'RUSTFLAGS': '-Zcache-proc-macros=yes'}, clear=False):
                env = h.environment(args, 'owned-declarative')
            self.assertEqual(env['JUSTRUST_CACHE_PROC_MACROS'], '0')
            self.assertIn('-Zjr-cache-declarative-macros=yes', env['RUSTFLAGS'])
            self.assertNotIn('-Zcache-proc-macros=yes', env['RUSTFLAGS'])
            for flag in h.base.FLAGS:
                self.assertIn(f'-Z{flag}=no', env['RUSTFLAGS'])
            self.assertNotIn('JUSTRUST_HOTPATCH', env)

    def test_false_success_and_counts(self):
        row = dict(status=0, timed_out=False, error=None)
        for text in ('Finished compilation', '', output().replace('1 passed', '2 passed')):
            self.assertFalse(h.accepted(row, text))
        self.assertTrue(h.accepted(row, output()))
        self.assertFalse(h.accepted(dict(row, status=-9), output()))
        self.assertFalse(h.accepted(dict(row, timed_out=True), output()))

    def test_missing_extra_and_stale(self):
        row = dict(status=0, timed_out=False, error=None)
        expected = h.test_results(output())
        for names in (('different',), (NAME, 'extra')):
            self.assertFalse(h.accepted(row, output(names), expected))
        self.assertFalse(h.accepted(row, output(), expected, 'JR_RUNTIME_EXECUTED_2'))
        self.assertFalse(h.accepted(row, output(marker='JR_RUNTIME_EXECUTED_20'), expected,
                                    'JR_RUNTIME_EXECUTED_2'))
        self.assertTrue(h.accepted(row, output(marker='JR_RUNTIME_EXECUTED_2'), expected,
                                   'JR_RUNTIME_EXECUTED_2'))
        for marker in ('JR_RUNTIME_EXECUTED_2 JR_RUNTIME_EXECUTED_2',
                       'JR_RUNTIME_EXECUTED_1 JR_RUNTIME_EXECUTED_2'):
            self.assertFalse(h.accepted(row, output(marker=marker), expected,
                                       'JR_RUNTIME_EXECUTED_2'))

    def test_native_test_duration_suffix(self):
        for suffix in (' <0.000s>', ' <1s>', ' <12.345s>'):
            for state in ('ok', 'FAILED', 'ignored'):
                passed, failed, ignored = [int(state == s) for s in ('ok', 'FAILED', 'ignored')]
                result = 'FAILED' if failed else 'ok'
                text = (f'test {NAME} ... {state}{suffix}\n'
                        f'test result: {result}. {passed} passed; {failed} failed; '
                        f'{ignored} ignored; 0 measured; 2 filtered out\n')
                parsed = h.test_results(text)
                self.assertIsNotNone(parsed, text)
                self.assertEqual(parsed['tests'], [(NAME, state)])
        for suffix in (' garbage', ' <0.000s> garbage', ' <nans>', ' <-1s>',
                       ' <1.2 s>', ' <.2s>', ' <1s> <2s>'):
            self.assertIsNone(h.test_results(output().replace('... ok', '... ok' + suffix)))
        text = output(marker='JR_RUNTIME_EXECUTED_1').replace('\nok\n', '\nok <0.000s>\n')
        self.assertTrue(h.accepted(dict(status=0, timed_out=False, error=None), text,
                                   marker='JR_RUNTIME_EXECUTED_1'))

    def test_failure_output(self):
        parsed = h.test_results(output(marker='panic: JR_RUNTIME_FAILURE', failed=True))
        self.assertEqual(parsed['failed'], 1)
        self.assertEqual(parsed['tests'], [(NAME, 'FAILED')])

    def test_same_edits_and_order(self):
        stacks = ['owned-off', 'all3on', 'blaze']
        for rep in range(5):
            self.assertEqual(len({h.edit_bytes(ORIGINAL, rep) for _ in stacks}), 1)
            self.assertEqual(h.order(stacks, rep)[0], stacks[rep % 3])
        self.assertNotEqual(h.edit_bytes(ORIGINAL, 0), h.edit_bytes(ORIGINAL, 1))
        with self.assertRaises(ValueError):
            h.edit_bytes(ORIGINAL + ORIGINAL)

    def test_environment_privacy(self):
        env = dict(h.PROFILES, SECRET_TOKEN='do not log', AWS_SECRET_ACCESS_KEY='secret',
                   BLAZE_BACKEND='llvm')
        logged = h.logged_environment(env)
        self.assertNotIn('SECRET_TOKEN', logged)
        self.assertNotIn('AWS_SECRET_ACCESS_KEY', logged)
        self.assertEqual(logged['CARGO_PROFILE_TEST_OPT_LEVEL'], '0')
        self.assertEqual(logged['BLAZE_BACKEND'], 'llvm')

    def test_explicit_compiler_nightly_detection_and_pin(self):
        for text in ('rustc 1.101.0-nightly (abc 2026-10-27)\n',
                     'rustc custom (abc)\nrelease: 1.101.0-nightly\n'):
            self.assertTrue(h.nightly_version(text))
        for text in ('rustc 1.101.0 (abc)\nrelease: 1.101.0\n',
                     'rustc 1.101.0-beta\nrelease: 1.101.0-beta\n',
                     'rustc stable\nnotes: nightly\n', 'not rustc nightly',
                     'rustc 1.101.0-nightly-fake\nrelease: 1.101.0\n'):
            self.assertFalse(h.nightly_version(text))
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            for nightly in (False, True):
                args = argparse.Namespace(output=root / str(nightly), cargo=Path('/fake/cargo'),
                                          rustc=Path('/fake/rustc'), compiler_nightly=nightly)
                with patch.dict(os.environ, {'JUSTRUST_TOOLCHAIN_NIGHTLY': 'unverified'}):
                    for stack in ('owned-off', 'all3on', 'all4on', 'all4hotpatch'):
                        env = h.environment(args, stack)
                        self.assertEqual(env.get('JUSTRUST_TOOLCHAIN_NIGHTLY'), '1' if nightly else None)
                        self.assertEqual(h.logged_environment(env).get('JUSTRUST_TOOLCHAIN_NIGHTLY'),
                                         '1' if nightly else None)

    def test_version_query_once_identity_verified(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            rustc = root / 'compiler'
            rustc.write_bytes(b'fake compiler, never loaded')
            args = argparse.Namespace(rustc=rustc, source=root, output=root, timeout=1)
            def fake(command, cwd, env, prefix, timeout):
                self.assertEqual(command, [str(rustc), '-vV'])
                prefix.with_suffix('.stdout').write_text('rustc 1.101.0-nightly (abc)\nrelease: 1.101.0-nightly\n')
                prefix.with_suffix('.stderr').write_text('')
                return dict(status=0, timed_out=False, error=None, elapsed_seconds=.001)
            with patch.object(h, 'run_process', side_effect=fake) as run:
                receipt = h.classify_compiler(args, h.base.digest(rustc))
                self.assertEqual(run.call_count, 1)
            self.assertTrue(receipt['nightly'])
            self.assertEqual(receipt['sha256_before'], receipt['sha256_after'])
            self.assertTrue((root / 'rustc-version.json').is_file())
            with patch.object(h, 'run_process') as run:
                with self.assertRaises(ValueError):
                    h.classify_compiler(args, 'wrong-hash')
                run.assert_not_called()
            args.output = root / 'mutation'
            args.output.mkdir()
            def changed(*a, **kw):
                row = fake(*a, **kw)
                rustc.write_bytes(b'compiler replaced during version query')
                return row
            with patch.object(h, 'run_process', side_effect=changed):
                with self.assertRaises(ValueError):
                    h.classify_compiler(args, h.base.digest(rustc))

    def test_blaze_backend_and_opt_pinned_and_logged(self):
        # No compiler or daemon is loaded. Exercise real isolation/environment
        # setup with an empty fake installed toolchain directory.
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            installed = root / 'installed'
            (installed / 'toolchains').mkdir(parents=True)
            with patch.dict(os.environ, {'BLAZE_HOME': str(installed),
                                        'BLAZE_BACKEND': 'wrong-inherited',
                                        'BLAZE_OPT': '3', 'PRIVATE_TOKEN': 'secret'}):
                for stack, backend in (('blaze', 'cranelift'), ('blaze-llvm', 'llvm')):
                    args = argparse.Namespace(output=root / stack)
                    env = h.environment(args, stack)
                    self.assertEqual(env['BLAZE_BACKEND'], backend)
                    self.assertEqual(env['BLAZE_OPT'], '0')
                    for logger in (h.logged_environment, h.base.logged_environment):
                        logged = logger(env)
                        self.assertEqual(logged['BLAZE_BACKEND'], backend)
                        self.assertEqual(logged['BLAZE_OPT'], '0')
                        self.assertNotIn('PRIVATE_TOKEN', logged)
                    for key, value in h.PROFILES.items():
                        self.assertEqual(env[key], value)

    def test_timeout_signal_exact_output(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            row = h.run_process([sys.executable, '-c',
                'import time; print("exact", flush=True); time.sleep(10)'], root,
                os.environ.copy(), root / 'timeout', .1)
            self.assertTrue(row['timed_out'])
            self.assertEqual(row['status'], -signal.SIGKILL)
            self.assertEqual((root / 'timeout.stdout').read_bytes(), b'exact\n')
            row = h.run_process([sys.executable, '-c',
                'import os,signal; os.kill(os.getpid(),signal.SIGTERM)'], root,
                os.environ.copy(), root / 'signal', 5)
            self.assertFalse(row['timed_out'])
            self.assertEqual(row['status'], -signal.SIGTERM)

    def exercise(self, root, crash=False, stale=False, raw_mode='valid', experiments=False, seeded=False, journal=True, macro=False, missing_journal_phase=None):
        source = root / 'source'
        (source / 'src').mkdir(parents=True)
        pinned = source / 'src/summary.rs'
        pinned.write_bytes(ORIGINAL)
        args = argparse.Namespace(source=source, output=root / 'out', samples=2,
            edit_file='src/summary.rs', edit_test=NAME, edit_anchor='fn shares_overlapping_time() {',
            blaze_llvm=True, command='test', test_filter='summary::tests::', timeout=1,
            binary='justrust', blaze=Path('/fake/blaze'), justrust=Path('/fake/justrust'),
            owned_experiments=experiments, native_macro_cache=macro,
            cargo=Path('/fake/cargo'), rustc=Path('/fake/rustc'),
            blaze_seed_cache=root / 'seed' if seeded else None,
            wild=root / 'fake-wild' if experiments or macro else None)
        if args.wild:
            args.wild.write_bytes(b'fake executable, never invoked')
            args.wild.chmod(0o700)
        if seeded:
            args.blaze_seed_cache.mkdir()
            (args.blaze_seed_cache / 'test.pack').write_bytes(b'private seeded pack')
        observed = []
        real_environment = h.environment
        def synthetic_environment(a, stack):
            if stack.startswith('blaze'):
                return {'BLAZE_CACHE': str(a.output / stack / 'cache')}
            return real_environment(a, stack)
        def fake(command, cwd, env, prefix, timeout):
            contents = pinned.read_bytes()
            observed.append((prefix.name, contents))
            if seeded and command[0] == '/fake/blaze':
                self.assertEqual((Path(env['BLAZE_CACHE']) / 'test.pack').read_bytes(),
                                 b'private seeded pack')
            if crash:
                raise RuntimeError('synthetic interruption')
            text, status = output(), 0
            if b'JR_RUNTIME_TYPE_ERROR' in contents:
                text, status = 'JR_RUNTIME_TYPE_ERROR error[E0308]', 101
            elif b'JR_RUNTIME_FAILURE' in contents:
                text, status = output(marker='JR_RUNTIME_FAILURE', failed=True), 101
            elif b'JR_RUNTIME_EXECUTED_' in contents and not stale:
                import re
                marker = re.search(rb'JR_RUNTIME_EXECUTED_\d+', contents).group().decode()
                text = output(marker=marker)
            if command[0] == '/fake/justrust':
                if args.wild:
                    private_wild = Path(env['JUSTRUST_HOME']) / 'toolchains/linkers/wild-0.10.0/wild'
                    self.assertEqual(private_wild.read_bytes(), args.wild.read_bytes())
                    self.assertNotEqual(private_wild.stat().st_ino, args.wild.stat().st_ino)
                recording = Path(env['JUSTRUST_HOME']) / 'runs' / f'run-{len(observed)}'
                if raw_mode != 'missing':
                    recording.mkdir(parents=True)
                    raw = ''.join(json.dumps({'t': i, 's': 'o', 'l': line}) + '\n'
                                  for i, line in enumerate(text.splitlines()))
                    (recording / 'output.jsonl').write_text(raw if raw_mode == 'valid' else '{bad json')
                    if (env.get('JUSTRUST_HOTPATCH') == '1' and journal
                            and not prefix.name.startswith(str(missing_journal_phase) + '-')):
                        (recording / 'hotpatch.jsonl').write_text(json.dumps({
                            'outcome': 'patched' if prefix.name.startswith('edit-') else 'full-link',
                            'detail': 'synthetic test evidence', 'ms': .1}) + '\n')
                text = 'justrust: condensed UI, no test names or diagnostics'
            prefix.with_suffix('.stdout').write_text(text)
            prefix.with_suffix('.stderr').write_text('')
            return dict(status=status, error=None, timed_out=False, elapsed_seconds=.001)
        with patch.object(h.base, 'identities', return_value={'fake': 'identity', 'rustc': {'sha256': 'fake'}}), \
             patch.object(h, 'classify_compiler', return_value={'nightly': False}), \
             patch.object(h, 'environment', side_effect=synthetic_environment), \
             patch.object(h, 'run_process', side_effect=fake):
            if crash:
                with self.assertRaises(RuntimeError):
                    h.execute(args)
            else:
                self.assertEqual(h.execute(args), 1 if stale or raw_mode != 'valid' or
                                 ((experiments or macro) and (not journal or missing_journal_phase)) else 0)
        self.assertEqual(pinned.read_bytes(), ORIGINAL)
        self.assertEqual((args.output / 'source.original').read_bytes(), ORIGINAL)
        return observed

    def test_end_to_end_gates_same_bytes_rotation(self):
        with tempfile.TemporaryDirectory() as temp:
            rows = self.exercise(Path(temp))
            metadata = json.loads((Path(temp) / 'out/metadata.json').read_text())
            self.assertEqual(metadata['stacks'], ['owned-off', 'all3on', 'blaze', 'blaze-llvm'])
            self.assertEqual(metadata['known_unsafe_external_input_stacks'], [])
            summary = json.loads((Path(temp) / 'out/summary.json').read_text())
            self.assertEqual(summary['known_unsafe_external_input_stacks'], [])
            for rep in range(2):
                edits = [(name, data) for name, data in rows if name.startswith(f'edit-{rep:03d}')]
                self.assertEqual(len(edits), 4)
                self.assertEqual(len({data for _, data in edits}), 1)
                self.assertIn('owned-off' if rep == 0 else 'all3on', edits[0][0])

    def test_owned_experiments_isolation_flags_and_logging(self):
        with tempfile.TemporaryDirectory() as temp:
            args = argparse.Namespace(output=Path(temp), cargo=Path('/fake/cargo'), rustc=Path('/fake/rustc'))
            envs = {s: h.environment(args, s) for s in ('owned-off', 'all3on', 'all4on', 'all4hotpatch')}
            self.assertEqual(len({e['CARGO_TARGET_DIR'] for e in envs.values()}), 4)
            self.assertEqual(len({e['HOME'] for e in envs.values()}), 4)
            for stack in ('owned-off', 'all3on'):
                self.assertNotIn('jr-cache-dead-code-items', envs[stack]['RUSTFLAGS'])
                self.assertNotIn('JUSTRUST_HOTPATCH', envs[stack])
            for stack in ('all4on', 'all4hotpatch'):
                self.assertEqual(envs[stack]['RUSTFLAGS'].split(),
                                 [f'-Z{f}=yes' for f in (*h.base.FLAGS, 'jr-cache-dead-code-items')])
            self.assertNotIn('JUSTRUST_HOTPATCH', envs['all4on'])
            for logger in (h.logged_environment, h.base.logged_environment):
                self.assertEqual(logger(envs['all4hotpatch'])['JUSTRUST_HOTPATCH'], '1')

    def test_experiments_and_seed_keep_all_gates(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            rows = self.exercise(root, experiments=True, seeded=True)
            for stack in ('all4on', 'all4hotpatch'):
                phases = [name for name, _ in rows if name.endswith('-' + stack)]
                for phase in ('prepare', 'type-error', 'type-recovery', 'test-failure', 'test-recovery'):
                    self.assertIn(f'{phase}-000-{stack}', phases)
                self.assertEqual(sum(name.startswith('edit-') for name in phases), 2)
            metadata = json.loads((root / 'out/metadata.json').read_text())
            self.assertEqual(set(metadata['wild_copies']), {'owned-off', 'all3on', 'all4on', 'all4hotpatch'})
            self.assertEqual(metadata['identities_before']['wild'], metadata['identities_after']['wild'])
            records = json.loads((root / 'out/records.json').read_text())
            hotpatch = next(r['raw_output']['hotpatch'] for r in records
                            if r['stack'] == 'all4hotpatch' and r['phase'] == 'edit')
            self.assertEqual(hotpatch['outcomes'], ['patched'])
            self.assertTrue((root / 'out' / hotpatch['artifact']).is_file())
            self.assertEqual(metadata['store_scope'], 'warm-store seeded')
            self.assertEqual(set(metadata['blaze_seed_manifests']), {'blaze', 'blaze-llvm'})
            for manifest in metadata['blaze_seed_manifests'].values():
                self.assertEqual(manifest['files'], {'test.pack': h.base.digest(root / 'seed/test.pack')})
            self.assertEqual((root / 'seed/test.pack').read_bytes(), b'private seeded pack')

    def test_hotpatch_without_journal_not_ranked(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            self.exercise(root, experiments=True, journal=False)
            summary = json.loads((root / 'out/summary.json').read_text())
            self.assertNotIn('all4hotpatch', summary['medians_seconds'])
            self.assertIn('all4on', summary['medians_seconds'])

    def test_native_macro_flags_isolation_and_unchanged_baselines(self):
        with tempfile.TemporaryDirectory() as temp:
            args = argparse.Namespace(output=Path(temp), cargo=Path('/fake/cargo'),
                                      rustc=Path('/shim/not-used-for-inference'))
            stacks = ('owned-off', 'all3on', 'all4on', 'all4hotpatch',
                      'owned-macro', 'owned-macro-hotpatch')
            with patch.dict(os.environ, {'RUSTFLAGS': 'secret inherited flags',
                                        'JUSTRUST_HOTPATCH': '1'}):
                envs = {s: h.environment(args, s) for s in stacks}
            for key in ('HOME', 'JUSTRUST_HOME', 'CARGO_TARGET_DIR'):
                self.assertEqual(len({e[key] for e in envs.values()}), len(stacks))
            for stack in stacks[:4]:
                self.assertNotIn('cache-proc-macros', envs[stack]['RUSTFLAGS'])
            for stack in stacks[4:]:
                self.assertEqual(envs[stack]['RUSTFLAGS'],
                                 envs['all4on']['RUSTFLAGS'] + ' -Zcache-proc-macros=yes')
                self.assertEqual(h.logged_environment(envs[stack])['RUSTFLAGS'],
                                 envs[stack]['RUSTFLAGS'])
            self.assertNotIn('JUSTRUST_HOTPATCH', envs['owned-macro'])
            self.assertEqual(envs['owned-macro-hotpatch']['JUSTRUST_HOTPATCH'], '1')
            left, right = (dict(envs[s]) for s in stacks[4:])
            left.pop('JUSTRUST_HOTPATCH', None)
            right.pop('JUSTRUST_HOTPATCH', None)
            left = {k: v.replace(str(args.output / 'owned-macro'), '<private>')
                    for k, v in left.items()}
            right = {k: v.replace(str(args.output / 'owned-macro-hotpatch'), '<private>')
                     for k, v in right.items()}
            self.assertEqual(left, right)

    def test_native_macro_actual_journal_gates_and_seed_policy(self):
        for phase in (None, 'prepare', 'edit'):
            with self.subTest(phase=phase), tempfile.TemporaryDirectory() as temp:
                root = Path(temp)
                rows = self.exercise(root, macro=True, seeded=True, missing_journal_phase=phase)
                summary = json.loads((root / 'out/summary.json').read_text())
                self.assertEqual('owned-macro-hotpatch' in summary['medians_seconds'], phase is None)
                self.assertIn('owned-macro', summary['medians_seconds'])
                self.assertEqual(summary['known_unsafe_external_input_stacks'],
                                 ['owned-macro', 'owned-macro-hotpatch'])
                self.assertIn('not correctness-certified', summary['macro_cache_safety'])
                metadata = json.loads((root / 'out/metadata.json').read_text())
                self.assertEqual(set(metadata['wild_copies']),
                                 {'owned-off', 'all3on', 'owned-macro', 'owned-macro-hotpatch'})
                self.assertEqual(set(metadata['blaze_seed_manifests']), {'blaze', 'blaze-llvm'})
                self.assertEqual(metadata['known_unsafe_external_input_stacks'],
                                 summary['known_unsafe_external_input_stacks'])
                if phase is None:
                    self.assertEqual(summary['edit_hotpatch_outcomes']['owned-macro-hotpatch'],
                                     ['patched', 'patched'])
                    for stack in ('owned-macro', 'owned-macro-hotpatch'):
                        for gate in ('prepare', 'type-error', 'type-recovery', 'test-failure', 'test-recovery'):
                            self.assertIn(f'{gate}-000-{stack}', [name for name, _ in rows])

    def test_native_macro_cli_requires_wild_before_execution(self):
        argv = ['compare', '--native-macro-cache']
        for key in ('source', 'output', 'blaze', 'justrust', 'rustc', 'cargo'):
            argv += ['--' + key, '/fake/' + key]
        with patch.object(sys, 'argv', argv), patch.object(h, 'execute') as execute:
            with self.assertRaises(SystemExit) as error:
                h.main()
            self.assertEqual(error.exception.code, 2)
            execute.assert_not_called()

    def test_native_macro_and_existing_experiments_are_distinct(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            self.exercise(root, macro=True, experiments=True)
            metadata = json.loads((root / 'out/metadata.json').read_text())
            self.assertEqual(len(metadata['stacks']), len(set(metadata['stacks'])))
            self.assertEqual(set(metadata['wild_copies']),
                             {'owned-off', 'all3on', 'all4on', 'all4hotpatch',
                              'owned-macro', 'owned-macro-hotpatch'})

    def test_hotpatch_journal_invalid_and_full_link_evidence(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / 'hotpatch.jsonl').write_text('{invalid')
            with self.assertRaises(ValueError):
                h.retained_hotpatch(root, root / 'bad')
            self.assertTrue((root / 'bad.hotpatch.jsonl').is_file())
            (root / 'hotpatch.jsonl').write_text(json.dumps({'outcome': 'full-link'}) + '\n')
            self.assertEqual(h.retained_hotpatch(root, root / 'fallback')['outcomes'], ['full-link'])

    def test_seed_copy_preserves_source_and_rejects_unsafe_paths(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            source = root / 'seed'
            source.mkdir()
            (source / 'nested').mkdir()
            (source / 'nested/a.pack').write_bytes(b'pristine')
            destination = root / 'new'
            manifest = h.seed_cache(source, destination)
            self.assertEqual(manifest['files'], h.seed_manifest(source))
            (destination / 'nested/a.pack').write_bytes(b'edited copy')
            self.assertEqual((source / 'nested/a.pack').read_bytes(), b'pristine')
            for unsafe in (source, source / 'child', destination):
                with self.assertRaises(ValueError):
                    h.seed_cache(source, unsafe)
            for name, target in (('file-link', source / 'nested/a.pack'), ('dir-link', source / 'nested')):
                link = source / name
                link.symlink_to(target)
                with self.assertRaises(ValueError):
                    h.seed_cache(source, root / ('rejected-' + name))
                link.unlink()
            alias = root / 'alias'
            alias.symlink_to(source)
            with self.assertRaises(ValueError):
                h.seed_cache(alias, root / 'rejected-root-link')
            os.mkfifo(source / 'fifo')
            with self.assertRaises(ValueError):
                h.seed_cache(source, root / 'rejected-fifo')

    def test_seed_mutation_during_copy_rejected(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            source = root / 'seed'
            source.mkdir()
            (source / 'a.pack').write_bytes(b'before')
            original_copy = h.shutil.copytree
            def mutate(*args, **kwargs):
                result = original_copy(*args, **kwargs)
                (source / 'a.pack').write_bytes(b'concurrent mutation')
                return result
            with patch.object(h.shutil, 'copytree', side_effect=mutate):
                with self.assertRaises(ValueError):
                    h.seed_cache(source, root / 'new')

    def test_missing_or_malformed_raw_logs_excluded(self):
        for mode in ('missing', 'malformed'):
            with self.subTest(mode=mode), tempfile.TemporaryDirectory() as temp:
                self.exercise(Path(temp), raw_mode=mode)
                summary = json.loads((Path(temp) / 'out/summary.json').read_text())
                self.assertNotIn('owned-off', summary['medians_seconds'])
                self.assertNotIn('all3on', summary['medians_seconds'])
                records = json.loads((Path(temp) / 'out/records.json').read_text())
                self.assertTrue(all(r.get('raw_output_error') for r in records
                                    if r['stack'] in ('owned-off', 'all3on')))

    def test_raw_logs_reject_stale_and_ambiguous_runs(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            env = {'JUSTRUST_HOME': str(root / 'private')}
            runs = root / 'private/runs'
            old = runs / 'old'
            old.mkdir(parents=True)
            (old / 'output.jsonl').write_text(json.dumps({'s': 'o', 'l': output()}) + '\n')
            before = h.recorded_runs(env)
            with self.assertRaises(ValueError):
                h.recorded_output(env, before, root / 'stale')
            for name in ('new-a', 'new-b'):
                (runs / name).mkdir()
            with self.assertRaises(ValueError):
                h.recorded_output(env, before, root / 'ambiguous')

    def test_raw_log_artifact_and_no_console_duplication(self):
        with tempfile.TemporaryDirectory() as temp:
            self.exercise(Path(temp))
            root = Path(temp) / 'out'
            records = json.loads((root / 'records.json').read_text())
            row = next(r for r in records if r['stack'] == 'owned-off' and r['phase'] == 'edit')
            self.assertEqual(row['elapsed_seconds'], .001)
            artifact = root / row['raw_output']['artifact']
            self.assertTrue(artifact.is_file())
            self.assertEqual(h.base.digest(artifact), row['raw_output']['sha256'])
            self.assertIn('JR_RUNTIME_EXECUTED_1', artifact.read_text())
            self.assertNotIn('JR_RUNTIME_EXECUTED', (root / 'edit-000-owned-off.stdout').read_text())
            self.assertTrue(row['accepted'])

    def test_restore_on_exception(self):
        with tempfile.TemporaryDirectory() as temp:
            self.exercise(Path(temp), crash=True)

    def test_stale_runtime_excluded(self):
        with tempfile.TemporaryDirectory() as temp:
            self.exercise(Path(temp), stale=True)


if __name__ == '__main__':
    unittest.main()
