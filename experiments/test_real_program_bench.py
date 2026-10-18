"""Unit tests plus explicitly synthetic fake-compiler CLI acceptance tests.

These tests do not establish Rust compatibility or measure compiler speed.
Run: python3 -m unittest discover -s experiments -p test_real_program_bench.py
"""
import argparse
from datetime import datetime, timezone
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

MODULE_PATH = Path(__file__).with_name("real_program_bench.py")
SPEC = importlib.util.spec_from_file_location("real_program_bench", MODULE_PATH)
bench = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(bench)


class HelpersTest(unittest.TestCase):
    def test_oracle_dimensions_alphabet_and_known_points(self):
        output = bench.mandelbrot_oracle()
        rows = output.splitlines()
        self.assertEqual(len(output), 4840)
        self.assertEqual(len(rows), 40)
        self.assertTrue(all(len(row) == 120 for row in rows))
        self.assertEqual(set(output), set(b" @#+.\n"))
        self.assertEqual(rows[20][80], ord("@"))  # c = 0
        self.assertEqual(rows[20][0], ord("@"))  # c = -2 stays bounded
        self.assertEqual(rows[0][0], ord(" "))
        self.assertTrue(output.endswith(b"\n"))

    def test_adler_oracle(self):
        a, b = 1, 0
        for _ in range(5553):
            a = (a + 255) % 65521
            b = (b + a) % 65521
        self.assertEqual(bench.expected_stdout("adler2-ff5553"),
                         f"ff_5553:{(b << 16) | a}\n".encode())
        self.assertEqual(bench.expected_stdout("mandelbrot"), bench.mandelbrot_oracle())
        with self.assertRaises(ValueError):
            bench.expected_stdout("unknown")

    def test_cpu_model_keeps_only_first_model(self):
        with mock.patch.object(bench.Path, "exists", return_value=True), mock.patch.object(
                bench.Path, "read_text", return_value="processor : 0\nmodel name : CPU model\n"
                "processor : 1\nmodel name : CPU model\n"):
            self.assertEqual(bench.cpu_model(), "CPU model")

    def test_positive_validators(self):
        self.assertEqual(bench.positive_int("5"), 5)
        self.assertEqual(bench.positive_timeout("0.1"), 0.1)
        for value in ("0", "-1"):
            with self.assertRaises(argparse.ArgumentTypeError):
                bench.positive_int(value)
        for value in ("0", "-1", "nan", "inf"):
            with self.assertRaises(argparse.ArgumentTypeError):
                bench.positive_timeout(value)

    def test_rotation_summary_and_flags(self):
        self.assertEqual(bench.rotate(bench.CONFIGS, 4),
                         ("stock-debug", "stock-release", "native"))
        self.assertEqual(bench.summary([])["median_ns"], None)
        self.assertEqual(bench.summary([10, 2, 6]),
                         dict(samples_ns=[10, 2, 6], count=3, median_ns=6,
                              min_ns=2, max_ns=10))
        self.assertEqual(bench.compiler_command("native", "jr", "rustc", "in", "out", 2),
                         ["jr", "in", "--workers", "2", "--debug-assertions", "yes", "-o", "out"])
        for name, level in (("stock-debug", "0"), ("stock-release", "3")):
            self.assertEqual(bench.compiler_command(name, "jr", "rustc", "in", "out", 1),
                             ["rustc", "--edition=2024", "-C", "opt-level=" + level,
                              "-C", "overflow-checks=yes", "-C", "debug-assertions=yes",
                              "-C", "panic=abort", "in", "-o", "out"])

    def test_defaults(self):
        args = bench.parser().parse_args(["--binary", "jr", "--output", "out"])
        self.assertEqual((args.repeats, args.runtime_repeats, args.timeout, args.workers),
                         (5, 7, 120, 1))
        self.assertEqual(args.source, bench.ROOT / "experiments/programs/mandelbrot.rs")

    def test_sanitization(self):
        recorder = bench.Recorder([("/home/alice/repo", "<repo>"),
                                   ("/home/alice", "<home>"),
                                   ("/work/temp", "<scratch>")], 1)
        self.assertEqual(recorder.sanitize({"args": ["/home/alice/repo/a", "/work/temp/b"]}),
                         {"args": ["<repo>/a", "<scratch>/b"]})

    def test_resolution_preserves_symlink_dispatch_name(self):
        with mock.patch.object(bench.shutil, "which", return_value="/tools/rustc"):
            self.assertEqual(bench.resolve_binary("rustc"), "/tools/rustc")

    def test_recorder_timeout_retains_full_partial_output(self):
        recorder = bench.Recorder([], 1)
        with mock.patch.object(bench.subprocess, "run", side_effect=subprocess.TimeoutExpired(
                ["compiler"], 1, output=b"full stdout\xff", stderr=b"full stderr")):
            event = recorder.run(["compiler"], "compile", timed=True)
        self.assertFalse(event["ok"])
        self.assertEqual(event["error"], "timeout")
        self.assertEqual(event["stdout"], "full stdout\\xff")
        self.assertEqual(event["stderr"], "full stderr")
        self.assertGreaterEqual(event["elapsed_ns"], 0)


