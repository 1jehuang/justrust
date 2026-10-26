"""Focused benchmark contract tests, without compiling Jcode."""
import importlib.util
import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).parent))
spec = importlib.util.spec_from_file_location("jcode_docs", Path(__file__).with_name("jcode-docs.py"))
driver = importlib.util.module_from_spec(spec)
spec.loader.exec_module(driver)


class DocsBenchmarkTests(unittest.TestCase):
    def test_patch_allowlist_and_exact_application(self):
        patch_bytes = b"diff --git a/src.rs b/src.rs\n--- a/src.rs\n+++ b/src.rs\n@@ -1 +1 @@\n-old\n+new\n"
        for allowed, succeeds in [(["src.rs"], True), (["other.rs"], False)]:
            with self.subTest(allowed=allowed), tempfile.TemporaryDirectory() as d:
                root = Path(d)
                (root / "src.rs").write_text("old\n")
                (root / ".gitignore").write_text("src.rs\n")
                if succeeds:
                    self.assertEqual(driver.apply_candidate(root, patch_bytes, allowed), ["src.rs"])
                    self.assertEqual((root / "src.rs").read_text(), "new\n")
                else:
                    with self.assertRaises(ValueError):
                        driver.apply_candidate(root, patch_bytes, allowed)

    def test_readme_cannot_be_allowlisted(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            (root / "README.md").write_text("old\n")
            data = b"diff --git a/README.md b/README.md\n--- a/README.md\n+++ b/README.md\n@@ -1 +1 @@\n-old\n+new\n"
            with self.assertRaisesRegex(ValueError, "README"):
                driver.apply_candidate(root, data, ["README.md"])

    def test_pair_guards(self):
        info = {k: "same" for k in ("execution", "toolchain", "rustc", "cargo", "justrust_binary_sha256", "optimization_env")}
        baseline = {"valid": True, "candidate": {"kind": "baseline"}, "env": info, "workload": {"pin": driver.PIN}}
        driver.validate_pair(baseline, info, baseline["workload"])
        for key in info:
            with self.subTest(key=key), self.assertRaises(ValueError):
                driver.validate_pair(baseline, {**info, key: "different"}, baseline["workload"])
        with self.assertRaises(ValueError):
            driver.validate_pair(baseline, info, {"pin": "other"})

    def test_baseline_uses_suite_and_retains_provenance(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            out = root / "baseline.json"
            info = {"justrust_binary_sha256": "digest"}
            captured = []
            def run(scenario, n, warmup, quiet):
                captured.append((scenario, n, warmup))
                return {"samples": [{"tests_passed": 9, "tests_failed": 0}], "stats": {}}
            with patch.dict(driver.os.environ, {"JUSTRUST_BENCH_WORK": str(root / "staging")}), \
                 patch.object(driver, "git", return_value=(driver.PIN + "\n").encode()), \
                 patch.object(driver.suite, "WORK", root / "work"), \
                 patch.object(driver.suite, "prepare", return_value=root), \
                 patch.object(driver.suite, "env_info", return_value=info), \
                 patch.object(driver.suite, "run_scenario", side_effect=run), \
                 patch.object(driver.suite, "verify_binary_identity"), \
                 patch.object(driver.suite, "table", return_value="table"):
                driver.main(["--out", str(out)])
                self.assertEqual(driver.suite.WORK.parent, root / "staging")
            data = json.loads(out.read_text())
            self.assertEqual(data["candidate"]["baseline_commit"], driver.PIN)
            self.assertEqual(data["candidate"]["kind"], "baseline")
            self.assertEqual(data["env"]["execution"], "remote")
            scenario, n, warmup = captured[0]
            self.assertEqual((n, warmup), (3, 1))
            self.assertEqual(scenario.cmd, driver.CMD)
            self.assertIn("{n}", scenario.edit.template)
            self.assertEqual(scenario.edit.file, "jcode/README.md")
            with self.assertRaises(SystemExit):
                driver.main(["--out", str(out)])

    def test_source_commit_diff_not_working_tree(self):
        from argparse import Namespace
        with patch.object(driver, "git", side_effect=[b"fullcommit\n", b"exact patch"]) as git:
            data, provenance = driver.candidate_patch(Namespace(patch=None, source_commit="revision", repo=Path("repo")))
        self.assertEqual(data, b"exact patch")
        self.assertEqual(provenance["commit"], "fullcommit")
        self.assertEqual(git.call_args.args[1:], ("diff", "--binary", driver.PIN, "fullcommit"))


if __name__ == "__main__":
    unittest.main()
