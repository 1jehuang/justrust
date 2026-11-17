"""Strict, test-only isolation for pinned base configuration and scheduler globals."""
from pathlib import Path
import difflib
import os
import shutil
import sys
import hashlib
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import suite

PATCH = 'bench/patches/jcode-base-global-fixtures-v1.patch'
COMPACTION = 'crates/jcode-base/src/compaction_tests.rs'
SCHEDULER = 'crates/jcode-base/src/provider/catalog_scheduler.rs'
NAME = 'test_context_usage_with_both_estimate_and_observed'
SCHEDULER_NAMES = ('ensure_started_is_a_noop_without_a_runtime',
                   'ensure_started_is_idempotent_under_a_runtime')
CONFIG_GUARD = '''    let _lock = crate::storage::lock_test_env();
    struct RestoreHome(Option<std::ffi::OsString>);
    impl Drop for RestoreHome {
        fn drop(&mut self) {
            if let Some(home) = &self.0 {
                crate::env::set_var("JCODE_HOME", home);
            } else {
                crate::env::remove_var("JCODE_HOME");
            }
            crate::config::Config::invalidate_cache();
        }
    }
    let home = tempfile::tempdir().unwrap();
    let _restore = RestoreHome(std::env::var_os("JCODE_HOME"));
    crate::env::set_var("JCODE_HOME", home.path());
    crate::config::Config::invalidate_cache();
'''
SCHEDULER_HELPER = '''
    // Route builders in unrelated async tests also start this process-global
    // OnceLock. Each scheduler contract needs a fresh process, not a mutex
    // covering only these two callers. The child executes the original body.
    fn in_fresh_scheduler_process(name: &str) -> bool {
        const CHILD: &str = "JCODE_BENCH_SCHEDULER_FIXTURE_CHILD";
        if std::env::var(CHILD).as_deref() == Ok(name) {
            return false;
        }
        let full_name = format!("provider::catalog_scheduler::tests::{name}");
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", &full_name, "--nocapture"])
            .env(CHILD, name)
            .output()
            .expect("run isolated scheduler fixture");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{full_name}: {stdout}\\n{stderr}");
        assert!(stdout.contains("1 passed; 0 failed"), "fixture was not selected: {stdout}");
        true
    }
'''


def original_source(source):
    repo, revision = suite.SNAPSHOTS['desktop']['repos']['jcode']
    return subprocess.check_output(['git', '-C', str(repo), 'show', f'{revision}:{source}'], text=True)


def corrections(source, original):
    if source == COMPACTION:
        anchor = f'fn {NAME}() {{\n'
        assert original.count(anchor) == 1
        return original.replace(anchor, anchor + CONFIG_GUARD)
    corrected = original.replace('    use super::*;\n', '    use super::*;\n' + SCHEDULER_HELPER)
    for name in SCHEDULER_NAMES:
        anchor = f'fn {name}() {{\n'
        assert corrected.count(anchor) == 1
        corrected = corrected.replace(anchor, anchor + f'        if in_fresh_scheduler_process("{name}") {{\n            return;\n        }}\n')
    return corrected


def write_patch():
    text = ''
    for source in (COMPACTION, SCHEDULER):
        original = original_source(source)
        text += ''.join(difflib.unified_diff(original.splitlines(True), corrections(source, original).splitlines(True), fromfile='a/' + source, tofile='b/' + source))
    (suite.ROOT / PATCH).write_text(text)


