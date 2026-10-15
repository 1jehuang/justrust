#!/usr/bin/env python3
"""Opt-in native toolchain attestation tests. No builds, network, or real Cargo."""
import argparse
import contextlib
import hashlib
import io
import json
import os
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

import suite

synth, versus = suite._synth()
SPEC = 'jr-123456789abc'


class ToolchainIdentityTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.home = self.root / 'private-home'
        self.prefix = self.home / 'toolchains' / 'staged' / SPEC
        self.prefix.mkdir(parents=True)
        self.manifest = self.root / 'private-identity.json'
        self.value = {'schema': 1, 'toolchain': SPEC, 'artifacts': {}}
        for role in suite.ToolchainIdentity.ROLES:
            relative = 'lib/librustc_driver-test.so' if role == 'driver' else 'bin/' + role
            path = self.prefix / relative
            path.parent.mkdir(exist_ok=True)
            path.write_bytes(('fixture-' + role).encode())
            path.chmod(0o755)
            self.value['artifacts'][role] = {'relative_path': relative,
                                          'sha256': hashlib.sha256(path.read_bytes()).hexdigest()}
        (self.prefix / 'justrust-toolchain.json').write_text(json.dumps({'id': SPEC}))
        self.save_manifest()
        self.home_patch = patch.object(suite, 'JR_HOME', self.home)
        self.home_patch.start()
        self.addCleanup(self.home_patch.stop)
        self.env_patch = patch.dict(os.environ, {}, clear=True)
        self.env_patch.start()
        self.addCleanup(self.env_patch.stop)

    def save_manifest(self):
        self.manifest.write_text(json.dumps(self.value))

    def identity(self):
        return suite.ToolchainIdentity(self.manifest, SPEC)

    def record(self, run_id='run-1', **changes):
        value = {'id': run_id, 'toolchain': SPEC, 'place': 'local',
                 'real_rustc': str(self.prefix / 'bin/rustc'),
                 'real_cargo': str(self.prefix / 'bin/cargo')}
        value.update(changes)
        path = self.home / 'runs' / run_id / 'meta.json'
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(value))
        return path

    def test_prefix_artifacts_and_recorded_selection_are_verified_without_private_paths(self):
        identity = self.identity()
        self.record()
        before = identity.verify_files()
        self.assertEqual(identity.verify_run('run-1', 'pre'), {'toolchain_identity_verified': True})
        self.assertEqual(identity.verify_files(), before)
        self.assertEqual(identity.invocations[0]['stage'], 'pre')
        self.assertEqual(len(identity.invocations[0]['meta_sha256']), 64)
        self.assertNotIn(str(self.root), json.dumps(before))
        self.assertNotIn('relative_path', json.dumps(before))
        self.assertIn('not-loaded-memory', before['scope'])

    def test_same_upstream_version_does_not_authorize_a_changed_compiler(self):
        identity = self.identity()
        (self.prefix / 'bin/rustc').write_bytes(b'same upstream -V, different compiler')
        with self.assertRaisesRegex(ValueError, 'artifact changed'):
            identity.verify_files()

    def test_driver_helper_or_marker_replacement_rejects(self):
        for role in ('driver', 'rustdoc', 'clippy-driver', 'cargo-clippy', 'cargo'):
            identity = self.identity()
            path = identity.paths[role]
            original = path.read_bytes()
            path.write_bytes(b'replacement')
            with self.subTest(role=role), self.assertRaises(ValueError):
                identity.verify_files()
            path.write_bytes(original)
        identity = self.identity()
        (self.prefix / 'justrust-toolchain.json').write_text('{}')
        with self.assertRaisesRegex(ValueError, 'marker changed'):
            identity.verify_files()

    def test_missing_or_substituted_invocation_never_passes(self):
        identity = self.identity()
        with self.assertRaises(ValueError):
            identity.verify_run('run-1', 'run')
        for changes in ({'id': 'other'}, {'toolchain': 'system'}, {'place': 'hosted'},
                        {'place': None}, {'real_rustc': '/usr/bin/rustc'},
                        {'real_cargo': '/usr/bin/cargo'}, {'real_rustc': 'rustc'},
                        {'real_cargo': None}):
            self.record(**changes)
            with self.subTest(changes=changes), self.assertRaises(ValueError):
                identity.verify_run('run-1', 'run')

    def test_bounded_strict_manifest_and_nonregular_inputs(self):
        original = json.loads(json.dumps(self.value))
        for change in ({'schema': True}, {'schema': 2}, {'extra': 1}, {'toolchain': 'system'},
                       {'artifacts': {}}, {'artifacts': None}):
            self.value = {**original, **change}
            self.save_manifest()
            with self.subTest(change=change), self.assertRaises(ValueError):
                self.identity()
        self.manifest.write_bytes(b' ' * 65537)
        with self.assertRaisesRegex(ValueError, 'byte bound'):
            self.identity()
        self.manifest.unlink()
        os.mkfifo(self.manifest)
        with self.assertRaisesRegex(ValueError, 'regular file'):
            self.identity()

    def test_traversal_escaped_symlink_and_wrong_roles_reject(self):
        for relative in ('../outside', '/usr/bin/rustc', 'bin/../rustc', 'bin//rustc',
                         'bin/./rustc', 'bin/cargo', 'bin\\rustc'):
            self.value['artifacts']['rustc']['relative_path'] = relative
            self.save_manifest()
            with self.subTest(relative=relative), self.assertRaises(ValueError):
                self.identity()
        self.value['artifacts']['rustc']['relative_path'] = 'bin/rustc'
        self.save_manifest()
        compiler = self.prefix / 'bin/rustc'
        outside = self.root / 'outside-rustc'
        outside.write_bytes(compiler.read_bytes())
        compiler.unlink()
        compiler.symlink_to(outside)
        with self.assertRaisesRegex(ValueError, 'escapes'):
            self.identity()

    def test_loader_override_and_machine_home_and_system_demand_reject(self):
        for name in ('LD_LIBRARY_PATH', 'LD_PRELOAD'):
            with patch.dict(os.environ, {name: '/private/override'}), self.assertRaises(ValueError):
                self.identity()
        with patch.object(suite, 'JR_HOME', Path.home() / '.justrust'), self.assertRaises(ValueError):
            self.identity()
        with self.assertRaises(ValueError):
            suite.ToolchainIdentity(self.manifest, 'system')

    def test_same_spec_two_private_homes_isolates_and_attests_different_hashes(self):
        first = self.identity().verify_files()
        other_home = self.root / 'other-home'
        import shutil
        shutil.copytree(self.home, other_home)
        compiler = other_home / 'toolchains/staged' / SPEC / 'bin/rustc'
        compiler.write_bytes(b'candidate same upstream version')
        other_value = json.loads(json.dumps(self.value))
        other_value['artifacts']['rustc']['sha256'] = suite.binary_sha256(compiler)
        other_manifest = self.root / 'other.json'
        other_manifest.write_text(json.dumps(other_value))
        with patch.object(suite, 'JR_HOME', other_home):
            second = suite.ToolchainIdentity(other_manifest, SPEC).verify_files()
        self.assertNotEqual(first['artifacts']['rustc'], second['artifacts']['rustc'])
        self.assertEqual(first['toolchain'], second['toolchain'])

    def test_actual_synthetic_run_audits_pre_prev_and_measured_records(self):
        identity = self.identity()
        plan = SimpleNamespace(snap='fixture', cwd='.', point=SimpleNamespace(i=1), expect_fail=False)
        stack = versus.Stack('controlled', 'fixture', True, {}, private_cache=True)
        calls = []

        def counted(argv, cwd, env, log):
            calls.append(list(argv))
            log.write_text('')
            self.record(env['JUSTRUST_RUN_ID_OVERRIDE'])
            return {'exit': 0, 'wall': 0.01}

        with patch.object(suite, 'new_id', side_effect=['pre-id', 'prev-id', 'run-id']), \
                patch.object(suite, 'foreign_load', return_value=(0.0, 0)), \
                patch.object(synth.gen, 'snapshot_root', return_value=self.root), \
                patch.object(versus, 'WORK', self.root / 'work'), \
                patch.object(versus, 'run_counted', side_effect=counted), \
                patch.object(versus, '_mark_remote'):
            for stage in ('pre', 'prev', 'run'):
                result = synth._run(stack, plan, ['check'], stage, identity=identity)
                self.assertTrue(result['toolchain_identity_verified'])
        self.assertEqual([r['stage'] for r in identity.invocations], ['pre', 'prev', 'run'])
        self.assertEqual(len(calls), 3)

    def test_run_point_propagates_demand_and_restores_source_on_audit_failure(self):
        identity = self.identity()
        plan = SimpleNamespace(snap='fixture', pre=['check'], prev=['build'], cmd=['test'])
        seen = []

        def execute(stack, plan, cmd, tag, **kwargs):
            self.assertIs(kwargs['identity'], identity)
            seen.append(tag)
            if tag == 'run':
                raise ValueError('wrong compiler')
            return {'exit': 0, 'wall': 0.01, 'log': str(self.root / tag)}

        with patch.object(synth, '_run', side_effect=execute), \
                patch.object(synth.gen, 'restore_all'), patch.object(synth.gen, 'apply', return_value=[]), \
                patch.object(synth.gen, 'restore') as restore, patch.object(synth.os, 'sync'), \
                self.assertRaisesRegex(ValueError, 'wrong compiler'):
            synth.run_point(SimpleNamespace(), plan, 1, identity=identity)
        self.assertEqual(seen, ['pre', 'prev', 'run'])
        restore.assert_called_once_with(plan, [])

    def test_resume_and_unsupported_cli_modes_reject_before_any_install(self):
        with self.assertRaisesRegex(ValueError, 'resumed'):
            synth.run([], 1, 40, 1, 0, resume=True, identity=self.identity())
        for remote, seed, resume in ((True, 1, False), (False, None, False), (False, 1, True)):
            args = argparse.Namespace(toolchain=SPEC, remote=remote, synth=seed, resume=resume,
                                      toolchain_identity=self.manifest)
            with patch.object(suite.subprocess, 'run') as run, self.assertRaises(SystemExit):
                suite.cmd_run(args)
            run.assert_not_called()

    def test_duplicate_manifest_keys_and_duplicate_invocation_ids_reject(self):
        self.manifest.write_text('{"schema":1,"schema":1}')
        with self.assertRaisesRegex(ValueError, 'duplicate'):
            self.identity()
        self.save_manifest()
        identity = self.identity()
        self.record()
        identity.verify_run('run-1', 'pre')
        with self.assertRaisesRegex(ValueError, 'run ID'):
            identity.verify_run('run-1', 'run')

    def test_noop_selection_is_not_a_compiler_execution_claim(self):
        identity = self.identity()
        self.record()
        self.assertEqual(identity.verify_run('run-1', 'run'), {'toolchain_identity_verified': True})
        public = json.dumps(identity.verify_files())
        self.assertIn('recorded-selection-not-loaded-memory', public)
        self.assertNotIn('compiler_executed', public)

    def test_changed_post_identity_never_writes_completed_result(self):
        args = argparse.Namespace(synth=1, points=40, n=1, quiet_wait=0, resume=False,
                                  out=str(self.root / 'must-not-exist.json'))
        identity = self.identity()

        def mutate(*args, **kwargs):
            identity.paths['driver'].write_bytes(b'substituted driver')
            return {'results': []}

        fake_engine = SimpleNamespace(run=mutate, table=lambda _: '')
        with patch.object(suite, '_synth', return_value=(fake_engine, versus)), \
                patch.object(suite, 'env_info', return_value={}), \
                patch.object(suite, 'verify_binary_identity'), \
                patch.object(suite, 'synthetic_stack_controls', return_value={}), \
                patch.object(suite, 'TOOLCHAIN', SPEC), self.assertRaises(ValueError):
            suite.cmd_run_synth(args, identity)
        self.assertFalse(Path(args.out).exists())

    def test_compare_requires_matched_attestation_protocol_and_actual_sample_audits(self):
        identity = self.identity()
        self.record()
        identity.verify_run('run-1', 'run')
        public = identity.verify_files()
        result = {'toolchain_identity': {'before': public, 'after': public,
                                        'invocations': identity.invocations},
                  'synth': {'results': [{'samples': {'controlled': [
                      {'id': 'run-1', 'toolchain_identity_verified': True}]}}]}}
        self.assertEqual(suite.toolchain_identity_comparison_issues(result, result), [])
        self.assertTrue(suite.toolchain_identity_comparison_issues(result, {}))
        self.assertEqual(suite.toolchain_identity_comparison_issues({}, {}), [])
        for change in ({'invocations': []}, {'after': {}}, {'before': None}):
            altered = {**result, 'toolchain_identity': {**result['toolchain_identity'], **change}}
            self.assertTrue(suite.toolchain_identity_comparison_issues(result, altered))
        for samples in (None, [], {}, {'controlled': None}, {'controlled': [{}]}):
            altered = {**result, 'synth': {'results': [{'samples': samples}]}}
            self.assertTrue(suite.toolchain_identity_comparison_issues(result, altered))

    def test_single_stack_legacy_output_and_calls_are_unchanged_without_demand(self):
        args = argparse.Namespace(synth=1, points=40, n=1, quiet_wait=0, resume=False,
                                  out=str(self.root / 'legacy.json'))
        fake_engine = SimpleNamespace(run=Mock(return_value={'results': []}), table=lambda _: '')
        with patch.object(suite, '_synth', return_value=(fake_engine, versus)), \
                patch.object(suite, 'env_info', return_value={}), \
                patch.object(suite, 'verify_binary_identity'), \
                patch.object(suite, 'synthetic_stack_controls', return_value={}), \
                patch.object(suite, 'TOOLCHAIN', SPEC), contextlib.redirect_stdout(io.StringIO()):
            suite.cmd_run_synth(args)
        self.assertNotIn('identity', fake_engine.run.call_args.kwargs)
        output = json.loads(Path(args.out).read_text())
        self.assertNotIn('toolchain_identity', output)
        self.assertNotIn('JUSTRUST_TOOLCHAIN_REQUIRED', fake_engine.run.call_args.args[0][0].env)

    def test_single_stack_demand_records_pre_post_and_forces_no_fallback(self):
        args = argparse.Namespace(synth=1, points=40, n=1, quiet_wait=0, resume=False,
                                  out=str(self.root / 'attested.json'))
        identity = self.identity()
        fake_engine = SimpleNamespace(run=Mock(return_value={'results': []}), table=lambda _: '')
        with patch.object(suite, '_synth', return_value=(fake_engine, versus)), \
                patch.object(suite, 'env_info', return_value={}), \
                patch.object(suite, 'verify_binary_identity'), \
                patch.object(suite, 'synthetic_stack_controls', return_value={}), \
                patch.object(suite, 'TOOLCHAIN', SPEC), contextlib.redirect_stdout(io.StringIO()):
            suite.cmd_run_synth(args, identity)
        self.assertIs(fake_engine.run.call_args.kwargs['identity'], identity)
        self.assertEqual(fake_engine.run.call_args.args[0][0].env['JUSTRUST_TOOLCHAIN_REQUIRED'], '1')
        output = json.loads(Path(args.out).read_text())
        self.assertEqual(output['toolchain_identity']['before'], output['toolchain_identity']['after'])
        self.assertNotIn(str(self.prefix), json.dumps(output))


if __name__ == '__main__':
    unittest.main()
