"""Pinned clipboard capture must belong to the test, not unrelated writers."""
from pathlib import Path
import hashlib
import os
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import suite
from test_login_event_fixture import copy_acceptance_snapshot, assert_registered_sources_match

PATCH = 'bench/patches/jcode-clipboard-capture-fixture-v1.patch'
SOURCE = 'crates/jcode-tui/src/tui/app/helpers.rs'
SOURCE_SHA = '51a69f2713ef780424eccf42e03178bdce32f621372dc62fd3da4ee67c991250'
PATCH_SHA = '612a3976157bc95ea4dada68c1a5baa354debf1f95dd6329e815705816540849'
TEST = 'crates/jcode-tui/src/tui/app/tests/scroll_copy_02/part_02.rs'
NAME = 'test_unicode_math_copy_badge_copies_latex_to_clipboard'
ANCHOR = "    app.handle_key(KeyCode::Char('S'), KeyModifiers::ALT)\n        .unwrap();\n"
UNRELATED = '''
    std::thread::spawn(|| {
        assert!(crate::tui::app::helpers::copy_to_clipboard("unrelated clipboard write"));
    }).join().unwrap();
'''
OWNED = '''
    assert!(crate::tui::app::helpers::copy_to_clipboard("owned invalid clipboard write"));
'''


def original():
    repo, revision = suite.SNAPSHOTS['desktop']['repos']['jcode']
    return subprocess.check_output(['git', '-C', str(repo), 'show', f'{revision}:{SOURCE}'], text=True)


def corrected(root):
    destination = root / SOURCE
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_text(original())
    suite.verify_fixture_input(destination, SOURCE_SHA)
    suite.verify_fixture_patch(suite.ROOT / PATCH, PATCH_SHA)
    subprocess.run(['git', 'apply', '--check', str(suite.ROOT / PATCH)], cwd=root, check=True)
    subprocess.run(['git', 'apply', str(suite.ROOT / PATCH)], cwd=root, check=True)
    return destination.read_text()


