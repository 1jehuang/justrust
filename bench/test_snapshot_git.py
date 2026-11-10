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

    def test_patch_content_changes_workdir_without_mutating_previous_snapshot(self):
        repair = self.root / "repair.patch"
        repair.write_text("diff --git a/hello.txt b/hello.txt\n--- a/hello.txt\n+++ b/hello.txt\n"
                          "@@ -1 +1 @@\n-hello\n+world\n")
        spec = {"repos": {"fixture": (self.source, self.rev)}, "cwd": "fixture",
                "patches": [("fixture", "repair.patch")]}
        with patch.object(suite, "ROOT", self.root), patch.object(suite, "WORK", self.root / "work"), \
                patch.object(suite, "SNAPSHOTS", {"fixture": spec}), \
                patch.object(suite, "bench_lock"), patch.dict(suite.os.environ, {"JUSTRUST_BENCH_SRC": ""}):
            first_fingerprint = suite.snapshot_fingerprint("fixture")
            first = suite.prepare("fixture")
            self.assertEqual((first / "fixture/hello.txt").read_text(), "world\n")
            self.assertEqual(suite.prepare("fixture"), first)
            self.assertEqual(suite.sh(["git", "-C", str(first / "fixture"), "rev-parse", "HEAD"]), self.commit)
            repair.write_text(repair.read_text().replace("+world", "+other"))
            self.assertNotEqual(suite.snapshot_fingerprint("fixture"), first_fingerprint)
            second = suite.prepare("fixture")
            self.assertNotEqual(first, second)
            self.assertEqual((second / "fixture/hello.txt").read_text(), "other\n")
            self.assertEqual((first / "fixture/hello.txt").read_text(), "world\n")

    def test_portable_math_font_repair_keeps_manifest_and_lockfile_together(self):
        repair_name = "bench/patches/desktop-portable-math-fonts.patch"
        self.assertIn(("jcode-desktop", repair_name), suite.SNAPSHOTS["desktop"]["patches"])
        repair = (suite.ROOT / repair_name).read_text()
        self.assertIn('features = ["debug-embed"]', repair)
        self.assertIn('diff --git a/Cargo.lock b/Cargo.lock', repair)
        self.assertIn('+ "rust-embed"', repair)
        self.assertNotIn('src/markdown.rs', repair)
        # Disabling the portability repair must invalidate prepared trees/results.
        original = suite.snapshot_fingerprint("desktop")
        spec = dict(suite.SNAPSHOTS["desktop"])
        spec["patches"] = [entry for entry in spec["patches"] if entry[1] != repair_name]
        with patch.dict(suite.SNAPSHOTS, {"desktop": spec}):
            self.assertNotEqual(suite.snapshot_fingerprint("desktop"), original)

    def test_updater_isolation_repair_is_fingerprinted_without_relaxing_assertions(self):
        repair_name = "bench/patches/desktop-hint-updater-state.patch"
        self.assertIn(("jcode-desktop", repair_name), suite.SNAPSHOTS["desktop"]["patches"])
        repair = (suite.ROOT / repair_name).read_text()
        self.assertIn("+        let _update_guard = updates::test_lock();", repair)
        self.assertIn("restart_action_changes_header_width_independently_of_coaching", repair)
        self.assertIn("+        updates::set(previous);", repair)
        self.assertIn("+        updates::set_release_status(release);", repair)
        removed = [line for line in repair.splitlines()
                   if line.startswith("-") and not line.startswith("---")]
        self.assertEqual(removed, [], "the repair must not remove existing assertions")
        original = suite.snapshot_fingerprint("desktop")
        spec = dict(suite.SNAPSHOTS["desktop"])
        spec["patches"] = [entry for entry in spec["patches"] if entry[1] != repair_name]
        with patch.dict(suite.SNAPSHOTS, {"desktop": spec}):
            self.assertNotEqual(suite.snapshot_fingerprint("desktop"), original)

    def test_row_click_updater_repair_is_fingerprinted_without_removing_assertions(self):
        repair_name = "bench/patches/desktop-row-click-updater-state.patch"
        self.assertIn(("jcode-desktop", repair_name), suite.SNAPSHOTS["desktop"]["patches"])
        repair = (suite.ROOT / repair_name).read_text()
        self.assertIn("live_tabs_switch_rows_and_stay_detached_from_panels", repair)
        self.assertIn("+        let _update_guard = crate::updates::test_lock();", repair)
        removed = [line for line in repair.splitlines()
                   if line.startswith("-") and not line.startswith("---")]
        self.assertEqual(removed, [], "the repair must preserve existing click assertions")
        original = suite.snapshot_fingerprint("desktop")
        spec = dict(suite.SNAPSHOTS["desktop"])
        spec["patches"] = [entry for entry in spec["patches"] if entry[1] != repair_name]
        with patch.dict(suite.SNAPSHOTS, {"desktop": spec}):
            self.assertNotEqual(suite.snapshot_fingerprint("desktop"), original)

    def test_fixup_content_also_changes_snapshot_fingerprint(self):
        spec = {"repos": {"fixture": (self.source, self.rev)}, "cwd": "fixture", "fixups": []}
        with patch.object(suite, "SNAPSHOTS", {"fixture": spec}):
            before = suite.snapshot_fingerprint("fixture")
            spec["fixups"].append(("fixture/hello.txt", "hello", "world", 1))
            self.assertNotEqual(suite.snapshot_fingerprint("fixture"), before)

    def test_named_comparison_rejects_repaired_source_mismatch(self):
        a = self.root / "old.json"
        b = self.root / "new.json"
        for path, fingerprint in ((a, "before"), (b, "after")):
            path.write_text(json.dumps({"env": {"snapshots": {}, "snapshot_fingerprints": fingerprint},
                                        "scenarios": {}}))
        with self.assertRaisesRegex(SystemExit, "different snapshot repairs"):
            suite.cmd_compare(argparse.Namespace(a=str(a), b=str(b), floor=10))

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
