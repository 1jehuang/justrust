"""Two observed auth-reading TUI fixtures must own their credential home."""
from pathlib import Path
import hashlib
import os
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import suite
from test_login_event_fixture import copy_acceptance_snapshot

PATCH = 'bench/patches/jcode-auth-reader-fixtures-v1.patch'
PATCH_SHA = '03a6d99d68e201e5fca55bc11a363222c9f7bb55ebd20dfcb5828b7a978308fc'
SUBSCRIPTION = 'crates/jcode-tui/src/tui/app/tests/commands_accounts_01/part_02.rs'
CHANGES = 'crates/jcode-tui/src/tui/app/tests/scroll_copy_01/part_01.rs'
INPUTS = {
    SUBSCRIPTION: 'ed61b673c53131832e59fa74c35187dcdb71bd578582ac1acd6eeaf0744a19f5',
    CHANGES: 'ae43b7c6904caffc24f5089076d216758fb92bd49b4ef8021281f849d2f0c4f3',
}
NAMES = {
    SUBSCRIPTION: 'test_subscription_command_shows_jcode_status_scaffold',
    CHANGES: 'changes_widget_end_to_end_on_real_git_repo',
}
CREDENTIALS = {
    SUBSCRIPTION: '''
    crate::subscription_catalog::persist_account_credentials(
        "fixture-prior-account-key", Some("fixture-account"), None, None,
    ).unwrap();
    // This production helper also stores metadata in the process environment.
    // Preserve only the persisted prior credential as the contamination.
    crate::subscription_catalog::clear_runtime_env();
    crate::env::remove_var(crate::subscription_catalog::JCODE_API_KEY_ENV);
''',
    CHANGES: '''
    for (file, key) in [
        ("anthropic.env", "ANTHROPIC_API_KEY"),
        ("openrouter.env", "OPENROUTER_API_KEY"),
        ("openai.env", "OPENAI_API_KEY"),
        ("gemini.env", "GEMINI_API_KEY"),
    ] {
        crate::provider_catalog::save_named_api_key(file, key, "fixture-prior-provider-key").unwrap();
    }
    crate::auth::AuthStatus::invalidate_cache();
''',
}


def original(source):
    repo, revision = suite.SNAPSHOTS['desktop']['repos']['jcode']
    return subprocess.check_output(['git', '-C', str(repo), 'show', f'{revision}:{source}'], text=True)


def corrected(root):
    for source, digest in INPUTS.items():
        target = root / source
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(original(source))
        suite.verify_fixture_input(target, digest)
    suite.verify_fixture_patch(suite.ROOT / PATCH, PATCH_SHA)
    subprocess.run(['git', 'apply', '--check', str(suite.ROOT / PATCH)], cwd=root, check=True)
    subprocess.run(['git', 'apply', str(suite.ROOT / PATCH)], cwd=root, check=True)
    return {source: (root / source).read_text() for source in INPUTS}


class AuthReaderFixtures(unittest.TestCase):
    def test_only_two_private_home_wrappers_preserving_every_original_assertion(self):
        with tempfile.TemporaryDirectory() as directory:
            after = corrected(Path(directory))
        for source, name in NAMES.items():
            before = original(source)
            start = before.index('fn ' + name + '() {')
            end = before.index('\n#[test]', start)
            body = before[start:end]
            expected = body.replace('fn ' + name + '() {\n', 'fn ' + name + '() {\n    with_local_jcode_home(|| {\n', 1)
            if source == SUBSCRIPTION:
                expected = expected.replace('    let _guard = crate::storage::lock_test_env();\n', '', 1)
            close = expected.rfind('}')
            expected = expected[:close] + '    });\n' + expected[close:]
            self.assertEqual(after[source], before[:start] + expected + before[end:])
            self.assertEqual(before.count('assert'), after[source].count('assert'))

    def test_strict_hashes_and_snapshot_identity(self):
        spec = suite.SNAPSHOTS['desktop']
        self.assertIn(('jcode', PATCH), spec['patches'])
        self.assertEqual(spec['fixture_patch_sha256'][PATCH], PATCH_SHA)
        self.assertEqual(hashlib.sha256((suite.ROOT / PATCH).read_bytes()).hexdigest(), PATCH_SHA)
        identity = suite.snapshot_fingerprint('desktop')
        with patch.dict(suite.SNAPSHOTS, {'desktop': dict(spec, patches=[p for p in spec['patches'] if p[1] != PATCH])}):
            self.assertNotEqual(identity, suite.snapshot_fingerprint('desktop'))
        with tempfile.TemporaryDirectory() as directory:
            bad = Path(directory) / 'bad'
            bad.write_text('unexpected bytes')
            with self.assertRaisesRegex(ValueError, 'corrupt or unversioned fixture patch'):
                suite.verify_fixture_patch(bad, PATCH_SHA)
            for source, digest in INPUTS.items():
                self.assertEqual(spec['fixture_input_sha256']['jcode/' + source], digest)
                with self.assertRaisesRegex(ValueError, 'unexpected pinned fixture input SHA256'):
                    suite.verify_fixture_input(bad, digest)


