#!/usr/bin/env python3
"""Offline regression checks for production-first client publication policy.

These checks do not release, activate, or call the production service. Full
published-client acceptance is a separate post-publication operation.
"""
from pathlib import Path
import re
import unittest

ROOT = Path(__file__).resolve().parents[1]


class ReleaseOrderTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.workflow = (ROOT / ".github/workflows/release.yml").read_text()
        cls.docs = (ROOT / "docs/toolchain-release-flow.md").read_text()

    def job(self, name):
        match = re.search(rf"^  {name}:\n(.*?)(?=^  [a-zA-Z_][\w-]*:|\Z)",
                          self.workflow, re.MULTILINE | re.DOTALL)
        self.assertIsNotNone(match, f"missing {name} job")
        return match.group(1)

    def test_no_successful_ci_publication_gate(self):
        before_release = self.workflow.split("  release:\n", 1)[0]
        for forbidden in ("Require successful CI", "Waiting for successful CI",
                          "actions/workflows/ci.yml/runs", "-f status=success"):
            self.assertNotIn(forbidden, before_release)
        self.assertNotRegex(before_release, r"\b(?:justrust|cargo)\s+(?:test|clippy)\b")

    def test_identity_and_package_sanity_still_precede_publication(self):
        self.assertIn("Verify release tag, stable ancestry, and version", self.job("preflight"))
        build = self.job("build")
        self.assertIn("check-linux-release.sh", build)
        self.assertIn("sha256sum", build)
        self.assertIn("shasum -a 256", build)
        self.assertIn("dist/*.sha256", build)
        release = self.job("release")
        self.assertIn("needs: [preflight, build]", release)
        self.assertIn("Release tag moved after preflight", release)
        self.assertIn("gh release create", release)

    def test_postpublication_workflow_uses_exact_published_client(self):
        caller = self.job("production-tests")
        self.assertIn("needs: [preflight, release]", caller)
        self.assertIn("uses: ./.github/workflows/client-production-tests.yml", caller)
        self.assertIn("tag: ${{ needs.preflight.outputs.tag }}", caller)
        self.assertIn("sha: ${{ needs.preflight.outputs.sha }}", caller)
        workflow = (ROOT / ".github/workflows/client-production-tests.yml").read_text()
        self.assertIn("workflow_call:", workflow)
        self.assertNotRegex(workflow, re.compile(r"^  (?:release|workflow_run):", re.MULTILINE))
        self.assertIn("ref: ${{ inputs.sha }}", workflow)
        self.assertIn('refs/tags/$RELEASE_TAG^{commit}', workflow)
        self.assertIn('dogfood.release_info(os.environ["RELEASE_TAG"], triple)', workflow)
        self.assertIn("dogfood.verified_binary(info, triple)", workflow)
        self.assertIn("dogfood.validate_release(binary", workflow)
        self.assertIn('production-dogfood.py --expect "$RELEASE_TAG" --activate', workflow)
        self.assertIn("justrust test --remote --locked", workflow)
        self.assertIn("dogfood.validate_run(receipt.read_text(), Path.cwd())", workflow)
        self.assertIn('JUSTRUST_REMOTE_REQUIRED: "1"', workflow)
        self.assertNotRegex(workflow, r"\bcargo\s+(?:install|build|test)\b")
        self.assertNotIn("JUSTRUST_TOOLCHAIN:", workflow)
        download = workflow.index("dogfood.verified_binary(info, triple)")
        activation = workflow.index('production-dogfood.py --expect "$RELEASE_TAG" --activate')
        suite = workflow.index("justrust test --remote --locked")
        self.assertLess(download, activation)
        self.assertLess(activation, suite)

    def test_embedded_workflow_python_compiles(self):
        workflow = (ROOT / ".github/workflows/client-production-tests.yml").read_text()
        blocks = re.findall(r"          python3 - <<'PY'\n(.*?)          PY", workflow, re.DOTALL)
        self.assertEqual(len(blocks), 2)
        import textwrap
        for block in blocks:
            compile(textwrap.dedent(block), "client-production-tests.yml", "exec")

    def test_download_step_waits_for_production_and_installs_verified_bytes(self):
        import importlib.util
        import os
        import tempfile
        import textwrap
        from types import SimpleNamespace
        from unittest import mock

        workflow = (ROOT / ".github/workflows/client-production-tests.yml").read_text()
        block = re.findall(r"          python3 - <<'PY'\n(.*?)          PY", workflow, re.DOTALL)[0]
        selected = {"tag": "v1.2.3"}
        dogfood = SimpleNamespace(
            target=mock.Mock(return_value="x86_64-unknown-linux-musl"),
            release_info=mock.Mock(side_effect=[ValueError("old selection"), selected]),
            verified_binary=mock.Mock(return_value=b"published bytes"),
            validate_release=mock.Mock(),
        )
        spec = SimpleNamespace(loader=SimpleNamespace(exec_module=lambda module: None))
        with tempfile.TemporaryDirectory() as directory:
            path_file = Path(directory) / "github-path"
            env = {"RUNNER_TEMP": directory, "GITHUB_PATH": str(path_file), "RELEASE_TAG": "v1.2.3"}
            with mock.patch.dict(os.environ, env), \
                    mock.patch.object(importlib.util, "spec_from_file_location", return_value=spec), \
                    mock.patch.object(importlib.util, "module_from_spec", return_value=dogfood), \
                    mock.patch("time.sleep") as sleep:
                exec(compile(textwrap.dedent(block), "download-step", "exec"), {})
            binary = Path(directory) / "published-client/justrust"
            self.assertEqual(binary.read_bytes(), b"published bytes")
            self.assertTrue(binary.stat().st_mode & 0o100)
            self.assertEqual(path_file.read_text().strip(), str(binary.parent))
            sleep.assert_called_once_with(15)
            dogfood.verified_binary.assert_called_once_with(selected, "x86_64-unknown-linux-musl")
            dogfood.validate_release.assert_called_once_with(binary, "1.2.3")

    def test_documented_shared_production_first_order(self):
        flattened = " ".join(self.docs.split())
        self.assertIn("bump -> commit -> build/package sanity -> tag/publish -> shared production -> test/measure -> keep or rollback", flattened)
        self.assertIn("Every customer uses the same shared production default", flattened)
        self.assertIn("justrust test --remote --locked", self.docs)
        self.assertNotIn("Do not tag an\nunvalidated candidate", self.docs)
        self.assertNotIn("directly gated by its validation evidence", self.docs)


if __name__ == "__main__":
    unittest.main()
