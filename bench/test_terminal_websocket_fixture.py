"""Version and constrain the mixed HTTP/WS pinned terminal fixture correction."""
import hashlib
from pathlib import Path
import re
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import suite

PATCH = "bench/patches/jcode-terminal-websocket-catalog-v1.patch"
SOURCE = "crates/jcode-provider-openai-runtime/src/openai_tests/persistent_terminal.rs"


class TerminalWebsocketFixture(unittest.TestCase):
    def test_identity_covers_patch_and_exact_pinned_input(self):
        spec = suite.SNAPSHOTS["desktop"]
        self.assertIn(("jcode", PATCH), spec["patches"])
        identity = suite.snapshot_fingerprint("desktop")
        for field, value in (("patches", [p for p in spec["patches"] if p[1] != PATCH]),
                             ("fixture_input_sha256", {})):
            with patch.dict(suite.SNAPSHOTS, {"desktop": dict(spec, **{field: value})}):
                self.assertNotEqual(identity, suite.snapshot_fingerprint("desktop"))
        text = (suite.ROOT / PATCH).read_text()
        self.assertEqual(hashlib.sha256(text.encode()).hexdigest(), spec["fixture_patch_sha256"][PATCH])
        self.assertEqual([line[6:] for line in text.splitlines() if line.startswith("+++ b/")], [SOURCE])
        self.assertNotIn("RUST_TEST_THREADS", text)
        self.assertNotIn("#[ignore", text)

    def test_corrupt_overlay_and_unexpected_input_fail_closed(self):
        spec = suite.SNAPSHOTS["desktop"]
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "bad.patch"
            path.write_text("wrong bytes")
            with self.assertRaisesRegex(ValueError, "corrupt or unversioned fixture patch"):
                suite.verify_fixture_patch(path, spec["fixture_patch_sha256"][PATCH])
            with self.assertRaisesRegex(ValueError, "unexpected pinned fixture input"):
                suite.verify_fixture_input(path, spec["fixture_input_sha256"]["jcode/" + SOURCE])

    def test_original_semantics_and_all_tests_remain(self):
        repo, revision = suite.SNAPSHOTS["desktop"]["repos"]["jcode"]
        original = subprocess.check_output(["git", "-C", str(repo), "show", f"{revision}:{SOURCE}"], text=True)
        self.assertEqual(hashlib.sha256(original.encode()).hexdigest(),
                         suite.SNAPSHOTS["desktop"]["fixture_input_sha256"]["jcode/" + SOURCE])
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            destination = root / SOURCE
            destination.parent.mkdir(parents=True)
            destination.write_text(original)
            subprocess.run(["git", "apply", "--check", str(suite.ROOT / PATCH)], cwd=root, check=True)
            subprocess.run(["git", "apply", str(suite.ROOT / PATCH)], cwd=root, check=True)
            corrected = destination.read_text()
        # The original file is byte-for-byte unchanged except the two accepts
        # in the public terminal fixture. Assertions, timeouts, inputs, tests,
        # failure frames, continuation state and concurrency tests all survive.
        replacement = "        let mut ws = terminal_fixture_accept_response(&listener).await;"
        original_accept = ("        let (tcp, _) = listener.accept().await.unwrap();\n"
                           "        let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();")
        tail = corrected[corrected.index("// Public Provider::complete"):]
        self.assertEqual(tail.replace(replacement, original_accept), original)
        tests = lambda text: set(re.findall(r"#\[tokio::test\]\s*async fn (\w+)", text))
        self.assertEqual(tests(corrected) - tests(original),
                         {"persistent_terminal_fixture_serves_catalog_before_response_upgrade"})
        self.assertTrue(tests(original) <= tests(corrected))
        self.assertIn('starts_with(b"GET /v1/models HTTP/1.1\\r\\n")', corrected)
        self.assertIn('starts_with(b"GET /v1/responses HTTP/1.1\\r\\n")', corrected)
        # No broad accept-error loop that could swallow an HTTP fallback POST.
        self.assertNotIn("if let Ok", corrected[:corrected.index("// Public Provider::complete")])
        self.assertIn("server.await.unwrap()", corrected[:corrected.index("// Public Provider::complete")])


if __name__ == "__main__":
    unittest.main()
