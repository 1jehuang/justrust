#!/usr/bin/env python3
"""Offline-only safety tests. No external network, deployed APIs or Rust builds."""
import importlib.util
import io
import json
from pathlib import Path
import socket
import subprocess
import tempfile
import threading
import time
import unittest
from contextlib import redirect_stdout, redirect_stderr
from types import SimpleNamespace
from unittest.mock import Mock, patch
import urllib.error

SPEC = importlib.util.spec_from_file_location("native_only_acceptance",
    Path(__file__).with_name("native-only-acceptance.py"))
D = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(D)
LEASE = "a" * 64
PIN = "b" * 64


def ready():
    return {"ready": True, "address": "127.0.0.1", "port": 2222, "user": "ubuntu",
        "host_keys": ["ssh-ed25519 QUJD"],
        "native": {"address": "127.0.0.1", "port": 443,
            "server_name": f"slot-{LEASE[:32]}.{LEASE[32:]}.pool.jcode.invalid",
            "certificate_sha256": PIN},
        "native_only": {"account_id": "account-fixture", "nonce": "c" * 64,
            "lease_id": LEASE, "certificate_sha256": PIN,
            "acknowledged": False, "expires_at": int(time.time()) + 120}}


def receipt():
    return dict(ready()["native_only"], accepted=True)


