#!/usr/bin/env python3
"""Scheduler harness checks without any compiler, network or hosted calls."""
import contextlib
import copy
import importlib.util
import io
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import suite

SPEC = importlib.util.spec_from_file_location('scheduler_sweep', Path(__file__).with_name('scheduler-sweep.py'))
sweep = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(sweep)


def sample(wall=1, noisy=False):
    return {'wall': wall, 'recorded_wall': wall, 'noisy': noisy, 'first_error': None,
            'startup': .1, 'compile': wall-.1, 'test_run': 0, 'cpu': wall,
            'link_secs': 0, 'split': {'frontend': wall-.1}, 'units': 1}


def segment(mode, wall=1, noisy=False):
    return {'mode': mode, 'result': {'started': 'start', 'finished': 'end',
            'env': {'optimization_env': {'JUSTRUST_CARGO_CRITICAL_PATH': '1' if mode == 'on' else '0'}},
            'scenarios': {'desktop-body-check': {'cmd': ['check'], 'samples': [sample(wall, noisy)]}}}}


def installed_marker():
    return {'id': 'nightly-2026-10-08', 'manifest_date': '2026-10-08',
            'manifest_sha256': 'fixture', 'rustc_version': 'fixture rustc',
            'components': ['rustc', 'cargo'], 'installed_at': 0, 'install_secs': 0}


