"""Safety and correctness tests for the disk-state restore benchmark."""

import argparse
from contextlib import redirect_stdout
import importlib.util
import io
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest import mock


SPEC = importlib.util.spec_from_file_location(
    "disk_state_restore", Path(__file__).with_name("disk_state_restore.py")
)
harness = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(harness)


class DiskStateRestoreTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.source = self.root / "source"
        self.disk = self.root / "disk"
        self.source.mkdir()
        self.disk.mkdir()
        (self.source / "incremental" / "session").mkdir(parents=True)
        (self.source / "empty").mkdir()
        (self.source / "incremental" / "session" / "state.bin").write_bytes(bytes(range(256)) * 8)
        (self.source / "libexample.rlib").write_bytes(b"real test bytes\x00\xff")
        (self.source / "zero").touch()
        self.original = harness.manifest(self.source)
        self.sentinel = self.disk / "unrelated"
        self.sentinel.write_text("preserve")

    def assert_preserved(self):
        self.assertEqual(harness.manifest(self.source), self.original)
        self.assertEqual(list(self.disk.iterdir()), [self.sentinel])
        self.assertEqual(self.sentinel.read_text(), "preserve")

    def test_default_runs_are_verified_and_isolated(self):
        report = harness.benchmark(self.source, self.disk)
        self.assertEqual(len(report["runs"]), 9)
        for workers in (1, 4, 16):
            runs = [run for run in report["runs"] if run["workers"] == workers]
            self.assertEqual([r["repetition"] for r in runs], [1, 2, 3])
            for run in runs:
                self.assertTrue(run["verified"])
                self.assertEqual(run["files"], 3)
                self.assertEqual(run["bytes"], 2048 + len(b"real test bytes\x00\xff"))
                self.assertGreaterEqual(run["seconds"], 0)
        self.assertEqual(report["machine"]["cpu_count"], os.cpu_count())
        self.assertEqual(report["disk_path"], str(self.disk.resolve()))
        self.assertIn("Warm-page-cache", report["cache_note"])
        json.dumps(report)
        self.assert_preserved()

    def test_custom_workers_repetitions_and_cli_json(self):
        output = io.StringIO()
        with redirect_stdout(output):
            self.assertEqual(harness.main([str(self.source), str(self.disk),
                                           "--workers", "1,2", "--repetitions", "4"]), 0)
        report = json.loads(output.getvalue())
        self.assertEqual(len(report["runs"]), 8)
        self.assertEqual([s["workers"] for s in report["summaries"]], [1, 2])
        self.assert_preserved()

    def test_overlaps_and_aliases_rejected(self):
        child = self.source / "child"
        child.mkdir()
        alias = self.root / "alias"
        alias.symlink_to(self.source, target_is_directory=True)
        for source, target in [(self.source, self.source), (self.source, child),
                               (self.source, self.root), (self.source, alias)]:
            with self.subTest(source=source, target=target):
                with self.assertRaisesRegex(ValueError, "overlap"):
                    harness.benchmark(source, target)
        self.assertTrue((self.source / "libexample.rlib").exists())

    def test_invalid_configuration(self):
        for workers, repetitions in [([], 3), ([0], 3), ([-1], 3), ([1, 1], 3),
                                     ([True], 3), ([1], 2), ([1], 3.5)]:
            with self.subTest(workers=workers, repetitions=repetitions):
                with self.assertRaises(ValueError):
                    harness.benchmark(self.source, self.disk, workers, repetitions)
        for value in ("", "0", "1,1", "a", "1,"):
            with self.assertRaises(argparse.ArgumentTypeError):
                harness.parse_workers(value)
        self.assert_preserved()

    def test_empty_source_rejected(self):
        empty = self.root / "empty"
        empty.mkdir()
        with self.assertRaisesRegex(ValueError, "at least one"):
            harness.benchmark(empty, self.disk)
        self.assert_preserved()

    def test_symlinks_and_special_files_rejected(self):
        for target in (self.source / "libexample.rlib", self.source / "incremental"):
            link = self.source / "link"
            link.symlink_to(target)
            with self.assertRaisesRegex(ValueError, "Unsupported"):
                harness.benchmark(self.source, self.disk)
            link.unlink()
        fifo = self.source / "fifo"
        os.mkfifo(fifo)
        with self.assertRaisesRegex(ValueError, "Unsupported"):
            harness.benchmark(self.source, self.disk)
        fifo.unlink()
        self.assert_preserved()

    def test_verification_detects_same_length_corruption_and_extra_entries(self):
        artifact = self.source / "libexample.rlib"
        old = artifact.read_bytes()
        artifact.write_bytes(b"x" * len(old))
        with self.assertRaisesRegex(ValueError, "verification failed"):
            harness.verify(self.source, self.original)
        artifact.write_bytes(old)
        (self.source / "extra").mkdir()
        with self.assertRaisesRegex(ValueError, "verification failed"):
            harness.verify(self.source, self.original)

    def test_cleanup_on_copy_failure_preserves_source_and_siblings(self):
        with mock.patch.object(harness.shutil, "copy2", side_effect=OSError("copy failed")):
            with self.assertRaisesRegex(OSError, "copy failed"):
                harness.benchmark(self.source, self.disk)
        self.assert_preserved()

    def test_cleanup_on_restore_corruption(self):
        real_copy = harness.copy_tree

        def corrupt_restore(source, destination, inventory, workers):
            real_copy(source, destination, inventory, workers)
            if destination.name == "restore":
                (destination / "zero").write_bytes(b"corrupt")

        with mock.patch.object(harness, "copy_tree", side_effect=corrupt_restore):
            with self.assertRaisesRegex(ValueError, "verification failed"):
                harness.benchmark(self.source, self.disk)
        self.assert_preserved()

    def test_hashing_is_outside_timed_regions(self):
        events = []
        real_manifest = harness.manifest

        def hashing(root):
            events.append("hash")
            return real_manifest(root)

        def clock():
            events.append("clock")
            return len(events)

        with mock.patch.object(harness, "manifest", side_effect=hashing), \
                mock.patch.object(harness.time, "perf_counter", side_effect=clock):
            harness.benchmark(self.source, self.disk, workers=[1, 2])
        # Every start-clock is immediately paired with end-clock, without hashes.
        clocks = [index for index, event in enumerate(events) if event == "clock"]
        self.assertEqual(len(clocks), 14)
        for start, end in zip(clocks[::2], clocks[1::2]):
            self.assertEqual(end, start + 1)
        self.assert_preserved()

    def test_parallel_copy_propagates_worker_errors(self):
        with mock.patch.object(harness.shutil, "copy2", side_effect=OSError("worker failed")):
            with self.assertRaisesRegex(OSError, "worker failed"):
                harness.copy_tree(self.source, self.disk / "owned", self.original, 4)

    def test_warm_default_never_evicts_or_fsyncs(self):
        with mock.patch.object(harness, "evict_snapshot_cache") as evict, \
                mock.patch.object(harness.os, "fsync") as fsync:
            report = harness.benchmark(self.source, self.disk, workers=[1])
        evict.assert_not_called()
        fsync.assert_not_called()
        self.assertFalse(report["evict_source_cache"])
        self.assertTrue(all(run["cache_preparation_seconds"] == 0 for run in report["runs"]))
        self.assert_preserved()

    def test_eviction_only_private_snapshot_fsync_then_advise(self):
        events = []

        def fsync(fd):
            path = Path(os.readlink(f"/proc/self/fd/{fd}"))
            self.assertIn(self.disk, path.parents)
            self.assertEqual(path.relative_to(self.disk).parts[1], "snapshot")
            self.assertNotIn(self.source, path.parents)
            events.append(("fsync", path))

        def advise(fd, offset, length, advice):
            path = Path(os.readlink(f"/proc/self/fd/{fd}"))
            self.assertEqual(events[-1], ("fsync", path))
            self.assertEqual((offset, length, advice), (0, 0, harness.os.POSIX_FADV_DONTNEED))
            events.append(("advise", path))

        with mock.patch.object(harness.os, "fsync", side_effect=fsync), \
                mock.patch.object(harness.os, "posix_fadvise", side_effect=advise), \
                mock.patch.object(harness, "read_bytes_counter", side_effect=[10, 110, 120, 320, 330, 630]):
            report = harness.benchmark(self.source, self.disk, workers=[1], evict_source_cache=True)
        self.assertEqual(len(events), 18)
        self.assertEqual([r["read_bytes_delta"] for r in report["runs"]], [100, 200, 300])
        self.assertTrue(report["evict_source_cache"])
        self.assertIn("not guaranteed", report["cache_note"])
        self.assert_preserved()

    def test_eviction_and_read_accounting_outside_copy_timing(self):
        events = []
        original_copy = harness.copy_tree
        original_verify = harness.verify

        def copy(*args):
            events.append("copy")
            return original_copy(*args)

        def verify(*args):
            events.append("verify")
            return original_verify(*args)

        def clock():
            events.append("clock")
            return len(events)

        def read_counter():
            events.append("read_counter")
            return 0

        with mock.patch.object(harness, "copy_tree", side_effect=copy), \
                mock.patch.object(harness, "verify", side_effect=verify), \
                mock.patch.object(harness, "evict_snapshot_cache", side_effect=lambda *args: events.append("evict")), \
                mock.patch.object(harness, "read_bytes_counter", side_effect=read_counter), \
                mock.patch.object(harness.time, "perf_counter", side_effect=clock):
            harness.benchmark(self.source, self.disk, workers=[1], evict_source_cache=True)
        for index, event in enumerate(events):
            if event == "evict":
                self.assertEqual(events[index - 1:index + 8],
                                 ["clock", "evict", "clock", "read_counter", "clock", "copy", "clock", "read_counter", "verify"])
        self.assert_preserved()

    def test_read_bytes_missing_or_malformed_is_nullable(self):
        for data in ("read_bytes: 123\nrchar: 9", "read_bytes: invalid", "rchar: 9"):
            with mock.patch.object(harness.Path, "read_text", return_value=data):
                self.assertEqual(harness.read_bytes_counter(), 123 if "123" in data else None)
        with mock.patch.object(harness.Path, "read_text", side_effect=OSError("unavailable")):
            self.assertIsNone(harness.read_bytes_counter())
        with mock.patch.object(harness, "read_bytes_counter", return_value=None):
            report = harness.benchmark(self.source, self.disk, workers=[1])
        self.assertTrue(all(run["read_bytes_delta"] is None for run in report["runs"]))

    def test_eviction_failure_cleans_private_tree_only(self):
        with mock.patch.object(harness.os, "posix_fadvise", side_effect=OSError("advice failed")):
            with self.assertRaisesRegex(OSError, "advice failed"):
                harness.benchmark(self.source, self.disk, workers=[1], evict_source_cache=True)
        self.assert_preserved()

    def test_cli_eviction_flag(self):
        output = io.StringIO()
        with mock.patch.object(harness, "evict_snapshot_cache") as evict, redirect_stdout(output):
            harness.main([str(self.source), str(self.disk), "--workers", "1", "--evict-source-cache"])
        self.assertEqual(evict.call_count, 3)
        self.assertTrue(json.loads(output.getvalue())["evict_source_cache"])
        self.assert_preserved()

    @unittest.skipUnless(hasattr(os, "posix_fadvise"), "POSIX advice unavailable")
    def test_real_private_snapshot_advice_smoke(self):
        report = harness.benchmark(self.source, self.disk, workers=[1], evict_source_cache=True)
        self.assertTrue(all(run["verified"] for run in report["runs"]))
        self.assert_preserved()

    def test_eviction_unsupported_is_explicit(self):
        with mock.patch.object(harness, "hasattr", return_value=False, create=True):
            with self.assertRaisesRegex(ValueError, "requires POSIX"):
                harness.benchmark(self.source, self.disk, workers=[1], evict_source_cache=True)
        self.assert_preserved()


if __name__ == "__main__":
    unittest.main()