class RetirementTests(unittest.TestCase):
    def fixture(self, directory):
        api = Mock()
        api.post.side_effect = lambda path, body: (
            {"native_only": True, "lease_id": LEASE} if body.get("action") == "ack"
            else {"native_only": False, "ssh": "restoring"})
        return D.Retirement(Path("/bin/true"), Path(directory), api, LEASE, 1)

    def workflow(self, fixture, fail=None):
        events = []
        def command(args, env=None):
            if "accept" in args:
                events.append("receipt")
                if fail == "receipt":
                    return json.dumps(dict(receipt(), lease_id="d" * 64))
                return json.dumps(receipt())
            if "client" in args:
                events.append("native")
                if fail == "native":
                    raise RuntimeError("native failed")
                self.assertNotIn("JCODE_API_KEY", env)
                self.assertEqual(env["JUSTRUST_HOSTED_NATIVE_ONLY"], "1")
                return "NATIVE_AFTER_SSH_CLOSED"
            return ""
        def port(address, port, desired, timeout):
            events.append(desired)
            if desired == fail:
                raise RuntimeError("port failed")
        def post(path, body):
            events.append(body["action"])
            self.assertEqual(body["lease_id"], LEASE)
            if body["action"] == "ack":
                if fail == "ack":
                    raise RuntimeError("ack failed")
                return {"native_only": True, "lease_id": LEASE}
            if fail == "rollback":
                raise RuntimeError("rollback failed")
            return {"native_only": False, "ssh": "restoring"}
        fixture.api.post.side_effect = post
        with patch.object(fixture, "command", side_effect=command), \
                patch.object(fixture, "connect", return_value=ready()) as connect, \
                patch.object(fixture, "legacy", side_effect=lambda: events.append("legacy")), \
                patch.object(D, "wait_port", side_effect=port):
            if fail:
                with self.assertRaises(RuntimeError):
                    fixture.execute()
            else:
                result = fixture.execute()
                self.assertTrue(result["rollback_fresh_pinned_ssh"])
                self.assertNotIn(LEASE, json.dumps(result))
                self.assertNotIn(PIN, json.dumps(result))
            if fail == "rollback":
                connect.assert_not_called()
        return events

    def test_full_order_requires_receipt_ack_actual_closed_native_and_restoration(self):
        with tempfile.TemporaryDirectory() as directory:
            events = self.workflow(self.fixture(directory))
        self.assertEqual(events, ["rollback", "banner", "receipt", "ack", "closed", "native", "rollback", "legacy"])

    def test_every_failed_gate_rolls_back_and_restores_without_success(self):
        for failure in ("banner", "receipt", "ack", "closed", "native"):
            with self.subTest(failure=failure), tempfile.TemporaryDirectory() as directory:
                events = self.workflow(self.fixture(directory), failure)
                self.assertEqual(events[-2:], ["rollback", "legacy"])
                if failure in ("banner", "receipt"):
                    self.assertNotIn("ack", events)
                if failure in ("banner", "receipt", "ack", "closed"):
                    self.assertNotIn("native", events)

    def test_failed_exact_lease_preflight_never_connects_or_restores_other_lease(self):
        with tempfile.TemporaryDirectory() as directory:
            events = self.workflow(self.fixture(directory), "rollback")
        self.assertEqual(events, ["rollback", "rollback"])

    def test_restore_failure_prevents_success(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = self.fixture(directory)
            with patch.object(fixture, "command", side_effect=["", json.dumps(receipt()), "NATIVE_AFTER_SSH_CLOSED"]), \
                    patch.object(fixture, "connect", return_value=ready()), \
                    patch.object(D, "wait_port"), \
                    patch.object(fixture, "legacy", side_effect=RuntimeError("restore failed")):
                with self.assertRaisesRegex(RuntimeError, "restore failed"):
                    fixture.execute()
            self.assertEqual(fixture.api.post.call_args.args[1]["action"], "rollback")

    def test_final_rollback_failure_never_runs_legacy_connect_or_claims_success(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = self.fixture(directory)
            fixture.api.post.side_effect = [
                {"native_only": False, "ssh": "restoring"},
                {"native_only": True, "lease_id": LEASE}, RuntimeError("lease changed")]
            with patch.object(fixture, "command", side_effect=["", json.dumps(receipt()), "NATIVE_AFTER_SSH_CLOSED"]), \
                    patch.object(fixture, "connect", return_value=ready()), \
                    patch.object(D, "wait_port"), patch.object(fixture, "legacy") as legacy:
                with self.assertRaisesRegex(RuntimeError, "lease changed"):
                    fixture.execute()
                legacy.assert_not_called()

    def test_early_keygen_and_connect_failures_still_attempt_bound_rollback(self):
        for phase in ("command", "connect"):
            with self.subTest(phase=phase), tempfile.TemporaryDirectory() as directory:
                fixture = self.fixture(directory)
                with patch.object(fixture, "command", return_value="") as command, \
                        patch.object(fixture, "connect", return_value=ready()) as connect, \
                        patch.object(fixture, "legacy") as legacy:
                    (command if phase == "command" else connect).side_effect = RuntimeError("early failure")
                    with self.assertRaisesRegex(RuntimeError, "early failure"):
                        fixture.execute()
                    self.assertEqual(fixture.api.post.call_args.args[1], {"action": "rollback", "lease_id": LEASE})
                    legacy.assert_called_once()

    def test_loopback_only_real_banner_then_real_refusal(self):
        # Entirely local offline fixture, never contact hosted or an external IP.
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            listener.listen()
            port = listener.getsockname()[1]
            def send_banner():
                connection, _ = listener.accept()
                with connection:
                    connection.sendall(b"SSH-2.0-offline-fixture\r\n")
            worker = threading.Thread(target=send_banner, daemon=True)
            worker.start()
            try:
                self.assertEqual(D.port_observation("127.0.0.1", port, 1), "banner")
            finally:
                worker.join(timeout=2)
            self.assertFalse(worker.is_alive())
        self.assertEqual(D.port_observation("127.0.0.1", port, 1), "closed")

    def test_receipt_all_bindings_and_literal_accepted_required(self):
        offer = ready()["native_only"]
        D.validate_receipt(receipt(), offer)
        for key in ("account_id", "lease_id", "nonce", "certificate_sha256", "accepted"):
            with self.subTest(key=key), self.assertRaises(RuntimeError):
                D.validate_receipt(dict(receipt(), **{key: "wrong"}), offer)
        with self.assertRaises(RuntimeError):
            D.validate_receipt(dict(receipt(), accepted=1), offer)

    def test_offer_rejects_replaced_lease_pin_expiry_replay_and_wrong_sni(self):
        D.validate_offer(ready(), LEASE)
        for key, value in (("lease_id", "d" * 64), ("certificate_sha256", "d" * 64),
                           ("expires_at", 0), ("acknowledged", True), ("nonce", "invalid")):
            modified = ready()
            modified["native_only"][key] = value
            with self.subTest(key=key), self.assertRaises(RuntimeError):
                D.validate_offer(modified, LEASE)
        modified = ready()
        modified["native"]["server_name"] = "other-slot.invalid"
        with self.assertRaises(RuntimeError):
            D.validate_offer(modified, LEASE)

    def test_unsafe_ssh_address_port_user_and_keys_rejected(self):
        for key, value in (("address", "-oProxyCommand=evil"), ("port", True),
                           ("port", 65536), ("user", "root"), ("host_keys", []),
                           ("host_keys", ["ssh-ed25519 QUJD\n* ssh-ed25519 QUJD"])):
            with self.subTest(key=key), self.assertRaises((RuntimeError, ValueError)):
                D.validate_ready(dict(ready(), **{key: value}))

    def test_network_timeout_and_route_errors_are_not_closed_evidence(self):
        for error in (TimeoutError(), OSError("unreachable")):
            with patch.object(socket, "create_connection", side_effect=error):
                self.assertEqual(D.port_observation("127.0.0.1", 2222, 0.01), "unknown")
        with patch.object(socket, "create_connection", side_effect=ConnectionRefusedError()):
            self.assertEqual(D.port_observation("127.0.0.1", 2222, 0.01), "closed")

    def test_only_real_banner_is_open_ssh_evidence(self):
        for payload, expected in ((b"SSH-2.0-fixture\r\n", "banner"), (b"", "open"), (b"HTTP/1.1", "open")):
            connection = Mock()
            connection.__enter__ = Mock(return_value=connection)
            connection.__exit__ = Mock(return_value=False)
            connection.recv.return_value = payload
            with patch.object(socket, "create_connection", return_value=connection):
                self.assertEqual(D.port_observation("127.0.0.1", 2222, .01), expected)

    def test_poll_requires_three_consecutive_actual_observations(self):
        observations = ["closed", "unknown", "closed", "closed", "closed"]
        with patch.object(D, "port_observation", side_effect=observations) as probe, \
                patch.object(D.time, "sleep"):
            D.wait_port("127.0.0.1", 2222, "closed", 1)
        self.assertEqual(probe.call_count, 5)

    def test_poll_expires_bounded_without_success_on_unknown(self):
        with patch.object(D.time, "monotonic", side_effect=[0, 0, 0, 1, 1]), \
                patch.object(D.time, "sleep"), \
                patch.object(D, "port_observation", return_value="unknown"):
            with self.assertRaises(RuntimeError):
                D.wait_port("127.0.0.1", 2222, "closed", .5)

    def test_connect_sends_exact_native_only_or_legacy_request(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = self.fixture(directory)
            Path(str(fixture.key) + ".pub").write_text("ssh-ed25519 QUJD\n")
            fixture.api.post.side_effect = None
            fixture.api.post.return_value = ready()
            for native in (True, False):
                fixture.connect(native)
                path, body = fixture.api.post.call_args.args
                self.assertEqual(path, "build/host/connect")
                self.assertEqual(body, {"public_key": "ssh-ed25519 QUJD", "platform": "linux",
                    "native_transport": native, "native_only": native})

    def test_legacy_rotates_identity_and_enforces_pinning_without_agent_or_master(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = self.fixture(directory)
            fixture.ready = ready()
            fixture.key.write_text("old key")
            Path(str(fixture.key) + "-cert.pub").write_text("old certificate")
            commands = []
            def command(args, env=None):
                commands.append(args)
                if args[0] == "ssh-keygen":
                    self.assertFalse(fixture.key.exists())
                    self.assertFalse(Path(str(fixture.key) + "-cert.pub").exists())
                    fixture.key.write_text("new key")
                    return ""
                return "NATIVE_RETIREMENT_LEGACY_OK"
            response = ready()
            response["ssh_certificate"] = "ssh-ed25519-cert-v01@openssh.com QUJD"
            with patch.object(fixture, "command", side_effect=command), \
                    patch.object(fixture, "connect", return_value=response) as connect, \
                    patch.object(D, "wait_port") as wait, \
                    patch.object(D.shutil, "which", return_value="/usr/bin/ssh"):
                fixture.legacy()
            connect.assert_called_once_with(False)
            self.assertEqual(wait.call_args.args[2], "banner")
            args = commands[-1]
            for option in ("ControlPath=none", "ControlMaster=no", "StrictHostKeyChecking=yes",
                           "IdentityAgent=none", "IdentitiesOnly=yes", "ProxyCommand=none"):
                self.assertIn(option, args)
            self.assertEqual(args[:3], ["/usr/bin/ssh", "-F", "/dev/null"])
            self.assertIn("[127.0.0.1]:2222 ssh-ed25519 QUJD", (fixture.fixture.root / "known_hosts").read_text())

    def test_legacy_endpoint_change_fails_before_any_ssh_probe(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = self.fixture(directory)
            fixture.ready = ready()
            with patch.object(fixture, "command"), \
                    patch.object(fixture, "connect", return_value=dict(ready(), port=2223)), \
                    patch.object(D, "wait_port") as wait:
                with self.assertRaisesRegex(RuntimeError, "different public SSH endpoint"):
                    fixture.legacy()
                wait.assert_not_called()

    def test_commands_capture_private_output_and_timeout_without_echo(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = self.fixture(directory)
            for result in (SimpleNamespace(returncode=1, stdout="secret", stderr="credential"),
                           subprocess.TimeoutExpired(["private"], 1, output="secret")):
                out = io.StringIO()
                options = {"side_effect": result} if isinstance(result, Exception) else {"return_value": result}
                with patch.object(D.subprocess, "run", **options) as run, \
                        redirect_stdout(out), redirect_stderr(out):
                    with self.assertRaises(RuntimeError) as error:
                        fixture.command(["private", "credential"])
                self.assertEqual(out.getvalue(), "")
                self.assertNotIn("credential", str(error.exception))
                self.assertNotIn("secret", str(error.exception))
                self.assertEqual(run.call_args.kwargs["timeout"], 1)
                self.assertTrue(run.call_args.kwargs["capture_output"])

    def test_api_refuses_redirect_insecure_base_and_credentials_in_url(self):
        for base in ("http://example.invalid/v1", "https://key@example.invalid/v1",
                     "https://example.invalid/v1?key=secret"):
            with self.assertRaises(RuntimeError):
                D.API(base, "private", 1)
        with self.assertRaises(RuntimeError):
            D.NoRedirect().redirect_request(None)

    def test_api_http_failure_never_echoes_body_headers_or_credentials(self):
        api = D.API("https://example.invalid/v1", "private-key", 1)
        with patch.object(api.opener, "open", side_effect=urllib.error.HTTPError(
                "https://example.invalid/private-key", 409, "secret-body", {}, None)):
            with self.assertRaises(RuntimeError) as error:
                api.post("build/host/native-only", {"lease_id": LEASE})
        self.assertEqual(str(error.exception), "control-plane request failed (HTTP 409)")

    def test_main_private_temporary_files_removed_on_failure(self):
        roots = []
        def execute(retirement):
            roots.append(retirement.fixture.root)
            self.assertEqual(retirement.fixture.root.stat().st_mode & 0o777, 0o700)
            self.assertEqual(retirement.fixture.env["JUSTRUST_HOSTED_NATIVE_ONLY"], "1")
            raise RuntimeError("offline failure")
        with patch.dict(D.os.environ, {"JCODE_API_KEY": "private"}), \
                patch.object(D.Retirement, "execute", execute), \
                patch.object(D.API, "post", side_effect=AssertionError("no live APIs")):
            with self.assertRaises(RuntimeError):
                D.main(["--candidate", "/bin/true", "--expected-lease", LEASE,
                        "--build-base", "https://example.invalid/v1"])
        self.assertTrue(roots)
        self.assertFalse(roots[0].exists())


if __name__ == "__main__":
    unittest.main()
