"""Exercise owned build-helper selection with a fake bootstrap, never a compiler."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


class OwnedBuildHelpersTests(unittest.TestCase):
    def exercise(self, enabled):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = Path(__file__).resolve().parents[1] / "toolchain/rustc/build-jr.sh"
            shutil.copyfile(source, root / "build-jr.sh")
            (root / "justrust.bootstrap.toml").write_text("[build]\n\n[rust]\n")
            bootstrap = root / "x"
            bootstrap.write_text("#!/bin/sh\nset -eu\nprintf '%s\\n' \"$@\" > args.txt\n"
                                 "mkdir -p build/host/stage1/bin\n"
                                 "printf 'fake compiler' > build/host/stage1/bin/rustc\n")
            bootstrap.chmod(0o755)
            env = dict(os.environ)
            for name in ("JR_TOOLS", "JR_STAGE", "JR_PGO_PROFILE"):
                env.pop(name, None)
            if enabled:
                env["JR_TOOLS"] = "1"
            result = subprocess.run(["bash", str(root / "build-jr.sh"), "dev"],
                                    cwd=root, env=env, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            args = (root / "args.txt").read_text().splitlines()
            self.assertEqual(args[:3], ["build", "--stage", "1"])
            self.assertIn("compiler/rustc", args)
            for tool in ("src/tools/rustdoc", "src/tools/clippy", "src/tools/rustfmt"):
                self.assertEqual(tool in args, enabled, tool)
            self.assertEqual((root / "build/jr/dev/bin/rustc").read_text(), "fake compiler")

    def test_owned_helper_build_includes_formatter_bootstrap_path(self):
        self.exercise(True)

    def test_compiler_only_mode_does_not_add_helpers(self):
        self.exercise(False)


if __name__ == "__main__":
    unittest.main()
