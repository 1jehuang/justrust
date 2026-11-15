#!/usr/bin/env python3
"""Real kernel probes, no Cargo processes or Rust builds."""
import fcntl
import tempfile
import unittest
from unittest import mock

import package_cache_contention as locks


class PackageCacheContentionTests(unittest.TestCase):
    def test_expected_matrix_is_independent_of_measurement(self):
        blocked = {("download", "download"), ("download", "mutate"),
                   ("mutate", "download"), ("mutate", "mutate"),
                   ("mutate", "shared"), ("shared", "mutate"),
                   ("target", "target"), ("other-target", "other-target")}
        for a in locks.MODES:
            for b in locks.MODES:
                with self.subTest(holder=a, contender=b):
                    self.assertEqual(locks.conflicts(a, b), (a, b) in blocked)

    def test_all_real_kernel_pairs(self):
        for a in locks.MODES:
            for b in locks.MODES:
                with self.subTest(holder=a, contender=b):
                    result = locks.measure(a, b, hold_seconds=0.01)
                    self.assertEqual(result["blocked"], locks.conflicts(a, b))
                    self.assertGreaterEqual(result["acquire_ns"], 0)
                    self.assertEqual(result["clock"], "monotonic_ns")

    def test_mutate_partial_acquisition_rolls_back_before_retry(self):
        with tempfile.TemporaryDirectory() as root:
            with locks.acquire(root, "download"):
                with self.assertRaises(BlockingIOError):
                    locks.acquire(root, "mutate", nonblocking=True)
                # A failed mutate must not leave its first, exclusive lock held.
                with locks.acquire(root, "shared", nonblocking=True):
                    pass
            with locks.acquire(root, "mutate", nonblocking=True):
                pass

    def test_target_uses_owned_build_lock_not_legacy_exclusive_lock(self):
        self.assertEqual(locks.MODES["target"], (
            ("target/debug/.cargo-build-lock", fcntl.LOCK_EX),
            ("target/debug/.cargo-lock", fcntl.LOCK_SH)))

    def test_receive_timeout_and_child_errors_fail_closed(self):
        channel = mock.Mock()
        channel.poll.return_value = False
        with self.assertRaisesRegex(ValueError, "timed out"):
            locks.receive(channel, timeout=0)
        channel.poll.return_value = True
        channel.recv.return_value = {"event": "error", "error": "probe failure"}
        with self.assertRaisesRegex(ValueError, "probe failure"):
            locks.receive(channel)

    def test_source_provenance_is_content_addressed(self):
        identity = locks.source_identity()
        self.assertEqual(set(identity), set(locks.SOURCES))
        for digest in identity.values():
            self.assertRegex(digest, r"^[a-f0-9]{64}$")


if __name__ == "__main__":
    unittest.main()
