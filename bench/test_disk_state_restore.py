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


if __name__ == "__main__":
    unittest.main()
