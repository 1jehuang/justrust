"""No-build regression tests for shared snapshot isolation and failure evidence."""
import contextlib
import json
import sys
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import suite
engine, _ = suite._synth()
REAL_RECOVER = engine.gen._recover_journal


class IsolationTests(unittest.TestCase):
    def setUp(self):
        self.plan = SimpleNamespace(snap="fixture", pre=["test", "--no-run"],
                                    cmd=["test"], prev=None, expect_fail=False,
                                    edits=[], point=SimpleNamespace(shape={}))
        self.stack = SimpleNamespace(name="fixture")
        self.context = contextlib.ExitStack()
        self.addCleanup(self.context.close)
        self.lock = self.context.enter_context(mock.patch.object(engine.gen, "_locked"))
        self.restore_all = self.context.enter_context(mock.patch.object(engine.gen, "restore_all"))
        self.restore = self.context.enter_context(mock.patch.object(engine.gen, "restore"))
        self.recover = self.context.enter_context(mock.patch.object(engine.gen, "_recover_journal"))
        self.apply = self.context.enter_context(mock.patch.object(engine.gen, "apply", return_value=[]))
        self.context.enter_context(mock.patch.object(engine.os, "sync"))

    def sample(self, **updates):
        return dict(exit=0, compile_failed=False, tests_failed=0, wall=1, **updates)

    def execute(self, measured):
        with tempfile.TemporaryDirectory() as tmp:
            prelog, log = Path(tmp) / "pre.log", Path(tmp) / "run.log"
            prelog.write_text("pre")
            log.write_text("measured evidence")
            pre = self.sample()
            pre["log"] = str(prelog)
            measured["log"] = str(log)
            with mock.patch.object(engine, "_run", side_effect=[pre, measured]):
                result = engine.run_point(self.stack, self.plan, 123)
            self.assertFalse(prelog.exists())
            return result, log.exists()

    def test_failed_tests_are_errors_and_keep_log(self):
        for exit_code in (0, 101):
            sample = self.sample()
            sample.update(exit=exit_code, tests_failed=1)
            result, kept = self.execute(sample)
            self.assertIn("error", result)
            self.assertTrue(kept)

    def test_success_discards_log_and_restores(self):
        result, kept = self.execute(self.sample())
        self.assertNotIn("error", result)
        self.assertFalse(kept)
        self.restore.assert_called_once_with(self.plan, [])

    def test_expected_compile_failure_is_still_valid(self):
        self.plan.expect_fail = True
        sample = self.sample()
        sample.update(exit=101, compile_failed=True)
        result, kept = self.execute(sample)
        self.assertNotIn("error", result)
        self.assertFalse(kept)

    def test_lock_precedes_snapshot_restoration(self):
        events = []
        self.lock.side_effect = lambda: events.append("lock")
        self.restore_all.side_effect = lambda snap: events.append("restore")
        self.execute(self.sample())
        self.assertEqual(events, ["lock", "restore"])

    def test_partial_apply_recovers_journal(self):
        self.apply.side_effect = OSError("interrupted edit")
        with mock.patch.object(engine, "_run", return_value={"exit": 0, "log": "unused"}), \
                mock.patch.object(engine, "_keep_log"):
            with self.assertRaisesRegex(OSError, "interrupted edit"):
                engine.run_point(self.stack, self.plan, 123)
        self.recover.assert_called_once_with()
        self.restore.assert_not_called()

    def test_measured_exception_restores_completed_edits(self):
        with mock.patch.object(engine, "_run", side_effect=[{"exit": 0}, RuntimeError("child")]), \
                mock.patch.object(engine, "_keep_log"):
            with self.assertRaisesRegex(RuntimeError, "child"):
                engine.run_point(self.stack, self.plan, 123)
        self.restore.assert_called_once_with(self.plan, [])

    def test_partial_apply_restores_real_files_and_removes_module(self):
        # Use the real recovery implementation against a disposable snapshot.
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / "lib.rs"
            source.write_text("edited")
            module = root / "jr_synth_mod_123.rs"
            module.write_text("new module")
            journal = root / "journal"
            journal.mkdir()
            (journal / "0").write_text("original")
            (journal / "index.json").write_text(json.dumps({"0": str(source)}))
            self.apply.side_effect = OSError("partial")
            # Delegate the recovery mock to the real filesystem implementation.
            with mock.patch.object(engine.gen, "JOURNAL", journal), \
                    mock.patch.object(engine, "_run", return_value={"exit": 0}), \
                    mock.patch.object(engine, "_keep_log"):
                self.recover.side_effect = REAL_RECOVER
                with self.assertRaisesRegex(OSError, "partial"):
                    engine.run_point(self.stack, self.plan, 123)
            self.assertEqual(source.read_text(), "original")
            self.assertFalse(module.exists())
            self.assertFalse(journal.exists())


if __name__ == "__main__":
    unittest.main()
