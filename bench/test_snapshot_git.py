"""Exact, isolated Git metadata for pinned archive snapshots."""
import argparse
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import suite


class SnapshotGitTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.source = self.root / "source"
        self.source.mkdir()
        subprocess.run(["git", "init", "--quiet", "--template=", str(self.source)], check=True)
        # Deterministic fixture objects, not commits in a developer checkout.
        stream = (b"blob\nmark :1\ndata 6\nhello\n\n"
                  b"commit refs/heads/fixture\nmark :2\n"
                  b"committer Snapshot Fixture <fixture@example.invalid> 1000000000 +0000\n"
                  b"data 8\ninitial\nM 100644 :1 hello.txt\n\n"
                  b"commit refs/heads/fixture\nmark :3\n"
                  b"committer Snapshot Fixture <fixture@example.invalid> 1000000001 +0000\n"
                  b"data 7\npinned\nfrom :2\nM 100644 :1 nested/file.txt\n\ndone\n")
        subprocess.run(["git", "-C", str(self.source), "fast-import", "--quiet"],
                       input=stream, check=True)
        self.commit = suite.sh(["git", "-C", str(self.source), "rev-parse", "fixture"])
        self.parent = suite.sh(["git", "-C", str(self.source), "rev-parse", "fixture^"])
        self.rev = self.commit[:12]
        self.dst = self.root / "snapshot"
        self.dst.mkdir()
        (self.dst / "hello.txt").write_text("hello\n")
        (self.dst / "nested").mkdir()
        (self.dst / "nested/file.txt").write_text("hello\n")
        subprocess.run(["git", "-C", str(self.source), "config", "remote.private.url",
                        "https://secret.invalid/never-copy"], check=True)
        (self.source / ".git/hooks").mkdir()
        (self.source / ".git/hooks/private-hook").write_text("private hook")

    def assert_snapshot(self):
        self.assertEqual(suite.sh(["git", "-C", str(self.dst), "rev-parse", "HEAD"]), self.commit)
        self.assertEqual(suite.sh(["git", "-C", str(self.dst), "status", "--porcelain"]), "")
        self.assertEqual(suite.sh(["git", "-C", str(self.dst), "log", "--format=%H"]), self.commit)
        self.assertEqual(suite.sh(["git", "-C", str(self.dst), "remote"]), "")
        self.assertFalse((self.dst / ".git/hooks/private-hook").exists())
        self.assertNotIn("secret.invalid", (self.dst / ".git/config").read_text())
        self.assertEqual((self.dst / ".git/shallow").read_text(), self.commit + "\n")
        absent = subprocess.run(["git", "-C", str(self.dst), "cat-file", "-e", self.parent],
                                capture_output=True)
        self.assertNotEqual(absent.returncode, 0)
        (self.dst / "hello.txt").write_text("edited\n")
        self.assertIn("hello.txt", suite.sh(["git", "-C", str(self.dst), "diff", "--name-only"]))
        self.assertEqual(suite.sh(["git", "-C", str(self.source), "show", "fixture:hello.txt"]), "hello")

    def test_local_snapshot_has_exact_commit_without_source_history_or_configuration(self):
        suite.prepare_snapshot_git(self.dst, self.source, self.rev, None)
        self.assert_snapshot()

    def test_remote_pack_is_self_contained_and_preserves_exact_commit(self):
        transport = self.root / "transport"
        transport.mkdir()
        objects = subprocess.check_output(["git", "-C", str(self.source), "rev-list",
                                           "--objects", "--no-walk", self.rev])
        with (transport / f"snapshot@{self.rev}.git-pack").open("wb") as pack:
            subprocess.run(["git", "-C", str(self.source), "pack-objects", "--stdout"],
                           input=objects, stdout=pack, check=True)
        suite.prepare_snapshot_git(self.dst, self.root / "unavailable-source", self.rev, str(transport))
        self.assert_snapshot()

    def test_missing_remote_pack_fails_before_snapshot_can_be_stamped(self):
        with self.assertRaisesRegex(SystemExit, "missing pinned Git metadata"):
            suite.prepare_snapshot_git(self.dst, self.source, self.rev, str(self.root))
        self.assertFalse((self.dst / ".bench-snapshot").exists())

    def test_snapshot_format_does_not_reuse_archive_only_worktrees(self):
        with patch.object(suite, "WORK", self.root):
            self.assertTrue(suite.snapshot_dir("desktop").name.startswith("v2-"))

    def test_named_comparison_rejects_snapshot_format_change(self):
        a = self.root / "a.json"
        b = self.root / "b.json"
        a.write_text(json.dumps({"env": {"snapshots": {"fixture": "pinned"}}, "scenarios": {}}))
        b.write_text(json.dumps({"env": {"snapshots": {"fixture": "pinned"}, "snapshot_format": 2},
                                "scenarios": {}}))
        with self.assertRaisesRegex(SystemExit, "different snapshot formats"):
            suite.cmd_compare(argparse.Namespace(a=str(a), b=str(b), floor=10))


if __name__ == "__main__":
    unittest.main()
