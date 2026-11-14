"""Test genuine zstd chunk encoding and parallel byte restoration."""
from contextlib import redirect_stdout
import ctypes.util
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest import mock

from bench import compressed_state_restore as harness


@unittest.skipUnless(ctypes.util.find_library("zstd"), "libzstd unavailable")
class CompressedStateRestoreTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.source = self.root / "source"
        self.disk = self.root / "disk"
        (self.source / "incremental/session/empty").mkdir(parents=True)
        self.disk.mkdir()
        (self.source / "incremental/session/state.bin").write_bytes(bytes(range(256)) * 20)
        (self.source / "artifact.rlib").write_bytes(b"artifact\x00" * 100)
        (self.source / "empty-file").touch()
        self.original = harness.manifest(self.source)
        self.sentinel = self.disk / "unrelated"
        self.sentinel.write_text("preserve")

    def preserved(self):
        self.assertEqual(harness.manifest(self.source), self.original)
        self.assertEqual(list(self.disk.iterdir()), [self.sentinel])
        self.assertEqual(self.sentinel.read_text(), "preserve")

    def test_real_codec_roundtrip_and_errors(self):
        codec = harness.Zstd()
        raw = bytes(range(256)) * 100
        compressed = codec.compress(raw, 3)
        self.assertEqual(compressed[:4], b"\x28\xb5\x2f\xfd")
        self.assertLess(len(compressed), len(raw))
        self.assertEqual(codec.decompress(compressed, len(raw)), raw)
        with self.assertRaises(ValueError):
            codec.decompress(b"not zstd", len(raw))
        with self.assertRaises(ValueError):
            codec.decompress(compressed, len(raw) + 1)

    def test_encode_chunks_are_real_independent_frames(self):
        codec = harness.Zstd()
        snapshot = self.disk / "frames"
        chunks, seconds = harness.encode(self.source, snapshot, self.original, codec, 1024, 3)
        self.assertGreaterEqual(seconds, 0)
        self.assertEqual(len(chunks), 6)
        for chunk in chunks:
            compressed = (snapshot / chunk["frame"]).read_bytes()
            raw = (self.source / chunk["file"]).read_bytes()
            self.assertEqual(codec.decompress(compressed, chunk["bytes"]),
                             raw[chunk["offset"]:chunk["offset"] + chunk["bytes"]])
            self.assertEqual(len(compressed), chunk["compressed_bytes"])
        for workers in (1, 4, 16):
            destination = self.disk / f"out-{workers}"
            harness.restore(snapshot, destination, self.original, chunks, codec, workers)
            harness.verify(destination, self.original)

    def test_default_benchmark_verifies_all_nine_restores(self):
        report = harness.benchmark(self.source, self.disk, chunk_bytes=1024)
        self.assertEqual(len(report["runs"]), 9)
        self.assertEqual(report["files"], 3)
        self.assertEqual(report["bytes"], 6020)
        self.assertEqual(report["codec"]["chunks"], 6)
        self.assertGreater(report["compressed_bytes"], 0)
        self.assertIn("Warm-page-cache", report["cache_note"])
        self.assertIn("not compiler", report["scope"])
        for workers in (1, 4, 16):
            runs = [r for r in report["runs"] if r["workers"] == workers]
            self.assertEqual(len(runs), 3)
            self.assertTrue(all(r["verified"] for r in runs))
        json.dumps(report)
        self.preserved()

    def test_cli_custom_settings_json(self):
        output = io.StringIO()
        with redirect_stdout(output):
            harness.main([str(self.source), str(self.disk), "--workers", "1,2",
                          "--repetitions", "4", "--chunk-bytes", "500", "--level", "1"])
        report = json.loads(output.getvalue())
        self.assertEqual(len(report["runs"]), 8)
        self.assertEqual(report["codec"]["level"], 1)
        self.assertEqual(report["codec"]["chunk_bytes"], 500)
        self.preserved()

    def test_bad_configuration_and_overlap(self):
        for options in ({"workers": []}, {"workers": [1, 1]}, {"workers": [0]},
                        {"repetitions": 2}, {"chunk_bytes": 0}, {"level": 20}):
            with self.subTest(options=options), self.assertRaises(ValueError):
                harness.benchmark(self.source, self.disk, **options)
        with self.assertRaisesRegex(ValueError, "overlap"):
            harness.benchmark(self.source, self.source)
        link = self.source / "link"
        link.symlink_to(self.source / "artifact.rlib")
        with self.assertRaisesRegex(ValueError, "Unsupported"):
            harness.benchmark(self.source, self.disk)
        link.unlink()
        self.preserved()

    def test_corruption_cleanup_and_worker_error(self):
        original_restore = harness.restore

        def corrupted(*args):
            seconds = original_restore(*args)
            (args[1] / "artifact.rlib").write_bytes(b"bad")
            return seconds

        with mock.patch.object(harness, "restore", side_effect=corrupted):
            with self.assertRaisesRegex(ValueError, "verification failed"):
                harness.benchmark(self.source, self.disk, chunk_bytes=1024)
        self.preserved()
        with mock.patch.object(harness.Zstd, "decompress", side_effect=ValueError("decode failed")):
            with self.assertRaisesRegex(ValueError, "decode failed"):
                harness.benchmark(self.source, self.disk, workers=[4], chunk_bytes=1024)
        self.preserved()

    def test_partial_pwrite_is_retried(self):
        real_pwrite = harness.os.pwrite

        def partial(fd, data, offset):
            return real_pwrite(fd, data[:17], offset)

        with mock.patch.object(harness.os, "pwrite", side_effect=partial):
            harness.benchmark(self.source, self.disk, workers=[4], chunk_bytes=1024)
        self.preserved()

    def test_verification_runs_after_restore_timer_stops(self):
        events = []
        original_restore = harness.restore
        original_verify = harness.verify

        def restore(*args):
            events.append("restore")
            return original_restore(*args)

        def verify(*args):
            events.append("verify")
            return original_verify(*args)

        def clock():
            events.append("clock")
            return len(events)

        with mock.patch.object(harness, "restore", side_effect=restore), \
                mock.patch.object(harness, "verify", side_effect=verify), \
                mock.patch.object(harness.time, "perf_counter", side_effect=clock):
            harness.benchmark(self.source, self.disk, workers=[1], chunk_bytes=1024)
        for index, event in enumerate(events):
            if event == "restore":
                end = events.index("verify", index)
                self.assertEqual(events[end - 1], "clock")
                self.assertEqual(events[index - 1], "clock")
        self.preserved()


if __name__ == "__main__":
    unittest.main()
