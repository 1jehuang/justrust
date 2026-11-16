#!/usr/bin/env python3
"""Unit guards and repeatable real-PTY acceptance for the pinned helper overlay.

Run unit guards: python3 -m unittest discover -s bench -p test_pty_fixture.py
Run concurrent real parser acceptance: python3 bench/test_pty_fixture.py --accept PATH
PATH is a disposable acceptance crate, never a project checkout. The local build
is intentional: repeated concurrent binary executions need the local artifact.
"""
import concurrent.futures
import hashlib
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import pty_fixture


def pinned_source():
    source = (Path(__file__).parent / "fixtures/terminal_setup_command.abff94eeb.rs").read_bytes()
    if hashlib.sha256(source).hexdigest() != pty_fixture.INPUT_SHA256:
        raise ValueError("corrupt vendored pinned PTY acceptance input")
    return source


def corrected_source(root):
    destination = root / pty_fixture.INPUT
    destination.parent.mkdir(parents=True, exist_ok=True)
    source = pinned_source()
    destination.write_bytes(source)
    pty_fixture.apply(root)
    return source.decode(), destination.read_text()


class PtyFixture(unittest.TestCase):
    def test_corrupt_patch_rejected_before_subprocess(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            target = root / pty_fixture.PATCH
            target.parent.mkdir(parents=True)
            target.write_text("corrupt patch")
            with patch.object(pty_fixture, "ROOT", root), patch.object(pty_fixture.subprocess, "run") as run:
                with self.assertRaisesRegex(ValueError, "corrupt"):
                    pty_fixture.apply(root)
                run.assert_not_called()

    def test_unexpected_source_rejected_before_patch(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / pty_fixture.INPUT
            source.parent.mkdir(parents=True)
            source.write_text("unexpected source")
            with patch.object(pty_fixture.subprocess, "run") as run:
                with self.assertRaisesRegex(ValueError, "input SHA256"):
                    pty_fixture.apply(root)
                run.assert_not_called()

    def test_exact_input_and_all_original_tests_preserved(self):
        with tempfile.TemporaryDirectory() as directory:
            old, new = corrected_source(Path(directory))
            self.assertEqual(hashlib.sha256(old.encode()).hexdigest(), pty_fixture.INPUT_SHA256)
            # Every test and assertion is byte-for-byte identical, no new tests,
            # no skip, environment fixture, or test-thread-count relaxation.
            self.assertEqual(old[old.index("#[test]"):], new[new.index("#[test]"):])
            self.assertNotIn("libc::fork", new)
            self.assertNotIn("read_to_string", new)
            self.assertIn("command.pre_exec", new)
            self.assertIn('assert_eq!(ready, [b\'R\']', new)
            self.assertNotIn("return None", new)

    def test_snapshot_and_runtime_identity(self):
        import suite
        import prerequisites
        spec = suite.SNAPSHOTS["desktop"]
        self.assertEqual(spec["repos"]["jcode"][1], "abff94eeb")
        self.assertEqual(spec["fixture_overlays"], [pty_fixture.identity()])
        before = suite.snapshot_fingerprint("desktop")
        with patch.dict(suite.SNAPSHOTS, {"desktop": dict(spec, fixture_overlays=[])}):
            self.assertNotEqual(before, suite.snapshot_fingerprint("desktop"))
            self.assertNotEqual(suite.snapshot_dir("desktop").name.split("-")[1], before[:12])
        self.assertEqual(prerequisites.provenance()["fixture_overlays"], [pty_fixture.identity()])


def acceptance(root):
    root = root.resolve()
    root.mkdir(parents=True, exist_ok=True)
    _, source = corrected_source(root / "pinned")
    # Extract the exact corrected helper plus the two exact pinned decoder tests.
    source = source[source.index("/// Versioned benchmark fixture"):]
    source = source[:source.index("#[test]\nfn decoded_shift_enter")]
    prefix = 'mod tui { pub mod terminal_setup { pub const SHIFT_ENTER_CSI_U: &str = "\\x1b[13;2u"; } }\n'
    (root / "src").mkdir(exist_ok=True)
    (root / "src/lib.rs").write_text(prefix + source)
    (root / "Cargo.toml").write_text('[package]\nname="pinned-pty-acceptance"\nversion="0.0.0"\nedition="2021"\n[dependencies]\nlibc="0.2"\ncrossterm={version="=0.29.0",features=["event-stream"]}\n')
    subprocess.run(["justrust", "test", "--local=real PTY concurrent acceptance needs a local test executable", "--lib", "--target-dir", str(root / "acceptance-target")],
                   cwd=root, check=True, timeout=180)
    binaries = [p for p in (root / "acceptance-target/debug/deps").glob("pinned_pty_acceptance-*")
                if p.is_file() and p.stat().st_mode & 0o111 and p.suffix != ".d"]
    if len(binaries) != 1:
        raise RuntimeError(f"expected one acceptance executable, got {binaries}")
    binary = binaries[0]
    def execute(index):
        result = subprocess.run([str(binary)], capture_output=True, text=True, timeout=20)
        if result.returncode or "2 passed; 0 failed; 0 ignored" not in result.stdout:
            raise RuntimeError(f"PTY iteration {index}: {result.stdout}\n{result.stderr}")
    with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
        list(pool.map(execute, range(160)))
    print("PTY acceptance: 160 executions, 8 concurrent processes, 320 real crossterm decodes passed")
    # Fault injection is confined to this disposable acceptance crate. The
    # measured snapshot receives only the immutable reviewed patch above.
    injected = source.replace(
        '        // This runs after exec in the same existing test, never after fork.',
        '''        let fault = std::thread::current().name().unwrap().to_owned();
        if fault == "fault_timeout" { std::thread::sleep(Duration::from_secs(60)); }
        if fault == "fault_malformed" {
            std::io::stderr().write_all(b"X").unwrap();
            std::thread::sleep(Duration::from_secs(60));
        }
        if fault == "fault_exit" {
            std::io::stderr().write_all(&[b'R', 1, 0]).unwrap();
            std::process::exit(3);
        }''', 1).replace(
            'let deadline = Instant::now() + Duration::from_secs(10);',
            'let deadline = Instant::now() + if name.starts_with("fault_") { Duration::from_millis(500) } else { Duration::from_secs(10) };', 1)
    failures = (Path(__file__).parent / "fixtures/pty_failure_acceptance.rs").read_text()
    (root / "src/lib.rs").write_text(prefix + injected + failures)
    subprocess.run(["justrust", "test", "--local=PTY failure injection needs local child cleanup assertions", "--lib", "--no-run", "--target-dir", str(root / "acceptance-target")],
                   cwd=root, check=True, timeout=180)
    for name in ("fault_timeout", "fault_malformed", "fault_exit"):
        for _ in range(5):
            result = subprocess.run([str(binary), "--exact", name], capture_output=True, text=True, timeout=5)
            if result.returncode or "1 passed; 0 failed; 0 ignored" not in result.stdout:
                raise RuntimeError(f"{name}: {result.stdout}\n{result.stderr}")
    print("PTY failure acceptance: timeout, malformed readiness, nonzero exit each passed 5 times with ECHILD cleanup assertions")
    print(pty_fixture.identity())


if __name__ == "__main__":
    if len(sys.argv) == 3 and sys.argv[1] == "--accept":
        acceptance(Path(sys.argv[2]))
    else:
        unittest.main()
