#!/usr/bin/env python3
"""Offline checks of ci/release-run-gates.py. No network, no GitHub calls."""
import importlib.util
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("release_run_gates", ROOT / "ci/release-run-gates.py")
gates = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gates)


def job(name, status, conclusion=None):
    return {"name": name, "status": status, "conclusion": conclusion}


X86, ARM = ("build " + target for target in gates.LINUX)
MAC = "build aarch64-apple-darwin"
WINDOWS = "build x86_64-pc-windows-msvc"


class LinuxGateTests(unittest.TestCase):
    def test_ready_while_macos_is_still_queued(self):
        jobs = [job(X86, "completed", "success"), job(ARM, "completed", "success"), job(MAC, "queued")]
        self.assertEqual(gates.linux_state(jobs, gates.LINUX), "ready")

    def test_waits_for_missing_or_running_linux_build(self):
        self.assertEqual(gates.linux_state([job(X86, "completed", "success")], gates.LINUX), "wait")
        jobs = [job(X86, "completed", "success"), job(ARM, "in_progress")]
        self.assertEqual(gates.linux_state(jobs, gates.LINUX), "wait")

    def test_failed_linux_build_fails_the_gate(self):
        jobs = [job(X86, "completed", "failure"), job(ARM, "in_progress")]
        self.assertIn("failure", gates.linux_state(jobs, gates.LINUX))

    def test_wait_times_out_and_returns_when_ready(self):
        listings = [[job(X86, "in_progress")],
                    [job(X86, "completed", "success"), job(ARM, "completed", "success")]]
        original = gates.run_jobs
        gates.run_jobs = lambda repo, run_id: listings.pop(0)
        try:
            slept = []
            gates.wait_linux("o/r", 1, gates.LINUX, 100, 20, clock=lambda: 0, sleep=slept.append)
            self.assertEqual(slept, [20])
            gates.run_jobs = lambda repo, run_id: []
            with self.assertRaises(SystemExit):
                gates.wait_linux("o/r", 1, gates.LINUX, 0, 20, clock=lambda: 0, sleep=slept.append)
        finally:
            gates.run_jobs = original


class SupersedeTests(unittest.TestCase):
    RUNS = [{"databaseId": 1, "headBranch": "v0.1.67", "status": "queued"},
            {"databaseId": 2, "headBranch": "v0.1.68", "status": "in_progress"},
            {"databaseId": 3, "headBranch": "v0.1.69", "status": "in_progress"},
            {"databaseId": 4, "headBranch": "v0.1.70", "status": "queued"},
            {"databaseId": 5, "headBranch": "v0.1.66", "status": "completed"},
            {"databaseId": 6, "headBranch": "stable", "status": "queued"}]
    QUEUED = [job(X86, "completed", "success"), job(WINDOWS, "completed", "failure"), job(MAC, "queued")]

    def test_cancels_only_older_runs_with_nothing_but_queued_optional_builds(self):
        jobs = {1: self.QUEUED, 2: [job(MAC, "queued"), job(ARM, "in_progress")]}
        asked = []

        def jobs_of(run_id):
            asked.append(run_id)
            return jobs[run_id]

        self.assertEqual(gates.superseded(self.RUNS, "v0.1.69", 3, jobs_of), [1])
        # Never inspects itself, newer, finished, or non-release runs.
        self.assertEqual(asked, [1, 2])

    def test_running_optional_build_or_other_work_is_left_alone(self):
        self.assertFalse(gates.only_queued_optional([job(MAC, "in_progress")]))
        self.assertFalse(gates.only_queued_optional([job(MAC, "queued"), job("release", "queued")]))
        self.assertFalse(gates.only_queued_optional([job(X86, "queued")]))
        self.assertFalse(gates.only_queued_optional([job(MAC, "completed", "success")]))
        self.assertTrue(gates.only_queued_optional(self.QUEUED))

    def test_unversioned_tag_supersedes_nothing(self):
        self.assertEqual(gates.superseded(self.RUNS, "v1.0.0-rc.1", 9, lambda run_id: self.QUEUED), [])


class WorkflowWiringTests(unittest.TestCase):
    def test_release_workflow_uses_both_gates(self):
        workflow = (ROOT / ".github/workflows/release.yml").read_text()
        self.assertIn("ci/release-run-gates.py supersede", workflow)
        self.assertIn("ci/release-run-gates.py wait-linux", workflow)
        self.assertIn("needs: [preflight, linux-published]", workflow)
        self.assertIn("actions: write", workflow)


if __name__ == "__main__":
    unittest.main()
