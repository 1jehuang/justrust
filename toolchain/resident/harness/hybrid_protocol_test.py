#!/usr/bin/env python3
"""Real jr-engine hybrid protocol acceptance tests.

Run with JR_HYBRID_TEST_ENGINE=/absolute/jr-engine and
JR_HYBRID_TEST_TOOLCHAIN=/absolute/pinned-toolchain. No compilation is launched
unless both are set. Every test has private native artifacts and daemon state.
The caller coordinates engine builds separately. These tests never replace the
installed shipping client.
"""

import os
from pathlib import Path
import select
import subprocess
import tempfile
import time
import unittest


ENGINE = os.environ.get("JR_HYBRID_TEST_ENGINE")
TOOLCHAIN = os.environ.get("JR_HYBRID_TEST_TOOLCHAIN")


def decision(data):
    """Only an exact positive miss receipt permits fallback, never an exit code."""
    if data == b"miss\n":
        return "miss"
    if data == b"committed\n":
        return "committed"
    raise ValueError(f"ambiguous hybrid decision: {data!r}")


class DecisionTests(unittest.TestCase):
    def test_exact_receipts(self):
        self.assertEqual(decision(b"miss\n"), "miss")
        self.assertEqual(decision(b"committed\n"), "committed")

    def test_ambiguous_receipts_never_mean_miss(self):
        for data in [b"", b"miss", b"miss\ncommitted\n", b"committed\nmiss\n", b"miss\n\n", b"MISS\n"]:
            with self.subTest(data=data), self.assertRaises(ValueError):
                decision(data)


