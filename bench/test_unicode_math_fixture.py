"""Unicode-math shortcut fixture must own its non-image terminal capability."""
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

PATCH = 'bench/patches/jcode-unicode-math-fixture-v1.patch'
PATCH_SHA = 'edf0e48b23c7191908bde3532e90fb1e14b3f4c35cfb3762676ff8e243eba11b'
SOURCE = 'crates/jcode-tui/src/tui/app/tests/scroll_copy_02/part_02.rs'
SOURCE_SHA = '9d43eab940f2f5bcaaaab6b3a03360a93828d4145fc74a915fac6686a9165775'
NAME = 'test_unicode_math_copy_badge_copies_latex_to_clipboard'
SCOPE = '    crate::tui::mermaid::with_image_protocol_override(Some(false), || {\n'


def original():
    repo, revision = suite.SNAPSHOTS['desktop']['repos']['jcode']
    return subprocess.check_output(['git', '-C', str(repo), 'show', f'{revision}:{SOURCE}'], text=True)


def corrected(root):
    target = root / SOURCE
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(original())
    suite.verify_fixture_input(target, SOURCE_SHA)
    suite.verify_fixture_patch(suite.ROOT / PATCH, PATCH_SHA)
    subprocess.run(['git', 'apply', '--check', str(suite.ROOT / PATCH)], cwd=root, check=True)
    subprocess.run(['git', 'apply', str(suite.ROOT / PATCH)], cwd=root, check=True)
    return target.read_text()


def capability_scope(source):
    start = source.index('fn ' + NAME + '() {\n') + len('fn ' + NAME + '() {\n')
    end = source.rfind('\n}')
    return source[:start] + SCOPE + source[start:end] + '\n    });' + source[end:]


def without_capability_scope(source):
    start = source.index('fn ' + NAME + '() {\n') + len('fn ' + NAME + '() {\n')
    if not source[start:].startswith(SCOPE):
        return source
    source = source[:start] + source[start + len(SCOPE):]
    end = source.rfind('\n    });\n}')
    if end < start:
        raise ValueError('unexpected Unicode fixture scope shape')
    return source[:end] + source[end + len('\n    });'):]