def acceptance(directory, full=False):
    import runtime_context
    os.environ[runtime_context.SELECTOR] = runtime_context.POLICY
    directory = directory.resolve()
    directory.mkdir(parents=True, exist_ok=True)
    suite.WORK = directory / 'work'
    snapshot = suite.prepare('desktop')
    diagnostics = directory / 'diagnostics'
    if diagnostics.exists():
        raise ValueError(f'diagnostic destination already exists: {diagnostics}')
    copy_acceptance_snapshot(snapshot, diagnostics)
    repo = diagnostics / 'jcode'
    env = dict(os.environ, JUSTRUST_TOOLCHAIN='system', RUSTUP_TOOLCHAIN='1.99.0')
    if env.get('RUST_TEST_THREADS') or env.get('JUSTRUST_TEST_PARALLEL') == '0':
        raise ValueError('fixture acceptance requires normal test parallelism')
    env.setdefault('CARGO_TARGET_DIR', str(directory / 'target'))
    before = {source: original(source) for source in INPUTS}
    with tempfile.TemporaryDirectory() as temporary:
        after = corrected(Path(temporary))

    def run(arguments, expected=0, assertion=None, cwd=repo):
        with runtime_context.runtime_context(env, cwd) as child:
            result = subprocess.run(['justrust', 'test', '--local=controlled auth reader fixture requires exact disposable binaries', *arguments], cwd=cwd, env=child, capture_output=True, text=True)
        print(result.stdout, end='', flush=True)
        print(result.stderr, end='', file=sys.stderr, flush=True)
        output = result.stdout + result.stderr
        if result.returncode != expected:
            raise RuntimeError(f'expected exit {expected}, got {result.returncode}')
        if expected == 0 and 'passed when rerun alone by name' in output:
            raise RuntimeError('acceptance requires clean parallel tests, not isolated retries')
        if assertion and assertion not in output:
            raise RuntimeError(f'original assertion not reached: {assertion}')
        if arguments[-1].startswith('benchmark_fixture_auth_') and expected == 0 and '1 passed' not in output:
            raise RuntimeError('corrected original fixture was not executed')

    for variant in ('before', 'after'):
        for source in INPUTS:
            (repo / source).write_text((before if variant == 'before' else after)[source])
        for source, name in NAMES.items():
            control = 'benchmark_fixture_auth_' + name
            caller = '''
#[test]
fn CONTROL() {
    let prior_home = tempfile::tempdir().unwrap();
    let _home = EnvRestoreGuard::set("JCODE_HOME", prior_home.path());
    {
        let _lock = crate::storage::lock_test_env();
        CREDENTIALS
    }
    ORIGINAL();
}
'''.replace('CONTROL', control).replace('CREDENTIALS', CREDENTIALS[source]).replace('ORIGINAL', name)
            (repo / source).write_text((before if variant == 'before' else after)[source] + caller)
            print('AUTH READER CONTROL', variant, name, flush=True)
            expected = 101 if variant == 'before' else 0
            assertion = ('msg.content.contains("/login jcode")' if source == SUBSCRIPTION else 'no agent-edit dot on clean Changes rows:') if expected else None
            run(['-p', 'jcode-tui', '--lib', control], expected, assertion)
            (repo / source).write_text((before if variant == 'before' else after)[source])
    # Same assertions still reject credential contamination inside the owning
    # private home. The control is inserted after the new scope opens, never in
    # measured source or a production path.
    for source, name in NAMES.items():
        anchor = 'fn ' + name + '() {\n    with_local_jcode_home(|| {\n'
        text = after[source].replace(anchor, anchor + CREDENTIALS[source], 1)
        (repo / source).write_text(text)
        print('AUTH READER OWNED CONTAMINATION', name, flush=True)
        assertion = 'msg.content.contains("/login jcode")' if source == SUBSCRIPTION else 'no agent-edit dot on clean Changes rows:'
        run(['-p', 'jcode-tui', '--lib', name], 101, assertion)
        (repo / source).write_text(after[source])
    if full:
        failures = []
        commands = [('jcode', ['-p', 'jcode-tui', '--lib'])] * 2
        commands += [(name, ['--workspace']) for name in ('jcode', 'jcode-desktop') for _ in range(2)]
        for index, (name, arguments) in enumerate(commands, 1):
            print('FULL AUTH READER ACCEPTANCE', index, name, arguments, flush=True)
            try:
                run(arguments, cwd=diagnostics / name)
            except RuntimeError as error:
                failures.append((index, name, str(error)))
        print('FULL AUTH READER ACCEPTANCE FAILURES', failures, flush=True)
        if failures:
            raise RuntimeError(f'full acceptance blocked: {failures}')


if __name__ == '__main__':
    if len(sys.argv) in (3, 4) and sys.argv[1] == '--accept':
        if len(sys.argv) == 4 and sys.argv[3] != '--full':
            raise ValueError('only --full is supported')
        acceptance(Path(sys.argv[2]), full=len(sys.argv) == 4)
    else:
        unittest.main()
