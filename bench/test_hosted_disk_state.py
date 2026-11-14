"""Exercise the hosted driver's remote protocol locally without hosted builds."""
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

SPEC = importlib.util.spec_from_file_location("hosted_disk_state", Path(__file__).with_name("hosted_disk_state.py"))
driver = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(driver)


class HostedDiskStateTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="disk-state-tests-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.target = self.root / "source/target/debug/incremental"
        (self.target / "session/nested").mkdir(parents=True)
        (self.target / "session/nested/state.bin").write_bytes(bytes(range(256)) * 16)
        (self.target / "session/empty").touch()
        self.sentinel = self.root / "source/target/debug/unrelated"
        self.sentinel.write_text("keep")

    def run_protocol(self, action, workers=1, root=None):
        return subprocess.run([sys.executable, "-c", driver.REMOTE,
                               str(root or self.root), action, str(workers)],
                              text=True, capture_output=True, timeout=20)

    def successful(self, action, workers=1):
        result = self.run_protocol(action, workers)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        return json.loads(result.stdout.split("DISK_STATE_JSON=", 1)[1])

    def test_snapshot_fresh_restore_all_workers(self):
        original = (self.target / "session/nested/state.bin").read_bytes()
        snapshot = self.successful("snapshot")
        self.assertEqual(snapshot["files"], 2)
        self.assertEqual(snapshot["bytes"], len(original))
        self.assertGreaterEqual(snapshot["seconds"], 0)
        self.assertIn("cpu_count", snapshot)
        self.assertIn("loadavg", snapshot)
        expected = json.loads((self.root / "state-manifest.json").read_text())
        self.assertEqual(len(expected), 2)
        self.assertTrue(all(len(digest) == 64 for digest in expected.values()))
        fresh = self.successful("fresh")
        self.assertEqual(fresh["files"], 0)
        self.assertFalse(self.target.exists())
        for workers in (1, 4, 16):
            restored = self.successful("restore", workers)
            self.assertEqual(restored["files"], 2)
            self.assertEqual(restored["bytes"], len(original))
            self.assertEqual((self.target / "session/nested/state.bin").read_bytes(), original)
            (self.target / "stale").write_text("remove on next restore")
        self.assertEqual(self.sentinel.read_text(), "keep")

    def test_snapshot_refuses_overwrite(self):
        self.successful("snapshot")
        result = self.run_protocol("snapshot")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("already exists", result.stderr)

    def test_missing_snapshot_refuses_before_removing_target(self):
        result = self.run_protocol("restore")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("valid snapshot required", result.stderr)
        self.assertTrue((self.target / "session/nested/state.bin").exists())

    def test_corrupted_snapshot_fails_verification(self):
        self.successful("snapshot")
        state = self.root / "state-snapshot/session/nested/state.bin"
        state.write_bytes(b"x" * state.stat().st_size)
        result = self.run_protocol("restore", 4)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("restore hash mismatch", result.stderr)
        self.assertEqual(self.sentinel.read_text(), "keep")

    def test_missing_and_empty_state_rejected(self):
        other = self.root / "disk-state-empty"
        other.mkdir()
        for populated in (False, True):
            if populated:
                (other / "source/target/debug/incremental").mkdir(parents=True)
            result = self.run_protocol("snapshot", root=other)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("missing or empty", result.stderr)

    def test_unsafe_root_and_unknown_action_rejected(self):
        unsafe = self.root / "not-an-experiment"
        unsafe.mkdir()
        result = self.run_protocol("fresh", root=unsafe)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("unsafe experiment root", result.stderr)
        result = self.run_protocol("invalid")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("bad action", result.stderr)
        self.assertTrue(self.target.exists())

    def test_transport_parses_marker_and_quotes_arguments(self):
        response = subprocess.CompletedProcess([], 0, "ssh banner\nDISK_STATE_JSON={\"files\": 2}\n", "")
        with mock.patch.object(driver.subprocess, "run", return_value=response) as run:
            self.assertEqual(driver.remote_python("print('hello')", "/a path/'quote", "restore", 4), {"files": 2})
        args, kwargs = run.call_args
        self.assertEqual(args[0][:3], ["justrust", "remote", "ssh"])
        self.assertIn("python3 -c", args[0][3])
        self.assertTrue(kwargs["capture_output"])
        self.assertEqual(kwargs["timeout"], 180)

    def test_transport_failures_are_not_hidden(self):
        for response in (subprocess.CompletedProcess([], 1, "stdout failure", "stderr failure"),
                         subprocess.CompletedProcess([], 0, "no marker", "diagnostic")):
            with mock.patch.object(driver.subprocess, "run", return_value=response):
                with self.assertRaises(RuntimeError) as caught:
                    driver.remote_python("pass")
                self.assertIn(response.stdout, str(caught.exception))
                self.assertIn(response.stderr, str(caught.exception))


if __name__ == "__main__":
    unittest.main()
