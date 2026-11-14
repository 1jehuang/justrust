#!/usr/bin/env python3
"""Opt-in SSH retirement acceptance for one explicit existing hosted lease.

OFFLINE tests do not run this entry point. Live use requires coordinator readiness,
JCODE_API_KEY in the environment, an explicit candidate and expected lease hash.
No builds, deployment, default-client changes, stop/release calls, or other leases.
Run only under the coordinator's exclusive canary-account fence: /connect has no
atomic lease precondition, so an external concurrent lease replacement cannot be
made safe by a client-side check. Exact-lease rollback preflights every run.
The candidate's pinned TLS accept operation obtains the supervisor receipt. The
account ack is not sent until every receipt binding matches. A finally block
always rolls back that exact lease and attempts fresh legacy pinned SSH auth.
"""
import argparse
import importlib.util
import ipaddress
import json
import math
import os
from pathlib import Path
import re
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
from urllib.parse import urlsplit

SPEC = importlib.util.spec_from_file_location(
    "native_hosted_acceptance", Path(__file__).with_name("native-hosted-acceptance.py"))
HOSTED = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(HOSTED)


def require(value, message):
    if not value:
        raise RuntimeError(message)


def hex64(value):
    return isinstance(value, str) and re.fullmatch(r"[0-9a-f]{64}", value) is not None


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):
        raise RuntimeError("control-plane redirect refused")


class API:
    def __init__(self, base, key, timeout):
        parsed = urlsplit(base)
        require(parsed.scheme == "https" and parsed.hostname and not parsed.username
                and not parsed.password and not parsed.query and not parsed.fragment,
                "an explicit HTTPS build API base is required")
        require(key and not any(c.isspace() or ord(c) < 32 for c in key),
                "JCODE_API_KEY must be supplied privately in the environment")
        self.base, self.key, self.timeout = base.rstrip("/"), key, timeout
        self.opener = urllib.request.build_opener(NoRedirect())

    def post(self, path, body):
        request = urllib.request.Request(self.base + "/" + path,
            data=json.dumps(body).encode(), method="POST",
            headers={"Authorization": "Bearer " + self.key, "Content-Type": "application/json",
                     "User-Agent": "justrust-native-acceptance/1"})
        try:
            with self.opener.open(request, timeout=self.timeout) as response:
                result = json.loads(response.read(1024 * 1024))
        except urllib.error.HTTPError as error:
            code = error.code
            error.close()
            raise RuntimeError(f"control-plane request failed (HTTP {code})") from None
        except Exception:
            raise RuntimeError("control-plane request failed (details suppressed for credential safety)") from None
        require(isinstance(result, dict), "invalid control-plane response")
        return result


def port_observation(address, port, timeout):
    # Numeric IPs avoid unbounded DNS resolution and prevent arbitrary SSH options.
    ipaddress.ip_address(address)
    try:
        connection = socket.create_connection((address, port), timeout=timeout)
    except ConnectionRefusedError:
        return "closed"
    except OSError:
        return "unknown"  # A timeout/unreachable route is NOT proof of retirement.
    with connection:
        connection.settimeout(timeout)
        try:
            banner = connection.recv(255)
        except OSError:
            return "open"
        return "banner" if banner.startswith(b"SSH-2.0-") else "open"


def wait_port(address, port, desired, timeout, probe_timeout=1):
    deadline = time.monotonic() + timeout
    consecutive = 0
    while time.monotonic() < deadline:
        remaining = deadline - time.monotonic()
        observed = port_observation(address, port, min(probe_timeout, max(0.001, remaining)))
        consecutive = consecutive + 1 if observed == desired else 0
        if consecutive >= 3:
            return
        remaining = deadline - time.monotonic()
        if remaining > 0:
            time.sleep(min(0.25, remaining))
    raise RuntimeError("actual public SSH port did not reach required " + desired + " state")


def validate_ready(ready):
    ipaddress.ip_address(ready.get("address", ""))
    require(type(ready.get("port")) is int and 0 < ready["port"] < 65536,
            "invalid public SSH port")
    require(ready.get("user") == "ubuntu", "unexpected hosted SSH user")
    keys = ready.get("host_keys")
    require(isinstance(keys, list) and keys and all(isinstance(k, str)
            and re.fullmatch(r"(?:ssh-[\w-]+|ecdsa-[\w-]+) [A-Za-z0-9+/=]+(?: [^\r\n]*)?", k)
            for k in keys), "missing or unsafe API-pinned host keys")


