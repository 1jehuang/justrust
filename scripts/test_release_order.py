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

    def test_published_bootstrap_replaces_cold_source_install(self):
        build = self.job("build")
        self.assertIn("HOSTED_CLIENT_TAG: v0.1.0-alpha.28", self.workflow)
        self.assertIn("c08dcefdb2bfcfbc69676a1d077526501e07907ca43152f9193916d5c4989581", self.workflow)
        self.assertIn('ci/install-published-client.py --tag "$HOSTED_CLIENT_TAG" --sha256 "$HOSTED_CLIENT_SHA256" --github-path', build)
        self.assertNotIn("install-hosted-client.sh", build)
        self.assertIn("Cache compatibility release dependencies", build)
        self.assertIn("key: client-release-${{ matrix.target }}", build)
        self.assertLess(build.index("Verify native release executable"), build.index("- name: Package"))
        self.assertIn('"$binary" update status', build)
        self.assertIn("Freeze service-selected release compiler once", self.job("preflight"))

    def test_publication_never_overwrites_public_assets(self):
        import json
        import os
        import subprocess
        import tempfile
        import textwrap

        release = self.job("release")
        script = textwrap.dedent(release.split("        run: |\n", 1)[1])
        fake_gh = '''#!/usr/bin/env python3
import json, os, pathlib, sys
args = sys.argv[1:]
log = pathlib.Path(os.environ["MOCK_LOG"])
with log.open("a") as out:
    out.write(json.dumps(args) + "\\n")
mode = os.environ["MOCK_MODE"]
if args[0] == "api":
    calls = [line for line in log.read_text().splitlines() if json.loads(line)[0] == "api"]
    print("moved" if mode == "moved" and len(calls) > 1 else "source-sha")
elif args[:2] == ["release", "view"]:
    if mode in ("new", "moved"):
        sys.exit(1)
    if "--json" in args:
        print("false" if mode == "public" else "true")
'''
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "dist").mkdir()
            (root / "dist/client.tar.gz").write_bytes(b"artifact")
            gh = root / "gh"
            gh.write_text(fake_gh)
            gh.chmod(0o755)
            for mode in ("new", "draft", "public", "moved"):
                with self.subTest(mode=mode):
                    log = root / f"{mode}.jsonl"
                    env = dict(os.environ, PATH=str(root) + os.pathsep + os.environ["PATH"],
                               MOCK_LOG=str(log), MOCK_MODE=mode, TAG="v1.2.3",
                               SHA="source-sha", GITHUB_REPOSITORY="owner/repo")
                    result = subprocess.run(["bash", "-c", script], cwd=root, env=env,
                                            text=True, capture_output=True)
                    calls = [json.loads(line) for line in log.read_text().splitlines()]
                    mutations = [args for args in calls if args[0] == "release" and args[1] != "view"]
                    if mode in ("new", "draft"):
                        self.assertEqual(result.returncode, 0, result.stderr)
                        self.assertEqual(mutations[-1][:2], ["release", "edit"])
                        self.assertIn("--draft=false", mutations[-1])
                        self.assertEqual(mutations[0][1], "create" if mode == "new" else "upload")
                        if mode == "new":
                            self.assertIn("--draft", mutations[0])
                    elif mode == "public":
                        self.assertNotEqual(result.returncode, 0)
                        self.assertEqual(mutations, [])
                        self.assertIn("immutable", result.stderr)
                    else:
                        self.assertNotEqual(result.returncode, 0)
                        self.assertFalse(any(args[1] == "edit" for args in mutations))
                        self.assertIn("moved during upload", result.stderr)

    def test_documented_shared_production_first_order(self):
        flattened = " ".join(self.docs.split())
        self.assertIn("bump -> commit -> build/package sanity -> tag/publish -> shared production -> test/measure -> fix forward", flattened)
        self.assertIn("Every customer uses the same shared production default", flattened)
        self.assertIn("justrust test --remote --locked", self.docs)
        self.assertNotIn("Do not tag an\nunvalidated candidate", self.docs)
        self.assertNotIn("directly gated by its validation evidence", self.docs)


if __name__ == "__main__":
    unittest.main()