class SchedulerSweepTests(unittest.TestCase):
    def test_aggregate_keeps_all_samples_noise_and_identity(self):
        parts = [segment(mode, i+1, i == 2) for i, mode in enumerate(sweep.MODES)]
        original = copy.deepcopy(parts)
        result = sweep.aggregate(parts, 'on', {'cargo_sha256': 'hash'}, 24)
        self.assertEqual(parts, original)
        self.assertEqual([s['wall'] for s in result['scenarios']['desktop-body-check']['samples']], [2, 3])
        self.assertEqual(result['scenarios']['desktop-body-check']['stats']['quiet_n'], 1)
        self.assertEqual(result['runs_per_scenario'], 2)
        self.assertEqual(result['scheduler_experiment']['ordering'], ['off', 'on', 'on', 'off'])
        self.assertEqual(result['scheduler_experiment']['identities'], {'cargo_sha256': 'hash'})

    def test_aggregate_rejects_mixed_environment_and_command(self):
        for field, value in [('env', {'mixed': True}), ('cmd', ['test'])]:
            parts = [segment(mode) for mode in sweep.MODES]
            if field == 'env':
                parts[3]['result']['env'] = value
            else:
                parts[3]['result']['scenarios']['desktop-body-check']['cmd'] = value
            with self.subTest(field=field), self.assertRaisesRegex(ValueError, 'mixed'):
                sweep.aggregate(parts, 'off', {}, 24)

    def test_identity_requires_candidate_and_hashes_selected_binaries(self):
        with tempfile.TemporaryDirectory() as tmp:
            prefix = Path(tmp)
            binaries = prefix / 'bin'
            binaries.mkdir()
            rustc, cargo, justrust = (binaries / name for name in ('rustc', 'jr-cargo', 'justrust'))
            (prefix / 'justrust-toolchain.json').write_text(json.dumps(installed_marker()))
            rustc.write_bytes(b'compiler')
            rustc.chmod(0o755)
            official = binaries / 'cargo'
            official.write_bytes(b'official')
            official.chmod(0o755)
            justrust.write_bytes(b'client')
            with patch.object(suite, 'toolchain_rustc', return_value=rustc), patch.object(suite, 'J', str(justrust)), \
                 patch.object(suite, 'JR_HOME', prefix):
                with self.assertRaisesRegex(ValueError, 'candidate'):
                    sweep.identities('nightly-2026-10-08')
                cargo.write_bytes(b'candidate')
                with self.assertRaisesRegex(ValueError, 'candidate'):
                    sweep.identities('nightly-2026-10-08')
                cargo.chmod(0o755)
                with patch.object(suite, 'sh', return_value='version'):
                    identity = sweep.identities('nightly-2026-10-08')
                self.assertEqual(identity['cargo_sha256'], suite.binary_sha256(cargo))
                self.assertEqual(identity['rustc_sha256'], suite.binary_sha256(rustc))
                self.assertEqual(identity['justrust_sha256'], suite.binary_sha256(justrust))

    def test_identity_rejects_candidate_symlink_outside_private_prefix(self):
        with tempfile.TemporaryDirectory() as tmp:
            prefix = Path(tmp) / 'private'
            prefix.mkdir()
            cargo = Path(tmp) / 'production-cargo'
            cargo.write_bytes(b'production')
            cargo.chmod(0o755)
            (prefix / 'jr-cargo').symlink_to(cargo)
            with patch.object(suite, 'toolchain_rustc', return_value=prefix / 'rustc'), \
                 patch.object(suite, 'JR_HOME', prefix), \
                 self.assertRaisesRegex(ValueError, 'escapes'):
                sweep.identities('nightly-2026-10-08')

    def test_identity_rejects_incomplete_or_invalid_installed_markers(self):
        with tempfile.TemporaryDirectory() as tmp:
            prefix = Path(tmp)
            binaries = prefix / 'bin'
            binaries.mkdir()
            for name in ('rustc', 'cargo', 'jr-cargo'):
                path = binaries / name
                path.write_bytes(b'fixture')
                path.chmod(0o755)
            marker = prefix / 'justrust-toolchain.json'
            invalid = ('not json', json.dumps([]), json.dumps({'id': 'nightly-2026-10-08'}),
                       json.dumps({**installed_marker(), 'installed_at': True}),
                       json.dumps({**installed_marker(), 'install_secs': float('inf')}),
                       json.dumps({**installed_marker(), 'id': 'wrong'}))
            for text in invalid:
                marker.write_text(text)
                with self.subTest(text=text), patch.object(suite, 'JR_HOME', prefix), \
                     patch.object(suite, 'toolchain_rustc', return_value=binaries / 'rustc'), \
                     patch.object(suite, 'sh') as run, self.assertRaisesRegex(ValueError, 'marker'):
                    sweep.identities('nightly-2026-10-08')
                run.assert_not_called()

    def test_guards_reject_unsafe_prefix_disabled_fork_and_unrecorded_hints(self):
        with tempfile.TemporaryDirectory() as tmp:
            for env in ({}, {'JUSTRUST_HOME': str(Path.home() / '.justrust')},
                        {'JUSTRUST_HOME': tmp, 'JUSTRUST_CARGO_FORK': '0'},
                        {'JUSTRUST_HOME': tmp, 'JUSTRUST_CARGO_CRITICAL_PATH_COSTS': 'private.json'}):
                argv = ['scheduler-sweep.py', '--toolchain', 'nightly-2026-10-08',
                        '--out-dir', str(Path(tmp) / 'out'), 'desktop-body-check']
                with self.subTest(env=env), patch.dict(os.environ, env, clear=True), \
                     patch('sys.argv', argv), patch.object(sweep.subprocess, 'run') as run, \
                     contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
                    sweep.main()
                run.assert_not_called()

    def test_main_runs_real_suite_entrypoint_abba_then_compare(self):
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / 'out'
            argv = ['scheduler-sweep.py', '--toolchain', 'nightly-2026-10-08',
                    '--out-dir', str(out), '--samples', '2', '--jobs', '8', 'desktop-body-check']
            seen = []
            def execute(cmd, **kwargs):
                self.assertTrue(kwargs['check'])
                if cmd[2] == 'run':
                    mode = 'on' if os.environ['JUSTRUST_CARGO_CRITICAL_PATH'] == '1' else 'off'
                    seen.append(mode)
                    self.assertEqual(os.environ['CARGO_BUILD_JOBS'], '8')
                    self.assertEqual(os.environ['JUSTRUST_CARGO_FORK'], '1')
                    self.assertEqual(os.environ['JUSTRUST_CARGO_DAEMON'], '0')
                    Path(cmd[cmd.index('--out')+1]).write_text(json.dumps(segment(mode)['result']))
                else:
                    self.assertEqual(cmd[2], 'compare')
                    self.assertTrue((out / 'off.json').is_file())
                    self.assertTrue((out / 'on.json').is_file())
            with patch.dict(os.environ, {'JUSTRUST_HOME': tmp}, clear=True), patch('sys.argv', argv), \
                 patch.object(sweep, 'identities', return_value={'cargo_sha256': 'fixed'}), \
                 patch.object(sweep, 'verify_recorded_selection') as verify, \
                 patch.object(sweep.subprocess, 'run', side_effect=execute) as run:
                sweep.main()
            self.assertEqual(seen, ['off', 'on', 'on', 'off'])
            self.assertEqual(run.call_count, 5)
            self.assertEqual(verify.call_count, 4)
            self.assertTrue((out / 'partial.json').is_file())

    def test_recorded_selection_rejects_fallback_missing_and_escaped_records(self):
        with tempfile.TemporaryDirectory() as tmp:
            home = Path(tmp)
            binaries = home / 'toolchains/nightly-2026-10-08/bin'
            binaries.mkdir(parents=True)
            for name in ('rustc', 'jr-cargo', 'cargo'):
                (binaries / name).write_bytes(name.encode())
            run = home / 'runs/owned-run'
            run.mkdir(parents=True)
            meta = {'id': 'owned-run', 'toolchain': 'nightly-2026-10-08', 'place': 'local',
                    'env': {'CARGO_BUILD_JOBS': '24'},
                    'real_cargo': str(binaries / 'jr-cargo'), 'real_rustc': str(binaries / 'rustc')}
            identity = {'cargo_sha256': suite.binary_sha256(binaries / 'jr-cargo'),
                        'rustc_sha256': suite.binary_sha256(binaries / 'rustc')}
            result = segment('off')['result']
            result['scenarios']['desktop-body-check']['samples'][0]['id'] = 'owned-run'
            with patch.object(suite, 'JR_HOME', home), \
                 patch.object(suite, 'toolchain_rustc', return_value=binaries / 'rustc'):
                (run / 'meta.json').write_text(json.dumps(meta))
                sweep.verify_recorded_selection(result, 'nightly-2026-10-08', identity, 24)
                for key, value in [('real_cargo', str(binaries / 'cargo')),
                                   ('real_rustc', '/different/compiler'), ('toolchain', 'system'),
                                   ('place', 'hosted'), ('id', 'different'),
                                   ('env', {'CARGO_BUILD_JOBS': '8'}), ('env', None),
                                   ('real_cargo', 'bin/jr-cargo'), ('real_rustc', ''),
                                   ('real_cargo', None)]:
                    (run / 'meta.json').write_text(json.dumps({**meta, key: value}))
                    with self.subTest(key=key), self.assertRaisesRegex(ValueError, 'selection'):
                        sweep.verify_recorded_selection(result, 'nightly-2026-10-08', identity, 24)
                for content in ('not json', '{}', '[]'):
                    (run / 'meta.json').write_text(content)
                    with self.subTest(content=content), self.assertRaisesRegex(ValueError, 'selection'):
                        sweep.verify_recorded_selection(result, 'nightly-2026-10-08', identity, 24)
                (run / 'meta.json').unlink()
                with self.assertRaisesRegex(ValueError, 'selection'):
                    sweep.verify_recorded_selection(result, 'nightly-2026-10-08', identity, 24)
                for rid in ('../escape', '/absolute', '', None, '.', '..'):
                    result['scenarios']['desktop-body-check']['samples'][0]['id'] = rid
                    with self.subTest(rid=rid), self.assertRaisesRegex(ValueError, 'selection'):
                        sweep.verify_recorded_selection(result, 'nightly-2026-10-08', identity, 24)
                result['scenarios']['desktop-body-check']['samples'][0]['id'] = 'owned-run'
                (run / 'meta.json').write_text(json.dumps(meta))
                (binaries / 'jr-cargo').write_bytes(b'replaced')
                with self.assertRaisesRegex(ValueError, 'selection binary identity'):
                    sweep.verify_recorded_selection(result, 'nightly-2026-10-08', identity, 24)

    def test_recorded_selection_failure_never_writes_completed_comparison(self):
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / 'out'
            argv = ['scheduler-sweep.py', '--toolchain', 'nightly-2026-10-08',
                    '--out-dir', str(out), 'desktop-body-check']
            def execute(cmd, **kwargs):
                Path(cmd[cmd.index('--out')+1]).write_text(json.dumps(segment('off')['result']))
            with patch.dict(os.environ, {'JUSTRUST_HOME': tmp}, clear=True), patch('sys.argv', argv), \
                 patch.object(sweep, 'identities', return_value={'cargo_sha256': 'fixed'}), \
                 patch.object(sweep, 'verify_recorded_selection', side_effect=ValueError('selection')) as verify, \
                 patch.object(sweep.subprocess, 'run', side_effect=execute) as run, \
                 self.assertRaisesRegex(ValueError, 'selection'):
                sweep.main()
            verify.assert_called_once()
            run.assert_called_once()
            self.assertFalse((out / 'off.json').exists())
            self.assertFalse((out / 'on.json').exists())
            self.assertTrue((out / '0-off.json').exists())

    def test_wrong_segment_mode_never_writes_completed_comparison(self):
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / 'out'
            argv = ['scheduler-sweep.py', '--toolchain', 'nightly-2026-10-08',
                    '--out-dir', str(out), 'desktop-body-check']
            def execute(cmd, **kwargs):
                if cmd[2] == 'run':
                    Path(cmd[cmd.index('--out')+1]).write_text(json.dumps(segment('on')['result']))
            with patch.dict(os.environ, {'JUSTRUST_HOME': tmp}, clear=True), patch('sys.argv', argv), \
                 patch.object(sweep, 'identities', return_value={'cargo_sha256': 'fixed'}), \
                 patch.object(sweep, 'verify_recorded_selection'), \
                 patch.object(sweep.subprocess, 'run', side_effect=execute), \
                 self.assertRaisesRegex(ValueError, 'mode'):
                sweep.main()
            self.assertFalse((out / 'off.json').exists())
            self.assertFalse((out / 'on.json').exists())
            self.assertTrue((out / '0-off.json').exists())

    def test_compiler_symlink_does_not_change_selected_cargo_sibling(self):
        with tempfile.TemporaryDirectory() as tmp:
            home = Path(tmp)
            binaries = home / 'toolchains/nightly-2026-10-08/bin'
            binaries.mkdir(parents=True)
            driver = binaries.parent / 'driver'
            driver.mkdir()
            (driver / 'rustc').write_bytes(b'compiler')
            (binaries / 'rustc').symlink_to(driver / 'rustc')
            (binaries / 'jr-cargo').write_bytes(b'candidate')
            run = home / 'runs/owned-run'
            run.mkdir(parents=True)
            (run / 'meta.json').write_text(json.dumps({
                'id': 'owned-run', 'toolchain': 'nightly-2026-10-08', 'place': 'local',
                'env': {'CARGO_BUILD_JOBS': '24'},
                'real_cargo': str(binaries / 'jr-cargo'), 'real_rustc': str(binaries / 'rustc')}))
            result = segment('off')['result']
            result['scenarios']['desktop-body-check']['samples'][0]['id'] = 'owned-run'
            identity = {'cargo_sha256': suite.binary_sha256(binaries / 'jr-cargo'),
                        'rustc_sha256': suite.binary_sha256(binaries / 'rustc')}
            with patch.object(suite, 'JR_HOME', home), \
                 patch.object(suite, 'toolchain_rustc', return_value=binaries / 'rustc'):
                sweep.verify_recorded_selection(result, 'nightly-2026-10-08', identity, 24)

    def test_replaced_toolchain_never_produces_completed_comparison(self):
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / 'out'
            argv = ['scheduler-sweep.py', '--toolchain', 'nightly-2026-10-08',
                    '--out-dir', str(out), 'desktop-body-check']
            def execute(cmd, **kwargs):
                Path(cmd[cmd.index('--out')+1]).write_text(json.dumps(segment('off')['result']))
            with patch.dict(os.environ, {'JUSTRUST_HOME': tmp}, clear=True), patch('sys.argv', argv), \
                 patch.object(sweep, 'identities', side_effect=[{'sha': 'old'}, {'sha': 'new'}]), \
                 patch.object(sweep.subprocess, 'run', side_effect=execute), \
                 self.assertRaisesRegex(ValueError, 'changed'):
                sweep.main()
            self.assertFalse((out / 'off.json').exists())
            self.assertFalse((out / 'on.json').exists())


