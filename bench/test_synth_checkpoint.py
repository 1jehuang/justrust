"""Checkpoint provenance gates, no builds or real snapshot edits."""
import argparse
import contextlib
import io
import json
import os
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest import mock

import suite
from synth import run as runner


class CheckpointTests(unittest.TestCase):
    def setUp(self):
        self.ctx = contextlib.ExitStack()
        self.addCleanup(self.ctx.close)
        self.root = Path(self.ctx.enter_context(tempfile.TemporaryDirectory()))
        self.binary = self.root / "driver"
        self.binary.write_bytes(b"version one")
        self.binary.chmod(0o755)
        self.partial = self.root / "out.partial.json"
        self.stack = runner.versus.Stack("fixture", "fixture", True,
                                         {"RUSTC": str(self.binary)})
        self.ctx.enter_context(mock.patch.object(suite, "J", str(self.binary)))
        self.ctx.enter_context(mock.patch.object(suite, "TOOLCHAIN", "system"))
        self.ctx.enter_context(mock.patch.object(suite, "JR_HOME", self.root / "home"))
        self.ctx.enter_context(mock.patch.dict(os.environ, {"PATH": str(self.root), "HOME": str(self.root)}, clear=True))
        # The cargo role also uses fixture bytes, without invoking any tool.
        self.ctx.enter_context(mock.patch.object(runner.shutil, "which", side_effect=lambda command, **kw: str(self.binary) if command == "cargo" else None))
        self.ctx.enter_context(mock.patch.object(runner.gen, "_locked"))
        self.ctx.enter_context(mock.patch.object(runner.gen, "sample", return_value=([0], {})))
        self.ctx.enter_context(mock.patch.object(runner.gen, "fingerprint", return_value="generator"))
        plan = SimpleNamespace(point=SimpleNamespace(i=0), snap="fixture", cmd=["check"],
                               to_json=lambda: {"i": 0})
        self.ctx.enter_context(mock.patch.object(runner.gen, "plan", return_value=plan))
        self.restore = self.ctx.enter_context(mock.patch.object(runner.gen, "restore_all"))
        self.measure = self.ctx.enter_context(mock.patch.object(runner, "run_point", return_value={"error": "retained failure"}))
        self.ctx.enter_context(contextlib.redirect_stderr(io.StringIO()))

    def run_fixture(self, resume=False):
        return runner.run([self.stack], 1, 1, 1, 0, partial=self.partial, resume=resume)

    def test_matching_identity_accepts_and_retains_failed_point(self):
        original = self.run_fixture()
        self.measure.reset_mock()
        resumed = self.run_fixture(resume=True)
        self.assertEqual(original["results"], resumed["results"])
        self.assertEqual(resumed["results"][0]["errors"], {"fixture": "retained failure"})
        self.measure.assert_not_called()

    def assert_rejected(self):
        saved = self.partial.read_bytes()
        self.restore.reset_mock()
        self.measure.reset_mock()
        with self.assertRaisesRegex(ValueError, "provenance"):
            self.run_fixture(resume=True)
        self.assertEqual(saved, self.partial.read_bytes())
        self.restore.assert_not_called()
        self.measure.assert_not_called()

    def test_same_path_changed_binary_rejected(self):
        self.run_fixture()
        self.binary.write_bytes(b"version two")
        self.assert_rejected()

    def test_changed_measurement_home_rejected(self):
        self.run_fixture()
        with mock.patch.object(suite, "JR_HOME", self.root / "isolated"):
            self.assert_rejected()

    def test_changed_controls_rejected(self):
        self.run_fixture()
        self.stack.env["JUSTRUST_CARGO_DAEMON"] = "1"
        self.assert_rejected()

    def test_changed_runtime_policy_rejected(self):
        self.run_fixture()
        with mock.patch.dict(os.environ, {"JUSTRUST_BENCH_RUNTIME_CONTEXT": "jcode-isolated-v2"}):
            self.assert_rejected()

    def test_changed_toolchain_bytes_rejected(self):
        compiler = self.root / "toolchain" / "bin" / "rustc"
        compiler.parent.mkdir(parents=True)
        compiler.write_bytes(b"compiler one")
        with mock.patch.object(suite, "TOOLCHAIN", "jr-fixture"), mock.patch.object(suite, "toolchain_rustc", return_value=compiler):
            self.run_fixture()
            compiler.write_bytes(b"compiler two")
            self.assert_rejected()

    def default_prefix(self, spec="jr-default", *, staged=False):
        home = self.root / "home"
        home.mkdir(exist_ok=True)
        (home / "toolchain-policy.json").write_text(json.dumps(
            {"toolchain": spec, "protocol": 1, "fetched_at": 1}))
        prefix = home / "toolchains"
        if staged:
            prefix = prefix / "staged"
        prefix = prefix / spec
        (prefix / "bin").mkdir(parents=True)
        (prefix / "justrust-toolchain.json").write_text(json.dumps(
            {"id": spec, "installed_at": 10}))
        for name in ("rustc", "cargo", "jr-cargo", "rustdoc"):
            (prefix / "bin" / name).write_bytes(name.encode())
            (prefix / "bin" / name).chmod(0o755)
        (prefix / "lib").mkdir()
        (prefix / "lib" / "librustc_driver-fixture.so").write_bytes(b"compiler driver")
        return prefix

    def test_system_label_binds_policy_selected_rustc_bytes(self):
        prefix = self.default_prefix()
        self.run_fixture()
        (prefix / "bin" / "rustc").write_bytes(b"new default compiler")
        self.assert_rejected()

    def test_system_label_binds_policy_selected_jr_cargo_bytes(self):
        prefix = self.default_prefix()
        self.run_fixture()
        (prefix / "bin" / "jr-cargo").write_bytes(b"new fork")
        self.assert_rejected()

    def test_system_label_binds_default_compiler_shared_library(self):
        prefix = self.default_prefix()
        self.run_fixture()
        (prefix / "lib" / "librustc_driver-fixture.so").write_bytes(b"new compiler driver")
        self.assert_rejected()

    def test_default_cargo_fork_executable_permission_change_rejected(self):
        prefix = self.default_prefix()
        self.run_fixture()
        (prefix / "bin" / "jr-cargo").chmod(0o644)
        self.assert_rejected()

    def test_new_default_candidate_rejected(self):
        self.default_prefix()
        (self.root / "home/toolchain-policy.json").unlink()
        self.run_fixture()
        self.default_prefix("jr-newer")
        (self.root / "home/toolchain-policy.json").unlink()
        self.assert_rejected()

    def test_changed_policy_selection_rejected(self):
        self.default_prefix()
        self.run_fixture()
        (self.root / "home/toolchain-policy.json").write_text(json.dumps(
            {"toolchain": "jr-other", "protocol": 1, "fetched_at": 2}))
        self.assert_rejected()

    def test_policy_timestamp_refresh_does_not_change_identity(self):
        self.default_prefix()
        original = self.run_fixture()
        (self.root / "home/toolchain-policy.json").write_text(json.dumps(
            {"toolchain": "jr-default", "protocol": 1, "fetched_at": 200}))
        self.assertEqual(original["results"], self.run_fixture(resume=True)["results"])

    def test_newest_active_fallback_metadata_changed_rejects(self):
        prefix = self.default_prefix()
        (self.root / "home/toolchain-policy.json").unlink()
        self.run_fixture()
        (prefix / "justrust-toolchain.json").write_text(json.dumps(
            {"id": "jr-default", "installed_at": 20}))
        self.assert_rejected()

    def test_staged_default_candidate_bytes_bound(self):
        prefix = self.default_prefix(staged=True)
        self.run_fixture()
        (prefix / "bin" / "jr-cargo").write_bytes(b"new staged fork")
        self.assert_rejected()

    def test_malformed_policy_fails_closed(self):
        self.default_prefix()
        (self.root / "home/toolchain-policy.json").write_text('{"toolchain": null}')
        with self.assertRaisesRegex(ValueError, "default toolchain policy"):
            self.run_fixture()
        self.restore.assert_not_called()
        self.measure.assert_not_called()

    def test_legacy_checkpoint_rejected(self):
        self.run_fixture()
        old = json.loads(self.partial.read_text())
        del old["checkpoint_identity"]
        self.partial.write_text(json.dumps(old))
        self.assert_rejected()

    def test_rustup_proxy_resolves_actual_selected_tool_bytes(self):
        rustup = self.root / "rustup"
        rustup.write_bytes(b"proxy")
        proxy = self.root / "rustc"
        proxy.symlink_to(rustup)
        self.stack.env["RUSTC"] = str(proxy)
        with mock.patch.object(runner.subprocess, "check_output", return_value=str(self.binary)) as resolve:
            self.run_fixture()
        self.assertEqual(resolve.call_args.args[0], [str(rustup), "which", "rustc"])
        saved = json.loads(self.partial.read_text())
        self.assertEqual(saved["checkpoint_identity"]["stacks"][0]["tools"]["rustc"],
                         suite.binary_sha256(self.binary))

    def test_malformed_results_rejected_without_overwrite(self):
        self.run_fixture()
        old = json.loads(self.partial.read_text())
        old["results"].append(old["results"][0])
        self.partial.write_text(json.dumps(old))
        saved = self.partial.read_bytes()
        with self.assertRaisesRegex(ValueError, "malformed"):
            self.run_fixture(resume=True)
        self.assertEqual(saved, self.partial.read_bytes())

    def test_interrupted_before_point_zero_replaces_old_checkpoint(self):
        self.run_fixture()
        self.binary.write_bytes(b"new version")
        self.measure.side_effect = KeyboardInterrupt
        with self.assertRaises(KeyboardInterrupt):
            self.run_fixture()
        saved = json.loads(self.partial.read_text())
        self.assertEqual(saved["results"], [])
        self.assertEqual(saved["checkpoint_identity"], runner.checkpoint_identity([self.stack], 0, None, 0))

    def test_explicit_output_scoped_by_full_path(self):
        a = argparse.Namespace(synth=1, out=str(self.root / "a/result.json"))
        b = argparse.Namespace(synth=1, out=str(self.root / "b/result.json"))
        self.assertNotEqual(suite.synthetic_partial_path(a), suite.synthetic_partial_path(b))
        self.assertEqual(suite.synthetic_partial_path(a), self.root / "a/result.json.partial.json")
        a.out = None
        self.assertEqual(suite.synthetic_partial_path(a), suite.RESULTS / "partial/synth1.json")


if __name__ == "__main__":
    unittest.main()
