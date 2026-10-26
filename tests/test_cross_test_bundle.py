"""Run with python3 -m unittest discover -s tests -p test_cross_test_bundle.py."""
import contextlib
import importlib.util
import io
import json
from pathlib import Path
import tempfile
import types
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location(
    "cross_test_bundle", Path(__file__).resolve().parents[1] / "scripts/cross-test-bundle.py"
)
helper = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(helper)
TARGET = "x86_64-unknown-linux-gnu"


class BundleTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.artifacts = self.root / "artifacts"
        self.artifacts.mkdir()
        self.binary = self.artifacts / "original"
        self.binary.write_bytes(b"fake binary")
        self.messages = self.root / "messages.jsonl"
        self.out = self.root / "bundle"
        self.results = self.root / "results.json"
        self.events = [self.artifact(self.binary), {"reason": "build-finished", "success": True}]

    def artifact(self, path):
        return {"reason": "compiler-artifact", "profile": {"test": True}, "executable": str(path)}

    def pack(self):
        self.messages.write_text("".join(json.dumps(e) + "\n" for e in self.events))
        with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            return helper.main(["bundle", "--messages", str(self.messages), "--artifact-root", str(self.artifacts), "--target", TARGET, "--out", str(self.out)])

    def run_bundle(self, *extra):
        with patch.object(helper.sys, "platform", "linux"), patch.object(helper.platform, "machine", return_value="x86_64"), contextlib.redirect_stderr(io.StringIO()):
            return helper.main(["run", "--bundle", str(self.out), "--results", str(self.results), *extra])

    def manifest(self):
        return json.loads((self.out / "manifest.json").read_text())

    def change_manifest(self, edit):
        data = self.manifest()
        edit(data)
        (self.out / "manifest.json").write_text(json.dumps(data))

    def test_bundle_never_executes_and_marks_not_tested(self):
        with patch.object(helper.subprocess, "run") as execute:
            self.assertEqual(self.pack(), 0)
            execute.assert_not_called()
        data = self.manifest()
        self.assertEqual(data["test_status"], "not-tested")
        self.assertEqual(data["tests"][0]["sha256"], helper.digest(self.binary))
        self.assertNotIn(str(self.root), json.dumps(data))

    def test_missing_failed_or_false_like_build_finished(self):
        for finish in ([], [{"reason": "build-finished", "success": False}], [{"reason": "build-finished", "success": 1}]):
            with self.subTest(finish=finish):
                self.events = [self.artifact(self.binary), *finish]
                self.assertEqual(self.pack(), 2)
                self.assertFalse(self.out.exists())

    def test_no_test_artifact(self):
        self.events[0]["profile"]["test"] = False
        self.assertEqual(self.pack(), 2)

    def test_outside_root(self):
        outsider = self.root / "secret"
        outsider.write_text("secret")
        self.events[0]["executable"] = str(outsider)
        self.assertEqual(self.pack(), 2)
        self.assertFalse(self.out.exists())

    def test_source_symlink_and_directory(self):
        link = self.artifacts / "link"
        link.symlink_to(self.binary)
        for source in (link, self.artifacts):
            self.events[0]["executable"] = str(source)
            self.assertEqual(self.pack(), 2)

    def test_no_overwrite(self):
        self.assertEqual(self.pack(), 0)
        original = (self.out / "manifest.json").read_bytes()
        self.assertEqual(self.pack(), 2)
        self.assertEqual((self.out / "manifest.json").read_bytes(), original)

    def test_success_forwarding_cwd_and_immutable_manifest(self):
        self.pack()
        original = (self.out / "manifest.json").read_bytes()
        with patch.object(helper.subprocess, "run", return_value=types.SimpleNamespace(returncode=0)) as execute:
            self.assertEqual(self.run_bundle("--cwd", str(self.root), "--", "filter", "--exact"), 0)
        execute.assert_called_once_with([str(self.out / "bin/test-0000"), "filter", "--exact"], cwd=str(self.root), shell=False, check=False)
        self.assertEqual(json.loads(self.results.read_text())["test_status"], "passed")
        self.assertEqual((self.out / "manifest.json").read_bytes(), original)

    def test_failure_and_spawn_error_are_recorded(self):
        self.pack()
        for outcome in (types.SimpleNamespace(returncode=1), OSError("wrong loader")):
            with self.subTest(outcome=outcome):
                kwargs = {"side_effect": outcome} if isinstance(outcome, Exception) else {"return_value": outcome}
                with patch.object(helper.subprocess, "run", **kwargs):
                    self.assertEqual(self.run_bundle(), 1)
                self.assertEqual(json.loads(self.results.read_text())["test_status"], "failed")
                self.results.unlink()

    def test_all_hashes_checked_before_execution(self):
        second = self.artifacts / "second"
        second.write_bytes(b"second binary")
        self.events.insert(1, self.artifact(second))
        self.pack()
        (self.out / "bin/test-0001").write_bytes(b"tampered")
        with patch.object(helper.subprocess, "run") as execute:
            self.assertEqual(self.run_bundle(), 2)
            execute.assert_not_called()
        self.assertFalse(self.results.exists())

    def test_unsafe_paths(self):
        self.pack()
        for name in ("../original", "/etc/passwd", "bin/../../secret", "bin\\test", "C:/file", "bin/./test", "bin//test"):
            with self.subTest(name=name):
                self.change_manifest(lambda m: m["tests"][0].update(path=name))
                with patch.object(helper.subprocess, "run") as execute:
                    self.assertEqual(self.run_bundle(), 2)
                    execute.assert_not_called()

    def test_binary_and_manifest_symlinks(self):
        self.pack()
        binary = self.out / "bin/test-0000"
        binary.unlink()
        binary.symlink_to(self.binary)
        self.assertEqual(self.run_bundle(), 2)
        binary.unlink()
        binary.write_bytes(self.binary.read_bytes())
        saved = self.root / "saved.json"
        (self.out / "manifest.json").rename(saved)
        (self.out / "manifest.json").symlink_to(saved)
        self.assertEqual(self.run_bundle(), 2)

    def test_schema_and_entry_validation(self):
        self.pack()
        original = (self.out / "manifest.json").read_bytes()
        mutations = [
            lambda m: m.update(schema_version=True),
            lambda m: m.update(schema_version=2),
            lambda m: m.update(target="unknown"),
            lambda m: m.update(tests=[]),
            lambda m: m.update(test_status="passed"),
            lambda m: m.update(extra=1),
            lambda m: m["tests"].append(m["tests"][0].copy()),
            lambda m: m["tests"][0].update(size=True),
            lambda m: m["tests"][0].update(sha256="z" * 64),
        ]
        for mutate in mutations:
            (self.out / "manifest.json").write_bytes(original)
            self.change_manifest(mutate)
            self.assertEqual(self.run_bundle(), 2)

    def test_duplicate_json_and_size_bound(self):
        self.pack()
        (self.out / "manifest.json").write_text('{"schema_version":1,"schema_version":1}')
        self.assertEqual(self.run_bundle(), 2)
        (self.out / "manifest.json").write_text(" " * (helper.MAX_JSON + 1))
        self.assertEqual(self.run_bundle(), 2)

    def test_platform_mismatch(self):
        self.pack()
        for target in ("aarch64-unknown-linux-gnu", "x86_64-pc-windows-msvc", "x86_64-apple-darwin"):
            self.change_manifest(lambda m: m.update(target=target))
            with patch.object(helper.subprocess, "run") as execute:
                self.assertEqual(self.run_bundle(), 2)
                execute.assert_not_called()

    def test_multiple_tests_continue_after_failure(self):
        second = self.artifacts / "second"
        second.write_bytes(b"second binary")
        self.events.insert(1, self.artifact(second))
        self.pack()
        with patch.object(helper.subprocess, "run", side_effect=[types.SimpleNamespace(returncode=7), types.SimpleNamespace(returncode=0)]) as execute:
            self.assertEqual(self.run_bundle(), 1)
            self.assertEqual(execute.call_count, 2)
        self.assertEqual([t["exit_code"] for t in json.loads(self.results.read_text())["tests"]], [7, 0])

    def test_parent_symlink_rejected(self):
        self.pack()
        alias = self.root / "alias"
        alias.symlink_to(self.out, target_is_directory=True)
        with self.assertRaisesRegex(ValueError, "symlink"):
            helper.load_manifest(alias)
        outside = self.root / "outside"
        outside.mkdir()
        result_alias = self.root / "results-alias"
        result_alias.symlink_to(outside, target_is_directory=True)
        self.results = result_alias / "new.json"
        with patch.object(helper.subprocess, "run") as execute:
            self.assertEqual(self.run_bundle(), 2)
            execute.assert_not_called()

    def test_malformed_and_oversized_cargo_json(self):
        arguments = ["bundle", "--messages", str(self.messages), "--artifact-root", str(self.artifacts), "--target", TARGET, "--out", str(self.out)]
        for text in ("not json\n", "[]\n", " " * (helper.MAX_JSON + 1)):
            self.messages.write_text(text)
            with contextlib.redirect_stderr(io.StringIO()):
                self.assertEqual(helper.main(arguments), 2)
            self.assertFalse(self.out.exists())

    def test_duplicate_artifacts_copied_once(self):
        self.events.insert(1, self.events[0].copy())
        self.assertEqual(self.pack(), 0)
        self.assertEqual(len(self.manifest()["tests"]), 1)

    def test_windows_gnu_bundle_and_platform_check(self):
        self.messages.write_text("".join(json.dumps(e) + "\n" for e in self.events))
        with contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(helper.main(["bundle", "--messages", str(self.messages), "--artifact-root", str(self.artifacts), "--target", "x86_64-pc-windows-gnu", "--out", str(self.out)]), 0)
        self.assertEqual(self.manifest()["tests"][0]["path"], "bin/test-0000.exe")
        with patch.object(helper.subprocess, "run") as execute:
            self.assertEqual(self.run_bundle(), 2)
            execute.assert_not_called()
        with patch.object(helper.sys, "platform", "win32"), patch.object(helper.platform, "machine", return_value="AMD64"), patch.object(helper.subprocess, "run", return_value=types.SimpleNamespace(returncode=0)), contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(helper.main(["run", "--bundle", str(self.out), "--results", str(self.results)]), 0)

    def test_non_test_modes_rejected_without_execution(self):
        self.pack()
        for argument in ("--list", "--help", "-h", "--bench", "--list=true", "--bench=true"):
            with self.subTest(argument=argument), patch.object(helper.subprocess, "run") as execute:
                self.assertEqual(self.run_bundle("--", argument), 2)
                execute.assert_not_called()
                self.assertFalse(self.results.exists())

    def test_results_refuse_overwrite_and_bundle_path(self):
        self.pack()
        with patch.object(helper.subprocess, "run") as execute:
            self.results.write_text("keep")
            self.assertEqual(self.run_bundle(), 2)
            self.assertEqual(self.results.read_text(), "keep")
            self.results = self.out / "result.json"
            self.assertEqual(self.run_bundle(), 2)
            self.results = self.root / "alias.json"
            self.results.symlink_to(self.out / "new.json")
            self.assertEqual(self.run_bundle(), 2)
            execute.assert_not_called()


if __name__ == "__main__":
    unittest.main()
