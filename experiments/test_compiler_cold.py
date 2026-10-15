"""Offline safety tests. These never flush the developer machine's caches."""
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import MagicMock, mock_open, patch
import compiler_cold


class ColdSafetyTests(unittest.TestCase):
    def test_mincore_reports_real_residency_without_reading_file_contents(self):
        with tempfile.TemporaryDirectory(dir=os.environ.get("JCODE_SCRATCH_DIR")) as directory:
            path = Path(directory) / "probe"
            path.write_bytes(b"x" * (compiler_cold.mmap.PAGESIZE + 1))
            resident, pages = compiler_cold.cached_pages(path)
            self.assertEqual(pages, 2)
            self.assertEqual(resident, 2)
            path.write_bytes(b"")
            self.assertEqual(compiler_cold.cached_pages(path), (0, 0))

    def test_flush_syncs_before_writing_drop_three(self):
        ordering = []
        handle = mock_open()
        handle.return_value.write.side_effect = lambda value: ordering.append(("write", value))
        with patch.object(compiler_cold.os, "sync", side_effect=lambda: ordering.append(("sync", None))), patch("builtins.open", handle):
            compiler_cold.drop_caches()
        self.assertEqual(ordering, [("sync", None), ("write", "3\n")])
        handle.assert_called_once_with("/proc/sys/vm/drop_caches", "w")

    def test_active_compiler_prevents_cache_flush(self):
        process = MagicMock()
        process.name = "123"
        (process / "comm").read_text.return_value = "rustc\n"
        with patch.object(compiler_cold.Path, "iterdir", return_value=[process]):
            with self.assertRaisesRegex(RuntimeError, "active compilation"):
                compiler_cold.ensure_idle()

    def test_idle_daemon_is_not_mistaken_for_active_compiler(self):
        process = MagicMock()
        process.name = "123"
        (process / "comm").read_text.return_value = "justrust\n"
        with patch.object(compiler_cold.Path, "iterdir", return_value=[process]):
            compiler_cold.ensure_idle()


if __name__ == "__main__":
    unittest.main()
