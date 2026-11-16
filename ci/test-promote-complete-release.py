#!/usr/bin/env python3
"""Offline promotion safety checks for legacy all-platform installers."""
import importlib.util
import json
from pathlib import Path
import unittest
from unittest import mock

spec = importlib.util.spec_from_file_location("promote", Path(__file__).with_name("promote-complete-release.py"))
promote = importlib.util.module_from_spec(spec)
spec.loader.exec_module(promote)
SHA = "a" * 40
TAG = "v0.1.0-alpha.31"


class PromotionTests(unittest.TestCase):
    def assets(self):
        return [dict(name=name, state="uploaded") for target in promote.TARGETS
                for name in (f"justrust-{target}.tar.gz", f"justrust-{target}.tar.gz.sha256", f"compiler-{target}.json")]

    def exercise(self, assets=None, latest="v0.1.0-alpha.30", moved=False, draft=False):
        self.mutations = []
        reads = 0
        def command(*args):
            nonlocal reads
            if args[0] == "release":
                self.mutations.append(args)
                return ""
            endpoint = args[1]
            if "/commits/" in endpoint:
                reads += 1
                return json.dumps(dict(sha="b" * 40 if moved and reads > 1 else SHA))
            if endpoint.endswith("/latest"):
                return json.dumps(dict(tag_name=latest))
            return json.dumps(dict(tag_name=TAG, draft=draft, assets=self.assets() if assets is None else assets))
        with mock.patch.object(promote, "command", side_effect=command):
            promote.promote(TAG, SHA, "owner/repo")

    def test_promotes_complete_release_metadata_only(self):
        self.exercise()
        self.assertEqual(self.mutations, [("release", "edit", TAG, "--latest", "--repo", "owner/repo")])

    def test_missing_or_unfinished_platform_never_promoted(self):
        for index in range(len(self.assets())):
            with self.subTest(index=index):
                assets = self.assets()
                assets.pop(index)
                with self.assertRaises(ValueError):
                    self.exercise(assets=assets)
                self.assertEqual(self.mutations, [])
        assets = self.assets()
        assets[0]["state"] = "starter"
        with self.assertRaises(ValueError):
            self.exercise(assets=assets)
        self.assertEqual(self.mutations, [])

    def test_same_or_newer_global_latest_is_not_replaced(self):
        for latest in (TAG, "v0.1.0-alpha.32", "v0.1.0"):
            self.exercise(latest=latest)
            self.assertEqual(self.mutations, [])

    def test_moved_tag_and_draft_never_promoted(self):
        for kwargs in (dict(moved=True), dict(draft=True)):
            with self.assertRaises(ValueError):
                self.exercise(**kwargs)
            self.assertEqual(self.mutations, [])

    def test_semver_prerelease_order(self):
        ordered = ("v0.1.0-alpha.9", "v0.1.0-alpha.10", "v0.1.0-beta", "v0.1.0", "v0.2.0-alpha.1")
        self.assertEqual(sorted(ordered, key=promote.version), list(ordered))
        with self.assertRaises(ValueError):
            promote.version("latest")


if __name__ == "__main__":
    unittest.main()
