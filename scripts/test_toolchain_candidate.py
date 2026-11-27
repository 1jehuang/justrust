#!/usr/bin/env python3
"""Cheap candidate BUILD tests. No compiler or Cargo build is run."""
import contextlib
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import tempfile
import threading
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("candidate", Path(__file__).with_name("toolchain-candidate.py"))
candidate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(candidate)


class CandidateTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name) / "repo"
        self.root.mkdir()
        self.git("init", "-q", "-b", "stable")
        self.git("config", "user.email", "test@example.invalid")
        self.git("config", "user.name", "Test")
        for path in ("toolchain/rustc/build-jr.sh", "toolchain/cargo/build-jr.sh",
                     "toolchain/shared/query_binding.rs",
                     "toolchain/rustc/git-commit-info", "docs/note.md"):
            file = self.root / path
            file.parent.mkdir(parents=True, exist_ok=True)
            file.write_text("original\n")
        (self.root / "toolchain/version.toml").write_text('version = "0.1.1"\n')
        self.git("add", ".")
        self.git("commit", "-qm", "fixture")
        self.revision = self.git("rev-parse", "HEAD")
        self.output = Path(self.temporary.name) / "output"

    def git(self, *args):
        return subprocess.check_output(["git", "-C", str(self.root), *args], text=True).strip()

    def plan(self):
        return candidate.make_plan(self.root, self.revision, self.output)

    def test_bootstrap_overlaps_cargo_but_training_is_serial(self):
        rendezvous = threading.Barrier(2, timeout=5)
        finished = []

        def run(name):
            if name in ("bootstrap", "cargo"):
                rendezvous.wait()
            else:
                self.assertEqual(set(finished[:2]), {"bootstrap", "cargo"})
            finished.append(name)

        candidate.build_stages(["bootstrap", "train", "optimize", "bolt", "cargo"], run)
        self.assertEqual(finished[2:], ["train", "optimize", "bolt"])

    def test_either_parallel_failure_blocks_training(self):
        for failed in ("bootstrap", "cargo"):
            seen = []
            def run(stage):
                seen.append(stage)
                if stage == failed:
                    raise ValueError(failed)
            with self.assertRaisesRegex(ValueError, failed):
                candidate.build_stages(["bootstrap", "train", "cargo"], run)
            self.assertNotIn("train", seen)

    def test_a_failed_cargo_build_is_repeated_before_training(self):
        seen = []

        def run(stage):
            seen.append(stage)
            if stage == "cargo" and seen.count("cargo") < 3:
                raise subprocess.CalledProcessError(1, ["build-jr.sh"])

        with contextlib.redirect_stderr(io.StringIO()) as err:
            candidate.build_stages(["bootstrap", "train", "cargo"], run)
        self.assertEqual(seen.count("cargo"), 3)
        self.assertEqual(seen[-1], "train")
        self.assertIn("attempt 2 of 3", err.getvalue())

    def test_a_cargo_build_that_keeps_failing_stops_the_candidate(self):
        seen = []

        def run(stage):
            seen.append(stage)
            if stage == "cargo":
                raise subprocess.CalledProcessError(1, ["build-jr.sh"])

        with contextlib.redirect_stderr(io.StringIO()):
            with self.assertRaises(subprocess.CalledProcessError):
                candidate.build_stages(["bootstrap", "train", "cargo"], run)
        self.assertEqual(seen.count("cargo"), candidate.CARGO_BUILD_ATTEMPTS)
        self.assertNotIn("train", seen)

    def test_plan_identity_and_no_side_effects(self):
        plan = self.plan()
        self.assertEqual(plan["candidate_id"], "jr-" + self.revision)
        self.assertEqual(plan["source_revision"], self.revision)
        self.assertEqual(plan["version"], "0.1.1")
        self.assertEqual(plan["release_tag"], "toolchain-v0.1.1")
        self.assertEqual(plan["commands"][0]["argv"], ["bash", "toolchain/rustc/build-jr.sh", "release"])
        cargo = plan["commands"][-1]
        # Routing is the user's (`justrust local off` sends it to hosted): the
        # release build never forces a local compile.
        self.assertNotIn("JUSTRUST_REMOTE", cargo["env"])
        self.assertNotIn("JUSTRUST_LOCAL", cargo["env"])
        self.assertEqual(cargo["cwd"], plan["cargo_build_root"])
        self.assertEqual(cargo["produces"], plan["artifacts"]["jr_cargo"])
        self.assertTrue(plan["artifacts"]["jr_cargo"].startswith(plan["cargo_build_root"] + "/"))
        self.assertFalse(self.output.exists())

    def test_invalid_committed_version_refused(self):
        (self.root / "toolchain/version.toml").write_text('version = "not-a-version"\n')
        self.git("add", ".")
        self.git("commit", "-qm", "invalid version")
        self.revision = self.git("rev-parse", "HEAD")
        with self.assertRaisesRegex(ValueError, "major.minor.patch"):
            self.plan()

    def test_dirty_version_refused(self):
        (self.root / "toolchain/version.toml").write_text('version = "0.1.2"\n')
        with self.assertRaisesRegex(ValueError, "dirty relevant"):
            self.plan()

    def test_cli_defaults_to_json_plan(self):
        output = io.StringIO()
        with patch.object(candidate, "ROOT", self.root), patch.object(candidate, "execute") as execute:
            with contextlib.redirect_stdout(output):
                self.assertEqual(candidate.main(["--source-commit", self.revision,
                                                 "--output-dir", str(self.output)]), 0)
            execute.assert_not_called()
        self.assertEqual(json.loads(output.getvalue())["state"], "planned")

    def test_reject_short_wrong_commit_and_branch(self):
        for revision in (self.revision[:12], "a" * 40, self.revision.upper()):
            with self.assertRaises(ValueError):
                candidate.make_plan(self.root, revision, self.output)
        self.git("checkout", "-qb", "main")
        with self.assertRaisesRegex(ValueError, "stable"):
            self.plan()

    def test_dirty_staged_and_unstaged_relevant_refused(self):
        file = self.root / "toolchain/rustc/build-jr.sh"
        file.write_text("modified\n")
        with self.assertRaisesRegex(ValueError, "dirty relevant"):
            self.plan()
        self.git("add", ".")
        with self.assertRaisesRegex(ValueError, "dirty relevant"):
            self.plan()

    def test_a_client_release_after_the_revision_does_not_stop_the_build(self):
        (self.root / "Cargo.toml").write_text('[package]\nversion = "0.1.2"\n')
        (self.root / "Cargo.lock").write_text("version = 4\n")
        self.git("add", ".")
        self.git("commit", "-qm", "Release v0.1.2")
        self.assertEqual(self.plan()["source_revision"], self.revision)

    def test_toolchain_source_committed_after_the_revision_stops_the_build(self):
        (self.root / "toolchain/shared/query_binding.rs").write_text("changed\n")
        self.git("add", ".")
        self.git("commit", "-qm", "compiler change")
        with self.assertRaisesRegex(ValueError, "toolchain source changed"):
            self.plan()

    def test_unrelated_dirty_allowed_and_untracked_excluded_from_archive(self):
        (self.root / "docs/note.md").write_text("unrelated\n")
        (self.root / "toolchain/untracked").write_text("not committed\n")
        self.assertEqual(self.plan()["source_revision"], self.revision)

    def run_mock_build(self, plan, fail=False, mismatch=False, no_cargo_binary=False):
        real_run = subprocess.run

        def run(argv, **kwargs):
            if argv[0] != "bash":
                return real_run(argv, **kwargs)
            self.assertEqual(kwargs["stdin"], subprocess.DEVNULL)
            self.assertEqual(kwargs["env"]["CI"], "1")
            snapshot = Path(plan["build_root"])
            self.assertFalse((snapshot / ".git").exists())
            # jr-cargo's tree: a git root holding toolchain/ without the compiler.
            cargo_root = Path(plan["cargo_build_root"])
            self.assertTrue((cargo_root / ".git").is_dir())
            self.assertTrue((cargo_root / "toolchain/cargo/build-jr.sh").is_file())
            self.assertTrue((cargo_root / "toolchain/shared/query_binding.rs").is_file())
            self.assertFalse((cargo_root / "toolchain/rustc").exists())
            self.assertFalse((cargo_root / "docs").exists())
            if "cargo" in argv[1]:
                self.assertEqual(kwargs["cwd"], str(cargo_root))
                if no_cargo_binary:
                    return subprocess.CompletedProcess(argv, 0)
            self.assertEqual((snapshot / "toolchain/rustc/git-commit-info").read_text().splitlines()[0], self.revision)
            if fail:
                raise subprocess.CalledProcessError(7, argv)
            artifact = (Path(plan["artifacts"]["compiler_sysroot"]) / "bin/rustc"
                        if "rustc" in argv[1] else Path(plan["artifacts"]["jr_cargo"]))
            artifact.parent.mkdir(parents=True, exist_ok=True)
            sha = "0" * 40 if mismatch else self.revision
            artifact.write_text(f"#!/bin/sh\nprintf 'commit-hash: {sha}\\n'\n")
            artifact.chmod(0o755)
            return subprocess.CompletedProcess(argv, 0)

        with patch.object(candidate.subprocess, "run", side_effect=run):
            return candidate.execute(plan)

    def test_execute_snapshot_helpers_provenance_and_no_overwrite(self):
        plan = self.plan()
        result = self.run_mock_build(plan)
        self.assertEqual(result["state"], "built")
        record = Path(plan["provenance_directory"]) / "provenance.json"
        self.assertEqual(json.loads(record.read_text())["commands"], plan["commands"])
        self.assertEqual((self.root / "toolchain/rustc/git-commit-info").read_text(), "original\n")
        with self.assertRaises(FileExistsError):
            candidate.execute(plan)

    def test_failure_recorded_without_claiming_built(self):
        plan = self.plan()
        with self.assertRaises(subprocess.CalledProcessError):
            self.run_mock_build(plan, fail=True)
        record = Path(plan["provenance_directory"]) / "provenance.json"
        self.assertEqual(json.loads(record.read_text())["state"], "failed")

    def test_actual_compiler_identity_required(self):
        with self.assertRaisesRegex(ValueError, "identity"):
            self.run_mock_build(self.plan(), mismatch=True)

    def test_cargo_helper_without_binary_fails(self):
        plan = self.plan()
        with self.assertRaisesRegex(ValueError, "did not produce executable artifact"):
            self.run_mock_build(plan, no_cargo_binary=True)
        record = Path(plan["provenance_directory"]) / "provenance.json"
        self.assertEqual(json.loads(record.read_text())["state"], "failed")

    def test_dirty_after_plan_refused_before_build(self):
        plan = self.plan()
        (self.root / "toolchain/rustc/build-jr.sh").write_text("changed\n")
        with self.assertRaisesRegex(ValueError, "dirty relevant"):
            candidate.execute(plan)
        self.assertFalse(self.output.exists())


if __name__ == "__main__":
    unittest.main()
