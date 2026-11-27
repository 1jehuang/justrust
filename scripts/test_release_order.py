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
        self.assertIn("Verify all platforms completed publication", release)
        self.assertIn("Publish checked platform independently", build)
        self.assertIn("ci/publish-platform-release.py", build)
        self.assertIn("gh release create", self.job("preflight"))
        self.assertNotIn("gh release upload", release)

    def test_postpublication_workflow_uses_exact_published_client(self):
        caller = self.job("production-tests")
        # Acceptance waits for the Linux builds only, never for macOS or Windows.
        self.assertIn("needs: [preflight, linux-published]", caller)
        self.assertIn("needs: preflight", self.job("linux-published"))
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

    def test_published_bootstrap_replaces_cold_source_install(self):
        build = self.job("build")
        # The bootstrap is the published production client, frozen once in
        # preflight. A tag or checksum written into the workflow would fall
        # behind the shipping client again.
        self.assertNotRegex(self.workflow, r"HOSTED_CLIENT_(?:TAG|SHA256|REV)")
        self.assertNotRegex(self.workflow, r"\bv\d+\.\d+\.\d+\b")
        preflight = self.job("preflight")
        self.assertIn("ci/install-published-client.py --resolve-production", preflight)
        self.assertIn('git merge-base --is-ancestor "refs/tags/$bootstrap_tag^{commit}" refs/remotes/origin/stable', preflight)
        self.assertIn("bootstrap_tag: ${{ steps.bootstrap.outputs.bootstrap_tag }}", preflight)
        self.assertIn("bootstrap_sha256: ${{ steps.bootstrap.outputs.bootstrap_sha256 }}", preflight)
        self.assertIn("BOOTSTRAP_TAG: ${{ needs.preflight.outputs.bootstrap_tag }}", build)
        self.assertIn("BOOTSTRAP_SHA256: ${{ needs.preflight.outputs.bootstrap_sha256 }}", build)
        self.assertIn('ci/install-published-client.py --tag "$BOOTSTRAP_TAG" --sha256 "$BOOTSTRAP_SHA256" --github-path', build)
        self.assertNotIn("--resolve-production", build)
        self.assertNotIn("install-hosted-client.sh", build)
        self.assertIn("Cache compatibility release dependencies", build)
        self.assertIn("key: client-release-${{ matrix.target }}", build)
        self.assertLess(build.index("Verify native release executable"), build.index("- name: Package"))
        self.assertIn('"$binary" update status', build)
        self.assertIn("Freeze service-selected release compiler once", self.job("preflight"))

    def test_platform_publication_does_not_wait_for_matrix_completion(self):
        build = self.job("build")
        self.assertIn("needs: preflight", build)
        self.assertIn("fail-fast: false", build)
        self.assertLess(build.index("- name: Package"), build.index("Publish checked platform independently"))
        self.assertIn('--target "${{ matrix.target }}" --dist "$publish_dir" --repo "$GITHUB_REPOSITORY"', build)
        preflight = self.job("preflight")
        self.assertIn('gh release create "$TAG" --draft --verify-tag --target "$SHA"', preflight)
        self.assertNotIn("--clobber", self.workflow)
        self.assertIn("justrust-hosted-linux", build)
        summary = self.job("release")
        self.assertNotIn("gh release edit", summary)
        self.assertNotIn("gh release create", summary)
        self.assertNotIn("gh release upload", summary)
        self.assertIn("ci/promote-complete-release.py", summary)
        promoter = (ROOT / "ci/promote-complete-release.py").read_text()
        self.assertIn('archive + ".sha256"', promoter)
        self.assertIn("missing compiler provenance", promoter)
        publisher = (ROOT / "ci/publish-platform-release.py").read_text()
        self.assertIn('"--latest=false"', publisher)

    def test_platform_build_inputs_do_not_share_test_mirror_paths(self):
        build = self.job("build")
        workspace = "${{ matrix.zig && format('{0}/client-release-v2/{1}', runner.temp, matrix.target) || format('{0}/release-client/{1}', github.workspace, matrix.target) }}"
        self.assertIn("working-directory: " + workspace, build)
        self.assertEqual(build.count("working-directory: ${{ runner.temp }}/client-release-v2/${{ matrix.target }}"), 2)
        self.assertIn(workspace + "/dist/*.tar.gz", build)
        self.assertIn(workspace + "/dist/*.sha256", build)
        self.assertIn("path: release-client/${{ matrix.target }}", build)
        self.assertIn("workspaces: release-client/${{ matrix.target }} -> target", build)
        # Preserve hosted Linux account fencing while separating mirrored paths.
        self.assertIn("justrust-hosted-linux", build)
        self.assertIn("cancel-in-progress: false", build)

    def test_hosted_source_copy_uses_non_overlapping_mirror_root(self):
        import os
        import subprocess
        import tempfile
        import textwrap
        build = self.job("build")
        step = build.split("- name: Isolate hosted release source from test mirror roots", 1)[1].split("      - uses:", 1)[0]
        script = textwrap.dedent(step.split("        run: |\n", 1)[1]).replace("${{ matrix.target }}", "x86_64-unknown-linux-musl")
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            mirror = base / "checkout"
            source = mirror / "release-client/x86_64-unknown-linux-musl"
            source.mkdir(parents=True)
            (source / ".git").mkdir()
            (source / ".git/config").write_text("copied repository metadata")
            (source / "Cargo.toml").write_text("committed client")
            runner_temp = base / "runner-temp"
            runner_temp.mkdir()
            subprocess.run(["bash", "-e", "-c", script], cwd=source,
                           env=dict(os.environ, RUNNER_TEMP=str(runner_temp)), check=True)
            isolated = runner_temp / "client-release-v2/x86_64-unknown-linux-musl"
            self.assertFalse(isolated.is_relative_to(mirror))
            self.assertEqual((isolated / "Cargo.toml").read_text(), "committed client")
            self.assertEqual((isolated / ".git/config").read_text(), "copied repository metadata")

    def test_documented_shared_production_first_order(self):
        flattened = " ".join(self.docs.split())
        self.assertIn("bump -> commit -> build/package sanity -> tag/publish -> shared production -> test/measure -> fix forward", flattened)
        self.assertIn("Every customer uses the same shared production default", flattened)
        self.assertIn("justrust test --remote --locked", self.docs)
        self.assertNotIn("Do not tag an\nunvalidated candidate", self.docs)
        self.assertNotIn("directly gated by its validation evidence", self.docs)


if __name__ == "__main__":
    unittest.main()