def validate_offer(ready, lease):
    offer, native = ready.get("native_only"), ready.get("native")
    require(isinstance(offer, dict) and isinstance(native, dict), "native-only capability not offered")
    require(offer.get("lease_id") == lease and hex64(lease), "lease changed, refusing promotion")
    require(hex64(offer.get("nonce")) and hex64(offer.get("certificate_sha256"))
            and native.get("certificate_sha256") == offer["certificate_sha256"]
            and isinstance(offer.get("account_id"), str) and offer["account_id"]
            and offer.get("acknowledged") is False
            and type(offer.get("expires_at")) is int
            and time.time() < offer["expires_at"] <= time.time() + 300,
            "invalid, expired, or already acknowledged native-only offer")
    require(native.get("address") == ready["address"]
            and native.get("server_name") == f"slot-{lease[:32]}.{lease[32:]}.pool.jcode.invalid"
            and type(native.get("port")) is int and 0 < native["port"] < 65536,
            "native endpoint is not bound to the explicit lease")
    return offer


def validate_receipt(receipt, offer):
    require(receipt.get("accepted") is True and all(receipt.get(key) == offer[key]
            for key in ("account_id", "lease_id", "nonce", "certificate_sha256")),
            "supervisor receipt does not match exact account, lease, nonce and pin")


