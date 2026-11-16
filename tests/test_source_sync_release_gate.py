"""No-build regression checks for the clean release benchmark gate."""
import json
import hashlib
import os
from pathlib import Path
import shlex
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
HELPER = ROOT / "ci/source-sync-release-gate.sh"


class SourceSyncReleaseGateTests(unittest.TestCase):
    def shell(self, body, env=None):
        return subprocess.run(
            ["bash", "-c", f"source {shlex.quote(str(HELPER))}\n{body}"],
            env=env, text=True, capture_output=True,
        )

    def benchmark(self, exit_code):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "bench").mkdir()
            (root / "bin").mkdir()
            (root / "evidence").mkdir()
            (root / "bench/suite.py").write_text(
                "import json, os, sys\n"
                "from pathlib import Path\n"
                "Path('invocation.json').write_text(json.dumps({'argv': sys.argv[1:], "
                "'env': dict(os.environ)}))\n"
                f"sys.exit({exit_code})\n"
            )
            env = dict(os.environ, JUSTRUST_REMOTE_REQUIRED="1", JUSTRUST_TOOLCHAIN="wrong")
            result = self.shell(
                f"root={shlex.quote(temporary)}\nwork=$root\nevidence=$root/evidence\n"
                "benchmark_binary before " + "a" * 40,
                env,
            )
            invocation = json.loads((root / "invocation.json").read_text())
            self.assertTrue((root / "evidence/before.log").is_file())
            return result, invocation

    def test_exact_full_gate_and_explicit_local_binary(self):
        result, invocation = self.benchmark(0)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(invocation["argv"][:9], [
            "run", "--synth", "1", "--points", "40", "-n", "1", "--stop-on-invalid", "-o",
        ])
        env = invocation["env"]
        self.assertEqual(env["JUSTRUST_LOCAL"], "1")
        self.assertTrue(env["JUSTRUST_LOCAL_REASON"])
        self.assertEqual(env["JUSTRUST"], env["JUSTRUST_BIN"])
        self.assertTrue(env["JUSTRUST"].endswith("/bin/before"))
        self.assertEqual(env["JUSTRUST_BENCH_REV"], "a" * 40)
        self.assertNotIn("JUSTRUST_REMOTE_REQUIRED", env)
        self.assertNotIn("JUSTRUST_TOOLCHAIN", env)
        self.assertNotIn("BENCH_NOISY_CORES", env)

    def test_invalid_sample_exit_survives_log_tee(self):
        result, _ = self.benchmark(2)
        self.assertEqual(result.returncode, 2, result.stderr)

    def test_mutable_or_abbreviated_refs_rejected(self):
        for revision in ("stable", "9411dc0c8", "A" * 40):
            result = self.shell(f"root=/nonexistent\nvalidate_ref {revision}")
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("40-character", result.stderr)

    def test_snapshot_pack_preserves_exact_commit_without_private_history(self):
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            source = base / "source"
            source.mkdir()
            def git(*args, **kwargs):
                return subprocess.check_output(["git", "-C", str(source), *args], **kwargs)
            git("init", "--quiet", "--template=")
            git("config", "user.name", "Fixture")
            git("config", "user.email", "fixture@example.invalid")
            (source / "private").write_text("unrelated private ancestor")
            git("add", ".")
            git("commit", "-qm", "private ancestor")
            parent = git("rev-parse", "HEAD").decode().strip()
            git("rm", "private")
            (source / "public").write_text("pinned source")
            git("add", ".")
            git("commit", "-qm", "pin")
            revision = git("rev-parse", "HEAD").decode().strip()
            (source / "untracked-secret").write_text("never exported")
            objects = git("rev-list", "--objects", "--no-walk", revision)
            payload = git("pack-objects", "--stdout", input=objects)
            pack = base / "pin.pack"
            pack.write_bytes(payload)
            checksum = hashlib.sha256(payload).hexdigest()
            target = base / "target"
            def import_pack(destination, sha=checksum, commit=revision):
                return self.shell("import_snapshot_pack " + " ".join(map(shlex.quote, [
                    str(destination), commit, str(pack), sha])))
            result = import_pack(target)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(subprocess.check_output([
                "git", "-C", str(target), "show", "HEAD:public"]), b"pinned source")
            self.assertNotEqual(subprocess.run([
                "git", "-C", str(target), "cat-file", "-e", parent],
                capture_output=True).returncode, 0)
            self.assertFalse((target / "untracked-secret").exists())
            self.assertNotEqual(import_pack(target).returncode, 0)
            self.assertNotEqual(import_pack(base / "bad", "0" * 64).returncode, 0)
            self.assertFalse((base / "bad").exists())
            self.assertNotEqual(import_pack(base / "wrong", commit=parent).returncode, 0)

    def test_minimal_archive_has_compile_inputs_not_toolchain_or_git(self):
        with tempfile.TemporaryDirectory() as temporary:
            result = self.shell(
                f"root={shlex.quote(str(ROOT))}\n"
                f"export_source f273df4428faf905ec97c735a4ef1b60d59bbb21 {shlex.quote(temporary)}"
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            tree = Path(temporary)
            for path in ("Cargo.toml", "Cargo.lock", "src/main.rs", "docs/AGENTS-snippet.md", "scripts/sign-release.py"):
                self.assertTrue((tree / path).is_file(), path)
            self.assertFalse((tree / "toolchain").exists())
            self.assertFalse((tree / ".git").exists())


if __name__ == "__main__":
    unittest.main()
