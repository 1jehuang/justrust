"""Keep header auth/provider reads stable across every scroll fixture render."""
from pathlib import Path
import hashlib
import re
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import suite
from test_hosted_tui_prerequisites import assertions, fixture_bodies, test_names

PATCH = "bench/patches/jcode-scroll-render-fixture-state.patch"
SOURCE = "crates/jcode-tui/src/tui/app/tests/scroll_copy_03.rs"
READERS = re.compile(r"(?:create_(?:scroll_)?test_app|anchor_test_app|compacted_history_app_with_remaining)\(")


def check_coverage(case, source):
    readers = {name: body for name, body in fixture_bodies(source).items() if READERS.search(body)}
    case.assertEqual(len(readers), 30)
    for name, body in readers.items():
        case.assertTrue(body.startswith("    with_local_jcode_home(|| {\n"), name)
        case.assertTrue(body.endswith("    });"), name)
        case.assertNotIn("lock_test_env()", body, name)
        # Environment ownership precedes render-state ownership, never inverted.
        if "scroll_render_test_lock()" in body:
            case.assertLess(body.index("with_local_jcode_home"), body.index("scroll_render_test_lock"), name)
    return readers


class ScrollRenderFixture(unittest.TestCase):
    def test_registered_patch_changes_snapshot_identity(self):
        spec = suite.SNAPSHOTS["desktop"]
        self.assertIn(("jcode", PATCH), spec["patches"])
        before = suite.snapshot_fingerprint("desktop")
        with patch.dict(suite.SNAPSHOTS, {"desktop": dict(spec, patches=[p for p in spec["patches"] if p[1] != PATCH])}):
            self.assertNotEqual(before, suite.snapshot_fingerprint("desktop"))
        text = (suite.ROOT / PATCH).read_text()
        targets = [line[6:] for line in text.splitlines() if line.startswith("+++ b/")]
        self.assertEqual(targets, [SOURCE])
        additions = [line[1:] for line in text.splitlines() if line.startswith("+") and not line.startswith("+++")]
        self.assertTrue(all(line.strip() in ("with_local_jcode_home(|| {", "});", "}") for line in additions))
        self.assertNotIn("RUST_TEST_THREADS", text)

    def test_corrupt_patch_rejected_before_snapshot_reuse(self):
        expected = suite.SNAPSHOTS["desktop"]["fixture_patch_sha256"][PATCH]
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            path = root / PATCH
            path.parent.mkdir(parents=True)
            path.write_text("corrupt fixture patch")
            spec = {"fixture_patch_sha256": {PATCH: expected}, "patches": [("jcode", PATCH)]}
            with patch.object(suite, "ROOT", root), patch.dict(suite.SNAPSHOTS, {"corrupt-fixture": spec}):
                with self.assertRaisesRegex(ValueError, "corrupt or unversioned fixture patch"):
                    suite.snapshot_fingerprint("corrupt-fixture")

    def test_unexpected_input_fails_closed_and_identity_changes(self):
        spec = suite.SNAPSHOTS["desktop"]
        before = suite.snapshot_fingerprint("desktop")
        with patch.dict(suite.SNAPSHOTS, {"desktop": dict(spec, fixture_input_sha256={})}):
            self.assertNotEqual(before, suite.snapshot_fingerprint("desktop"))
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "unexpected.rs"
            path.write_text("unexpected fixture source")
            with self.assertRaisesRegex(ValueError, "unexpected pinned fixture input SHA256"):
                suite.verify_fixture_input(path, spec["fixture_input_sha256"]["jcode/" + SOURCE])

    def test_exact_scope_transform_preserves_every_test_and_assertion(self):
        repo, revision = suite.SNAPSHOTS["desktop"]["repos"]["jcode"]
        original = subprocess.check_output(["git", "-C", str(repo), "show", f"{revision}:{SOURCE}"], text=True)
        expected_hash = suite.SNAPSHOTS["desktop"]["fixture_input_sha256"]["jcode/" + SOURCE]
        self.assertEqual(hashlib.sha256(original.encode()).hexdigest(), expected_hash)
        def scope(match):
            if not READERS.search(match.group(2)):
                return match.group(0)
            return match.group(1) + "    with_local_jcode_home(|| {\n" + match.group(2) + "\n    });\n}"
        expected = re.sub(r"(#\[test\]\nfn \w+\(\) \{\n)(.*?)(?=\n\})\n\}", scope, original, flags=re.S)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            destination = root / SOURCE
            destination.parent.mkdir(parents=True)
            destination.write_text(original)
            subprocess.run(["git", "apply", "--check", str(suite.ROOT / PATCH)], cwd=root, check=True)
            subprocess.run(["git", "apply", str(suite.ROOT / PATCH)], cwd=root, check=True)
            corrected = destination.read_text()
        self.assertEqual(corrected, expected)
        self.assertEqual(test_names(corrected), test_names(original))
        self.assertEqual(assertions(corrected), assertions(original))
        self.assertEqual(original.count("#[ignore"), corrected.count("#[ignore"))
        readers = check_coverage(self, corrected)
        self.assertIn("command_palette_open_does_not_move_existing_rows", readers)
        # Coverage is constructor-derived, not a whitelist of observed failures.
        for name in readers:
            anchor = f"fn {name}() {{\n    with_local_jcode_home(|| {{\n"
            with self.assertRaisesRegex(AssertionError, name):
                check_coverage(self, corrected.replace(anchor, f"fn {name}() {{\n"))


if __name__ == "__main__":
    unittest.main()
