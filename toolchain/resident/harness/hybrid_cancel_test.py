#!/usr/bin/env python3
"""Actual postcommit native compiler disconnect acceptance.

Run separately after a coordinated engine build, with JR_HYBRID_TEST_ENGINE and
JR_HYBRID_TEST_TOOLCHAIN. A real ready compiler is paused to hold its dispatch
in flight deterministically, then the committed request process disappears.
No fake engine, installed client replacement, or background ownership bypass.
"""

import os
from pathlib import Path
import re
import select
import signal
import subprocess
import sys
import time
import unittest

import hybrid_protocol_test as protocol


@unittest.skipUnless(sys.platform == "linux" and protocol.ENGINE and protocol.TOOLCHAIN,
                     "requires Linux and explicit frozen engine/toolchain")
class CompilationCancellationTests(unittest.TestCase):
    def test_committed_client_disconnect_reaps_compiler_and_daemon_rewarms(self):
        fixture = protocol.EngineTests()
        fixture.setUp()
        self.addCleanup(fixture.tearDown)
        fixture.prewarm("test", "--lib")
        status = subprocess.run([protocol.ENGINE, "status"], cwd=fixture.project,
                                env=fixture.env, capture_output=True, timeout=20, check=True)
        text = status.stdout.decode() + status.stderr.decode()
        daemon = int(re.search(r"daemon: pid (\d+)", text).group(1))
        workers = re.findall(r"worker: pid (\d+),", text)
        self.assertEqual(len(workers), 1, text)
        worker = int(workers[0])

        def daemon_sockets():
            count = 0
            for fd in Path(f"/proc/{daemon}/fd").iterdir():
                try:
                    count += os.readlink(fd).startswith("socket:[")
                except FileNotFoundError:
                    pass
            return count

        baseline_sockets = daemon_sockets()

        def identity(pid):
            try:
                stat = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()
                return stat[19]
            except FileNotFoundError:
                return None

        witness = identity(worker)
        self.assertIsNotNone(witness)
        # Only this private daemon's witnessed compiler may be signalled. If a test
        # assertion fails, resume it before fixture stop handles namespace cleanup.
        def resume_owned():
            if identity(worker) == witness:
                os.kill(worker, signal.SIGCONT)
        self.addCleanup(resume_owned)
        os.kill(worker, signal.SIGSTOP)
        fixture.source.write_text(fixture.source.read_text() + "\npub fn changed() -> u32 { 42 }\n")
        read_fd, write_fd = os.pipe()
        env = dict(fixture.env, JR_HYBRID_DECISION_FD=str(write_fd))
        try:
            client = subprocess.Popen([protocol.ENGINE, "cargo", "hybrid-ready", "--", "test", "--lib"],
                                      cwd=fixture.project, env=env, pass_fds=(write_fd,),
                                      stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        finally:
            os.close(write_fd)
        self.addCleanup(os.close, read_fd)
        def reap_client():
            if client.poll() is None:
                client.kill()
            client.communicate(timeout=10)
        self.addCleanup(reap_client)
        self.assertTrue(select.select([read_fd], [], [], 20)[0], "missing execution decision")
        self.assertEqual(os.read(read_fd, 4096), b"committed\n")
        # Admission's socket, its EOF-watcher clone and the real compiler socket
        # must all exist. This witnesses actual post-ACK dispatch, rather than
        # merely killing a client between publishing its receipt and sending ACK.
        deadline = time.monotonic() + 5
        while daemon_sockets() < baseline_sockets + 3 and time.monotonic() < deadline:
            time.sleep(0.01)
        self.assertGreaterEqual(daemon_sockets(), baseline_sockets + 3,
                                "daemon never entered actual compiler dispatch")
        self.assertIsNone(client.poll())
        self.assertEqual(identity(worker), witness)
        self.assertEqual(fixture.executions(), [])
        started = time.monotonic()
        client.kill()
        stdout, stderr = client.communicate(timeout=10)
        deadline = time.monotonic() + 5
        while identity(worker) == witness and time.monotonic() < deadline:
            time.sleep(0.01)
        self.assertIsNone(identity(worker), "disconnected committed compiler was not reaped")
        self.assertLess(time.monotonic() - started, 5)
        self.assertEqual(fixture.executions(), [], (stdout, stderr))
        self.assertTrue(Path(f"/proc/{daemon}").exists(), "daemon must survive request cancellation")
        # A ready-only miss is allowed after invalidating the stopped compiler, but
        # cancellation itself must never have silently replayed Cargo or user tests.
        missed = fixture.ready("test", "--lib")
        self.assertEqual(missed[:2], (0, b"miss\n"), missed)
        fixture.prewarm("test", "--lib")
        warmed = fixture.ready("test", "--lib")
        self.assertEqual(warmed[:2], (0, b"committed\n"), warmed)
        self.assertEqual(fixture.executions(), ["fresh"])
        status = subprocess.run([protocol.ENGINE, "status"], cwd=fixture.project,
                                env=fixture.env, capture_output=True, timeout=20, check=True)
        self.assertIn(f"daemon: pid {daemon},", status.stdout.decode() + status.stderr.decode())


if __name__ == "__main__":
    unittest.main(verbosity=2)
