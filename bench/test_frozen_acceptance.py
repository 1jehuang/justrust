import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location(
    "frozen_acceptance", Path(__file__).with_name("frozen-acceptance.py"))
fa = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(fa)


class Sink:
    def __init__(self):
        self.buffer = io.BytesIO()


class FrozenAcceptanceTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(dir=os.environ.get("JCODE_SCRATCH_DIR"))
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.client = self.root / "frozen-client"
        self.client.write_bytes(b"verified fixture executable")
        self.client.chmod(0o700)
        self.sha = fa.digest(self.client)
        self.runs = self.root / "runs"
        self.runs.mkdir()
        self.out = self.root / "outputs"
        self.env = {"JCODE_SESSION_ID": "assigned-session"}
        self.calls = []
        self.sink = Sink()

    def meta(self, rid, args, **changes):
        data = {"id": rid, "args": args, "agent_session": "assigned-session",
                "place": "hosted", "toolchain": "pinned", "ncpu": 24}
        data.update(changes)
        path = self.runs / rid
        path.mkdir()
        (path / "meta.json").write_text(json.dumps(data))

    def fake(self, command, **kwargs):
        self.calls.append((command, kwargs))
        self.meta(str(len(self.calls)), command[1:])
        return subprocess.CompletedProcess(command, 0, b"full diagnostics\n")

    def accept(self, run=None):
        with patch.object(fa.subprocess, "run", side_effect=run or self.fake), \
                patch.object(fa.sys, "stdout", self.sink):
            return fa.accept(self.client, self.sha, self.root, self.runs,
                             self.out, self.env, "pinned", 5)

    def receipt(self):
        return json.loads((self.out / "receipt.json").read_text())

    def test_complete_exact_command_order_and_full_logs(self):
        r = self.accept()
        self.assertTrue(r["complete"])
        self.assertEqual([x[0][1:] for x in self.calls], [x[1] for x in fa.WORKFLOWS])
        self.assertEqual({x[0][0] for x in self.calls}, {str(self.client)})
        for _, kwargs in self.calls:
            self.assertEqual(kwargs["env"]["JUSTRUST_NO_UPDATE"], "1")
            self.assertNotIn("JUSTRUST_RUN_ID_OVERRIDE", kwargs["env"])
            self.assertEqual(kwargs["timeout"], 5)
            self.assertEqual(kwargs["stdin"], subprocess.DEVNULL)
        self.assertEqual(self.sink.buffer.getvalue(), b"full diagnostics\n" * 4)
        self.assertEqual(len(list(self.out.glob("*.log"))), 4)
        self.assertEqual(self.receipt()["client_sha256"], self.sha)
        self.assertEqual(self.out.stat().st_mode & 0o777, 0o700)

    def test_initial_wrong_digest_runs_nothing(self):
        self.sha = "0" * 64
        with self.assertRaisesRegex(ValueError, "digest mismatch"):
            self.accept()
        self.assertEqual(self.calls, [])
        self.assertFalse(self.out.exists())

    def test_symlink_client_refused(self):
        alias = self.root / "alias"
        alias.symlink_to(self.client)
        with self.assertRaisesRegex(ValueError, "symlink"):
            fa.verify_client(alias, self.sha)

    def test_nonexecutable_or_malformed_digest_refused(self):
        self.client.chmod(0o600)
        with self.assertRaises(ValueError):
            fa.verify_client(self.client, self.sha)
        with self.assertRaises(ValueError):
            fa.verify_client(self.client, "X" * 64)

    def test_override_agent_or_missing_session_refused(self):
        for env in ({"JCODE_SESSION_ID": "s", "JUSTRUST_RUN_ID_OVERRIDE": "id"},
                    {"JCODE_SESSION_ID": "s", "JUSTRUST_ON_AGENT": "1"}, {}):
            with self.subTest(env=env), self.assertRaises(ValueError):
                fa.verify_environment(env)

    def test_binary_changed_during_workflow_retained(self):
        def mutate(command, **kwargs):
            result = self.fake(command, **kwargs)
            self.client.write_bytes(b"replaced client")
            return result
        with self.assertRaisesRegex(ValueError, "digest mismatch"):
            self.accept(mutate)
        self.assertFalse(self.receipt()["complete"])
        self.assertEqual(len(self.calls), 1)
        self.assertEqual((self.out / "check.log").read_bytes(), b"full diagnostics\n")

    def test_local_fallback_rejected(self):
        self.meta("a", ["check"], place="local")
        with self.assertRaisesRegex(ValueError, "actually run hosted"):
            fa.recorded_run(self.runs, set(), ["check"], "assigned-session", "pinned")

    def test_wrong_compiler_rejected(self):
        self.meta("a", ["check"], toolchain="system")
        with self.assertRaisesRegex(ValueError, "compiler selection"):
            fa.recorded_run(self.runs, set(), ["check"], "assigned-session", "pinned")

    def test_missing_ambiguous_and_mismatched_run_rejected(self):
        with self.assertRaisesRegex(ValueError, "exactly one"):
            fa.recorded_run(self.runs, set(), ["check"], "assigned-session", "pinned")
        self.meta("a", ["check"])
        self.meta("b", ["check"])
        with self.assertRaisesRegex(ValueError, "exactly one"):
            fa.recorded_run(self.runs, set(), ["check"], "assigned-session", "pinned")
        with self.assertRaisesRegex(ValueError, "identity mismatch"):
            self.meta("c", ["check"], id="wrong")
            fa.recorded_run(self.runs, {"a", "b"}, ["check"], "assigned-session", "pinned")

    def test_foreign_and_previous_records_do_not_contaminate(self):
        self.meta("old", ["check"])
        self.meta("foreign", ["check"], agent_session="other")
        self.meta("own", ["check"])
        r = fa.recorded_run(self.runs, {"old"}, ["check"], "assigned-session", "pinned")
        self.assertEqual(r["id"], "own")

    def test_failing_workflow_retains_full_output_and_exit(self):
        def fail(command, **kwargs):
            r = self.fake(command, **kwargs)
            r.returncode = 101
            r.stdout = b"error: expected compiler failure\n"
            return r
        with self.assertRaisesRegex(ValueError, "workflow failed"):
            self.accept(fail)
        r = self.receipt()
        self.assertFalse(r["complete"])
        self.assertEqual(r["steps"][0]["exit"], 101)
        self.assertEqual(r["steps"][0]["run"]["place"], "hosted")
        self.assertEqual(self.sink.buffer.getvalue(), b"error: expected compiler failure\n")

    def test_timeout_retains_partial_output(self):
        def timeout(command, **kwargs):
            raise subprocess.TimeoutExpired(command, 5, output=b"partial diagnostic\n")
        with self.assertRaisesRegex(ValueError, "timed out"):
            self.accept(timeout)
        r = self.receipt()
        self.assertFalse(r["complete"])
        self.assertTrue(r["steps"][0]["timed_out"])
        self.assertEqual((self.out / "check.log").read_bytes(), b"partial diagnostic\n")

    def test_existing_output_directory_is_never_overwritten(self):
        self.out.mkdir()
        sentinel = self.out / "private"
        sentinel.write_bytes(b"keep")
        with self.assertRaises(FileExistsError):
            self.accept()
        self.assertEqual(sentinel.read_bytes(), b"keep")
        self.assertFalse(self.calls)

    def test_public_cli_help_and_digest_failure(self):
        command = [sys.executable, str(Path(fa.__file__))]
        help_result = subprocess.run(command + ["--help"], capture_output=True, check=False)
        self.assertEqual(help_result.returncode, 0)
        failure = subprocess.run(command + ["--client", str(self.client), "--sha256", "0" * 64,
                                           "--repo", str(self.root), "--out-dir", str(self.out),
                                           "--toolchain", "pinned"],
                                 capture_output=True, check=False)
        self.assertEqual(failure.returncode, 1)
        self.assertIn(b"digest mismatch", failure.stderr)
        self.assertFalse(self.out.exists())


if __name__ == "__main__":
    unittest.main()
