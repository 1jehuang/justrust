import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


class ProfileEvidenceTests(unittest.TestCase):
    def test_cli_retains_noise_hashes_and_marks_known_bad_legacy_label(self):
        script = Path(__file__).parent / "collect-profile-evidence.py"
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "t1").mkdir()
            profile = b"retained raw profile"
            (root / "t1" / "unit.mm_profdata").write_bytes(profile)
            (root / "prof-wrapper.sh").write_text("#!/bin/sh\n")
            capture = {"scenario": "desktop-test-edit", "toolchain": "jr-fixture",
                       "profile_events": "generic-activity,query-blocked",
                       "env": {"benchmark_repo_rev": None, "justrust_binary_revision": "source-head",
                               "justrust_binary_sha256": "actual-binary-hash"},
                       "runs": [{"frontend_threads": 1, "profiled": True,
                                 "result": {"id": "run-1", "noisy": True, "wall": 2}}]}
            raw = json.dumps(capture).encode()
            (root / "capture.json").write_bytes(raw)
            output = root / "provenance.json"
            command = [sys.executable, str(script), str(root), "--out", str(output)]
            subprocess.run(command, check=True)
            plain = json.loads(output.read_text())["captures"][0]
            self.assertEqual(plain["env"]["justrust_binary_revision"], "source-head")
            subprocess.run([*command, "--legacy-launcher-source-label"], check=True)
            actual = json.loads(output.read_text())["captures"][0]
            self.assertIsNone(actual["env"]["justrust_binary_revision"])
            self.assertEqual(actual["env"]["unverified_reported_binary_revision"], "source-head")
            self.assertEqual(actual["env"]["benchmark_repo_rev"], "source-head")
            self.assertEqual(actual["env"]["justrust_binary_sha256"], "actual-binary-hash")
            self.assertEqual(actual["capture_sha256"], hashlib.sha256(raw).hexdigest())
            self.assertEqual(actual["profiles"][0]["sha256"], hashlib.sha256(profile).hexdigest())
            self.assertEqual(actual["profiles"][0]["bytes"], len(profile))
            self.assertTrue(actual["runs"][0]["result"]["noisy"])
            self.assertEqual((root / "capture.json").read_bytes(), raw)
            self.assertNotIn("speedup", actual)


if __name__ == "__main__":
    unittest.main()