class UnicodeMathFixture(unittest.TestCase):
    def test_only_capability_scope_preserving_every_original_byte(self):
        before = original()
        with tempfile.TemporaryDirectory() as directory:
            after = corrected(Path(directory))
        start = before.index('fn ' + NAME + '() {\n') + len('fn ' + NAME + '() {\n')
        end = before.rfind('\n}')
        expected = before[:start] + SCOPE + before[start:end] + '\n    });' + before[end:]
        self.assertEqual(after, expected)
        self.assertEqual(capability_scope(before), after)
        self.assertEqual(without_capability_scope(after), before)
        self.assertEqual(before.count('assert'), after.count('assert'))

    def test_prior_overlays_survive_control_and_restoration(self):
        source = original().replace('fn test_copy_badge_modifier_highlights_while_held() {',
            'fn test_copy_badge_modifier_highlights_while_held() {\n    // prior registered overlay retained')
        after = capability_scope(source)
        self.assertEqual(without_capability_scope(after), source)
        self.assertIn('// prior registered overlay retained', after)
        self.assertEqual(without_capability_scope(source), source)

    def test_full_acceptance_rejects_any_registered_source_drift(self):
        with tempfile.TemporaryDirectory() as directory:
            snapshot = Path(directory) / 'snapshot'
            diagnostics = Path(directory) / 'diagnostics'
            for root in (snapshot, diagnostics):
                for name in ('jcode', 'jcode-desktop'):
                    (root / name).mkdir(parents=True)
                    (root / name / 'fixture.rs').write_text('exact registered bytes')
            with patch('subprocess.check_output', side_effect=['fixture.rs\n', b'pinned', b'pinned'] * 2):
                assert_registered_sources_match(snapshot, diagnostics)
            (diagnostics / 'jcode' / 'fixture.rs').write_text('lost prior overlay')
            with patch('subprocess.check_output', return_value='fixture.rs\n'):
                with self.assertRaisesRegex(RuntimeError, 'acceptance source differs from registered jcode'):
                    assert_registered_sources_match(snapshot, diagnostics)
            (diagnostics / 'jcode' / 'fixture.rs').write_text('exact registered bytes')
            with patch('subprocess.check_output', side_effect=['fixture.rs\n', b'pinned', b'different']):
                with self.assertRaisesRegex(RuntimeError, 'pinned Git HEAD differs'):
                    assert_registered_sources_match(snapshot, diagnostics)

    def test_strict_hashes_and_snapshot_identity(self):
        spec = suite.SNAPSHOTS['desktop']
        self.assertIn(('jcode', PATCH), spec['patches'])
        self.assertEqual(spec['fixture_patch_sha256'][PATCH], PATCH_SHA)
        self.assertEqual(spec['fixture_input_sha256']['jcode/' + SOURCE], SOURCE_SHA)
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
    copy_acceptance_snapshot(snapshot, diagnostics)
    repo = diagnostics / 'jcode'
    env = dict(os.environ, JUSTRUST_TOOLCHAIN='system', RUSTUP_TOOLCHAIN='1.99.0')
    if env.get('RUST_TEST_THREADS') or env.get('JUSTRUST_TEST_PARALLEL') == '0':
        raise ValueError('fixture acceptance requires normal test parallelism')
    env.setdefault('CARGO_TARGET_DIR', str(directory / 'target'))
    registered = (repo / SOURCE).read_text()
    before = without_capability_scope(registered)
    after = capability_scope(before)
    # Preserve every prior registered overlay, not just the raw pinned file.
    # Before registration the candidate adds its scope, after registration it
    # must reconstruct the already-prepared source byte for byte.
    if ('jcode', PATCH) in suite.SNAPSHOTS['desktop']['patches'] and after != registered:
        raise RuntimeError('candidate scope does not reconstruct registered source')

    def run(arguments, expected=0, assertion=None, cwd=repo):
        with runtime_context.runtime_context(env, cwd) as child:
            if child['HOME'] == env['HOME']:
                raise RuntimeError('isolated-v4 context was not activated')
            result = subprocess.run(['justrust', 'test', '--local=controlled Unicode math capability fixture requires exact disposable binaries', *arguments], cwd=cwd, env=child, capture_output=True, text=True)
        print(result.stdout, end='', flush=True)
        print(result.stderr, end='', file=sys.stderr, flush=True)
        output = result.stdout + result.stderr
        if result.returncode != expected:
            raise RuntimeError(f'expected exit {expected}, got {result.returncode}')
        if expected == 0 and 'passed when rerun alone by name' in output:
            raise RuntimeError('acceptance requires clean parallel tests, not isolated retries')
        if assertion and assertion not in output:
            raise RuntimeError(f'original assertion not reached: {assertion}')
        if arguments[-1] == 'benchmark_fixture_unicode_math_global_image_control' and expected == 0 and '1 passed' not in output:
            raise RuntimeError('corrected original fixture was not executed')

    caller = '''
#[test]
fn benchmark_fixture_unicode_math_global_image_control() {
    // A real producer from sibling pinned render tests, not an event mock.
    std::thread::spawn(|| crate::tui::mermaid::set_video_export_mode(true)).join().unwrap();
    struct RestoreVideo;
    impl Drop for RestoreVideo {
        fn drop(&mut self) { crate::tui::mermaid::set_video_export_mode(false); }
    }
    let _restore = RestoreVideo;
    test_unicode_math_copy_badge_copies_latex_to_clipboard();
}
'''
    target = repo / SOURCE
    try:
        for variant, source, expected in [('before', before, 101), ('after', after, 0)]:
            print('UNICODE MATH REAL GLOBAL CONTROL', variant, flush=True)
            target.write_text(source + caller)
            run(['-p', 'jcode-tui', '--lib', 'benchmark_fixture_unicode_math_global_image_control'], expected,
                'math frame should get a badge:' if expected else None)
        anchor = "    app.handle_key(KeyCode::Char('S'), KeyModifiers::ALT)\n        .unwrap();\n"
        start = after.index(anchor, after.index('fn ' + NAME)) + len(anchor)
        target.write_text(after[:start] + '    assert!(crate::tui::app::helpers::copy_to_clipboard("owned invalid clipboard write"));\n' + after[start:])
        print('UNICODE MATH OWNED WRONG COPY', flush=True)
        run(['-p', 'jcode-tui', '--lib', NAME], 101, 'left: Some("owned invalid clipboard write")')
    finally:
        target.write_text(after)
    if full:
        assert_registered_sources_match(snapshot, diagnostics)
        failures = []
        commands = [('jcode', ['-p', 'jcode-tui', '--lib'])] * 2
        commands += [(name, ['--workspace']) for name in ('jcode', 'jcode-desktop') for _ in range(2)]
        for index, (name, arguments) in enumerate(commands, 1):
            print('FULL UNICODE MATH ACCEPTANCE', index, name, arguments, flush=True)
            try:
                run(arguments, cwd=diagnostics / name)
            except RuntimeError as error:
                failures.append((index, name, str(error)))
        print('FULL UNICODE MATH ACCEPTANCE FAILURES', failures, flush=True)
        if failures:
            raise RuntimeError(f'full acceptance blocked: {failures}')


if __name__ == '__main__':
    if len(sys.argv) in (3, 4) and sys.argv[1] == '--accept':
        if len(sys.argv) == 4 and sys.argv[3] != '--full':
            raise ValueError('only --full is supported')
        acceptance(Path(sys.argv[2]), full=len(sys.argv) == 4)
    else:
        unittest.main()
