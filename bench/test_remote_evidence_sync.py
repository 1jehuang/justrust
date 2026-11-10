"""Exercise the actual on-remote rsync options against nested host evidence."""
from pathlib import Path
import os
import shlex
import shutil
import subprocess
import tempfile
import unittest


@unittest.skipUnless(shutil.which("rsync"), "rsync required")
class EvidenceSyncTests(unittest.TestCase):
    def test_source_sync_preserves_nested_host_only_results(self):
        script = (Path(__file__).parent / "on-remote.sh").read_text()
        line = next(line.strip() for line in script.splitlines()
                    if line.strip().startswith("rsync_t --delete"))
        options = shlex.split(line)[1:-2]
        helper = next(line.strip() for line in script.splitlines()
                      if line.strip().startswith("rsync -"))
        helper_tokens = shlex.split(helper)
        base_command = helper_tokens[:helper_tokens.index("-e")]
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "source"
            dest = Path(directory) / "dest"
            source.mkdir()
            dest.mkdir()
            (source / "bench").mkdir()
            (source / "toolchain/rustc/build").mkdir(parents=True)
            (source / "toolchain/rustc/build/large-artifact").write_text("do not upload")
            (dest / "toolchain").mkdir()
            (dest / "toolchain/host-only").write_text("preserve")
            (source / "source.rs").write_text("new source with changed size")
            (dest / "source.rs").write_text("old source")
            (dest / "removed-source.rs").write_text("stale")
            evidence = ["bench/results/old.json", "bench/results/critpath/host/t1/unit.mm_profdata",
                        "bench/results/critpath/host/capture.json", "bench/results/partial/marker"]
            for name in evidence:
                path = dest / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(b"host-only evidence\x00\x01")
            subprocess.run([*base_command, *options, str(source) + "/", str(dest) + "/"], check=True)
            self.assertEqual((dest / "source.rs").read_text(), "new source with changed size")
            self.assertFalse((dest / "removed-source.rs").exists())
            self.assertFalse((dest / "toolchain/rustc/build/large-artifact").exists())
            self.assertEqual((dest / "toolchain/host-only").read_text(), "preserve")
            for name in evidence:
                self.assertEqual((dest / name).read_bytes(), b"host-only evidence\x00\x01")

    def test_remote_runs_pin_distinct_client_binaries(self):
        script = (Path(__file__).parent / "on-remote.sh").read_text()
        line = next(line.strip() for line in script.splitlines()
                    if line.strip().startswith("printf 'export PATH="))
        self.assertIn('"R:bench-bin/$NAME/.justrust.upload"', script)
        with tempfile.TemporaryDirectory() as home:
            for name in ("baseline", "candidate"):
                generated = subprocess.run(
                    ["bash", "-c", 'NAME="$1"\n' + line, "generate", name],
                    text=True, capture_output=True, check=True).stdout
                observed = subprocess.run(
                    ["bash", "-c", generated + '\nprintf "%s\\n" "$JUSTRUST_BIN" "$JUSTRUST_NO_UPDATE" "${PATH%%:*}"'],
                    env={"HOME": home, "PATH": os.environ["PATH"]},
                    text=True, capture_output=True, check=True).stdout.splitlines()
                self.assertEqual(observed, [f"{home}/bench-bin/{name}/justrust", "1",
                                            f"{home}/bench-bin/{name}"])

    def test_capture_default_and_shell_syntax(self):
        script = Path(__file__).parent / "critpath-capture.sh"
        subprocess.run(["bash", "-n", str(script)], check=True)
        subprocess.run(["bash", "-n", str(script.parent / "on-remote.sh")], check=True)
        self.assertIn('"generic-activity,query-blocked"', script.read_text())
        self.assertIn('"profile_events": events', script.read_text())
        remote = (script.parent / "on-remote.sh").read_text()
        self.assertIn('echo "unset JUSTRUST_BENCH_REV"', remote)
        self.assertIn('export JUSTRUST_BENCH_REPO_REV=$rev', remote)
        self.assertNotIn('export JUSTRUST_BENCH_REV=$rev', remote)


if __name__ == "__main__":
    unittest.main()