class Retirement:
    def __init__(self, candidate, root, api, lease, timeout):
        self.fixture = HOSTED.Acceptance(candidate, root, None, timeout)
        self.fixture.env["JUSTRUST_HOSTED_NATIVE_ONLY"] = "1"
        self.api, self.lease, self.timeout = api, lease, timeout
        self.key = root / "identity"
        self.ready = None

    def command(self, args, env=None):
        # Never echo argv, endpoints, subprocess streams, keys, API bodies or receipts.
        try:
            result = subprocess.run(args, capture_output=True, text=True,
                env=HOSTED.direct_environment(self.fixture.env, self.fixture.home) if env is None else env,
                timeout=self.timeout, stdin=subprocess.DEVNULL)
        except Exception:
            raise RuntimeError("acceptance subprocess failed (private details suppressed)") from None
        require(result.returncode == 0, "acceptance subprocess returned failure (private details suppressed)")
        return result.stdout

    def connect(self, native_only):
        public = Path(str(self.key) + ".pub").read_text().strip()
        deadline = time.monotonic() + self.timeout
        while time.monotonic() < deadline:
            ready = self.api.post("build/host/connect", {
                "public_key": public, "platform": "linux",
                "native_transport": native_only, "native_only": native_only})
            if ready.get("ready") is True:
                validate_ready(ready)
                return ready
            require(ready.get("state") == "restoring",
                    "existing lease is not ready, refusing provisioning/recovery polling")
            time.sleep(min(1, max(0, deadline - time.monotonic())))
        raise RuntimeError("connect readiness timed out")

    def legacy(self):
        # New identity and no ControlMaster/agent reuse prove fresh authentication,
        # not survival of a connection admitted before the SSH retirement.
        for path in (self.key, Path(str(self.key) + ".pub"), Path(str(self.key) + "-cert.pub")):
            path.unlink(missing_ok=True)
        self.command(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-f", str(self.key)])
        ready = self.connect(False)
        if self.ready is not None:
            require((ready["address"], ready["port"]) == (self.ready["address"], self.ready["port"]),
                    "legacy restoration moved to a different public SSH endpoint")
        wait_port(ready["address"], ready["port"], "banner", self.timeout)
        known = self.fixture.root / "known_hosts"
        host = ready["address"] if ready["port"] == 22 else f'[{ready["address"]}]:{ready["port"]}'
        known.write_text("".join(host + " " + key + "\n" for key in ready["host_keys"]))
        options = ["-F", "/dev/null", "-i", str(self.key), "-p", str(ready["port"])]
        cert = ready.get("ssh_certificate")
        if cert is not None:
            require(isinstance(cert, str) and cert and "\n" not in cert and "\r" not in cert,
                    "invalid fresh SSH certificate")
            cert_path = Path(str(self.key) + "-cert.pub")
            cert_path.write_text(cert)
            options += ["-o", "CertificateFile=" + str(cert_path)]
        for option in ("BatchMode=yes", "IdentitiesOnly=yes", "IdentityAgent=none",
                       "ControlMaster=no", "ControlPath=none", "ControlPersist=no",
                       "StrictHostKeyChecking=yes", "GlobalKnownHostsFile=/dev/null",
                       "UserKnownHostsFile=" + str(known), "ConnectTimeout=5",
                       "ProxyCommand=none", "ProxyJump=none", "LogLevel=ERROR"):
            options += ["-o", option]
        ssh = shutil.which("ssh")
        require(ssh is not None, "system SSH unavailable")
        output = self.command([ssh, *options, "ubuntu@" + ready["address"], "printf NATIVE_RETIREMENT_LEGACY_OK"])
        require(output == "NATIVE_RETIREMENT_LEGACY_OK", "fresh pinned SSH authentication failed")

    def execute(self):
        try:
            # Non-allocating exact-lease account preflight. Never call /connect
            # on an absent/replaced lease. This also starts from SSH-enabled state.
            preflight = self.api.post("build/host/native-only", {"action": "rollback", "lease_id": self.lease})
            require(preflight.get("native_only") is False and preflight.get("ssh") == "restoring",
                    "existing exact-lease preflight failed")
            self.command(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-f", str(self.key)])
            # A positive baseline rules out networks that simply block SSH.
            self.ready = self.connect(True)
            offer = validate_offer(self.ready, self.lease)
            wait_port(self.ready["address"], self.ready["port"], "banner", self.timeout)
            endpoint = dict(self.ready["native"], key_path=str(self.key),
                            resume_path=str(self.fixture.home / "retirement resume"))
            private_env = HOSTED.direct_environment(self.fixture.env, self.fixture.home)
            raw = self.command([str(self.fixture.candidate), "__native", "accept",
                                json.dumps(endpoint), offer["nonce"]], private_env)
            try:
                receipt = json.loads(raw)
                validate_receipt(receipt, offer)
            except Exception:
                raise RuntimeError("invalid or mismatched supervisor acceptance receipt") from None
            ack = self.api.post("build/host/native-only", dict(action="ack",
                **{key: offer[key] for key in ("lease_id", "nonce", "certificate_sha256")}))
            require(ack.get("native_only") is True and ack.get("lease_id") == self.lease,
                    "exact lease acknowledgement failed")
            wait_port(self.ready["address"], self.ready["port"], "closed", self.timeout)
            output = self.command([str(self.fixture.candidate), "__native", "client",
                json.dumps(endpoint), "printf NATIVE_AFTER_SSH_CLOSED"], private_env)
            require(output == "NATIVE_AFTER_SSH_CLOSED", "native command failed after actual SSH closure")
        finally:
            # Even failed accept/ack/port/native gates request exact-lease rollback.
            # Lease changes fail closed in the API, never rollback someone else's.
            try:
                rollback = self.api.post("build/host/native-only", {"action": "rollback", "lease_id": self.lease})
                require(rollback.get("native_only") is False and rollback.get("ssh") == "restoring",
                        "exact-lease rollback was not accepted")
            finally:
                # Do not connect if rollback failed, which could touch a replacement lease.
                if 'rollback' in locals() and rollback.get("native_only") is False and rollback.get("ssh") == "restoring":
                    self.legacy()
        return {"candidate_sha256": self.fixture.candidate_sha256,
                "supervisor_receipt_and_ack": True, "public_ssh_refused": True,
                "native_command_after_closure": True, "rollback_fresh_pinned_ssh": True}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--expected-lease", required=True, help="explicit existing lease hash, never printed")
    parser.add_argument("--build-base", required=True, help="explicit HTTPS API base including /v1")
    parser.add_argument("--timeout", type=float, default=60, help="bounded per-phase timeout in seconds")
    args = parser.parse_args(argv)
    require(math.isfinite(args.timeout) and 0 < args.timeout <= 300, "timeout must be in (0, 300]")
    require(hex64(args.expected_lease), "expected lease must be a lowercase SHA256 hash")
    candidate = args.candidate.resolve(strict=True)
    require(candidate.is_file() and os.access(candidate, os.X_OK), "explicit executable candidate required")
    api = API(args.build_base, os.environ.get("JCODE_API_KEY"), min(args.timeout, 10))
    with tempfile.TemporaryDirectory(prefix="justrust-native-retirement-") as directory:
        root = Path(directory)
        root.chmod(0o700)
        retirement = Retirement(candidate, root, api, args.expected_lease, args.timeout)
        result = retirement.execute()
    print(json.dumps(result))


if __name__ == "__main__":
    try:
        main()
    except Exception:
        print("Native-only acceptance FAILED. Rollback/restoration may need coordinator attention. "
              "Private exception details suppressed. No success claimed.", file=sys.stderr)
        sys.exit(1)