class SyntheticCliTest(unittest.TestCase):
    """Real runner CLI with fake compilers, never real Rust measurements."""

    def setUp(self):
        scratch = Path(os.environ.get("JCODE_SCRATCH_DIR", bench.ROOT / "target/real-program-bench-scratch"))
        scratch.mkdir(parents=True, exist_ok=True)
        self.temporary = tempfile.TemporaryDirectory(prefix="synthetic-tests-", dir=scratch)
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.source = self.root / "synthetic.rs"
        self.source.write_text("// synthetic placeholder, not Rust acceptance\n")
        self.output = self.root / "result.json"
        self.scratch = self.root / "scratch"
        oracle = bench.mandelbrot_oracle()
        compiler = f'''#!{sys.executable}
import os
from pathlib import Path
import sys
import time
if "--version" in sys.argv:
    print("synthetic rustc version; not real compiler")
    raise SystemExit(0)
out = Path(sys.argv[sys.argv.index("-o") + 1])
mode = os.environ.get("SYNTHETIC_BENCH_MODE", "ok")
native = out.name.startswith("native")
if native and mode == "warmup-fail":
    print("full synthetic failure stdout")
    print("full synthetic failure stderr", file=sys.stderr)
    raise SystemExit(9)
if native and mode == "timeout":
    print("partial timeout stdout", flush=True)
    print("partial timeout stderr", file=sys.stderr, flush=True)
    time.sleep(10)
if native and mode == "missing":
    raise SystemExit(0)
payload = {oracle!r}
if mode == "adler":
    payload = {bench.expected_stdout('adler2-ff5553')!r}
if native and mode == "wrong":
    payload += b"extra output"
if native and mode == "measured-fail" and "measurement-0" in out.name:
    raise SystemExit(8)
if native and mode == "measured-wrong" and "measurement-0" in out.name:
    payload = b"incorrect"
program = {('#!' + sys.executable + chr(10))!r} + "import sys\\nsys.stdout.buffer.write(" + repr(payload) + ")\\n"
if native and mode == "runtime-fail":
    program = {('#!' + sys.executable + chr(10))!r} + "from pathlib import Path\\nimport sys\\np = Path(__file__).with_suffix('.runs')\\nn = int(p.read_text()) if p.exists() else 0\\np.write_text(str(n + 1))\\nsys.stdout.buffer.write(" + repr(payload) + ")\\nsys.exit(7 if n else 0)\\n"
out.write_text(program)
out.chmod(0o700)
'''
        self.compiler = self.root / "synthetic-compiler"
        self.compiler.write_text(compiler)
        self.compiler.chmod(0o700)

    def invoke(self, mode="ok", extra=()):
        command = [sys.executable, str(MODULE_PATH), str(self.source),
                   "--binary", str(self.compiler), "--rustc", str(self.compiler),
                   "--output", str(self.output), "--scratch-dir", str(self.scratch),
                   "--repeats", "2", "--runtime-repeats", "2", *extra]
        env = dict(os.environ, SYNTHETIC_BENCH_MODE=mode)
        result = subprocess.run(command, capture_output=True, env=env, timeout=30)
        report = json.loads(self.output.read_text())
        self.assertEqual(list(self.scratch.iterdir()), [])
        return result, report

    def test_synthetic_cli_success_rotation_counts_and_cleanup(self):
        result, report = self.invoke()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(report["ok"])
        self.assertIn("not cold or incremental", report["measurement"])
        self.assertIn("no optimizer comparable", report["comparability"])
        for config in bench.CONFIGS:
            self.assertEqual(report["results"][config]["compile"]["count"], 2)
            self.assertEqual(report["results"][config]["runtime"]["count"], 2)
            self.assertGreater(report["results"][config]["executable_size_bytes"], 0)
        events = report["events"]
        for phase in ("measurement-compile", "runtime"):
            self.assertEqual([event["config"] for event in events if event["phase"] == phase],
                             [*bench.CONFIGS, *bench.rotate(bench.CONFIGS, 1)])
        compiles = [event for event in events if event["phase"].endswith("-compile")]
        self.assertEqual(len({event["command"][-1] for event in compiles}), 9)
        self.assertTrue(all("elapsed_ns" not in event for event in events
                            if event["phase"].startswith("warmup")))
        serialized = json.dumps(report)
        self.assertNotIn(str(Path.home()), serialized)
        self.assertNotIn(str(self.scratch), serialized)
        self.assertIn("<scratch>", serialized)
        self.assertEqual(len(report["metadata"]["source_sha256"]), 64)
        self.assertEqual(len(report["metadata"]["binary_sha256"]), 64)
        self.assertEqual(report["metadata"]["runner_sha256"], bench.sha256(MODULE_PATH))
        recorded_at = datetime.fromisoformat(report["metadata"]["recorded_at"])
        self.assertEqual(recorded_at.utcoffset(), timezone.utc.utcoffset(recorded_at))
        self.assertNotIn("cpu_info", report["metadata"])
        self.assertIn("per-pixel print IO", report["runtime_caveat"])

    def test_synthetic_adler_cli_and_compiler_revision(self):
        result, report = self.invoke("adler", ("--oracle", "adler2-ff5553",
                                               "--compiler-revision", "af3eb295f"))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(report["metadata"]["compiler_revision"], "af3eb295f")
        self.assertEqual(report["metadata"]["oracle"], "adler2-ff5553")
        for config in bench.CONFIGS:
            self.assertEqual(report["results"][config]["runtime"]["count"], 2)

    def test_synthetic_warmup_failures_preserve_json(self):
        for mode in ("warmup-fail", "wrong", "missing", "timeout"):
            with self.subTest(mode=mode):
                if self.output.exists():
                    self.output.unlink()
                result, report = self.invoke(mode, ("--timeout", "0.2") if mode == "timeout" else ())
                self.assertEqual(result.returncode, 1)
                self.assertFalse(report["ok"])
                self.assertIn("warmup failed", report["error"])
                self.assertTrue(all(value["compile"]["count"] == 0
                                    for value in report["results"].values()))
                self.assertFalse(any(event["phase"] == "runtime" for event in report["events"]))
                if mode == "warmup-fail":
                    event = next(e for e in report["events"] if e["config"] == "native")
                    self.assertEqual(event["returncode"], 9)
                    self.assertEqual(event["stdout"], "full synthetic failure stdout\n")
                    self.assertEqual(event["stderr"], "full synthetic failure stderr\n")
                if mode == "timeout":
                    event = next(e for e in report["events"] if e["config"] == "native")
                    self.assertEqual(event["error"], "timeout")
                    self.assertEqual(event["stdout"], "partial timeout stdout\n")
                    self.assertEqual(event["stderr"], "partial timeout stderr\n")

    def test_synthetic_measured_failure_excluded_from_samples(self):
        for mode in ("measured-fail", "measured-wrong"):
            with self.subTest(mode=mode):
                if self.output.exists():
                    self.output.unlink()
                result, report = self.invoke(mode)
                self.assertEqual(result.returncode, 1)
                self.assertEqual(report["results"]["native"]["compile"]["count"], 1)
                self.assertEqual(report["results"]["native"]["runtime"]["count"], 2)
                self.assertEqual(report["results"]["stock-debug"]["compile"]["count"], 2)

    def test_synthetic_runtime_failures_are_not_successful_samples(self):
        result, report = self.invoke("runtime-fail")
        self.assertEqual(result.returncode, 1)
        self.assertEqual(report["results"]["native"]["compile"]["count"], 2)
        self.assertEqual(report["results"]["native"]["runtime"], bench.summary([]))
        events = [event for event in report["events"]
                  if event["phase"] == "runtime" and event["config"] == "native"]
        self.assertEqual([event["returncode"] for event in events], [7, 7])
        self.assertTrue(all(event["stdout"] == bench.mandelbrot_oracle().decode()
                            for event in events))

    def test_synthetic_output_is_exclusive(self):
        self.output.write_text("do not replace")
        result = subprocess.run([sys.executable, str(MODULE_PATH), "--binary", str(self.compiler),
                                 "--output", str(self.output)], capture_output=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.output.read_text(), "do not replace")

    def test_missing_source_preserves_json(self):
        self.source.unlink()
        result, report = self.invoke()
        self.assertEqual(result.returncode, 1)
        self.assertFalse(report["ok"])
        self.assertIn("error", report)


if __name__ == "__main__":
    unittest.main()
