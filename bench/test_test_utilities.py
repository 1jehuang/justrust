"""Runtime prerequisite regressions for pinned browser and stdin tests."""
import importlib.util
from pathlib import Path
import shutil
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("test_utilities", ROOT / "ci/check-test-utilities.py")
utilities = importlib.util.module_from_spec(spec)
spec.loader.exec_module(utilities)


class TestUtilitiesTests(unittest.TestCase):
    def test_standalone_sleep_survives_copy_under_browser_host_basename(self):
        utilities.check_sleep()

    def test_early_exiting_renamed_program_is_rejected(self):
        with self.assertRaisesRegex(RuntimeError, "standalone GNU sleep"):
            utilities.check_sleep(shutil.which("false"))

    def test_head_has_detectable_read_and_preserves_input(self):
        utilities.check_head()

    def test_early_exiting_head_is_rejected(self):
        with self.assertRaisesRegex(RuntimeError, "detectable blocking head"):
            utilities.check_head(shutil.which("false"))

    def test_unobservable_reader_is_rejected_not_treated_as_valid(self):
        with mock.patch.object(utilities, "reading_stdin", return_value=False):
            with self.assertRaisesRegex(RuntimeError, "provision GNU head"):
                utilities.check_head()

    def test_utility_provenance_contains_actual_binary_hashes(self):
        result = utilities.check()
        self.assertEqual(set(result), {"sleep", "head"})
        for binary in result.values():
            self.assertTrue(Path(binary["path"]).is_absolute())
            self.assertEqual(len(binary["sha256"]), 64)

    def test_read_zero_is_detectable_without_wchan_fallback(self):
        with mock.patch.object(Path, "read_text", return_value="0 0x0 0x1234"):
            self.assertTrue(utilities.reading_stdin(123))

    def test_splice_wait_on_cloned_descriptor_is_not_stdin_read(self):
        with mock.patch.object(Path, "read_text", side_effect=["275 0x3 0x0", "splice_from_pipe_next"]):
            self.assertFalse(utilities.reading_stdin(123))

    def test_pipe_read_fallback_requires_stdin_pipe(self):
        with mock.patch.object(Path, "read_text", side_effect=["0 0x3", "pipe_read"]):
            with mock.patch.object(utilities.os, "readlink", return_value="pipe:[123]"):
                self.assertTrue(utilities.reading_stdin(123))

    def test_gate_validates_utilities_before_hosted_builds(self):
        helper = (ROOT / "ci/source-sync-release-gate.sh").read_text()
        self.assertLess(helper.index('python3 "$root/ci/check-test-utilities.py"'),
                        helper.index('build_binary before "$BASELINE_REF"'))
        workflow = (ROOT / ".github/workflows/source-sync-release-gate.yml").read_text()
        self.assertIn("coreutils-from-gnu coreutils-from-uutils-", workflow)


if __name__ == "__main__":
    unittest.main()
