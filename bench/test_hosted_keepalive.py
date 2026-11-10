"""Bounded lease renewal, with no live API or builds in the test harness."""
from pathlib import Path
import os
import subprocess
import tempfile
import unittest

HELPER = Path(__file__).with_name("hosted-keepalive.sh")


class HostedLeaseTests(unittest.TestCase):
    def run_helper(self, ssh_body, jr_body, maximum="3"):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            ssh = root / "ssh"
            jr = root / "justrust"
            for path, body in ((ssh, ssh_body), (jr, jr_body)):
                path.write_text("#!/bin/bash\nset -eu\n" + body)
                path.chmod(0o755)
            log = root / "calls"
            result = subprocess.run(
                ["bash", str(HELPER), str(jr), "ubuntu@fixture", "bench-fixture.pid",
                 "00000000-0000-0000-0000-000000000000", maximum, str(ssh)],
                env={**os.environ, "CALLS": str(log), "JUSTRUST_BENCH_LEASE_INTERVAL": "1"},
                text=True, capture_output=True, timeout=10)
            return result, log.read_text() if log.exists() else ""

    def test_live_master_requires_explicit_api_refresh(self):
        result, calls = self.run_helper(
            'echo "ssh $*" >> "$CALLS"\n',
            'echo "jr $*" >> "$CALLS"\nexit 1\n')
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn("jr remote env --refresh", calls)
        self.assertIn("/proc/sys/kernel/random/boot_id", calls)
        self.assertIn("stat=", calls)

    def test_dead_or_rebooted_guest_never_renews(self):
        result, calls = self.run_helper('exit 1\n', 'echo unexpected >> "$CALLS"\n')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls, "")

    def test_renewals_stop_at_bounded_deadline(self):
        result, calls = self.run_helper('exit 0\n', 'echo renew >> "$CALLS"\n', "2")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertGreaterEqual(calls.count("renew"), 1)
        self.assertLessEqual(calls.count("renew"), 2)

    def test_invalid_lifetime_never_contacts_host(self):
        result, calls = self.run_helper('echo ssh >> "$CALLS"\n', 'exit 0\n', "0")
        self.assertEqual(result.returncode, 2)
        self.assertEqual(calls, "")


if __name__ == "__main__":
    unittest.main()
