"""Probe mechanics tests, not evidence that native Rust compilation works."""
import contextlib
import importlib.util
import io
import json
from pathlib import Path
import shutil
import sys
import tempfile
import unittest
from unittest import mock

SPEC = importlib.util.spec_from_file_location(
    "adler2_repo_probe", Path(__file__).with_name("adler2_repo_probe.py"))
probe = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(probe)
ROOT = Path(__file__).resolve().parents[1]


class ProbeTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.runner = probe.Probe(self.root, 5)

    def repository(self):
        repo = self.root / "repo"
        (repo / "src").mkdir(parents=True)
        (repo / "Cargo.toml").write_text(
            '[package]\nname="adler2"\nversion="2.0.1"\nedition="2021"\n')
        for name in ("lib.rs", "algo.rs"):
            shutil.copyfile(ROOT / "fixtures/adler2_adapted/upstream" / name,
                            repo / "src" / name)
        return repo

    def test_source_pins_and_extra_files(self):
        repo = self.repository()
        self.assertEqual(probe.validate_repo(repo), probe.PINS)
        (repo / "src/extra.rs").write_text("")
        with self.assertRaisesRegex(ValueError, "unexpected"):
            probe.validate_repo(repo)

    def test_source_modification_rejected(self):
        repo = self.repository()
        with (repo / "src/lib.rs").open("a") as handle:
            handle.write("\n// modified\n")
        with self.assertRaisesRegex(ValueError, "pin mismatch"):
            probe.validate_repo(repo)

    def test_wrong_manifest_rejected(self):
        repo = self.repository()
        (repo / "Cargo.toml").write_text('[package]\nname="different"\n')
        with self.assertRaisesRegex(ValueError, "expected Adler2"):
            probe.validate_repo(repo)

    def test_exit_zero_without_artifact_fails(self):
        self.assertFalse(self.runner.run("missing", [sys.executable, "-c", "pass"],
                                         self.root / "not-created"))
        self.assertEqual(self.runner.steps[-1]["status"], "missing_artifact")

    def test_wrong_output_fails(self):
        self.assertFalse(self.runner.run("wrong", [sys.executable, "-c", "print('wrong')"],
                                         expected_stdout=b"right\n"))
        self.assertEqual(self.runner.steps[-1]["status"], "wrong_output")

    def test_zero_test_success_does_not_pass_upstream_tests(self):
        self.assertFalse(self.runner.run("zero-tests", [sys.executable, "-c",
            "print('test result: ok. 0 passed; 0 failed; 0 ignored;')"],
            expected_test_count=6))
        self.assertEqual(self.runner.steps[-1]["status"], "wrong_output")

    def test_nonzero_and_full_diagnostics_retained(self):
        self.assertFalse(self.runner.run("reject", [sys.executable, "-c",
            "import sys; print('full diagnostic', file=sys.stderr); sys.exit(7)"]))
        self.assertEqual(self.runner.steps[-1]["exit_code"], 7)
        self.assertEqual((self.root / "reject.stderr").read_text(), "full diagnostic\n")

    def test_timeout_is_not_language_rejection(self):
        self.runner.timeout = 0.01
        self.assertFalse(self.runner.run("timeout", [sys.executable, "-c",
                                                                   "import time; time.sleep(5)"]))
        self.assertEqual(self.runner.steps[-1]["status"], "timeout")

    def test_missing_executable_is_infrastructure(self):
        self.assertFalse(self.runner.run("absent", [self.root / "absent"]))
        self.assertEqual(self.runner.steps[-1]["status"], "infrastructure_error")

    def test_matrix_does_not_count_source_check_as_repository_success(self):
        class RecordingProbe:
            output = self.root

            def run(self, name, command, *args, **kwargs):
                calls.append((name, command))
                return name.startswith("stock-") or name.endswith("source-check")

            def skip(self, name, reason):
                skipped.append(name)

        calls, skipped = [], []
        stock, native = probe.exercise(RecordingProbe(), Path("upstream"), "native", "rustc")
        self.assertTrue(stock)
        self.assertFalse(native)
        self.assertEqual(len(skipped), 8)
        self.assertEqual(len([name for name, _ in calls if name.endswith("source-check")]), 4)
        self.assertFalse(any(name.startswith("native-") and name.endswith("consumer-run")
                             for name, _ in calls))
        for name, command in calls:
            if name.endswith("source-check"):
                self.assertIn(Path("upstream/src/lib.rs"), command)
                self.assertIn("--edition=2021", command)
            elif name.startswith("native-"):
                self.assertIn("--manifest-path", command)

    def test_existing_output_never_overwritten(self):
        repo = self.repository()
        sentinel = self.root / "keep.txt"
        sentinel.write_text("keep")
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as error:
            probe.main(["--repo", str(repo), "--native", sys.executable,
                        "--rustc", sys.executable, "--output-dir", str(self.root)])
        self.assertEqual(error.exception.code, 2)
        self.assertEqual(sentinel.read_text(), "keep")

    def test_missing_dependency_is_not_language_rejection(self):
        self.assertFalse(self.runner.run("dependency", [sys.executable, "-c",
            "import sys; print('package is not cached under registry', file=sys.stderr); sys.exit(1)"]))
        self.assertEqual(self.runner.steps[-1]["status"], "dependency_unavailable")

    def test_environment_blocker_has_distinct_report(self):
        repo = self.repository()
        output = self.root / "environment"

        def blocked(runner, *args):
            runner.steps.append(dict(name="native-project", status="dependency_unavailable"))
            return True, False

        with mock.patch.object(probe, "exercise", side_effect=blocked), \
                mock.patch.object(probe.Probe, "run", return_value=True):
            result = probe.main(["--repo", str(repo), "--native", sys.executable,
                                 "--rustc", sys.executable, "--output-dir", str(output)])
        self.assertEqual(result, 2)
        report = json.loads((output / "report.json").read_text())
        self.assertEqual(report["status"], "environment_blocked")
        self.assertEqual(len(report["environment_blockers"]), 1)
        self.assertEqual(report["compatibility_blockers"], [])
        self.assertEqual(probe.validate_repo(output / "upstream"), probe.PINS)
        self.assertEqual(report["native_sha256"], probe.sha(output / "jr-rust"))

    def test_summary_exit_codes_distinguish_blocker_and_bad_oracle(self):
        repo = self.repository()
        for index, (stock, native, code, status) in enumerate([
                (True, True, 0, "passed"),
                (True, False, 1, "compatibility_blocked"),
                (False, False, 2, "infrastructure_error")]):
            output = self.root / str(index)
            with mock.patch.object(probe, "exercise", return_value=(stock, native)), \
                    mock.patch.object(probe.Probe, "run", return_value=True):
                actual = probe.main(["--repo", str(repo), "--native", sys.executable,
                                     "--rustc", sys.executable, "--output-dir", str(output)])
            self.assertEqual(actual, code)
            self.assertEqual(json.loads((output / "report.json").read_text())["status"], status)


if __name__ == "__main__":
    unittest.main()
