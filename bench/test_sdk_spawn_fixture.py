"""SDK auth fixture never executes an inode written during parallel tests."""
from pathlib import Path
import hashlib
import os
import subprocess
import sys
import tempfile
import unittest

import suite
from test_login_event_fixture import assert_registered_sources_match

PATCH = 'bench/patches/jcode-sdk-spawn-fixture-v1.patch'
PATCH_SHA = '797bab34b91444588b08b9337f10dd3d7ef78f1377214ad26d88ffa978f394cd'
SOURCE = 'crates/jcode-sdk/src/auth/tests.rs'
SOURCE_SHA = '511d39606994fecea02614f4ecb8dfccf4d538a85902603ecd6775d5ed1d5e31'
SCRIPT = 'crates/jcode-sdk/src/auth/fixture.py'
# Retain the original runtime-written inode's writer exactly across exec. This
# deterministically models a sibling's fork inheriting that writer before close.
CONTROL = '''
    #[test]
    fn benchmark_sdk_spawn_writer_control() {
        use std::os::unix::fs::PermissionsExt;
        let (dir, client) = fixture("success");
        let old_binary = dir.path().join("fixture.py");
        let writer = std::fs::OpenOptions::new().create(true).write(true)
            .truncate(false).open(&old_binary).unwrap();
        std::fs::set_permissions(&old_binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        let error = std::process::Command::new(&old_binary).spawn().unwrap_err();
        assert_eq!(error.raw_os_error(), Some(26));
        eprintln!("SDK_RAW_CONTROL raw_os_error={:?} error={:?}", error.raw_os_error(), error);
        let flow = client.begin("OpenAI", Some("work")).unwrap();
        let prompt = flow.start().unwrap();
        assert_eq!(prompt.input_kind, AuthInputKind::CallbackUrl);
        assert!(!flow.submit_callback("private-fixture-secret").unwrap().validation_warning);
        assert_eq!(std::fs::read_to_string(dir.path().join("stdin-ok")).unwrap(), "True");
        drop(writer);
    }
'''


def original():
    repo, revision = suite.SNAPSHOTS['desktop']['repos']['jcode']
    return subprocess.check_output(['git', '-C', str(repo), 'show', f'{revision}:{SOURCE}'], text=True)


def apply(root):
    suite.verify_fixture_patch(suite.ROOT / PATCH, PATCH_SHA)
    suite.verify_fixture_input(root / SOURCE, SOURCE_SHA)
    subprocess.run(['git', 'apply', '--check', str(suite.ROOT / PATCH)], cwd=root, check=True)
    subprocess.run(['git', 'apply', str(suite.ROOT / PATCH)], cwd=root, check=True)


class SdkSpawnFixture(unittest.TestCase):
    def test_exact_script_extraction_and_all_original_tests_unchanged(self):
        before = original()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            target = root / SOURCE
            target.parent.mkdir(parents=True)
            target.write_text(before)
            apply(root)
            after = target.read_text()
            script = (root / SCRIPT).read_text()
            self.assertEqual(script, before.split('            r#"', 1)[1].split('"#,', 1)[0])
            self.assertEqual((root / SCRIPT).stat().st_mode & 0o777, 0o755)
            boundary = '    #[test]\n    fn oauth_round_trip_uses_stdin_and_scoped_flow_not_argv()'
            self.assertEqual(before.split(boundary, 1)[1], after.split(boundary, 1)[1])
            self.assertEqual(before.count('assert'), after.count('assert'))
            start = before.index('        let binary = dir.path().join("fixture.py");')
            end = before.index('        std::fs::write(dir.path().join("mode"), mode).unwrap();', start)
            replacement = '''        // Never write an executable while sibling tests can fork: their children
        // can transiently inherit a writer and make exec fail with ETXTBSY.
        // Only per-flow data is mutable. The executable is a prepared fixture.
        let binary = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/auth/fixture.py");
'''
            expected = before[:start] + replacement + before[end:]
            expected = expected.replace('    use std::os::unix::fs::PermissionsExt;\n', '', 1)
            self.assertEqual(after, expected)
            self.assertEqual(before.split('#[cfg(unix)]', 1)[0], after.split('#[cfg(unix)]', 1)[0])
            self.assertIn('timeout: Duration::from_secs(3)', after)
            self.assertNotIn('set_permissions', after)
            self.assertNotIn('std::fs::write(\n', after)

    def test_registered_identity_and_hashes(self):
        from unittest.mock import patch
        spec = suite.SNAPSHOTS['desktop']
        self.assertIn(('jcode', PATCH), spec['patches'])
        self.assertEqual(spec['fixture_patch_sha256'][PATCH], PATCH_SHA)
        self.assertEqual(spec['fixture_input_sha256']['jcode/' + SOURCE], SOURCE_SHA)
        identity = suite.snapshot_fingerprint('desktop')
        without = dict(spec, patches=[entry for entry in spec['patches'] if entry[1] != PATCH])
        with patch.dict(suite.SNAPSHOTS, {'desktop': without}):
            self.assertNotEqual(identity, suite.snapshot_fingerprint('desktop'))

    def test_strict_digests(self):
        self.assertEqual(hashlib.sha256((suite.ROOT / PATCH).read_bytes()).hexdigest(), PATCH_SHA)
        self.assertEqual(hashlib.sha256(original().encode()).hexdigest(), SOURCE_SHA)
        with tempfile.TemporaryDirectory() as temporary:
            bad = Path(temporary) / 'bad'
            bad.write_text('unexpected input')
            with self.assertRaisesRegex(ValueError, 'unexpected pinned fixture input SHA256'):
                suite.verify_fixture_input(bad, SOURCE_SHA)
            with self.assertRaisesRegex(ValueError, 'corrupt or unversioned fixture patch'):
                suite.verify_fixture_patch(bad, PATCH_SHA)


