"""Pinned GitHub applet readiness, with deterministic section completion order."""
import json
from pathlib import Path
import subprocess
import tempfile
import types
import unittest
from unittest.mock import patch

import suite


REPAIR = "bench/patches/desktop-github-applet-readiness.patch"
RUNTIME = "crates/jcode-desktop-ui/src/applet_runtime.rs"


class GithubAppletReadinessTests(unittest.TestCase):
    def pinned(self, path):
        source, rev = suite.SNAPSHOTS["desktop"]["repos"]["jcode-desktop"]
        result = subprocess.run(["git", "-C", str(source), "show", f"{rev}:{path}"],
                                capture_output=True, text=True)
        if result.returncode:
            self.skipTest("pinned desktop source unavailable: " + result.stderr)
        return result.stdout

    def test_registration_invalidates_snapshot_identity(self):
        self.assertIn(("jcode-desktop", REPAIR), suite.SNAPSHOTS["desktop"]["patches"])
        before = suite.snapshot_fingerprint("desktop")
        spec = dict(suite.SNAPSHOTS["desktop"])
        spec["patches"] = [entry for entry in spec["patches"] if entry[1] != REPAIR]
        with patch.dict(suite.SNAPSHOTS, {"desktop": spec}):
            self.assertNotEqual(before, suite.snapshot_fingerprint("desktop"))

    def test_patch_applies_to_pin_and_preserves_entire_remaining_test(self):
        original = self.pinned(RUNTIME)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            target = root / RUNTIME
            target.parent.mkdir(parents=True)
            target.write_text(original)
            subprocess.run(["git", "-C", str(root), "apply", "--check", str(suite.ROOT / REPAIR)], check=True)
            subprocess.run(["git", "-C", str(root), "apply", str(suite.ROOT / REPAIR)], check=True)
            repaired = target.read_text()
        old_wait = '        wait("inbox rows", &|r| doc(r).contains("acme/app#7"));'
        new_wait = '''        // Sections refresh independently. My PRs can mount its row before the
        // review request finishes, so wait for both facts asserted below.
        wait("inbox rows and review count", &|r| {
            let document = doc(r);
            document.contains("acme/app#7")
                && document.contains("Review requested 1")
        });'''
        self.assertEqual(repaired, original.replace(old_wait, new_wait, 1))
        self.assertIn('assert!(doc(&runtime).contains("Review requested 1"));', repaired)

    def test_my_prs_completion_before_review_reproduces_old_predicate_race(self):
        # Execute the actual pinned provider, replacing only external I/O and
        # thread launch. Queued callbacks make the adverse ordering deterministic.
        cards = types.SimpleNamespace(Cards=lambda *args: None, claims=lambda: [])
        namespace = {"__name__": "pinned_github_provider"}
        with patch.dict("sys.modules", {"tool_cards": cards}):
            exec(compile(self.pinned("applets/github/provider.py"), "provider.py", "exec"), namespace)
        app = namespace["App"]()
        app.instance = "github#launch0"
        jobs = []
        messages = []
        app.spawn = lambda fn, *args: jobs.append(lambda: fn(*args))
        namespace["send"] = messages.append
        row = {"number": 7, "title": "Add pills", "url": "https://github.com/acme/app/pull/7",
               "repository": {"nameWithOwner": "acme/app"}, "author": {"login": "ana"}}
        namespace["section_search"] = lambda section, *args: [row] if section in ("review", "prs") else []
        app.refresh_all()
        self.assertEqual(len(jobs), len(namespace["SECTIONS"]))
        jobs[1]()  # My PRs finishes while review remains queued.
        document = json.dumps(messages[-1]["document"])
        self.assertIn("acme/app#7", document)  # Original wait returns too early.
        self.assertNotIn("Review requested 1", document)  # Original assertion fails.
        jobs[0]()  # Review now completes, satisfying both readiness conditions.
        document = json.dumps(messages[-1]["document"])
        self.assertIn("acme/app#7", document)
        self.assertIn("Review requested 1", document)


if __name__ == "__main__":
    unittest.main()
