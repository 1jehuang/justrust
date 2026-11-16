"""Pinned clipboard PATH readers must cooperate with existing env writers."""
from pathlib import Path
import hashlib
import re
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import suite

PATCH = 'bench/patches/jcode-clipboard-path-lock-v1.patch'
SOURCE = 'crates/jcode-tui/src/tui/app/helpers/clipboard_helper.rs'
GUARD = '        let _lock = crate::storage::lock_test_env();\n'
READERS = (
    'helper_that_exits_successfully_counts_as_a_copy',
    'helper_that_exits_nonzero_falls_through',
    'missing_helper_binary_falls_through',
    'long_lived_helper_counts_as_a_copy_without_blocking',
)


def corrected_source(root):
    repo, revision = suite.SNAPSHOTS['desktop']['repos']['jcode']
    original = subprocess.check_output(['git', '-C', str(repo), 'show', f'{revision}:{SOURCE}'], text=True)
    expected = suite.SNAPSHOTS['desktop']['fixture_input_sha256']['jcode/' + SOURCE]
    destination = root / SOURCE
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_text(original)
    suite.verify_fixture_input(destination, expected)
    subprocess.run(['git', 'apply', '--check', str(suite.ROOT / PATCH)], cwd=root, check=True)
    subprocess.run(['git', 'apply', str(suite.ROOT / PATCH)], cwd=root, check=True)
    return original, destination.read_text()


class ClipboardFixture(unittest.TestCase):
    def test_only_four_lock_guards_added_preserving_entire_workload(self):
        with tempfile.TemporaryDirectory() as directory:
            original, corrected = corrected_source(Path(directory))
        expected = original
        for name in READERS:
            anchor = f'    fn {name}() {{\n'
            self.assertEqual(expected.count(anchor), 1)
            expected = expected.replace(anchor, anchor + GUARD)
        self.assertEqual(corrected, expected)
        # All existing ordering tests already acquire this exact lock before
        # constructing StubPath, whose Drop restores PATH before guard release.
        writers = re.findall(r'fn (\w+)\(\) \{\n' + re.escape(GUARD) + r'        let _stubs = StubPath::with', original)
        self.assertEqual(len(writers), 4)
        for name in READERS:
            self.assertIn(f'fn {name}() {{\n' + GUARD, corrected)

    def test_hashes_fail_closed_and_change_snapshot_identity(self):
        spec = suite.SNAPSHOTS['desktop']
        self.assertIn(('jcode', PATCH), spec['patches'])
        digest = spec['fixture_patch_sha256'][PATCH]
        self.assertEqual(hashlib.sha256((suite.ROOT / PATCH).read_bytes()).hexdigest(), digest)
        before = suite.snapshot_fingerprint('desktop')
        with patch.dict(suite.SNAPSHOTS, {'desktop': dict(spec, patches=[p for p in spec['patches'] if p[1] != PATCH])}):
            self.assertNotEqual(before, suite.snapshot_fingerprint('desktop'))
        with tempfile.TemporaryDirectory() as directory:
            bad = Path(directory) / 'bad'
            bad.write_text('unexpected bytes')
            with self.assertRaisesRegex(ValueError, 'corrupt or unversioned fixture patch'):
                suite.verify_fixture_patch(bad, digest)
            with self.assertRaisesRegex(ValueError, 'unexpected pinned fixture input SHA256'):
                suite.verify_fixture_input(bad, spec['fixture_input_sha256']['jcode/' + SOURCE])


def acceptance(root):
    root.mkdir(parents=True, exist_ok=True)
    _, corrected = corrected_source(root / 'pinned')
    # Visibility only, in a disposable diagnostic crate. Measured source gets
    # exactly the four guards checked above, no extra assertions or tests.
    diagnostic = corrected.replace('//!', '//').replace('mod tests {', 'pub mod tests {')
    for name in READERS:
        diagnostic = diagnostic.replace(f'    fn {name}()', f'    pub fn {name}()')
    prefix = '''mod env {
    pub fn set_var(k: &str, v: impl AsRef<std::ffi::OsStr>) { std::env::set_var(k,v); }
    pub fn remove_var(k: &str) { std::env::remove_var(k); }
}
mod storage {
    pub static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    pub static ATTEMPTS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    pub fn lock_test_env() -> std::sync::MutexGuard<'static, ()> {
        ATTEMPTS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        LOCK.lock().unwrap()
    }
}
'''
    regression = '''
#[test]
fn deterministic_path_writer_reader_cooperation() {
    use std::sync::atomic::Ordering::SeqCst;
    let readers: [fn(); 4] = [READERS];
    for reader in readers {
        let owner = storage::LOCK.lock().unwrap();
        let original = std::env::var_os("PATH").unwrap();
        std::env::set_var("PATH", "/nonexistent-clipboard-fixture-path");
        // Reproduce the original cat failure without scheduling or sleeps.
        assert!(!copy_via_clipboard_helper("cat", &[], "hello"));
        storage::ATTEMPTS.store(0, SeqCst);
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let child = std::thread::spawn(move || { reader(); done_tx.send(()).unwrap(); });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while storage::ATTEMPTS.load(SeqCst) == 0 {
            assert!(std::time::Instant::now() < deadline, "reader never attempted shared lock");
            std::thread::yield_now();
        }
        assert_eq!(done_rx.try_recv(), Err(std::sync::mpsc::TryRecvError::Empty));
        std::env::set_var("PATH", original);
        drop(owner);
        child.join().unwrap();
        done_rx.recv().unwrap();
    }
}
'''.replace('READERS', ', '.join('tests::' + name for name in READERS))
    (root / 'src').mkdir(exist_ok=True)
    (root / 'src/lib.rs').write_text(prefix + diagnostic + regression)
    (root / 'Cargo.toml').write_text('[package]\nname="pinned-clipboard-lock-acceptance"\nversion="0.0.0"\nedition="2021"\n[dependencies]\ntempfile="3"\n')
    subprocess.run(['justrust', 'test', '--lib', '--', '--test-threads=1'], cwd=root, check=True, timeout=600)


if __name__ == '__main__':
    if len(sys.argv) == 3 and sys.argv[1] == '--accept':
        acceptance(Path(sys.argv[2]).resolve())
    else:
        unittest.main()