def acceptance(directory):
    """Use existing exact diagnostics, restore controls, attest before full SDK."""
    import runtime_context
    os.environ[runtime_context.SELECTOR] = runtime_context.POLICY
    directory = directory.resolve()
    # suite.prepare uses its fingerprinted directory, not a guessed basename.
    suite.WORK = directory / 'work'
    snapshot = suite.prepare('desktop')
    diagnostics = directory / 'diagnostics'
    repo = diagnostics / 'jcode'
    assert_registered_sources_match(snapshot, diagnostics)
    target = repo / SOURCE
    before = target.read_text()
    registered = ('jcode', PATCH) in suite.SNAPSHOTS['desktop']['patches']
    if registered:
        subprocess.run(['git', 'apply', '-R', str(suite.ROOT / PATCH)], cwd=repo, check=True)
        before = target.read_text()
    apply(repo)
    after = target.read_text()
    env = dict(os.environ, JUSTRUST_TOOLCHAIN='system', RUSTUP_TOOLCHAIN='1.99.0')
    if env.get('RUST_TEST_THREADS') or env.get('JUSTRUST_TEST_PARALLEL') == '0':
        raise ValueError('normal parallelism required')
    env.setdefault('CARGO_TARGET_DIR', str(directory / 'target'))

    def run(arguments, expected=0, marker=None):
        with runtime_context.runtime_context(env, repo) as child:
            if child['HOME'] == env['HOME']:
                raise RuntimeError('parent isolated-v4 policy was not activated')
            result = subprocess.run(['justrust', 'test', '--local=controlled SDK executable fixture proof', '-p', 'jcode-sdk', '--lib', *arguments], cwd=repo, env=child, capture_output=True, text=True)
        print(result.stdout, end='', flush=True)
        print(result.stderr, end='', file=sys.stderr, flush=True)
        output = result.stdout + result.stderr
        if result.returncode != expected or (marker and marker not in output):
            raise RuntimeError(f'expected exit {expected} and marker {marker!r}, got {result.returncode}')
        if expected == 0 and 'passed when rerun alone by name' in output:
            raise RuntimeError('isolated retries cannot establish acceptance')

    try:
        for label, source, expected in [('negative', before, 101), ('positive', after, 0)]:
            print('SDK WRITER CONTROL', label, flush=True)
            target.write_text(source.replace('    fn fixture(mode:', CONTROL + '\n    fn fixture(mode:', 1))
            run(['benchmark_sdk_spawn_writer_control', '--', '--nocapture'], expected, 'SDK_RAW_CONTROL raw_os_error=Some(26)')
    finally:
        target.write_text(after)
    if not registered:
        raise RuntimeError('controls passed, register exact patch before whole-SDK source attestation')
    assert_registered_sources_match(snapshot, diagnostics)
    if (snapshot / 'jcode' / SCRIPT).read_bytes() != (repo / SCRIPT).read_bytes():
        raise RuntimeError('added executable source differs from registered fixture')
    if (repo / SCRIPT).stat().st_mode & 0o777 != 0o755:
        raise RuntimeError('prepared fixture must be executable')
    run([])
    run([])


if __name__ == '__main__':
    if len(sys.argv) == 3 and sys.argv[1] == '--accept':
        acceptance(Path(sys.argv[2]))
    else:
        unittest.main()
