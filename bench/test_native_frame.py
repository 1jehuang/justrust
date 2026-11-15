"""Offline tests only: no Rust jobs or real benchmark measurements."""
import contextlib
import importlib.util
import io
import json
from pathlib import Path
import sys
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location("native_frame", Path(__file__).with_name("native-frame.py"))
runner = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(runner)


def records():
    return [{"round": r, "size": s, "frames": runner.SIZES[s], "coalesced": c,
             "wall": 0.125 + i / 1000}
            for i, (r, s, c) in enumerate(runner.expected_order())]


def lines(samples):
    return [runner.PREFIX + json.dumps(sample) + "\n" for sample in samples]


class ParserTests(unittest.TestCase):
    def test_independent_rust_order_and_release_command(self):
        expected = [
            (0, 64, False), (0, 64, True), (0, 64, True), (0, 64, False),
            (1, 64, False), (1, 64, True), (1, 64, True), (1, 64, False),
            (2, 64, False), (2, 64, True), (2, 64, True), (2, 64, False),
            (0, 32768, False), (0, 32768, True), (0, 32768, True), (0, 32768, False),
            (1, 32768, False), (1, 32768, True), (1, 32768, True), (1, 32768, False),
            (2, 32768, False), (2, 32768, True), (2, 32768, True), (2, 32768, False),
        ]
        self.assertEqual(runner.expected_order(), expected)
        samples = [{"round": r, "size": s, "frames": 32768 if s == 64 else 4096,
                    "coalesced": c, "wall": 0.25} for r, s, c in expected]
        self.assertEqual(runner.parse_samples(lines(samples)), samples)
        self.assertEqual(runner.COMMAND, ["justrust", "test", "--lib", "--locked", "--release",
                                         "native_frame_tls_benchmark", "--", "--ignored", "--nocapture"])

    def test_stream_and_watchdog_without_rust(self):
        with contextlib.redirect_stdout(io.StringIO()) as output:
            code, captured = runner.stream_command(
                [sys.executable, "-c", "print('full child output')"], timeout=5)
        self.assertEqual(code, 0)
        self.assertEqual(captured, ["full child output\n"])
        self.assertEqual(output.getvalue(), "full child output\n")
        with self.assertRaisesRegex(ValueError, "exceeded"):
            runner.stream_command([sys.executable, "-c", "import time; time.sleep(30)"], timeout=0.1)

    def test_complete(self):
        samples = records()
        self.assertEqual(runner.parse_samples(["ordinary output\n", *lines(samples)]), samples)

    def test_incomplete(self):
        for count in (0, 1, 23):
            with self.subTest(count=count), self.assertRaises(ValueError):
                runner.parse_samples(lines(records()[:count]))

    def test_duplicate_extra_and_order(self):
        samples = records()
        for invalid in ([samples[0], *samples], [*samples, samples[-1]],
                        [samples[1], samples[0], *samples[2:]], samples[4:] + samples[:4]):
            with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                runner.parse_samples(lines(invalid))

    def test_invalid_values(self):
        for key, values in {"wall": [0, -1, float("nan"), float("inf"), -float("inf"), 10**1000, True, "1"],
                            "size": [True, 128, 64.0], "frames": [True, 4096],
                            "round": [True, -1, 3], "coalesced": [0, "false"]}.items():
            for value in values:
                samples = records()
                samples[0][key] = value
                with self.subTest(key=key, value=value), self.assertRaises(ValueError):
                    runner.parse_samples(lines(samples))

    def test_invalid_json_fields(self):
        for line in (runner.PREFIX + "{", runner.PREFIX + "[]", "NATIVE_FRAME_BENCH{}",
                     runner.PREFIX + '{"wall":1,"wall":2}'):
            with self.subTest(line=line), self.assertRaises(ValueError):
                runner.parse_samples([line])
        for mutate in (lambda s: s.pop("wall"), lambda s: s.update(extra=1)):
            samples = records()
            mutate(samples[0])
            with self.assertRaises(ValueError):
                runner.parse_samples(lines(samples))

    def test_pair_is_compare_compatible_and_honestly_uncontrolled(self):
        import argparse
        import suite
        from runtime_context import require_matching_context
        env = {"snapshots": {"focused": "test"}, "snapshot_format": "test",
               "snapshot_fingerprints": {"focused": "hash"}, "runtime_prerequisites": {},
               "justrust_rev": "offline-fixture", "runtime_context": {"policy": "inherited"}}
        before, after = runner.results(records(), env, "fixture-hash", "fixture-time")
        require_matching_context(before, after)
        self.assertEqual(before["env"], after["env"])
        for result in (before, after):
            self.assertFalse(result["speed_claim_eligible"])
            self.assertFalse(result["default_promotion"])
            for scenario in result["scenarios"].values():
                self.assertEqual(len(scenario["samples"]), 6)
                self.assertTrue(all(s["noisy"] is None for s in scenario["samples"]))
        with tempfile.TemporaryDirectory() as temporary:
            paths = [Path(temporary) / name for name in ("before.json", "after.json")]
            for path, result in zip(paths, (before, after)):
                path.write_text(json.dumps(result))
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                suite.cmd_compare(argparse.Namespace(a=str(paths[0]), b=str(paths[1]), floor=10.0))
            self.assertEqual(output.getvalue().count("uncontrolled (noisy or unknown samples)"), 2)


if __name__ == "__main__":
    unittest.main()
