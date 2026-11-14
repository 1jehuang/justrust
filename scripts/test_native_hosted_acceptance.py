#!/usr/bin/env python3
"""Offline tests for the native hosted acceptance driver's safety contracts."""
import importlib.util
import io
import json
import os
from pathlib import Path
import pwd
import socket
import subprocess
import sys
import tempfile
import unittest
from contextlib import redirect_stdout
from types import SimpleNamespace
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location(
    "native_hosted_acceptance", Path(__file__).with_name("native-hosted-acceptance.py"))
DRIVER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(DRIVER)


class NativeAcceptanceTests(unittest.TestCase):
    def test_direct_environment_cannot_refresh_control_plane_credentials(self):
        original = {"JCODE_API_KEY": "do-not-propagate", "JCODE_HOME": "/normal/config",
                    "JUSTRUST_HOME": "/isolated", "PATH": "/guard:/usr/bin"}
        env = DRIVER.direct_environment(original, Path("/isolated"))
        self.assertNotIn("JCODE_API_KEY", env)
        self.assertEqual(env["JCODE_HOME"], "/isolated/no-control-plane-credentials")
        self.assertEqual(env["JCODE_API_BASE"], "https://127.0.0.1:9")
        self.assertEqual(env["JUSTRUST_BUILD_BASE"], "https://127.0.0.1:9")
        self.assertEqual(original["JCODE_API_KEY"], "do-not-propagate")

    def test_expiry_wait_cannot_finish_before_65_seconds(self):
        with patch.object(DRIVER.time, "monotonic", side_effect=[0, 0, 64, 65, 65.1]), \
                patch.object(DRIVER.time, "sleep") as sleep:
            self.assertGreaterEqual(DRIVER.wait_fresh_key_expiry(), 65)
        self.assertEqual([call.args[0] for call in sleep.call_args_list], [65, 1])

    def security_probe(self, uid=1000, user="ubuntu", secret_error=None, content=b"BOOT_IMAGE=/vmlinuz"):
        output = io.StringIO()
        if secret_error is None:
            secret_error = PermissionError("private root secret")
        with patch.object(os, "geteuid", return_value=uid), \
                patch.object(pwd, "getpwuid", return_value=SimpleNamespace(pw_name=user)), \
                patch.object(os, "open", side_effect=secret_error), \
                patch.object(Path, "read_bytes", return_value=content), \
                redirect_stdout(output):
            exec(DRIVER.GUEST_SECURITY_PROBE, {})
        return json.loads(output.getvalue())

    def test_guest_probe_requires_positive_permission_denial_and_checks_kernel_views(self):
        receipt = self.security_probe()
        self.assertEqual(receipt["uid"], 1000)
        self.assertEqual(receipt["user"], "ubuntu")
        self.assertEqual(receipt["root_secrets_denied"], 2)
        self.assertEqual(receipt["kernel_views"], ["/proc/cmdline", "/proc/bootconfig"])

    def test_guest_probe_rejects_root_wrong_user_and_missing_secrets(self):
        for options in ({"uid": 0}, {"user": "root"},
                        {"secret_error": FileNotFoundError("absent is not permission denial")}):
            with self.subTest(options=options), self.assertRaises(RuntimeError):
                self.security_probe(**options)

    def test_guest_probe_never_accepts_readable_secret(self):
        with patch.object(os, "geteuid", return_value=1000), \
                patch.object(pwd, "getpwuid", return_value=SimpleNamespace(pw_name="ubuntu")), \
                patch.object(os, "open", return_value=123), \
                patch.object(os, "close") as close:
            with self.assertRaisesRegex(RuntimeError, "can open"):
                exec(DRIVER.GUEST_SECURITY_PROBE, {})
            close.assert_called_once_with(123)

    def test_guest_probe_rejects_kernel_token_without_echoing_it(self):
        for content in (b"pool.token=do-not-echo-token", b"pool.api=https://example.invalid"):
            with self.subTest(content=content), self.assertRaises(RuntimeError) as error:
                self.security_probe(content=content)
            self.assertNotIn("do-not-echo-token", str(error.exception))

    def expiry_fixture(self, directory):
        fixture = DRIVER.Acceptance(Path("/bin/true"), Path(directory), None, 5)
        key = fixture.home / "fresh identity"
        key.write_bytes(b"fixture key, never used outside mocked client")
        endpoint = {"address": "127.0.0.1", "port": 443, "server_name": "fixture",
                    "certificate_sha256": "ab" * 32, "key_path": str(key),
                    "resume_path": str(fixture.home / "normal resume")}
        state = fixture.home / "remote/hosted/state.json"
        state.parent.mkdir(parents=True)
        state.write_text(json.dumps({"native": endpoint}))
        return fixture, endpoint

    def test_expiry_gate_uses_direct_resume_without_fresh_key_or_api_refresh(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture, endpoint = self.expiry_fixture(directory)
            calls = []

            def direct(target, script, expected=0):
                calls.append((target, script, expected))
                if "NATIVE_SECURITY_PROBE" in script:
                    stdout = json.dumps({"user": "ubuntu", "uid": 1000,
                                         "root_secrets_denied": 2, "kernel_views": ["/proc/cmdline"]})
                    return SimpleNamespace(stdout=stdout, stderr="")
                if target["certificate_sha256"] != endpoint["certificate_sha256"]:
                    return SimpleNamespace(stdout="", stderr="native certificate pin mismatch")
                if target["resume_path"].endswith("unenrolled after expiry"):
                    return SimpleNamespace(stdout="", stderr="native authentication explicitly rejected before execution")
                return SimpleNamespace(stdout="NATIVE_EXPIRED_KEY_RESUME_OK", stderr="")

            with patch.object(fixture, "direct", side_effect=direct), \
                    patch.object(fixture, "remote", side_effect=AssertionError("API refresh forbidden")), \
                    patch.object(DRIVER, "wait_fresh_key_expiry", return_value=65.1) as wait:
                receipt = fixture.expiry_and_security(endpoint)
            wait.assert_called_once()
            self.assertTrue(receipt["direct_resume_after_expiry"])
            self.assertTrue(receipt["wrong_pin_failed_closed"])
            self.assertEqual(len(calls), 7)
            fresh = [call for call in calls if call[0]["resume_path"].endswith("unenrolled after expiry")]
            self.assertEqual(fresh[0][0]["key_path"], endpoint["key_path"])
            self.assertEqual(fresh[0][2], 1)
            resumed = calls[-1]
            self.assertTrue(resumed[0]["key_path"].endswith("fresh-key-must-not-be-used"))
            self.assertIn("test ! -e /tmp/justrust-native-expired-key-", resumed[1])

    def test_wrong_pin_gate_does_not_accept_unrelated_network_failure(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture, endpoint = self.expiry_fixture(directory)
            security = json.dumps({"user": "ubuntu", "uid": 1000,
                                   "root_secrets_denied": 2, "kernel_views": ["/proc/cmdline"]})
            with patch.object(fixture, "direct", side_effect=[
                    SimpleNamespace(stdout="", stderr=""), SimpleNamespace(stdout=security, stderr=""),
                    SimpleNamespace(stdout="", stderr=""), SimpleNamespace(stdout="", stderr="connection refused")]), \
                    patch.object(DRIVER, "wait_fresh_key_expiry") as wait:
                with self.assertRaisesRegex(RuntimeError, "specifically on pin"):
                    fixture.expiry_and_security(endpoint)
                wait.assert_not_called()

    def test_expiry_gate_refuses_shortened_wait_before_testing_expired_key(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture, endpoint = self.expiry_fixture(directory)
            security = json.dumps({"user": "ubuntu", "uid": 1000,
                                   "root_secrets_denied": 2, "kernel_views": ["/proc/cmdline"]})
            replies = [SimpleNamespace(stdout="", stderr=""), SimpleNamespace(stdout=security, stderr=""),
                       SimpleNamespace(stdout="", stderr=""), SimpleNamespace(stdout="", stderr="pin mismatch"),
                       SimpleNamespace(stdout="", stderr="")]
            with patch.object(fixture, "direct", side_effect=replies) as direct, \
                    patch.object(DRIVER, "wait_fresh_key_expiry", return_value=64.9):
                with self.assertRaisesRegex(RuntimeError, "required 65 seconds"):
                    fixture.expiry_and_security(endpoint)
                self.assertEqual(direct.call_count, 5)

    def test_direct_client_wires_credential_free_environment_to_candidate(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture, endpoint = self.expiry_fixture(directory)
            fixture.env["JCODE_API_KEY"] = "not-forwarded"
            with patch.object(fixture, "run") as run:
                fixture.direct(endpoint, "printf test", expected=37)
            self.assertEqual(run.call_args.args[:2], ("__native", "client"))
            self.assertEqual(run.call_args.kwargs["expected"], 37)
            self.assertNotIn("JCODE_API_KEY", run.call_args.kwargs["env"])
            self.assertEqual(run.call_args.kwargs["env"]["JUSTRUST_BUILD_BASE"], "https://127.0.0.1:9")

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
