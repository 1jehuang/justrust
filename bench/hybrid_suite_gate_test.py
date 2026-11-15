#!/usr/bin/env python3
"""Unit/adapter subprocess tests only. These are not benchmark evidence."""
import argparse
import contextlib
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import hybrid_suite_gate as gate


class HybridSuiteGateTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.tool = self.root / 'tool'
        self.tool.write_text(f'#!{sys.executable}\nimport os,sys,json\nprint(json.dumps([sys.argv[1:], os.environ.get("JUSTRUST_NATIVE_ENGINE"), os.environ.get("JUSTRUST_HYBRID"), os.environ.get("JUSTRUST_HYBRID_TOOLCHAIN")]))\n')
        self.tool.chmod(0o700)
        self.toolchain = self.root / 'toolchain'
        (self.toolchain / 'bin').mkdir(parents=True)
        for name in ('rustc', 'cargo'):
            path = self.toolchain / 'bin' / name
            path.write_text(self.tool.read_text())
            path.chmod(0o700)

    def prepare(self):
        output = self.root / 'gate'
        args = argparse.Namespace(output=output, baseline=self.tool,
                                  candidate=self.tool, engine=self.tool, seed=1,
                                  native_toolchain=self.toolchain)
        with contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(gate.prepare(args), 0)
        return output

    def test_candidate_adapter_real_subprocess_preserves_cli_opts_in_env(self):
        output = self.prepare()
        for command in ('build', 'check', 'test', 'run'):
            child = subprocess.check_output([str(output / 'candidate-exec'), command, '--lib'], text=True)
            self.assertEqual(json.loads(child), [[command, '--lib'], str(self.tool), 'ready', str(self.toolchain)])

    def test_candidate_adapter_control_commands_are_not_hybrid(self):
        output = self.prepare()
        env = {k: v for k, v in os.environ.items() if k not in ('JUSTRUST_NATIVE_ENGINE', 'JUSTRUST_HYBRID_TOOLCHAIN')}
        env['JUSTRUST_HYBRID'] = 'ready'
        for command in ('-V', 'cache', 'toolchain', 'early', 'clippy'):
            child = subprocess.check_output([str(output / 'candidate-exec'), command], env=env, text=True)
            self.assertEqual(json.loads(child), [[command], None, None, None])

    def test_baseline_adapter_never_adds_hybrid(self):
        output = self.prepare()
        env = {k: v for k, v in os.environ.items() if k not in ('JUSTRUST_NATIVE_ENGINE', 'JUSTRUST_HYBRID_TOOLCHAIN')}
        env['JUSTRUST_HYBRID'] = 'ready'
        child = subprocess.check_output([str(output / 'baseline-exec'), 'test', '--lib'], env=env, text=True)
        self.assertEqual(json.loads(child), [['test', '--lib'], None, None, None])

    def test_prewarm_control_is_explicit_and_symmetric(self):
        self.tool.write_text(f'#!{sys.executable}\nimport os,json\nprint(json.dumps([os.environ.get("JUSTRUST_HYBRID_PREWARM"), os.environ.get("JUSTRUST_HYBRID")]))\n')
        for native in (False, True):
            adapter = self.root / ('prewarm-native' if native else 'prewarm-off')
            adapter.write_text(gate.adapter_text(self.tool, self.tool if native else None,
                                                self.toolchain if native else None,
                                                prewarm_after_success=True))
            adapter.chmod(0o700)
            env = dict(os.environ)
            env.pop('JUSTRUST_HYBRID_PREWARM', None)
            for command in ('build', 'check', 'test', 'run'):
                child = json.loads(subprocess.check_output([str(adapter), command], env=env, text=True))
                self.assertEqual(child, ['after-success', 'ready' if native else None])
            child = json.loads(subprocess.check_output([str(adapter), 'clippy'], env=env, text=True))
            self.assertEqual(child, [None, None])

    def test_fixture_xdg_mode_preserves_private_home_for_both_adapters(self):
        from runtime_context import runtime_context
        self.tool.write_text(f'#!{sys.executable}\nimport os,json\nprint(json.dumps({{k: os.environ.get(k) for k in ("HOME", "JCODE_HOME", "XDG_CONFIG_HOME", "JCODE_LATEX_RENDERING", "JCODE_LATEX_COMMAND", "JCODE_DVIPNG_COMMAND", "JCODE_PDFLATEX_COMMAND", "JCODE_PDFTOCAIRO_COMMAND")}}))\n')
        for native in (False, True):
            adapter = self.root / ('fixture-native' if native else 'fixture-off')
            adapter.write_text(gate.adapter_text(self.tool, self.tool if native else None,
                                                self.toolchain if native else None,
                                                fixture_xdg_home=True,
                                                fixture_latex_unicode=True,
                                                fixture_no_tex=True))
            adapter.chmod(0o700)
            selected = dict(os.environ, JUSTRUST_BENCH_RUNTIME_CONTEXT='jcode-isolated-v2')
            with patch.dict(os.environ, selected, clear=True):
                with runtime_context(selected, self.root) as env:
                    # Real stack_env intentionally does not forward the parent
                    # runtime selector. Test the actual child-facing contract.
                    env.pop('JUSTRUST_BENCH_RUNTIME_CONTEXT', None)
                    child = json.loads(subprocess.check_output([str(adapter), 'test'], env=env, text=True))
                    self.assertIsNone(child['JCODE_HOME'])
                    self.assertEqual(child['JCODE_LATEX_RENDERING'], 'unicode')
                    for command in ('LATEX', 'DVIPNG', 'PDFLATEX', 'PDFTOCAIRO'):
                        path = Path(child['JCODE_' + command + '_COMMAND'])
                        self.assertEqual(path, Path(env['HOME']) / '.benchmark-no-tex' / command.lower())
                        self.assertFalse(path.exists())
                    self.assertEqual(child['HOME'], env['HOME'])
                    self.assertEqual(child['XDG_CONFIG_HOME'], str(Path(env['HOME']) / '.config'))
                    self.assertNotEqual(child['HOME'], selected.get('HOME'))

    def test_fixture_git_identity_requires_private_home_policy(self):
        output = self.root / 'git-gate'
        args = argparse.Namespace(output=output, baseline=self.tool, candidate=self.tool,
                                  engine=self.tool, seed=1, native_toolchain=self.toolchain,
                                  fixture_git_identity=True, fixture_xdg_home=False)
        with self.assertRaisesRegex(ValueError, 'Git identity requires explicit fixture XDG'):
            gate.prepare(args)
        self.assertFalse(output.exists())

    def test_fixture_git_identity_real_commit_is_symmetric_and_private(self):
        from runtime_context import runtime_context
        self.tool.write_text(f'''#!{sys.executable}
import json,os,pathlib,subprocess
home=pathlib.Path(os.environ['HOME'])
assert not (home / '.gitconfig').exists()
repo=home / 'git-fixture'
repo.mkdir()
def git(*args):
    return subprocess.check_output(['git','-C',str(repo),*args],text=True,stderr=subprocess.STDOUT)
git('init','-q','--initial-branch=main')
git('config','commit.gpgsign','false')
git('commit','--allow-empty','-qm','Benchmark fixture')
print(json.dumps(dict(identity=git('log','-1','--format=%an|%ae|%cn|%ce').strip(),
                     home=str(home),global_config=(home / '.gitconfig').exists(),
                     env={{k:os.environ.get(k) for k in ('GIT_AUTHOR_NAME','GIT_AUTHOR_EMAIL','GIT_COMMITTER_NAME','GIT_COMMITTER_EMAIL')}})))
''')
        identities = []
        for native in (False, True):
            adapter = self.root / ('git-native' if native else 'git-off')
            adapter.write_text(gate.adapter_text(self.tool, self.tool if native else None,
                                                self.toolchain if native else None,
                                                fixture_xdg_home=True, fixture_git_identity=True))
            adapter.chmod(0o700)
            selected = dict(os.environ, JUSTRUST_BENCH_RUNTIME_CONTEXT='jcode-isolated-v2',
                            GIT_AUTHOR_NAME='ambient wrong identity', GIT_AUTHOR_EMAIL='ambient@example.invalid')
            with patch.dict(os.environ, selected, clear=True):
                with runtime_context(selected, self.root) as env:
                    env.pop('JUSTRUST_BENCH_RUNTIME_CONTEXT', None)
                    child = json.loads(subprocess.check_output([str(adapter), 'test'], env=env, text=True))
                    self.assertEqual(child['home'], env['HOME'])
                    self.assertFalse(child['global_config'])
                    identities.append(child['env'])
                    self.assertEqual(child['identity'], 'Hybrid benchmark fixture|hybrid-benchmark@example.invalid|Hybrid benchmark fixture|hybrid-benchmark@example.invalid')
        self.assertEqual(identities[0], identities[1])

    def test_fixture_git_identity_provenance_and_hashes(self):
        output = self.root / 'git-gate'
        args = argparse.Namespace(output=output, baseline=self.tool, candidate=self.tool,
                                  engine=self.tool, seed=1, native_toolchain=self.toolchain,
                                  fixture_git_identity=True, fixture_xdg_home=True)
        with contextlib.redirect_stdout(io.StringIO()):
            gate.prepare(args)
        report = gate.load_verified(output)
        self.assertTrue(report['fixture_git_identity'])
        for stack in ('baseline', 'candidate'):
            adapter = report['adapters'][stack]
            self.assertEqual(gate.sha256(adapter['path']), adapter['sha256'])
            self.assertIn("'hybrid-benchmark@example.invalid'", Path(adapter['path']).read_text())
        default = json.loads((self.prepare() / 'gate.json').read_text())
        self.assertFalse(default['fixture_git_identity'])
        self.assertNotEqual(default['adapters']['baseline']['sha256'], report['adapters']['baseline']['sha256'])

    def test_fixture_unicode_requires_private_home_policy(self):
        output = self.root / 'gate'
        args = argparse.Namespace(output=output, baseline=self.tool, candidate=self.tool,
                                  engine=self.tool, seed=1, native_toolchain=self.toolchain,
                                  fixture_latex_unicode=True, fixture_xdg_home=False)
        with self.assertRaisesRegex(ValueError, 'requires explicit fixture XDG'):
            gate.prepare(args)
        self.assertFalse(output.exists())

    def test_fixture_xdg_metadata_probe_needs_no_private_home(self):
        adapter = self.root / 'fixture-metadata'
        adapter.write_text(gate.adapter_text(self.tool, fixture_xdg_home=True))
        adapter.chmod(0o700)
        child = subprocess.run([str(adapter), '--version'], text=True, capture_output=True)
        self.assertEqual(child.returncode, 0, child.stderr)

    def test_fixture_xdg_mode_refuses_inherited_runtime(self):
        adapter = self.root / 'unsafe-fixture'
        adapter.write_text(gate.adapter_text(self.tool, fixture_xdg_home=True))
        adapter.chmod(0o700)
        env = dict(os.environ)
        env.pop('JUSTRUST_BENCH_RUNTIME_CONTEXT', None)
        child = subprocess.run([str(adapter), 'test'], env=env, text=True, capture_output=True)
        self.assertNotEqual(child.returncode, 0)
        self.assertIn('requires jcode-isolated-v2', child.stderr)
        self.assertFalse(child.stdout)

    def test_missing_native_toolchain_refuses_before_creating_evidence(self):
        output = self.root / 'gate'
        args = argparse.Namespace(output=output, baseline=self.tool, candidate=self.tool,
                                  engine=self.tool, seed=1, native_toolchain=None)
        with self.assertRaisesRegex(ValueError, 'refusing an all-fallback'):
            gate.prepare(args)
        self.assertFalse(output.exists())

    def test_incomplete_native_toolchain_refuses_before_creating_evidence(self):
        (self.toolchain / 'bin/cargo').unlink()
        output = self.root / 'gate'
        args = argparse.Namespace(output=output, baseline=self.tool, candidate=self.tool,
                                  engine=self.tool, seed=1, native_toolchain=self.toolchain)
        with self.assertRaisesRegex(argparse.ArgumentTypeError, 'expected executable'):
            gate.prepare(args)
        self.assertFalse(output.exists())

    def test_native_compiler_mutation_refuses_run(self):
        output = self.prepare()
        compiler = self.toolchain / 'bin/rustc'
        compiler.write_text(compiler.read_text() + '# changed compiler\n')
        with self.assertRaisesRegex(ValueError, 'input changed'):
            gate.load_verified(output)

    def test_missing_engine_refuses_before_creating_evidence(self):
        output = self.root / 'gate'
        args = argparse.Namespace(output=output, baseline=self.tool, candidate=self.tool,
                                  engine=self.root / 'missing-engine', seed=1,
                                  native_toolchain=self.toolchain)
        with self.assertRaisesRegex(argparse.ArgumentTypeError, 'expected executable'):
            gate.prepare(args)
        self.assertFalse(output.exists())

    def test_identity_change_refuses_run(self):
        output = self.prepare()
        self.tool.write_text(self.tool.read_text() + '# changed\n')
        with self.assertRaisesRegex(ValueError, 'input changed'):
            gate.load_verified(output)

    def test_adapter_change_refuses_run(self):
        output = self.prepare()
        adapter = output / 'candidate-exec'
        adapter.write_text(adapter.read_text() + '# changed\n')
        with self.assertRaisesRegex(ValueError, 'input changed'):
            gate.load_verified(output)

    def test_prepare_refuses_existing_evidence(self):
        output = self.prepare()
        with self.assertRaises(FileExistsError):
            gate.prepare(argparse.Namespace(output=output, baseline=self.tool,
                                           candidate=self.tool, engine=self.tool,
                                           native_toolchain=self.toolchain))

    def test_comprehensive_commands_identical_controls(self):
        output = self.prepare()
        report = gate.load_verified(output)
        before = report['commands']['baseline']
        after = report['commands']['candidate']
        self.assertEqual(before[:-1], after[:-1])
        self.assertIn('--synth', before)
        self.assertEqual(before[before.index('--synth') + 1], '1')
        self.assertEqual(before[before.index('--points') + 1], '40')
        self.assertEqual(before[before.index('-n') + 1], '1')
        self.assertIn('--require-valid', report['commands']['compare'])
        self.assertEqual(report['tools']['weights']['sha256'], gate.sha256(gate.HERE / 'synth/weights.json'))

    def test_incomplete_comparison_refused(self):
        output = self.prepare()
        with self.assertRaisesRegex(ValueError, 'both complete'):
            gate.compare(argparse.Namespace(output=output))

    def test_controls_bind_performance_environment_not_secrets(self):
        base = {'PATH': '/bin', 'JUSTRUST_LOCAL': '1', 'SECRET': 'a'}
        self.assertEqual(gate.control_identity(base), gate.control_identity(dict(base, SECRET='b')))
        self.assertNotEqual(gate.control_identity(base), gate.control_identity(dict(base, JUSTRUST_LOCAL='0')))
        self.assertNotEqual(gate.control_identity(base), gate.control_identity(dict(base, JCODE_HOME='/private/runtime')))

    def test_outcome_parity_accepts_equal_counts_and_exit(self):
        row = dict(exit=0, compile_failed=False, tests_failed=0, tests_passed=12)
        def report(name, sample):
            return {'synth': {'stack_names': [name], 'results': [{'i': 0, 'samples': {name: [sample]}}]}}
        baseline = report('before', row)
        self.assertEqual(gate.outcome_parity(baseline, report('after', dict(row))), [])
        candidate = report('after', dict(row, tests_passed=0))
        self.assertIn('tests_passed', gate.outcome_parity(baseline, candidate)[0])
        candidate = report('after', dict(row, exit=101))
        self.assertIn('exit', gate.outcome_parity(baseline, candidate)[0])
        missing = dict(row)
        del missing['tests_passed']
        self.assertIn('tests_passed', gate.outcome_parity(baseline, report('after', missing))[0])

    def test_outcome_parity_rejects_missing_replicates(self):
        baseline = {'synth': {'stack_names': ['suite'], 'results': [{'i': 0, 'samples': {'suite': []}}]}}
        self.assertIn('replicate', gate.outcome_parity(baseline, baseline)[0])
        self.assertIn('exactly one', gate.outcome_parity({}, {})[0])

    def test_real_process_log_retains_failure_output(self):
        log = self.root / 'failure.log'
        command = [sys.executable, '-c', 'import sys; print("actual child failure", flush=True); sys.exit(7)']
        with contextlib.redirect_stdout(io.StringIO()):
            result = gate.invoke(command, dict(os.environ), log)
        self.assertEqual(result['returncode'], 7)
        self.assertIn('actual child failure', log.read_text())
        self.assertGreater(result['wall_seconds'], 0)
        self.assertEqual(result['log_sha256'], gate.sha256(log))

    def test_failed_run_prevents_following_stack(self):
        output = self.prepare()
        args = argparse.Namespace(output=output, stack='both')
        # Stub execution tests orchestration only, never emitted as measured data.
        with patch.object(gate, 'invoke', return_value={'returncode': 9}) as invoke:
            self.assertEqual(gate.run(args), 9)
        self.assertEqual(invoke.call_count, 1)
        report = gate.load_verified(output)
        self.assertEqual(report['status'], 'blocked')
        self.assertNotIn('candidate', report['runs'])


if __name__ == '__main__':
    unittest.main()
