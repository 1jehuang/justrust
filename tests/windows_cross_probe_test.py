#!/usr/bin/env python3
"""Standard-library tests, including real installed LLVM interfaces."""
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import tomllib
import unittest
from unittest.mock import patch

SCRIPT = Path(__file__).resolve().parents[1] / "scripts/windows-cross-probe.py"
spec = importlib.util.spec_from_file_location("probe", SCRIPT)
probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(probe)


class WindowsCrossProbe(unittest.TestCase):
    def test_opt_in_required_and_no_output_created(self):
        with tempfile.TemporaryDirectory() as root:
            out = Path(root) / "untouched"
            result = subprocess.run(["python3", SCRIPT, "--out", out], capture_output=True)
            self.assertEqual(result.returncode, 2)
            self.assertFalse(out.exists())

    def test_existing_output_is_never_overwritten(self):
        with tempfile.TemporaryDirectory() as root:
            marker = Path(root) / "marker"
            marker.write_text("keep")
            result = subprocess.run(["python3", SCRIPT, "--opt-in", "--out", root], capture_output=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(marker.read_text(), "keep")
            self.assertEqual(list(Path(root).iterdir()), [marker])

    def test_environment_does_not_forward_credentials_or_rust_flags(self):
        with patch.dict(os.environ, {"AZURE_CLIENT_SECRET": "secret", "AWS_SECRET_ACCESS_KEY": "secret",
                                    "SSH_AUTH_SOCK": "secret", "RUSTFLAGS": "bad", "CARGO_HOME": "secret"}):
            self.assertEqual(set(probe.clean_env()) - {"PATH", "HOME", "LANG", "TMPDIR"}, set())

    def test_bootstrap_config_and_wrapper_with_spaces(self):
        with tempfile.TemporaryDirectory(prefix="windows cross ") as root:
            out = Path(root) / "out"
            out.mkdir()
            sdk = Path(root) / "sdk"
            for p in ("crt/include", "sdk/include/ucrt", "sdk/include/um", "sdk/include/shared",
                      "crt/lib/aarch64", "sdk/lib/ucrt/aarch64", "sdk/lib/um/aarch64"):
                (sdk / p).mkdir(parents=True)
            wrapper, _, _ = probe.bootstrap(out, sdk, "aarch64-pc-windows-msvc", "/usr/bin/clang", "/usr/bin/lld-link")
            config = tomllib.loads((out / ".cargo/config.toml").read_text())
            self.assertIn("aarch64-pc-windows-msvc", config["target"])
            self.assertEqual(config["env"]["CC_aarch64_pc_windows_msvc"], str(wrapper))
            subprocess.run(["sh", "-n", wrapper], check=True)
            self.assertIn("--target=aarch64-pc-windows-msvc", wrapper.read_text())
            self.assertIn("aarch64", str(config["target"]["aarch64-pc-windows-msvc"]["rustflags"]))
            if shutil.which("clang"):
                source = out / "wrapper probe.c"
                source.write_text("int answer(void) { return 42; }\n")
                obj = out / "wrapper probe.obj"
                subprocess.run([wrapper, "/c", source, f"/Fo{obj}"], check=True)
                self.assertEqual(int.from_bytes(obj.read_bytes()[:2], "little"), 0xAA64)
            if shutil.which("cargo"):
                result = subprocess.run(["cargo", "metadata", "--offline", "--no-deps",
                                         "--format-version", "1"], cwd=out,
                                        capture_output=True, text=True, check=True)
                self.assertEqual(json.loads(result.stdout)["packages"][0]["name"], "windows-cross-probe")

    def test_missing_sdk_rejected(self):
        with tempfile.TemporaryDirectory() as root:
            with self.assertRaisesRegex(ValueError, "missing SDK directory"):
                probe.bootstrap(Path(root), Path(root) / "absent", "x86_64-pc-windows-msvc", "clang", "lld-link")

    def test_invalid_pe_rejected(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "bad.exe"
            path.write_bytes(b"not PE")
            with self.assertRaises(ValueError):
                probe.machine(path)

    @unittest.skipUnless(shutil.which("clang") and shutil.which("lld-link") and shutil.which("rustc"),
                         "installed LLVM and rustc required")
    def test_actual_cross_compile_and_link_both_architectures(self):
        for target, (_, expected) in probe.TARGETS.items():
            with self.subTest(target=target), tempfile.TemporaryDirectory(prefix="cross probe ") as root:
                out = Path(root) / "out"
                result = subprocess.run(["python3", SCRIPT, "--opt-in", "--out", out,
                                         "--target", target], capture_output=True, text=True)
                self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
                report = json.loads((out / "report.json").read_text())
                self.assertIn("freestanding_pe", report, result.stdout)
                self.assertEqual(probe.machine(out / "probe.exe"), expected)
                self.assertIn("no pre-provisioned Windows SDK/CRT supplied", report["blockers"])
                self.assertTrue(all(c["exit_code"] == 0 for c in report["commands"]))


if __name__ == "__main__":
    unittest.main()