class ClipboardCaptureFixture(unittest.TestCase):
    def test_test_only_ownership_and_every_production_byte_preserved(self):
        before = original()
        with tempfile.TemporaryDirectory() as directory:
            after = corrected(Path(directory))
        # The entire production clipboard implementation and all unrelated
        # helper code remain byte-identical. All fixture assertions are outside
        # this patch's only source file and cannot be changed by it.
        start = before.index('/// Test-only clipboard sink.')
        end = '    #[cfg(not(test))]\n    {\n        // On Windows'
        self.assertEqual(before[:start], after[:start])
        self.assertEqual(before.split(end, 1)[1], after.split(end, 1)[1])
        self.assertIn('#[cfg(test)]\nthread_local! {', after)
        # Every original mutation/capture branch stays, merely under TLS.
        for operation in ('*sink = Some(String::new());', '*sink = None;',
                          'captured.clear();', 'captured.push_str(text);',
                          'None => *sink = Some(text.to_string()),'):
            self.assertEqual(before.count(operation), after.count(operation))
        # Reversing only the ownership wrappers reconstructs the exact original.
        source = after.replace('''thread_local! {
    // Each test owns its capture, including writes without a render-state lock.
    static TEST_CLIPBOARD: std::sync::Mutex<Option<String>> = const {
        std::sync::Mutex::new(None)
    };
}''', 'static TEST_CLIPBOARD: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);')
        source = source.replace('''    TEST_CLIPBOARD.with(|clipboard| {
        if let Ok(mut sink) = clipboard.lock() {
            *sink = Some(String::new());
        }
    });''', '''    if let Ok(mut sink) = TEST_CLIPBOARD.lock() {
        *sink = Some(String::new());
    }''')
        source = source.replace('TEST_CLIPBOARD.with(|clipboard| clipboard.lock().ok().and_then(|sink| sink.clone()))', 'TEST_CLIPBOARD.lock().ok().and_then(|sink| sink.clone())')
        source = source.replace('''    TEST_CLIPBOARD.with(|clipboard| {
        if let Ok(mut sink) = clipboard.lock() {
            *sink = None;
        }
    });''', '''    if let Ok(mut sink) = TEST_CLIPBOARD.lock() {
        *sink = None;
    }''')
        source = source.replace('''        TEST_CLIPBOARD.with(|clipboard| {
            if let Ok(mut sink) = clipboard.lock() {
                match sink.as_mut() {
                    Some(captured) => {
                        captured.clear();
                        captured.push_str(text);
                    }
                    None => *sink = Some(text.to_string()),
                }
            }
        });''', '''        if let Ok(mut sink) = TEST_CLIPBOARD.lock() {
            match sink.as_mut() {
                Some(captured) => {
                    captured.clear();
                    captured.push_str(text);
                }
                None => *sink = Some(text.to_string()),
            }
        }''')
        self.assertEqual(source, before)

    def test_strict_registry_hashes_and_new_snapshot_identity(self):
        spec = suite.SNAPSHOTS['desktop']
        self.assertIn(('jcode', PATCH), spec['patches'])
        self.assertEqual(spec['fixture_input_sha256']['jcode/' + SOURCE], SOURCE_SHA)
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
            with self.assertRaisesRegex(ValueError, 'unexpected pinned fixture input SHA256'):
                suite.verify_fixture_input(bad, SOURCE_SHA)


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
    # Preserve exact pinned Git metadata, not an empty synthesized repository.
    copy_acceptance_snapshot(snapshot, diagnostics)
    repo = diagnostics / 'jcode'
    env = dict(os.environ, JUSTRUST_TOOLCHAIN='system', RUSTUP_TOOLCHAIN='1.99.0')
    if env.get('RUST_TEST_THREADS') or env.get('JUSTRUST_TEST_PARALLEL') == '0':
        raise ValueError('fixture acceptance requires normal test parallelism')
    env.setdefault('CARGO_TARGET_DIR', str(directory / 'target'))
    test = (repo / TEST).read_text()
    with tempfile.TemporaryDirectory() as temporary:
        helper = corrected(Path(temporary))

    def run(arguments, expected=0, failure=None, cwd=repo):
        with runtime_context.runtime_context(env, cwd) as child:
            result = subprocess.run(['justrust', 'test', '--local=controlled pinned clipboard fixture requires exact disposable binaries', *arguments], cwd=cwd, env=child, capture_output=True, text=True)
        print(result.stdout, end='', flush=True)
        print(result.stderr, end='', file=sys.stderr, flush=True)
        output = result.stdout + result.stderr
        if result.returncode != expected:
            raise RuntimeError(f'expected exit {expected}, got {result.returncode}')
        if expected == 0 and 'passed when rerun alone by name' in output:
            raise RuntimeError('acceptance requires clean normal-parallel tests, not single-test retries')
        if failure and ('assertion `left == right` failed' not in output or failure not in output):
            raise RuntimeError('original exact clipboard-text assertion not reached')
        if arguments[-1] == NAME and expected == 0 and '1 passed' not in output:
            raise RuntimeError('corrected clipboard fixture was not executed')

    for variant, control, expected in [('before', UNRELATED, 101), ('after', UNRELATED, 0), ('after', OWNED, 101)]:
        print('CLIPBOARD CONTROL', variant, 'unrelated' if control == UNRELATED else 'owned-invalid-write', flush=True)
        index = test.index(ANCHOR, test.index('fn ' + NAME)) + len(ANCHOR)
        (repo / TEST).write_text(test[:index] + control + test[index:])
        (repo / SOURCE).write_text(original() if variant == 'before' else helper)
        failure = ('left: Some("unrelated clipboard write")' if control == UNRELATED else 'left: Some("owned invalid clipboard write")') if expected else None
        run(['-p', 'jcode-tui', '--lib', NAME], expected, failure)
    (repo / TEST).write_text(test)
    (repo / SOURCE).write_text(helper)
    if full:
        (repo / SOURCE).write_bytes((snapshot / 'jcode' / SOURCE).read_bytes())
        assert_registered_sources_match(snapshot, diagnostics)
        for _ in range(2):
            print('FULL PINNED TUI ACCEPTANCE', flush=True)
            run(['-p', 'jcode-tui', '--lib'])
        for name in ('jcode', 'jcode-desktop'):
            for _ in range(2):
                print('FULL PINNED WORKSPACE ACCEPTANCE', name, flush=True)
                run(['--workspace'], cwd=diagnostics / name)


if __name__ == '__main__':
    if len(sys.argv) in (3, 4) and sys.argv[1] == '--accept':
        if len(sys.argv) == 4 and sys.argv[3] != '--full':
            raise ValueError('only --full is supported')
        acceptance(Path(sys.argv[2]), full=len(sys.argv) == 4)
    else:
        unittest.main()