@unittest.skipUnless(ENGINE and TOOLCHAIN, "set JR_HYBRID_TEST_ENGINE and JR_HYBRID_TEST_TOOLCHAIN")
class EngineTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="jrhy-")
        self.root = Path(self.temp.name)
        self.project = self.root / "project"
        self.project.mkdir()
        (self.project / "src").mkdir()
        self.home = self.root / "home"
        self.home.mkdir(mode=0o700)
        self.marker = self.root / "executions"
        self.manifest = self.project / "Cargo.toml"
        self.manifest.write_text('[package]\nname="hybrid_acceptance"\nversion="0.1.0"\nedition="2021"\n')
        self.source = self.project / "src/lib.rs"
        self.write_test()
        tc = Path(TOOLCHAIN).resolve()
        self.env = dict(os.environ)
        for key in list(self.env):
            if key.startswith(("RA_JIT_", "BLAZE_", "JR_HYBRID_")):
                del self.env[key]
        self.env.update({
            "JR_ENGINE_HOME": str(self.home),
            "CARGO_TARGET_DIR": str(self.root / "native-target"),
            "RUSTUP_TOOLCHAIN": str(tc),
            "RUSTC": str(tc / "bin/rustc"),
            "CARGO": str(tc / "bin/cargo"),
            "PATH": str(tc / "bin") + os.pathsep + self.env.get("PATH", ""),
            "RA_JIT_SPARE": "0",
            "BLAZE_SPARE": "0",
            "RA_JIT_MANIFEST_CONTENT_GENERATION": "1",
            "RA_JIT_SANDBOX": "off",
            "BLAZE_SANDBOX": "off",
            "HYBRID_ACCEPTANCE_MARKER": str(self.marker),
        })

    def tearDown(self):
        try:
            subprocess.run([ENGINE, "stop"], cwd=self.project, env=self.env,
                           stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=30, check=False)
        finally:
            self.temp.cleanup()

    def write_test(self, fails=False):
        self.source.write_text('''#[test]
fn once() {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new().create(true).append(true)
        .open(std::env::var("HYBRID_ACCEPTANCE_MARKER").unwrap()).unwrap();
    writeln!(file, "fresh").unwrap();
    assert!(%s);
}
''' % ("false" if fails else "true"))

    def invoke(self, mode, *args):
        read_fd, write_fd = os.pipe() if mode == "hybrid-ready" else (None, None)
        env = dict(self.env)
        if write_fd is not None:
            env["JR_HYBRID_DECISION_FD"] = str(write_fd)
        try:
            process = subprocess.Popen([ENGINE, "cargo", mode, "--", *args],
                                       cwd=self.project, env=env,
                                       pass_fds=() if write_fd is None else (write_fd,),
                                       stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        finally:
            if write_fd is not None:
                os.close(write_fd)
        try:
            stdout, stderr = process.communicate(timeout=180)
            if read_fd is None:
                return process.returncode, b"", stdout.decode(errors="replace"), stderr.decode(errors="replace")
            # A daemon accidentally retaining the decision FD must not hang the test.
            data = bytearray()
            deadline = time.monotonic() + 3
            while time.monotonic() < deadline:
                if not select.select([read_fd], [], [], max(0, deadline - time.monotonic()))[0]:
                    self.fail("decision descriptor inherited by a daemon or worker")
                chunk = os.read(read_fd, 4096)
                if not chunk:
                    break
                data.extend(chunk)
                self.assertLess(len(data), 4096, "unbounded decision output")
            else:
                self.fail("decision pipe did not reach EOF")
            return process.returncode, bytes(data), stdout.decode(errors="replace"), stderr.decode(errors="replace")
        except BaseException:
            process.kill()
            process.communicate()
            raise
        finally:
            if read_fd is not None:
                os.close(read_fd)

    def ready(self, *args):
        return self.invoke("hybrid-ready", *args)

    def prewarm(self, *args):
        result = self.invoke("hybrid-prewarm", *args)
        self.assertEqual(result[0], 0, result)
        self.assertFalse(self.marker.exists(), "prewarm executed a user program or test")
        return result

    def executions(self):
        return self.marker.read_text().splitlines() if self.marker.exists() else []

    def test_cold_miss_does_not_start_daemon_or_execute(self):
        before = set(self.home.rglob("*.sock"))
        result = self.ready("test", "--lib")
        self.assertEqual((result[0], decision(result[1])), (0, "miss"), result)
        self.assertEqual(self.executions(), [])
        self.assertEqual(set(self.home.rglob("*.sock")), before)
        self.assertFalse((self.root / "native-target").exists(), result)

    def test_prewarm_never_runs_tests_and_ready_runs_once(self):
        self.prewarm("test", "--lib")
        result = self.ready("test", "--lib")
        self.assertEqual((result[0], decision(result[1])), (0, "committed"), result)
        self.assertEqual(self.executions(), ["fresh"])

    def test_ready_no_run_never_executes(self):
        self.prewarm("test", "--lib")
        result = self.ready("test", "--lib", "--no-run")
        self.assertEqual((result[0], decision(result[1])), (0, "committed"), result)
        self.assertEqual(self.executions(), [])

    def test_assertion_failure_is_committed_and_not_retried(self):
        self.prewarm("test", "--lib")
        self.write_test(fails=True)
        result = self.ready("test", "--lib")
        self.assertNotEqual(result[0], 0, result)
        self.assertEqual(decision(result[1]), "committed", result)
        self.assertEqual(self.executions(), ["fresh"])
        self.assertIn("FAILED", result[2] + result[3], result)

    def test_type_error_is_committed_with_diagnostics(self):
        self.prewarm("test", "--lib")
        self.source.write_text("pub fn broken() { let _: u32 = false; }\n")
        result = self.ready("test", "--lib")
        self.assertNotEqual(result[0], 0, result)
        self.assertEqual(decision(result[1]), "committed", result)
        self.assertEqual(self.executions(), [])
        self.assertTrue("mismatch" in result[2] + result[3] or "expected" in result[2] + result[3], result)

    def test_manifest_same_timestamp_change_is_miss(self):
        self.prewarm("test", "--lib")
        previous = self.manifest.stat()
        self.manifest.write_text(self.manifest.read_text().replace('version="0.1.0"', 'version="0.2.0"'))
        os.utime(self.manifest, ns=(previous.st_atime_ns, previous.st_mtime_ns))
        result = self.ready("test", "--lib")
        self.assertEqual((result[0], decision(result[1])), (0, "miss"), result)
        self.assertEqual(self.executions(), [])

    def test_configuration_change_is_miss(self):
        self.prewarm("test", "--lib")
        config_dir = self.project / ".cargo"
        config_dir.mkdir()
        (config_dir / "config.toml").write_text('[build]\nrustflags=["--cfg", "hybrid_changed"]\n')
        result = self.ready("test", "--lib")
        self.assertEqual((result[0], decision(result[1])), (0, "miss"), result)
        self.assertEqual(self.executions(), [])

    def test_custom_build_directory_never_writes_shipping_outputs(self):
        self.prewarm("test", "--lib")
        shipping = self.root / "shipping-build"
        shipping.mkdir()
        sentinel = shipping / "sentinel"
        sentinel.write_text("shipping must remain untouched")
        before = {path.relative_to(shipping): path.read_bytes()
                  for path in shipping.rglob("*") if path.is_file()}
        # CLI configuration has precedence over an inherited native build-dir.
        self.env["CARGO_BUILD_BUILD_DIR"] = str(self.root / "native-target/build")
        config = f'build.build-dir="{shipping}"'
        result = self.ready("test", "--lib", "--config", config)
        self.assertEqual((result[0], decision(result[1])), (0, "miss"), result)
        result = self.invoke("hybrid-prewarm", "test", "--lib", "--config", config)
        self.assertNotEqual(result[0], 0, result)
        after = {path.relative_to(shipping): path.read_bytes()
                 for path in shipping.rglob("*") if path.is_file()}
        self.assertEqual(after, before)
        self.assertEqual(self.executions(), [])

    def test_prewarm_doc_never_runs_documentation(self):
        self.source.write_text('''/// ```
/// std::fs::write(std::env::var("HYBRID_ACCEPTANCE_MARKER").unwrap(), "doc").unwrap();
/// ```
pub fn documented() {}
''')
        # Doc-only prewarm may be rejected as unsupported, but must never execute.
        result = self.invoke("hybrid-prewarm", "test", "--doc")
        self.assertFalse(self.marker.exists(), result)

    def test_prewarm_run_never_executes_binary(self):
        (self.project / "src/main.rs").write_text('''fn main() {
    std::fs::write(std::env::var("HYBRID_ACCEPTANCE_MARKER").unwrap(), "run").unwrap();
    std::process::exit(77);
}
''')
        self.prewarm("run")
        result = self.ready("run")
        self.assertEqual((result[0], decision(result[1])), (77, "committed"), result)
        self.assertEqual(self.marker.read_text(), "run")


if __name__ == "__main__":
    unittest.main(verbosity=2)
