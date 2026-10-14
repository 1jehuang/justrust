import importlib.util
import json
import os
from pathlib import Path
import runpy
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("toolchain_source", ROOT / "scripts/toolchain-source.py")
source = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(source)


class OwnedToolchainTests(unittest.TestCase):
    def test_owned_sources_licenses_and_shared_cargo(self):
        manifest = json.loads(source.MANIFEST.read_text())
        self.assertEqual([item["name"] for item in manifest["imported"]], ["rustc", "cargo", "backtrace"])
        for component in ("rustc", "cargo"):
            root = ROOT / "toolchain" / component
            self.assertTrue((root / "Cargo.toml").is_file())
            self.assertTrue((root / "LICENSE-APACHE").is_file())
            self.assertTrue((root / "LICENSE-MIT").is_file())
            self.assertFalse((root / ".git").exists())
        self.assertEqual((ROOT / manifest["shared"]["path"]).resolve(), ROOT / "toolchain/cargo")
        self.assertTrue((ROOT / "toolchain/rustc/library/backtrace/src/lib.rs").is_file())
        self.assertTrue((ROOT / "toolchain/rustc/library/backtrace/LICENSE-MIT").is_file())

    def test_no_nested_agent_policy_or_owned_gitlinks(self):
        for path in (ROOT / "toolchain").rglob("*"):
            if "build" in path.relative_to(ROOT / "toolchain").parts:
                continue
            if path.name.lower() in {"agents.md", "claude.md", "justrust.md", "ai_policy.md"}:
                self.fail(f"nested guidance: {path}")
        listing = subprocess.check_output(["git", "ls-files", "--stage", "toolchain"], cwd=ROOT, text=True)
        self.assertFalse(any(line.startswith("160000 ") for line in listing.splitlines()))
        entries = {line.split("\t", 1)[1]: line.split()[0] for line in listing.splitlines()}
        for path in ("toolchain/rustc/compiler/rustc_driver_impl/src/lib.rs",
                     "toolchain/rustc/library/backtrace/src/lib.rs",
                     "toolchain/rustc/src/tools/rustfmt/tests/target/array_comment.rs",
                     "toolchain/cargo/src/lib.rs",
                     "toolchain/cargo/.cargo/config.toml"):
            self.assertEqual(entries.get(path), "100644", f"source not tracked: {path}")
        self.assertEqual(entries.get("toolchain/rustc/src/tools/cargo"), "120000")

    def test_ci_llvm_pin_is_separate_from_imported_fork_identity(self):
        manifest = json.loads(source.MANIFEST.read_text())
        llvm = ROOT / "toolchain/rustc/src/bootstrap/ci-llvm/git-commit-info"
        self.assertEqual(llvm.read_text().splitlines()[0], manifest["ci_llvm"]["rust_commit"])
        self.assertNotEqual(manifest["ci_llvm"]["rust_commit"], manifest["imported"][0]["commit"])
        self.assertNotEqual(manifest["ci_llvm"]["rust_commit"], manifest["ci_llvm"]["source_commit"])

    def test_technical_doc_and_test_fixtures_preserved(self):
        for path in ("toolchain/rustc/src/doc/rustc-dev-guide/src/queries/example-0.html",
                     "toolchain/rustc/src/tools/rust-analyzer/docs/book/src/contributing/testing.md",
                     "toolchain/cargo/.cargo/config.toml",
                     "toolchain/cargo/tests/testsuite/version.rs"):
            self.assertTrue((ROOT / path).is_file(), path)
        fixture = "toolchain/rustc/src/tools/rustfmt/tests/writemode/target/stdin.xml"
        raw = (ROOT / fixture).read_bytes()
        self.assertIn(b"\r\n", raw)
        self.assertEqual(subprocess.check_output(["git", "show", ":" + fixture], cwd=ROOT), raw)

    def test_factors_use_owned_compiler_and_preserve_overrides(self):
        with patch.dict(os.environ, {}, clear=False):
            os.environ.pop("JR_RUSTC_DIR", None)
            os.environ.pop("JR_RUSTC_MODE", None)
            with patch.object(sys, "path", [str(ROOT / "bench")] + sys.path):
                defaults = runpy.run_path(str(ROOT / "bench/factors.py"))
            self.assertEqual(Path(defaults["JR_RUSTC_ENV"]["RUSTC"]),
                             ROOT / "toolchain/rustc/build/jr/pgo-bolt-stage2/bin/rustc")
        with patch.dict(os.environ, {"JR_RUSTC_DIR": "/custom/compiler", "JR_RUSTC_MODE": "release"}):
            with patch.object(sys, "path", [str(ROOT / "bench")] + sys.path):
                custom = runpy.run_path(str(ROOT / "bench/factors.py"))
            self.assertEqual(custom["JR_RUSTC_ENV"]["RUSTC"], "/custom/compiler/build/jr/release/bin/rustc")


class MaterializationTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(dir=os.environ.get("JCODE_SCRATCH_DIR"))
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.repo = self.root / "upstream"
        self.repo.mkdir()
        subprocess.run(["git", "init", "-q", str(self.repo)], check=True)
        (self.repo / "src").mkdir()
        (self.repo / "src/lib.rs").write_text("pub fn example() {}\n")
        (self.repo / "LICENSE").write_text("fixture license\n")
        (self.repo / "AGENTS.md").write_text("inherited guidance\n")
        (self.repo / "technical.md").write_text("technical docs\n")
        (self.repo / ".gitignore").write_text("target/\n")
        (self.repo / "target").mkdir()
        (self.repo / "target/built").write_text("not source\n")
        subprocess.run(["git", "-C", str(self.repo), "add", "."], check=True)
        # These isolated fixtures use the caller's configured Git identity.
        subprocess.run(["git", "-C", str(self.repo), "commit", "-qm", "source fixture"], check=True)
        self.spec = {"name": "fixture", "path": "toolchain/rustc/src/fixture",
                     "commit": source.git(self.repo, "rev-parse", "HEAD"), "origin": str(self.repo)}

    def materialize(self, local=True):
        return source.materialize(self.spec, self.root, self.repo if local else None, self.root / "cache")

    def test_local_archive_only_without_builds_git_or_policy(self):
        result = self.materialize()
        self.assertTrue((result / "src/lib.rs").is_file())
        self.assertTrue((result / "LICENSE").is_file())
        self.assertTrue((result / "technical.md").is_file())
        self.assertFalse((result / "AGENTS.md").exists())
        self.assertFalse((result / ".git").exists())
        self.assertFalse((result / "target").exists())
        self.assertEqual((result / "git-commit-info").read_text().splitlines()[0], self.spec["commit"])
        self.assertTrue((self.repo / ".git").is_dir())
        self.assertTrue((self.repo / "target/built").is_file())

    def test_fetch_archive_path(self):
        self.assertTrue((self.materialize(local=False) / "src/lib.rs").is_file())

    def test_refuses_overwriting_source_edits(self):
        self.materialize()
        with self.assertRaisesRegex(ValueError, "refusing to overwrite"):
            self.materialize()

    def test_refuses_path_escape_and_bad_revision(self):
        self.spec["path"] = "../escape"
        with self.assertRaisesRegex(ValueError, "escapes"):
            self.materialize()
        self.spec["commit"] = "HEAD"
        with self.assertRaisesRegex(ValueError, "full lowercase Git SHA"):
            self.materialize()

    def test_refuses_nested_gitlinks(self):
        subprocess.run(["git", "-C", str(self.repo), "update-index", "--add", "--cacheinfo",
                        f'160000,{self.spec["commit"]},nested'], check=True)
        subprocess.run(["git", "-C", str(self.repo), "commit", "-qm", "nested fixture"], check=True)
        self.spec["commit"] = source.git(self.repo, "rev-parse", "HEAD")
        with self.assertRaisesRegex(ValueError, "nested gitlinks"):
            self.materialize()


if __name__ == "__main__":
    unittest.main()
