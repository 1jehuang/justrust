#!/usr/bin/env python3
"""Local-only tests: real Git repositories, mocked build commands."""

import contextlib
import fcntl
import io
import multiprocessing
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import validate_publish as workflow


def contend(lock, connection):
    with workflow.locked(lock):
        connection.send("acquired")
    connection.close()


def queued_target(repo, lock, connection):
    original_run = workflow.run
    original_hosted = workflow.hosted

    def run(command, cwd, capture=False):
        if command[0] == "justrust":
            tree = Path(cwd)
            connection.send(("built", (tree / "source").read_text(),
                             (tree / "untracked.rs").read_text()))
            return None
        return original_run(command, cwd, capture)

    def hosted(command, cwd, lock):
        connection.send(("queued",))
        original_hosted(command, cwd, lock)

    with patch.object(workflow, "run", side_effect=run), patch.object(workflow, "hosted", side_effect=hosted):
        result = workflow.main(["target", "--repo", str(repo), "--lock", str(lock), "--test", "suite"])
    connection.send(("result", result))
    connection.close()


class WorkflowTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.repo = self.root / "repo"
        self.remote = self.root / "remote.git"
        self.git("init", "--bare", str(self.remote), cwd=self.root)
        self.git("init", "-b", "main", str(self.repo), cwd=self.root)
        self.git("config", "user.email", "test@example.invalid")
        self.git("config", "user.name", "Test")
        (self.repo / "source").write_text("base")
        self.commit("base")
        self.git("remote", "add", "origin", str(self.remote))
        self.git("push", "origin", "main")
        self.base = self.git("rev-parse", "HEAD")
        (self.repo / "source").write_text("candidate")
        self.candidate = self.commit("candidate")
        self.calls = []
        self.fail = None
        self.on_build = None
        self.real_run = workflow.run
        self.lock = self.root / "build.lock"

    def git(self, *args, cwd=None):
        return subprocess.check_output(["git", *args], cwd=cwd or self.repo,
                                       stderr=subprocess.DEVNULL, text=True).strip()

    def commit(self, message):
        self.git("add", "source")
        self.git("commit", "-m", message)
        return self.git("rev-parse", "HEAD")

    def fake_run(self, command, cwd, capture=False):
        if command[0] not in ("cargo", "justrust"):
            return self.real_run(command, cwd, capture)
        self.calls.append((command, Path(cwd), (Path(cwd) / "source").read_text()))
        if self.on_build:
            self.on_build()
        if self.fail and self.fail(command):
            raise subprocess.CalledProcessError(1, command)
        return None

    def invoke(self, mode="publish", extra=(), integration=True):
        args = [mode, "--repo", str(self.repo), "--lock", str(self.lock)]
        if integration:
            args += ["--test", "suite"]
        if mode == "publish":
            args += ["--candidate", self.candidate]
        with patch.object(workflow, "run", side_effect=self.fake_run), contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            return workflow.main(args + list(extra))

    def remote_head(self):
        return self.git("--git-dir=" + str(self.remote), "rev-parse", "main")

    def test_immutable_archive_dirty_preserved_validation_default(self):
        (self.repo / "source").write_text("dirty")
        (self.repo / "untracked").write_text("keep")
        self.assertEqual(self.invoke(extra=["--filter", "one", "--lib"]), 0)
        self.assertTrue(all(source == "candidate" and cwd != self.repo for _, cwd, source in self.calls))
        self.assertEqual((self.repo / "source").read_text(), "dirty")
        self.assertEqual((self.repo / "untracked").read_text(), "keep")
        self.assertEqual(self.remote_head(), self.base)
        commands = [command for command, _, _ in self.calls]
        self.assertEqual(commands[0], ["justrust", "test", "--test", "suite", "one"])
        self.assertIn(["justrust", "test", "--test", "suite"], commands)
        self.assertIn(["justrust", "test", "--workspace", "--lib"], commands)
        self.assertEqual(commands[-1], ["justrust", "clippy", "--workspace", "--all-targets", "--", "-D", "warnings"])

    def test_target_fail_fast_no_full_gates(self):
        self.fail = lambda command: True
        self.assertEqual(self.invoke("target", ["--test", "other"]), 1)
        self.assertEqual(len(self.calls), 1)
        self.assertNotEqual(self.calls[0][1], self.repo)

    def test_target_success_runs_only_requested_targets(self):
        self.assertEqual(self.invoke("target", ["--test", "other", "--filter", "one"]), 0)
        self.assertEqual([command for command, _, _ in self.calls], [
            ["justrust", "test", "--test", "suite", "one"],
            ["justrust", "test", "--test", "other", "one"],
        ])

    def test_package_unit_targets_are_filtered_bounded_and_deduplicated(self):
        self.assertEqual(self.invoke("target", ["--unit", "--package", "justrust-graph",
                                               "--package", "justrust-cache", "--package", "justrust-graph",
                                               "--filter", "name"], integration=False), 0)
        self.assertEqual([command for command, _, _ in self.calls], [
            ["justrust", "test", "--package", "justrust-graph", "--lib", "name"],
            ["justrust", "test", "--package", "justrust-cache", "--lib", "name"],
        ])

    def test_unit_without_package_defaults_to_root(self):
        self.assertEqual(self.invoke("target", ["--unit"], integration=False), 0)
        self.assertEqual([command for command, _, _ in self.calls], [["justrust", "test", "--lib"]])

    def test_package_integration_selection_is_preserved(self):
        self.assertEqual(self.invoke("target", ["--package", "justrust-graph"]), 0)
        self.assertEqual(self.calls[0][0], ["justrust", "test", "--package", "justrust-graph", "--test", "suite"])

    def test_unit_publication_requires_unfiltered_package_and_workspace_lib(self):
        self.fail = lambda command: command == ["justrust", "test", "--workspace", "--lib"]
        self.assertEqual(self.invoke(extra=["--unit", "--package", "justrust-graph", "--filter", "name", "--push"],
                                     integration=False), 1)
        commands = [command for command, _, _ in self.calls]
        self.assertEqual(commands[0], ["justrust", "test", "--package", "justrust-graph", "--lib", "name"])
        self.assertIn(["justrust", "test", "--package", "justrust-graph", "--lib"], commands)
        self.assertEqual(commands[-1], ["justrust", "test", "--workspace", "--lib"])
        self.assertEqual(self.remote_head(), self.base)

    def test_missing_selector_and_invalid_package_rejected(self):
        for extra in ([], ["--package", "justrust-graph"], ["--unit", "--package=--workspace"],
                      ["--unit", "--package", "../bad"], ["--unit"] + ["--package=p"] * 33):
            with self.subTest(extra=extra), self.assertRaises(SystemExit) as error:
                self.invoke("target", extra, integration=False)
            self.assertEqual(error.exception.code, 2)
        self.assertEqual(self.calls, [])

    def test_option_like_filter_and_unbounded_target_rejected(self):
        for extra in (["--filter=--nocapture"], ["--test", "../bad"]):
            with self.subTest(extra=extra), self.assertRaises(SystemExit) as error:
                self.invoke("target", extra)
            self.assertEqual(error.exception.code, 2)
        self.assertEqual(self.calls, [])

    def test_unfiltered_touched_suite_is_required_after_filtered_pass(self):
        self.fail = lambda command: command == ["justrust", "test", "--test", "suite"]
        self.assertEqual(self.invoke(extra=["--filter", "one", "--push"]), 1)
        self.assertEqual(self.calls[-1][0], ["justrust", "test", "--test", "suite"])
        self.assertEqual(self.remote_head(), self.base)

    def test_publication_target_failure_blocks_full_gates_and_push(self):
        self.fail = lambda command: True
        self.assertEqual(self.invoke(extra=["--push"]), 1)
        self.assertEqual(len(self.calls), 1)
        self.assertEqual(self.remote_head(), self.base)

    def test_each_full_gate_failure_prevents_push(self):
        for failing in (["cargo", "fmt", "--all", "--check"],
                        ["justrust", "test", "--test", "graph_integrity"],
                        ["justrust", "test", "--test", "full_programs"],
                        ["justrust", "test", "--workspace", "--no-run"],
                        ["justrust", "test", "--workspace", "--lib"],
                        ["justrust", "clippy", "--workspace", "--all-targets", "--", "-D", "warnings"]):
            with self.subTest(gate=failing):
                self.calls.clear()
                self.fail = lambda command: command == failing
                self.assertEqual(self.invoke(extra=["--push", "--lib"]), 1)
                self.assertEqual(self.calls[-1][0], failing)
                self.assertEqual(self.remote_head(), self.base)

    def test_exact_hash_pushed_when_head_moves_during_validation(self):
        def move_head():
            self.on_build = None
            (self.repo / "source").write_text("later")
            self.commit("later")
        self.on_build = move_head
        self.assertEqual(self.invoke(extra=["--push"]), 0)
        self.assertEqual(self.remote_head(), self.candidate)
        self.assertNotEqual(self.git("rev-parse", "HEAD"), self.candidate)

    def test_candidate_may_differ_from_initial_head(self):
        (self.repo / "source").write_text("later")
        later = self.commit("later")
        self.assertEqual(self.invoke(extra=["--push"]), 0)
        self.assertEqual(self.remote_head(), self.candidate)
        self.assertEqual(self.git("rev-parse", "HEAD"), later)

    def test_remote_race_rejected_normally(self):
        def advance_remote():
            self.on_build = None
            (self.repo / "source").write_text("remote later")
            later = self.commit("remote later")
            self.git("push", "origin", later + ":refs/heads/main")
        self.on_build = advance_remote
        self.assertEqual(self.invoke(extra=["--push"]), 1)
        self.assertNotEqual(self.remote_head(), self.candidate)

    def test_nonancestor_refused_before_build(self):
        self.git("push", "origin", "main")
        self.candidate = self.base
        self.assertEqual(self.invoke(extra=["--push"]), 1)
        self.assertEqual(self.calls, [])

    def test_queued_target_uses_snapshot_captured_before_lock_wait(self):
        (self.repo / "source").write_text("captured dirty")
        (self.repo / "untracked.rs").write_text("captured untracked")
        parent, child = multiprocessing.Pipe()
        with workflow.locked(self.lock):
            worker = multiprocessing.Process(target=queued_target, args=(self.repo, self.lock, child))
            worker.start()
            try:
                self.assertTrue(parent.poll(5))
                self.assertEqual(parent.recv(), ("queued",))
                self.assertFalse(parent.poll(0.1))
                (self.repo / "source").write_text("later dirty")
                (self.repo / "untracked.rs").write_text("later untracked")
            except BaseException:
                worker.terminate()
                worker.join(5)
                raise
        self.assertTrue(parent.poll(5))
        self.assertEqual(parent.recv(), ("built", "captured dirty", "captured untracked"))
        self.assertTrue(parent.poll(5))
        self.assertEqual(parent.recv(), ("result", 0))
        worker.join(5)
        self.assertEqual(worker.exitcode, 0)
        self.assertEqual((self.repo / "source").read_text(), "later dirty")
        self.assertEqual((self.repo / "untracked.rs").read_text(), "later untracked")

    def test_snapshot_includes_cargo_inputs_excludes_caches_preserves_links(self):
        for name in ("Cargo.toml", "crates/engine/Cargo.toml", "crates/engine/src/lib.rs",
                     "tests/fixtures/data.bin", "target/artifact", "__pycache__/cache.pyc",
                     "data/runs/history.jsonl", "experiments/results/archive.tar.gz", "ignored.rs"):
            path = self.repo / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(name)
        (self.repo / ".gitignore").write_text("ignored.rs\n")
        (self.repo / "relative.rs").symlink_to("source")
        tree = self.root / "snapshot"
        tree.mkdir()
        workflow.snapshot(self.repo, tree)
        for name in ("Cargo.toml", "crates/engine/Cargo.toml", "crates/engine/src/lib.rs", "tests/fixtures/data.bin"):
            self.assertEqual((tree / name).read_text(), name)
        for name in (".git", "target", "__pycache__", "data/runs", "experiments/results", "ignored.rs"):
            self.assertFalse((tree / name).exists())
        self.assertTrue((tree / "relative.rs").is_symlink())
        self.assertEqual(os.readlink(tree / "relative.rs"), "source")
        self.assertEqual((tree / "relative.rs").read_text(), "candidate")

    def test_snapshot_rejects_external_links_and_oversized_inputs(self):
        (self.repo / "escape").symlink_to("../outside")
        self.assertEqual(self.invoke("target"), 1)
        self.assertEqual(self.calls, [])
        (self.repo / "escape").unlink()
        with patch.object(workflow, "MAX_SNAPSHOT_FILE_BYTES", 1):
            self.assertEqual(self.invoke("target"), 1)
        self.assertEqual(self.calls, [])

    def test_push_remote_verification_mismatch_is_failure(self):
        original_run = self.real_run

        def wrong_remote(command, cwd, capture=False):
            if command[:2] == ["git", "ls-remote"]:
                return self.base + "\trefs/heads/main"
            return original_run(command, cwd, capture)

        self.real_run = wrong_remote
        self.assertEqual(self.invoke(extra=["--push"]), 1)
        self.assertEqual(self.remote_head(), self.candidate)

    def test_shared_lock_serializes_without_replacing_inode(self):
        parent, child = multiprocessing.Pipe()
        with workflow.locked(self.lock):
            inode = self.lock.stat().st_ino
            worker = multiprocessing.Process(target=contend, args=(self.lock, child))
            worker.start()
            self.assertFalse(parent.poll(0.15))
        self.assertTrue(parent.poll(5))
        self.assertEqual(parent.recv(), "acquired")
        worker.join(5)
        self.assertEqual(worker.exitcode, 0)
        self.assertEqual(self.lock.stat().st_ino, inode)

    def test_hosted_commands_hold_build_and_publication_locks(self):
        def check_locks():
            for path in (self.lock, Path(str(self.lock) + ".publication")):
                with path.open("a") as stream:
                    with self.assertRaises(BlockingIOError):
                        fcntl.flock(stream, fcntl.LOCK_EX | fcntl.LOCK_NB)
        self.on_build = check_locks
        self.assertEqual(self.invoke(), 0)


if __name__ == "__main__":
    unittest.main()
