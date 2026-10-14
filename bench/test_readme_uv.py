"""Unit validation for readme_uv, without running a Rust build/benchmark."""
import io
import json
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile
import unittest
from unittest import mock

try:
    from . import readme_uv as uv
except ImportError:
    import readme_uv as uv


class ReadmeUvTests(unittest.TestCase):
    def setUp(self):
        uv.scratch_root().mkdir(parents=True, exist_ok=True)
        self.temp = tempfile.TemporaryDirectory(prefix="test-readme-uv-", dir=uv.scratch_root())
        self.work = Path(self.temp.name)
        self.addCleanup(self.temp.cleanup)

    @staticmethod
    def samples(rounds=3):
        return [{"row": row, "stack": stack, "round": index + 1,
                 "status": "ok", "wall_seconds": wall, "measured": True,
                 "run_id": f"{row}-{stack}-{index}"}
                for row in uv.ROWS for stack in uv.STACKS
                for index, wall in enumerate([3.0, 1.0, 2.0][:rounds])]

    def test_complete_aggregation_and_prewarm_exclusion(self):
        samples = self.samples()
        samples.append(dict(samples[0], measured=False, wall_seconds=999))
        for row in uv.aggregate(samples, 3).values():
            for cell in row.values():
                self.assertTrue(cell["all_ok"])
                self.assertEqual(cell["median_seconds"], 2.0)
                self.assertEqual(cell["samples"], 3)
                self.assertEqual(len(cell["run_ids"]), 3)

    def test_failed_partial_duplicate_and_running_rows_have_no_median(self):
        for mutation in ("failed", "running", "missing", "duplicate"):
            with self.subTest(mutation=mutation):
                samples = self.samples()
                if mutation == "missing":
                    del samples[0]
                elif mutation == "duplicate":
                    samples.append(dict(samples[0]))
                else:
                    samples[0]["status"] = mutation
                cell = uv.aggregate(samples, 3)["fresh-check"]["stock"]
                self.assertFalse(cell["all_ok"])
                self.assertIsNone(cell["median_seconds"])

    def test_subset_aggregation_and_rotation(self):
        subset = ("stock", "local")
        self.assertEqual(set(uv.aggregate(self.samples(), 3, subset)["fresh-check"]), set(subset))
        self.assertEqual(uv.rotating_order(0), ("stock", "local", "remote"))
        self.assertEqual(uv.rotating_order(1), ("local", "remote", "stock"))
        self.assertEqual(uv.rotating_order(2), ("remote", "stock", "local"))
        self.assertEqual(uv.rotating_order(1, subset), ("local", "stock"))

    def test_remote_fallback_is_rejected(self):
        for remote in (None, {}, {"machine": ""}, {"machine": " "}, {"machine": 1}, []):
            with self.subTest(remote=remote), self.assertRaisesRegex(ValueError, "fell back locally"):
                uv.validate_summary("remote", {"id": "abc", "exit": 0, "remote": remote}, "abc")

    def test_remote_actual_toolchain_recorded_not_assumed_equal_to_client(self):
        summary = {"id": "abc", "exit": 0, "toolchain": "host-nightly", "remote": {"machine": "hosted"}}
        identity = uv.validate_summary("remote", summary, "abc")
        self.assertEqual(identity["selected"], "host-nightly")
        self.assertEqual(identity["identity_source"], "host summary")
        with self.assertRaisesRegex(ValueError, "unexpectedly ran remotely"):
            uv.validate_summary("local", summary, "abc")

    def test_missing_wrong_id_and_failed_summary_rejected(self):
        for summary in (None, {"id": "wrong", "exit": 0}, {"id": "abc", "exit": 1}):
            with self.subTest(summary=summary), self.assertRaises(ValueError):
                uv.validate_summary("local", summary, "abc")

    def test_stock_environment_removes_wrappers_and_uses_system_compiler(self):
        with mock.patch.dict(os.environ, {"RUSTC_WRAPPER": "shim", "RUSTC_WORKSPACE_WRAPPER": "other",
                                         "JUSTRUST_RUN_ID_OVERRIDE": "parent", "RUSTC": "wrong"}):
            env = uv.sample_environment("stock", self.work / "target", None)
            self.assertNotIn("RUSTC_WRAPPER", env)
            self.assertNotIn("RUSTC_WORKSPACE_WRAPPER", env)
            self.assertNotIn("JUSTRUST_RUN_ID_OVERRIDE", env)
            self.assertEqual(env["RUSTC"], "/usr/bin/rustc")
            local = uv.sample_environment("local", self.work / "target", "abc")
            self.assertEqual(local["JUSTRUST_RUN_ID_OVERRIDE"], "abc")
            self.assertEqual(local["JUSTRUST_NO_UPDATE"], "1")

    def test_body_edit_restores_even_when_build_raises(self):
        path = self.work / uv.EDIT_FILE
        path.parent.mkdir(parents=True)
        original = ("impl Manifest {\n" + uv.EDIT_ANCHOR + "        self.requirements.len()\n    }\n}\n").encode()
        path.write_bytes(original)
        with self.assertRaisesRegex(RuntimeError, "build failed"):
            with uv.body_edit(self.work, "unique_1") as info:
                edited = path.read_text()
                self.assertIn(uv.EDIT_ANCHOR + '        std::hint::black_box("readme_uv_unique_1");\n', edited)
                self.assertNotEqual(info["original_sha256"], info["edited_sha256"])
                raise RuntimeError("build failed")
        self.assertEqual(path.read_bytes(), original)
        with uv.body_edit(self.work, "unique_2"):
            self.assertIn("readme_uv_unique_2", path.read_text())
        self.assertEqual(path.read_bytes(), original)

    def test_archive_checkout_isolated_git_index_includes_ignored_sources(self):
        archive = self.work / "fixture.tar"
        with tarfile.open(archive, "w") as stream:
            for name, data in {"Cargo.lock": b"lock", ".gitignore": b"hidden.rs\n",
                               "hidden.rs": b"source", "Cargo.toml": b"[workspace]\n"}.items():
                entry = tarfile.TarInfo(name)
                entry.size = len(data)
                stream.addfile(entry, io.BytesIO(data))
        checkout = uv.extract_checkout(archive, self.work / "checkout")
        self.assertEqual(uv.run_text(["git", "rev-parse", "--show-toplevel"], checkout), str(checkout))
        self.assertIn("hidden.rs", uv.run_text(["git", "ls-files"], checkout))
        self.assertFalse((checkout / ".git/commondir").exists())
        self.assertFalse((checkout / "target").exists())

    def test_atomic_incremental_receipt_not_complete_until_finished(self):
        output = self.work / "result.json"
        result = {"status": "running", "samples": self.samples(), "rounds": 3}
        uv.save_result(output, result)
        self.assertFalse(json.loads(output.read_text())["all_ok"])
        result["status"] = "complete"
        uv.save_result(output, result)
        self.assertTrue(json.loads(output.read_text())["all_ok"])
        self.assertFalse(output.with_name("result.json.tmp").exists())

    def test_execution_nonzero_is_failed_with_sanitized_tail_and_correct_flags(self):
        executable = self.work / "fake-client"
        executable.write_bytes(b"fake executable")
        binaries = {key: {"path": str(executable), "sha256": uv.sha256(executable), "version": "fake"}
                    for key in ("cargo", "rustc", "justrust")}
        result = {"status": "running", "samples": [], "rounds": 1}

        def fake_run(command, **kwargs):
            self.assertEqual(command[1:3], ["check", "--remote"])
            self.assertIn("--locked", command)
            kwargs["stdout"].write(b"error: token=private-value Bearer abc123\n")
            return subprocess.CompletedProcess(command, 1)

        with mock.patch.object(uv.subprocess, "run", side_effect=fake_run):
            sample = uv.execute(result, self.work / "result.json", "remote", "fresh-check", 0,
                                self.work, binaries, self.work)
        self.assertEqual(sample["status"], "failed")
        self.assertEqual(sample["exit"], 1)
        self.assertNotIn("private-value", sample["error_tail"])
        self.assertNotIn("abc123", sample["error_tail"])
        self.assertTrue(Path(sample["log"]).exists())
        self.assertGreaterEqual(sample["wall_seconds"], 0)
        self.assertIsNone(uv.aggregate(result["samples"], 1)["fresh-check"]["remote"]["median_seconds"])

    def test_all_commands_are_locked(self):
        self.assertTrue(all("--locked" in command for command in uv.ROWS.values()))
        self.assertEqual(uv.ROWS["fresh-test-no-run"], ["test", "-p", "uv-resolver", "--no-run", "--locked"])

    def test_resume_preserves_completed_samples_and_excludes_interrupted(self):
        samples = self.samples()
        for sample in samples:
            sample["exit"] = 0
        before = json.dumps(samples, sort_keys=True)
        interrupted = dict(samples[0], status="running", exit=None)
        result = {"samples": samples + [interrupted]}
        uv.mark_interrupted(result)
        self.assertEqual(json.dumps(result["samples"][:-1], sort_keys=True), before)
        self.assertEqual(interrupted["status"], "interrupted")
        self.assertEqual(interrupted["phase"], "interrupted")
        self.assertFalse(interrupted["measured"])
        self.assertIs(uv.completed_sample(result, "fresh-check", "stock", 0), samples[0])
        self.assertTrue(uv.aggregate(result["samples"], 3)["fresh-check"]["stock"]["all_ok"])

    def test_resume_retry_checkout_unique_without_touching_original(self):
        original = self.work / "round-1/stock/fresh-test-no-run"
        original.mkdir(parents=True)
        marker = original / "preserve"
        marker.write_text("original artifacts")
        with mock.patch.object(uv, "extract_checkout", side_effect=lambda archive, path: path):
            retry = uv.fresh_checkout(self.work / "uv.tar", self.work, "fresh-test-no-run", "stock", 0)
            another = uv.fresh_checkout(self.work / "uv.tar", self.work, "fresh-test-no-run", "stock", 0)
        self.assertNotEqual(retry, original)
        self.assertNotEqual(retry, another)
        self.assertIn("-retry-", retry.name)
        self.assertEqual(marker.read_text(), "original artifacts")

    def test_resume_completed_round_never_executes_again(self):
        samples = self.samples(rounds=1)
        for sample in samples:
            sample["exit"] = 0
            sample["checkout"] = str(self.work / sample["stack"])
        result = {"rounds": 1, "stacks": uv.STACKS, "samples": samples}
        before = json.dumps(samples, sort_keys=True)
        with mock.patch.object(uv, "execute") as execute, mock.patch.object(uv, "fresh_checkout") as fresh:
            uv.run_rounds(result, self.work / "receipt.json", self.work / "uv.tar", {}, self.work)
        execute.assert_not_called()
        fresh.assert_not_called()
        self.assertEqual(json.dumps(result["samples"], sort_keys=True), before)


if __name__ == "__main__":
    unittest.main()
