"""Offline checks for the manual retirement workflow, never dispatch or use secrets."""
import contextlib
import hashlib
import io
import os
from pathlib import Path
import subprocess
import tempfile
import textwrap
import unittest
from unittest import mock


WORKFLOW = Path(__file__).resolve().parents[1] / ".github/workflows/native-retirement-validation.yml"
BODY = b"offline candidate fixture, never executed"
DIGEST = hashlib.sha256(BODY).hexdigest()
HOST = "jcode-justrust-prebuilt-production-280196252242.s3.amazonaws.com"
OFFICIAL = "https://" + HOST + "/v1/native/"


def download_code():
    workflow = WORKFLOW.read_text()
    return textwrap.dedent(workflow.split("python3 - <<'PY'\n", 1)[1].split("          PY\n", 1)[0])


class RetirementWorkflowTests(unittest.TestCase):
    def run_download(self, url, body=BODY, digest=DIGEST):
        with tempfile.TemporaryDirectory() as directory:
            opener = mock.Mock()
            opener.open.return_value = io.BytesIO(body)
            with mock.patch.dict(os.environ, {"CANDIDATE_URL": url,
                    "CANDIDATE_SHA256": digest, "RUNNER_TEMP": directory}), \
                    mock.patch("urllib.request.build_opener", return_value=opener) as build, \
                    contextlib.redirect_stdout(io.StringIO()):
                try:
                    exec(compile(download_code(), str(WORKFLOW), "exec"), {})
                except SystemExit:
                    self.assertFalse((Path(directory) / "native-retirement-candidate").exists())
                    raise
                target = Path(directory) / "native-retirement-candidate"
                self.assertEqual(target.read_bytes(), body)
                self.assertEqual(target.stat().st_mode & 0o777, 0o700)
                handler = build.call_args.args[0]
                with self.assertRaises(RuntimeError):
                    handler.redirect_request(None, None, None, None, None, None)
            return opener

    def test_official_hash_bound_url_downloads_without_authentication(self):
        url = OFFICIAL + DIGEST + "/justrust"
        opener = self.run_download(url)
        opener.open.assert_called_once_with(url, timeout=60)

    def test_arbitrary_hosts_paths_and_redirect_like_urls_are_rejected_before_network(self):
        valid = OFFICIAL + DIGEST + "/justrust"
        invalid = [
            valid.replace("https:", "http:"), valid.replace(HOST, "evil.invalid"),
            valid.replace(HOST, HOST + ".evil.invalid"),
            valid.replace(HOST, "user@" + HOST),
            valid.replace(HOST, HOST + ":443"),
            valid.replace(DIGEST, "0" * 64), valid + "?x=1", valid + "#x",
            valid.replace("/justrust", "/../justrust"),
            valid.replace("/justrust", "/%2e%2e/justrust"),
            valid.replace("/justrust", "/other-linux"),
        ]
        for url in invalid:
            with self.subTest(url=url), mock.patch("urllib.request.build_opener") as build:
                with self.assertRaises(SystemExit):
                    self.run_download(url)
                build.assert_not_called()

    def test_empty_or_mismatched_download_is_removed(self):
        url = OFFICIAL + DIGEST + "/justrust"
        for body in (b"", b"changed bytes"):
            with self.subTest(body=body), self.assertRaises(SystemExit):
                self.run_download(url, body)

    def test_download_wall_deadline_removes_partial_file(self):
        url = OFFICIAL + DIGEST + "/justrust"
        with mock.patch("time.monotonic", side_effect=[0, 121]):
            with self.assertRaises(SystemExit):
                self.run_download(url)

    def test_download_error_removes_partial_file(self):
        url = OFFICIAL + DIGEST + "/justrust"
        with tempfile.TemporaryDirectory() as directory:
            response = mock.MagicMock()
            response.__enter__.return_value = response
            response.read.side_effect = [BODY, TimeoutError("offline timeout")]
            opener = mock.Mock()
            opener.open.return_value = response
            with mock.patch.dict(os.environ, {"CANDIDATE_URL": url,
                    "CANDIDATE_SHA256": DIGEST, "RUNNER_TEMP": directory}), \
                    mock.patch("urllib.request.build_opener", return_value=opener):
                with self.assertRaises(SystemExit):
                    exec(compile(download_code(), str(WORKFLOW), "exec"), {})
            self.assertFalse((Path(directory) / "native-retirement-candidate").exists())

    def test_input_validation_requires_fence_and_immutable_hex_values(self):
        workflow = WORKFLOW.read_text()
        first = workflow.split('      - name: Validate immutable inputs', 1)[1]
        script = textwrap.dedent(first.split('        run: |\n', 1)[1].split('\n      - uses:', 1)[0])
        valid = dict(os.environ, EXCLUSIVE_CANARY_FENCE="true", SOURCE_REF="a" * 40,
                     CANDIDATE_SHA256=DIGEST, EXPECTED_LEASE="b" * 64)
        result = subprocess.run(["bash", "-c", script], env=valid, capture_output=True)
        self.assertEqual(result.returncode, 0)
        for key, value in (("EXCLUSIVE_CANARY_FENCE", "false"), ("SOURCE_REF", "stable"),
                           ("SOURCE_REF", "a" * 39), ("CANDIDATE_SHA256", "A" * 64),
                           ("EXPECTED_LEASE", ""), ("SOURCE_REF", "$(exit 0)")):
            with self.subTest(key=key, value=value):
                result = subprocess.run(["bash", "-c", script], env=dict(valid, **{key: value}),
                                        capture_output=True)
                self.assertNotEqual(result.returncode, 0)

    def test_manual_trusted_source_and_secret_scope_contract(self):
        workflow = WORKFLOW.read_text()
        self.assertIn("  workflow_dispatch:\n", workflow)
        self.assertNotIn("  push:", workflow)
        self.assertNotIn("  pull_request:", workflow)
        self.assertIn("if: github.ref == 'refs/heads/stable'", workflow)
        self.assertIn('[[ "$EXCLUSIVE_CANARY_FENCE" == true ]]', workflow)
        self.assertIn('git merge-base --is-ancestor "$SOURCE_REF" refs/remotes/origin/stable', workflow)
        self.assertIn("persist-credentials: false", workflow)
        self.assertIn("BUILD_BASE: https://build.jcode.sh/v1", workflow)
        self.assertEqual(workflow.count("secrets."), 1)
        self.assertIn("JCODE_API_KEY: ${{ secrets.JCODE_API_KEY }}", workflow)
        secret_step = workflow.index("      - name: Exact-lease retirement")
        self.assertGreater(workflow.index("secrets.JCODE_API_KEY"), secret_step)
        self.assertIn('python3 scripts/native-only-acceptance.py', workflow)
        self.assertIn('--expected-lease "$EXPECTED_LEASE"', workflow)
        self.assertIn('--timeout 60', workflow)
        self.assertNotIn("continue-on-error:", workflow)


if __name__ == "__main__":
    unittest.main()