class WeightedSchedulerTests(unittest.TestCase):
    def test_bounded_regular_cost_file_and_digest_only_provenance(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'private-project-costs.json'
            path.write_text('{"package-name":42}')
            raw, info = suite.load_scheduler_hints(path)
            self.assertEqual(raw, path.read_bytes())
            self.assertEqual(info, {'sha256': suite.binary_sha256(path), 'packages': 1})
            with patch.dict(os.environ, {'JUSTRUST_CARGO_CRITICAL_PATH': '1',
                                         'JUSTRUST_CARGO_CRITICAL_PATH_COSTS': str(path)}, clear=True):
                provenance = suite.optimization_env()
            self.assertEqual(provenance['JUSTRUST_CARGO_CRITICAL_PATH_COSTS_SHA256'], info['sha256'])
            self.assertNotIn(str(path), json.dumps(provenance))

    def test_invalid_tables_and_nonregular_files_are_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'costs.json'
            for raw in (b'[]', b'null', b'{"p":true}', b'{"p":0}', b'{"p":-1}',
                        b'{"p":1.5}', b'{"p":18446744073709551616}', b' ' * 65537,
                        json.dumps({str(i): 1 for i in range(1025)}).encode()):
                path.write_bytes(raw)
                with self.subTest(raw=raw[:50]), self.assertRaises(ValueError):
                    suite.load_scheduler_hints(path)
            path.write_text('{"p":1}')
            link = Path(tmp) / 'link'
            link.symlink_to(path)
            for bad in (link, Path(tmp), Path(tmp) / 'absent'):
                with self.subTest(path=bad), self.assertRaises(ValueError):
                    suite.load_scheduler_hints(bad)

    def run_weighted(self, tmp, mutation=False, wrong_digest=False):
        out = Path(tmp) / 'out'
        source = Path(tmp) / 'source.json'
        source.write_text('{"p":42}')
        digest = suite.binary_sha256(source)
        argv = ['scheduler-sweep.py', '--toolchain', 'nightly-2026-10-08',
                '--out-dir', str(out), '--costs', str(source), 'desktop-body-check']
        seen = []
        def execute(cmd, **kwargs):
            if cmd[2] == 'run':
                mode = 'on' if os.environ['JUSTRUST_CARGO_CRITICAL_PATH'] == '1' else 'off'
                hint = os.environ.get('JUSTRUST_CARGO_CRITICAL_PATH_COSTS')
                seen.append((mode, hint))
                result = segment(mode)['result']
                if mode == 'on':
                    self.assertEqual(Path(hint), out.resolve() / 'costs.json')
                    self.assertEqual(Path(hint).read_bytes(), source.read_bytes())
                    self.assertEqual(Path(hint).stat().st_mode & 0o777, 0o400)
                    result['env']['optimization_env']['JUSTRUST_CARGO_CRITICAL_PATH_COSTS_SHA256'] = (
                        'wrong' if wrong_digest else digest)
                    if mutation:
                        Path(hint).chmod(0o600)
                        Path(hint).write_text('{"p":99}')
                else:
                    self.assertIsNone(hint)
                Path(cmd[cmd.index('--out')+1]).write_text(json.dumps(result))
        with patch.dict(os.environ, {'JUSTRUST_HOME': tmp}, clear=True), patch('sys.argv', argv), \
             patch.object(sweep, 'identities', return_value={'cargo_sha256': 'fixed'}), \
             patch.object(sweep, 'verify_recorded_selection'), \
             patch.object(sweep.subprocess, 'run', side_effect=execute):
            sweep.main()
        return out, seen, digest

    def test_explicit_weighted_sweep_captures_table_and_all_segment_proof(self):
        with tempfile.TemporaryDirectory() as tmp:
            out, seen, digest = self.run_weighted(tmp)
            self.assertEqual([m for m, _ in seen], list(sweep.MODES))
            for mode in ('off', 'on'):
                result = json.loads((out / (mode+'.json')).read_text())
                proof = result['scheduler_experiment']['identities']['cost_hints']
                self.assertEqual(proof, {'sha256': digest, 'packages': 1})

    def test_replaced_cost_snapshot_never_writes_completed_comparison(self):
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaisesRegex(ValueError, 'cost'):
                self.run_weighted(tmp, mutation=True)
            self.assertFalse((Path(tmp) / 'out/off.json').exists())

    def test_missing_or_wrong_recorded_digest_never_writes_comparison(self):
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaisesRegex(ValueError, 'cost'):
                self.run_weighted(tmp, wrong_digest=True)
            self.assertFalse((Path(tmp) / 'out/off.json').exists())


if __name__ == '__main__':
    unittest.main()