class BaseGlobalFixtures(unittest.TestCase):
    def test_exact_additions_preserve_every_original_statement_and_assertion(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for source in (COMPACTION, SCHEDULER):
                original = original_source(source)
                target = root / source
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_text(original)
                suite.verify_fixture_input(target, suite.SNAPSHOTS['desktop']['fixture_input_sha256']['jcode/' + source])
            subprocess.run(['git', 'apply', '--check', str(suite.ROOT / PATCH)], cwd=root, check=True)
            subprocess.run(['git', 'apply', str(suite.ROOT / PATCH)], cwd=root, check=True)
            for source in (COMPACTION, SCHEDULER):
                self.assertEqual((root / source).read_text(), corrections(source, original_source(source)))
            # Only insertions in test code. No production scheduler changes.
            before, after = original_source(SCHEDULER).split('#[cfg(test)]', 1)[0], (root / SCHEDULER).read_text().split('#[cfg(test)]', 1)[0]
            self.assertEqual(before, after)
            self.assertNotIn('\n-', (suite.ROOT / PATCH).read_text().replace('\n---', ''))

    def test_fail_closed_hashes_and_snapshot_identity(self):
        spec = suite.SNAPSHOTS['desktop']
        self.assertIn(('jcode', PATCH), spec['patches'])
        self.assertEqual(hashlib.sha256((suite.ROOT / PATCH).read_bytes()).hexdigest(), spec['fixture_patch_sha256'][PATCH])
        before = suite.snapshot_fingerprint('desktop')
        with patch.dict(suite.SNAPSHOTS, {'desktop': dict(spec, patches=[p for p in spec['patches'] if p[1] != PATCH])}):
            self.assertNotEqual(before, suite.snapshot_fingerprint('desktop'))
        with tempfile.TemporaryDirectory() as directory:
            bad = Path(directory) / 'bad'
            bad.write_text('unexpected bytes')
            with self.assertRaisesRegex(ValueError, 'corrupt or unversioned fixture patch'):
                suite.verify_fixture_patch(bad, spec['fixture_patch_sha256'][PATCH])
            for source in (COMPACTION, SCHEDULER):
                with self.assertRaisesRegex(ValueError, 'unexpected pinned fixture input SHA256'):
                    suite.verify_fixture_input(bad, spec['fixture_input_sha256']['jcode/' + source])


COMPACTION_CONTROL = '''
#[test]
fn benchmark_fixture_compaction_prior_cap_control() {
    let home = tempfile::tempdir().unwrap();
    crate::env::set_var("JCODE_HOME", home.path());
    crate::config::Config::invalidate_cache();
    let mut cfg = crate::config::Config::default();
    cfg.compaction.max_context_tokens = 10_000;
    cfg.save().unwrap();
    test_context_usage_with_both_estimate_and_observed();
    assert_eq!(crate::config::config().compaction.max_context_tokens, 10_000);
}
'''
SCHEDULER_CONTROL = '''
    #[test]
    fn benchmark_fixture_scheduler_prior_start_control() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async { ensure_started(); });
        drop(runtime);
        assert!(SCHEDULER_STARTED.get().is_some());
        ensure_started_is_a_noop_without_a_runtime();
        ensure_started_is_idempotent_under_a_runtime();
    }
'''


def acceptance(directory, full=False):
    """Deterministic sufficient-contamination controls, never measured sources.

    Local execution is required to exercise this exact disposable self-exec
    binary. This is fixture acceptance, not a performance comparison. Neither
    live sources nor the published CLI are modified.
    """
    import runtime_context
    os.environ[runtime_context.SELECTOR] = runtime_context.POLICY
    directory = directory.resolve()
    directory.mkdir(parents=True, exist_ok=True)
    suite.WORK = directory / 'work'
    snapshot = suite.prepare('desktop')
    diagnostics = directory / 'diagnostics'
    if diagnostics.exists():
        raise ValueError(f'diagnostic destination already exists: {diagnostics}')
    shutil.copytree(snapshot, diagnostics, ignore=shutil.ignore_patterns('target', '.git'))
    repo = diagnostics / 'jcode'
    env = os.environ.copy()
    env.setdefault('JUSTRUST_TOOLCHAIN', 'system')
    # Both variants share compiler flags and target dir. Only the versioned
    # fixture correction differs, plus identical additive diagnostic callers.
    env['CARGO_TARGET_DIR'] = str(directory / 'target')
    controls = ('benchmark_fixture_compaction_prior_cap_control',
                'benchmark_fixture_scheduler_prior_start_control')
    outcomes = {}
    for variant in ('before', 'after'):
        for source, control in ((COMPACTION, COMPACTION_CONTROL), (SCHEDULER, SCHEDULER_CONTROL)):
            text = original_source(source)
            if variant == 'after':
                text = corrections(source, text)
            if source == COMPACTION:
                text += control
            else:
                index = text.rfind('}')
                text = text[:index] + control + text[index:]
            (repo / source).write_text(text)
        for control in controls:
            print(f'ACCEPTANCE {variant} {control}', flush=True)
            with runtime_context.runtime_context(env, repo) as child:
                result = subprocess.run(
                    ['justrust', 'test',
                     '--local=controlled fixture isolation diagnostic must self-exec the exact disposable binary',
                     '-p', 'jcode-base', '--lib', control],
                    cwd=repo, env=child, capture_output=True, text=True)
            # Replay everything, including the expected baseline errors. Check
            # the actual original assertion, not merely an arbitrary exit 101.
            print(result.stdout, end='', flush=True)
            print(result.stderr, end='', file=sys.stderr, flush=True)
            output = result.stdout + result.stderr
            outcomes[variant, control] = result.returncode
            expected = 101 if variant == 'before' else 0
            if result.returncode != expected:
                raise RuntimeError(f'{variant} {control}: expected {expected}, got {result.returncode}')
            if variant == 'before':
                assertion = ('char estimate should be low: 1.2535' if 'compaction' in control
                             else 'assertion failed: SCHEDULER_STARTED.get().is_none()')
                if assertion not in output or '0 passed; 1 failed' not in output:
                    raise RuntimeError(f'{control}: original failure was not reproduced')
            elif '1 passed, 0 failed' not in output:
                raise RuntimeError(f'{control}: corrected diagnostic was not selected')
    print('ACCEPTANCE all original assertions reproduced before and passed after', outcomes, flush=True)
    if full:
        # Remove additive diagnostic callers before representative acceptance.
        # Every workspace file now equals the fully prepared measured fixture.
        for source in (COMPACTION, SCHEDULER):
            (repo / source).write_bytes((snapshot / 'jcode' / source).read_bytes())
        for arguments in (['-p', 'jcode-base', '--lib'],
                          ['-p', 'jcode-base', '--lib'], ['--workspace']):
            print('FULL FIXTURE ACCEPTANCE', arguments, flush=True)
            with runtime_context.runtime_context(env, repo) as child:
                result = subprocess.run(
                    ['justrust', 'test',
                     '--local=controlled fixture acceptance must self-exec exact disposable binary',
                     *arguments], cwd=repo, env=child)
            if result.returncode:
                raise RuntimeError(f'full fixture acceptance failed: {arguments}')


if __name__ == '__main__':
    if len(sys.argv) in (3, 4) and sys.argv[1] == '--accept':
        if len(sys.argv) == 4 and sys.argv[3] != '--full':
            raise ValueError('the only acceptance option is --full')
        acceptance(Path(sys.argv[2]), full=len(sys.argv) == 4)
    else:
        unittest.main()
