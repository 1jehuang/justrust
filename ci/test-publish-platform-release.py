#!/usr/bin/env python3
"""Offline safety tests for the platform release helper. Never calls real gh."""
import argparse
import contextlib
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location("publisher", Path(__file__).with_name("publish-platform-release.py"))
publisher = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(publisher)


class FakeGH:
    def __init__(self, args):
        self.args = args
        self.assets = {}
        self.draft = True
        self.latest = "v0.1.0-alpha.29"
        self.immutable = False
        self.sha = args.sha
        self.calls = []
        self.uploads = []
        self.downloads = []
        self.use_digest = True
        self.fail_name = None
        self.move_after = None
        self.race_name = None
        self.race_mismatch = False
        self.release_missing = False

    def run(self, command, capture_output):
        assert command[0] == "gh"
        assert capture_output
        assert "--clobber" not in command
        self.calls.append(command)
        args = command[1:]
        if args[0] == "api":
            endpoint = args[1]
            if "/commits/" in endpoint:
                output = json.dumps({"sha": self.sha}).encode()
            elif endpoint.endswith("/releases/42"):
                if self.release_missing:
                    return subprocess.CompletedProcess(command, 1, b"", b"release not found")
                assets = []
                for index, (name, body) in enumerate(self.assets.items(), 1):
                    asset = {"name": name, "id": index, "size": len(body)}
                    if self.use_digest:
                        asset["digest"] = "sha256:" + hashlib.sha256(body).hexdigest()
                    assets.append(asset)
                output = json.dumps({"tag_name": self.args.tag, "draft": self.draft,
                                     "immutable": self.immutable, "assets": assets}).encode()
            elif "/releases/assets/" in endpoint:
                self.downloads.append(endpoint)
                output = list(self.assets.values())[int(endpoint.rsplit("/", 1)[1]) - 1]
            else:
                raise AssertionError(command)
        elif args[:2] == ["release", "view"]:
            if self.release_missing:
                return subprocess.CompletedProcess(command, 1, b"", b"release not found")
            output = json.dumps({"databaseId": 42}).encode()
        elif args[:2] == ["release", "upload"]:
            path = Path(args[3])
            name = path.name
            if name == self.race_name:
                self.assets[name] = b"different bytes" if self.race_mismatch else path.read_bytes()
                return subprocess.CompletedProcess(command, 1, b"", b"already exists")
            if name == self.fail_name or name in self.assets:
                return subprocess.CompletedProcess(command, 1, b"", b"upload rejected")
            self.assets[name] = path.read_bytes()
            self.uploads.append(name)
            if name == self.move_after:
                self.sha = "b" * 40
            output = b""
        elif args[:2] == ["release", "edit"]:
            base = ["release", "edit", self.args.tag, "--draft=false"]
            assert args in (base + ["--repo", self.args.repo],
                            base + ["--latest=false", "--repo", self.args.repo])
            # GitHub defaults a newly published draft to make_latest=true.
            if "--latest=false" not in args:
                self.latest = self.args.tag
            self.draft = False
            output = b""
        else:
            raise AssertionError(command)
        return subprocess.CompletedProcess(command, 0, output, b"")


class PublisherTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.dist = Path(self.temp.name) / "dist"
        self.dist.mkdir()
        self.args = argparse.Namespace(tag="v0.1.0-alpha.30", sha="a" * 40,
                                       target=publisher.TARGETS[0], dist=str(self.dist), repo="owner/repo")
        self.make_assets()
        self.fake = FakeGH(self.args)
        self.mock = patch.object(publisher.subprocess, "run", self.fake.run)
        self.mock.start()
        self.addCleanup(self.mock.stop)

    def make_assets(self, suffix="json"):
        for path in self.dist.iterdir():
            path.unlink()
        self.archive = f"justrust-{self.args.target}.tar.gz"
        self.provenance = f"compiler-{self.args.target}.{suffix}"
        self.checksum = self.archive + ".sha256"
        (self.dist / self.archive).write_bytes(b"archive fixture")
        (self.dist / self.provenance).write_bytes(b"compiler fixture")
        digest = hashlib.sha256(b"archive fixture").hexdigest()
        (self.dist / self.checksum).write_text(f"{digest}  {self.archive}\n")
        self.order = [self.archive, self.provenance, self.checksum]

    def publish(self):
        with contextlib.redirect_stdout(io.StringIO()):
            publisher.publish(self.args)

    def assert_rejected(self, pattern):
        with self.assertRaisesRegex(publisher.PublishError, pattern):
            self.publish()

    def test_all_targets_publish_checksum_last_and_retry(self):
        for target in publisher.TARGETS:
            for suffix in ("json", "txt"):
                with self.subTest(target=target, suffix=suffix):
                    self.args.target = target
                    self.make_assets(suffix)
                    self.fake = FakeGH(self.args)
                    with patch.object(publisher.subprocess, "run", self.fake.run):
                        self.publish()
                        self.assertEqual(self.fake.uploads, self.order)
                        self.assertFalse(self.fake.draft)
                        self.publish()
                        self.assertEqual(self.fake.uploads, self.order)

    def test_partial_draft_publication_preserves_global_latest(self):
        # Neither an older draft finishing late nor a newer partial release may
        # change latest/download until the all-platform summary promotes it.
        for tag in ("v0.1.0-alpha.28", "v0.1.0-alpha.30"):
            with self.subTest(tag=tag):
                self.args.tag = tag
                self.fake = FakeGH(self.args)
                with patch.object(publisher.subprocess, "run", self.fake.run):
                    self.publish()
                self.assertFalse(self.fake.draft)
                self.assertEqual(self.fake.latest, "v0.1.0-alpha.29")
                edits = [c for c in self.fake.calls if c[1:3] == ["release", "edit"]]
                self.assertEqual(len(edits), 1)
                self.assertIn("--latest=false", edits[0])

    def test_draft_lookup_uses_authenticated_identity_not_public_tag_endpoint(self):
        self.publish()
        self.assertTrue(any(c[1:3] == ["release", "view"] for c in self.fake.calls))
        self.assertFalse(any("/releases/tags/" in part for c in self.fake.calls for part in c))

    def test_public_release_append_preserves_other_platform(self):
        self.fake.draft = False
        self.fake.assets["another-platform.tar.gz"] = b"other"
        self.publish()
        self.assertEqual(self.fake.assets["another-platform.tar.gz"], b"other")
        self.assertFalse(any(c[1:3] == ["release", "edit"] for c in self.fake.calls))

    def test_digestless_retry_downloads_exact_bytes(self):
        self.fake.use_digest = False
        self.fake.assets.update({name: (self.dist / name).read_bytes() for name in self.order})
        self.publish()
        self.assertTrue(self.fake.downloads)
        self.assertEqual(self.fake.uploads, [])

    def test_same_size_different_bytes_fail_before_any_upload(self):
        for use_digest in (True, False):
            with self.subTest(digest=use_digest):
                self.fake.use_digest = use_digest
                self.fake.assets[self.provenance] = b"X" * len((self.dist / self.provenance).read_bytes())
                self.assert_rejected("bytes differ")
                self.assertEqual(self.fake.uploads, [])
                self.assertTrue(self.fake.draft)

    def test_immutable_fails_without_setting_changes(self):
        self.fake.immutable = True
        self.assert_rejected("immutable")
        self.assertEqual(self.fake.uploads, [])
        self.assertFalse(any(c[1:3] in (["release", "upload"], ["release", "edit"]) for c in self.fake.calls))

    def test_tag_moved_before_upload(self):
        self.fake.sha = "b" * 40
        self.assert_rejected("tag moved")
        self.assertEqual(self.fake.uploads, [])

    def test_tag_moved_during_upload_leaves_no_checksum_or_publication(self):
        self.fake.move_after = self.provenance
        self.assert_rejected("tag moved")
        self.assertNotIn(self.checksum, self.fake.assets)
        self.assertTrue(self.fake.draft)

    def test_failed_provenance_upload_leaves_no_checksum(self):
        self.fake.fail_name = self.provenance
        self.assert_rejected("upload rejected")
        self.assertNotIn(self.checksum, self.fake.assets)
        self.assertTrue(self.fake.draft)

    def test_concurrent_identical_upload_is_safe(self):
        self.fake.race_name = self.archive
        self.publish()
        self.assertFalse(self.fake.draft)
        self.assertEqual(set(self.fake.assets), set(self.order))

    def test_concurrent_mismatched_upload_fails_without_replacement(self):
        self.fake.race_name = self.archive
        self.fake.race_mismatch = True
        self.assert_rejected("bytes differ")
        self.assertEqual(self.fake.assets[self.archive], b"different bytes")
        self.assertNotIn(self.checksum, self.fake.assets)
        self.assertTrue(self.fake.draft)

    def test_missing_release_is_not_created(self):
        self.fake.release_missing = True
        self.assert_rejected("release not found")
        self.assertEqual(self.fake.uploads, [])
        self.assertFalse(any(c[1:3] in (["release", "upload"], ["release", "edit"]) for c in self.fake.calls))

    def test_tag_moved_after_checksum_does_not_publish_draft(self):
        self.fake.move_after = self.checksum
        self.assert_rejected("tag moved")
        self.assertTrue(self.fake.draft)

    def test_dist_traversal_rejected(self):
        self.args.dist = str(self.dist / ".." / "dist")
        self.assert_rejected("traversal")
        self.assertEqual(self.fake.calls, [])

    def test_empty_asset_rejected(self):
        (self.dist / self.provenance).write_bytes(b"")
        self.assert_rejected("empty asset")
        self.assertEqual(self.fake.calls, [])

    def test_checksum_mismatch(self):
        (self.dist / self.archive).write_bytes(b"corrupted")
        self.assert_rejected("checksum must match")
        self.assertEqual(self.fake.calls, [])

    def test_checksum_traversal_or_multiple_lines(self):
        original = (self.dist / self.checksum).read_text()
        for invalid in (original.replace(self.archive, "../" + self.archive), original * 2):
            (self.dist / self.checksum).write_text(invalid)
            self.assert_rejected("checksum must match")

    def test_extra_file_or_directory(self):
        extra = self.dist / "unexpected"
        extra.write_bytes(b"x")
        self.assert_rejected("only archive")
        extra.unlink()
        extra.mkdir()
        self.assert_rejected("only archive")
        self.assertEqual(self.fake.calls, [])

    def test_both_or_wrong_provenance(self):
        (self.dist / f"compiler-{self.args.target}.txt").write_bytes(b"x")
        self.assert_rejected("exactly one")
        (self.dist / self.provenance).unlink()
        (self.dist / f"compiler-{self.args.target}.txt").rename(self.dist / "compiler-wrong.txt")
        self.assert_rejected("exactly one")

    def test_asset_symlink_and_directory_rejected(self):
        path = self.dist / self.archive
        path.unlink()
        path.symlink_to(self.dist / self.provenance)
        self.assert_rejected("regular non-symlink")
        path.unlink()
        path.mkdir()
        self.assert_rejected("regular non-symlink")
        self.assertEqual(self.fake.calls, [])

    def test_dist_symlink_rejected(self):
        link = Path(self.temp.name) / "link"
        link.symlink_to(self.dist, target_is_directory=True)
        self.args.dist = str(link)
        self.assert_rejected("ancestors must not be symlinks")

    def test_invalid_cli_values_before_gh(self):
        for name, values in {"tag": ["--bad", "v1/../../x", "v1..2", "v1.", "v1.lock", "v1\n"],
                             "sha": ["a" * 39, "A" * 40, "--bad"],
                             "repo": ["--bad", "a/b/c", "a/../b", "https://github.com/a/b"],
                             "target": ["unknown", "../x"]}.items():
            original = getattr(self.args, name)
            for value in values:
                with self.subTest(name=name, value=value):
                    setattr(self.args, name, value)
                    self.assert_rejected("invalid|must be|unsupported")
            setattr(self.args, name, original)
        self.assertEqual(self.fake.calls, [])

    def test_binary_checksum_marker_allowed(self):
        path = self.dist / self.checksum
        path.write_text(path.read_text().replace("  ", " *"))
        self.publish()


if __name__ == "__main__":
    unittest.main()
