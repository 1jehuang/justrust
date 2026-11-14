#!/usr/bin/env python3
"""Offline tests for the native hosted acceptance driver's safety contracts."""
import importlib.util
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location(
    "native_hosted_acceptance", Path(__file__).with_name("native-hosted-acceptance.py"))
DRIVER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(DRIVER)


class NativeAcceptanceTests(unittest.TestCase):
    def test_environment_requires_native_and_preserves_credentials_only_in_environment(self):
        with patch.dict(os.environ, {"JUSTRUST_HOME": "/normal/state",
                                    "JUSTRUST_REMOTE": "off",
                                    "JUSTRUST_HOSTED_NATIVE": "off",
                                    "JUSTRUST_BUILD_BASE": "https://wrong.example",
                                    "CARGO_TARGET_DIR": "/normal/target",
                                    "RUSTC_WRAPPER": "/normal/wrapper",
                                    "JCODE_API_KEY": "fixture-credential"}):
            env = DRIVER.environment(Path("/isolated"), Path("/guard"))
        self.assertEqual(env["JUSTRUST_HOME"], "/isolated")
        self.assertEqual(env["JUSTRUST_HOSTED_NATIVE"], "require")
        self.assertEqual(env["JUSTRUST_REMOTE"], "force")
        self.assertEqual(env["JUSTRUST_HOSTED_SESSION"], "1")
        self.assertEqual(env["JCODE_API_KEY"], "fixture-credential")
        self.assertNotIn("JUSTRUST_BUILD_BASE", env)
        self.assertNotIn("CARGO_TARGET_DIR", env)
        self.assertNotIn("RUSTC_WRAPPER", env)
        self.assertTrue(env["PATH"].startswith("/guard" + os.pathsep))

    def test_record_requires_single_new_hosted_execution_and_expected_status(self):
        row = {"id": "new", "cwd": "/fixture", "args": ["test", "--lib"],
               "exit": 0, "place": "hosted"}
        self.assertEqual(DRIVER.select_record([row], set(), Path("/fixture"), "test", 0), "new")
        for rows, seen, exit_code in [
            ([row, dict(row, id="duplicate")], set(), 0),
            ([row], {"new"}, 0),
            ([dict(row, place="local")], set(), 0),
            ([dict(row, place="remote")], set(), 0),
            ([row], set(), 101),
        ]:
            with self.assertRaises(RuntimeError):
                DRIVER.select_record(rows, seen, Path("/fixture"), "test", exit_code)

    def test_session_identity_handles_shell_quoted_paths(self):
        script = 'export PATH="$HOME/bin:$PATH"; exec '\
                 "'/private/pin with '\\''quote/justrust' __remote-session create abc123 ''"
        self.assertEqual(DRIVER.session_script(script), ("create", "abc123"))
        self.assertEqual(DRIVER.session_script(script.replace("create", "attach")), ("attach", "abc123"))
        for invalid in ("true", "__remote-session", "__remote-session serve abc123", "'unterminated"):
            self.assertIsNone(DRIVER.session_script(invalid))

    def test_socket_and_process_selection_is_scoped_to_private_home(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            home = root / "private state"
            for pid, state_home, action in [(123, home, "client"),
                                            (124, root / "normal", "client"),
                                            (125, home, "rsync")]:
                proc = root / str(pid)
                (proc / "fd").mkdir(parents=True)
                (proc / "net").mkdir()
                (proc / "environ").write_bytes(b"JUSTRUST_HOME=" + os.fsencode(state_home) + b"\0")
                args = ["/candidate", "__native", action, json.dumps({"port": 443}),
                        "exec /private/pin/justrust __remote-session create token ''"]
                (proc / "cmdline").write_bytes("\0".join(args).encode() + b"\0")
                (proc / "stat").write_text(f"{pid} (native ) client) S " + " ".join(["0"] * 18 + ["54321"]))
                (proc / "fd/7").symlink_to("socket:[777]")
                (proc / "net/tcp").write_text(
                    "header\n0: 0100007F:1234 0100007F:01BB 01 0:0 00:0 0 1000 0 777\n")
            self.assertEqual(DRIVER.native_sessions(home, True, root), [(123, "54321", "create", "token")])
            (root / "123/net/tcp").write_text("header\n")
            self.assertEqual(DRIVER.native_sessions(home, True, root), [])
            self.assertEqual(len(DRIVER.native_sessions(home, False, root)), 1)

    def test_drop_refuses_missing_process_before_signalling(self):
        with patch.object(DRIVER, "native_sessions", return_value=[]), \
                patch.object(os, "pidfd_open") as open_pid:
            with self.assertRaises(RuntimeError):
                DRIVER.drop_session(Path("/private"))
            open_pid.assert_not_called()

    @unittest.skipUnless(sys.platform == "linux", "requires real Linux /proc socket tables")
    def test_established_socket_observes_real_connection_owned_by_process(self):
        proc = Path("/proc") / str(os.getpid())
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            listener.listen()
            port = listener.getsockname()[1]
            with socket.create_connection(("127.0.0.1", port)) as client:
                accepted, _ = listener.accept()
                with accepted:
                    self.assertTrue(DRIVER.established_socket(proc, port))
                    self.assertGreaterEqual(client.fileno(), 0)
            self.assertFalse(DRIVER.established_socket(proc, port))

    def test_drop_refuses_reused_process_before_signalling(self):
        with patch.object(DRIVER, "native_sessions", return_value=[(123, "old", "create", "token")]), \
                patch.object(DRIVER, "process_start", return_value="new"), \
                patch.object(os, "pidfd_open") as open_pid:
            with self.assertRaises(RuntimeError):
                DRIVER.drop_session(Path("/private"))
            open_pid.assert_not_called()

    def test_fixture_generates_bulk_edits_and_ssh_guard_never_runs_real_ssh(self):
        with tempfile.TemporaryDirectory(prefix="native 'fixture'-") as directory:
            fixture = DRIVER.Acceptance(Path("/bin/true"), Path(directory), None, 5)
            self.assertNotEqual(fixture.candidate, Path("/bin/true"))
            self.assertIn("'quotes'", fixture.candidate.name)
            self.assertEqual(DRIVER.digest_file(fixture.candidate), DRIVER.digest_file(Path("/bin/true")))
            fixture.write_project(1)
            self.assertEqual(len(list((fixture.project / "bulk").iterdir())), 305)
            first = (fixture.project / "src/quoted ' payload.bin").read_bytes()
            fixture.write_project(2)
            second = (fixture.project / "src/quoted ' payload.bin").read_bytes()
            self.assertNotEqual(first, second)
            self.assertEqual(len(first), len(second))
            self.assertIn("sync_all", (fixture.project / "src/lib.rs").read_text())
            result = subprocess.run([str(fixture.guard / "ssh"), "never-connect"], capture_output=True)
            self.assertEqual(result.returncode, 125)
            self.assertTrue(fixture.ssh_used.exists())
            with self.assertRaises(RuntimeError):
                fixture.run("--version")


if __name__ == "__main__":
    unittest.main()
