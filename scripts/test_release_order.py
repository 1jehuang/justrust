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

    def test_documented_shared_production_first_order(self):
        flattened = " ".join(self.docs.split())
        self.assertIn("bump -> commit -> build/package sanity -> tag/publish -> shared production -> test/measure -> keep or rollback", flattened)
        self.assertIn("Every customer uses the same shared production default", flattened)
        self.assertIn("justrust test --remote --locked", self.docs)
        self.assertNotIn("Do not tag an\nunvalidated candidate", self.docs)
        self.assertNotIn("directly gated by its validation evidence", self.docs)


if __name__ == "__main__":
    unittest.main()
