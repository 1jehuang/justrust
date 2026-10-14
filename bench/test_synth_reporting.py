"""Eligibility of comprehensive synthetic reports, including public wrappers."""
import argparse
import contextlib
import copy
import io
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

import suite

run, _ = suite._synth()


def fixture(names=("old",), points=2, reps=1):
    return {"seed": 1, "points": points, "reps": reps, "generator": "fixture",
            "sample_info": {"calls": 100, "coverage": 1}, "stack_names": list(names),
            "results": [{"i": i, "shape": {"command": "check", "change": "body"},
                         "cmd": ["check"], "pre": ["check"], "prev": None,
                         "snap": "fixture", "cwd": ".", "edits": [], "expect_fail": False, "errors": {},
                         "samples": {s: [{"wall": 2.0, "noisy": False, "compile_failed": False,
                                          "exit": 0, "tests_failed": 0} for _ in range(reps)] for s in names},
                         "stats": {s: {"n": reps, "median": 2.0, "instructions": None} for s in names}}
                        for i in range(points)]}


class ReportingTests(unittest.TestCase):
    def blocked(self, data):
        self.assertEqual(run._complete(data), [])
        self.assertIsNone(run.summarize(data)["overall"][data["stack_names"][0]]["mean_wall"])
        text = run.table(data)
        self.assertIn("blocked", text.lower())
        self.assertNotIn("**expected wait per call**", text)
        self.assertIn("blocked", run.compare(data, data).lower())
        self.assertIn("2.00s", run.detail(data))

    def test_invalid_coverage_and_noise(self):
        for change in (lambda d: d["results"].pop(),
                       lambda d: d.update(partial=True),
                       lambda d: d["results"][0]["samples"]["old"][0].update(noisy=True),
                       lambda d: d["results"][0]["samples"]["old"][0].pop("noisy"),
                       lambda d: d["results"][0]["samples"].pop("old"),
                       lambda d: d.update(reps=2),
                       lambda d: d["results"][1].update(i=0),
                       lambda d: d["results"][0].update(i=-1),
                       lambda d: d["results"][0].update(i="0"),
                       lambda d: d["results"][0].pop("i"),
                       lambda d: d["results"][0].update(i=2),
                       lambda d: d["results"][0]["errors"].update(old="failed")):
            with self.subTest(change=change):
                d = fixture()
                change(d)
                self.blocked(d)

    def test_incompatible_identity(self):
        for key, value in (("seed", 2), ("generator", "other"), ("sample_info", {}),
                           ("stack_names", ["other"]), ("reps", 2)):
            a, b = fixture(), fixture()
            b[key] = value
            self.assertIn("blocked", run.compare(a, b).lower())
        for key, value in (("cmd", ["test"]), ("pre", ["build"]), ("prev", ["clippy"]),
                           ("edits", ["different"]), ("expect_fail", True), ("snap", "other"),
                           ("cwd", "other"), ("shape", {"command": "test", "change": "body"})):
            with self.subTest(key=key):
                b = fixture()
                b["results"][0][key] = value
                self.assertIn("blocked", run.compare(fixture(), b).lower())

    def test_clean_and_expected_compile_error(self):
        d = fixture()
        d["results"][0]["expect_fail"] = True
        d["results"][0]["samples"]["old"][0].update(compile_failed=True, exit=101)
        self.assertEqual(run.summarize(d)["overall"]["old"]["mean_wall"], 2)
        self.assertIn("**expected wait per call**", run.table(d))
        self.assertIn("+0%", run.compare(d, d))
        d["results"][0]["expect_fail"] = False
        self.blocked(d)

    def cli(self, command, *data):
        with tempfile.TemporaryDirectory() as td:
            paths = []
            for i, d in enumerate(data):
                path = Path(td) / f"{i}.json"
                path.write_text(json.dumps({"env": {"justrust_rev": "fixture", "snapshots": {"fixture": "pinned"},
                                                     "host": "fixture", "cpu": "fixture", "ncpu": 4}, "synth": d}))
                paths.append(str(path))
            result = subprocess.run([sys.executable, str(Path(suite.__file__)), command, *paths],
                                    text=True, capture_output=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            return result.stdout

    def test_published_partial_public_show_compare(self):
        d = json.loads((Path(suite.__file__).parent / "results/scheduler-comprehensive-2172-20261014/partial-off.json").read_text())
        text = self.cli("show", d)
        self.assertIn("blocked", text.lower())
        self.assertIn("20.53s", text)
        self.assertNotIn("**expected wait per call**", text)
        self.assertIn("blocked", self.cli("compare", d, d).lower())

    def test_public_positional_remap_noise_and_clean(self):
        a, b = fixture(), fixture(("new",))
        self.assertIn("+0%", self.cli("compare", a, b))
        b["results"][0]["samples"]["new"][0]["noisy"] = True
        self.assertIn("blocked", self.cli("compare", a, b).lower())
        original = copy.deepcopy(b)
        env = {"snapshots": {"fixture": "pinned"}, "host": "fixture", "cpu": "fixture", "ncpu": 4}
        with mock.patch.object(suite.json, "loads", side_effect=[{"env": {**env, "justrust_rev": "a"}, "synth": a},
                                                               {"env": {**env, "justrust_rev": "b"}, "synth": b}]), \
                mock.patch.object(Path, "read_text", return_value="{}"), \
                contextlib.redirect_stdout(io.StringIO()):
            suite.cmd_compare(argparse.Namespace(a="a", b="b"))
        self.assertEqual(b, original)

    def test_empty_results_blocked(self):
        d = fixture()
        d["results"] = []
        self.assertIn("blocked", run.compare(d, d).lower())

    def test_invalid_timing_and_stale_summary(self):
        for value in (0, -1, float("nan"), float("inf"), "2"):
            with self.subTest(value=value):
                d = fixture()
                d["results"][0]["samples"]["old"][0]["wall"] = value
                self.blocked(d)
        d = fixture()
        d["results"][0]["stats"]["old"]["median"] = 3
        self.blocked(d)
        d = fixture()
        d["results"][0]["stats"]["old"]["instructions"] = 100
        self.blocked(d)

    def test_clean_replicates_and_stats_key_order(self):
        a, b = fixture(("one", "two"), reps=2), fixture(("new-one", "new-two"), reps=2)
        for d, name in ((a, "two"), (b, "new-two")):
            for p in d["results"]:
                p["stats"][name]["median"] = 4
                for sample in p["samples"][name]:
                    sample["wall"] = 4
        for p in b["results"]:
            p["stats"] = dict(reversed(list(p["stats"].items())))
        text = self.cli("compare", a, b)
        self.assertEqual(text.count("+0%"), 2)
        self.assertIn("4.00", text)

    def test_exit_outcome_provenance(self):
        for key in ("exit", "compile_failed", "tests_failed"):
            d = fixture()
            d["results"][0]["samples"]["old"][0].pop(key)
            self.blocked(d)
        for changes in ({"exit": 101}, {"exit": None}, {"compile_failed": None},
                        {"tests_failed": 1}, {"tests_failed": None}):
            with self.subTest(changes=changes):
                d = fixture()
                d["results"][0]["samples"]["old"][0].update(changes)
                self.blocked(d)
        d = fixture()
        d["results"][0]["expect_fail"] = True
        d["results"][0]["samples"]["old"][0].update(compile_failed=True, exit=0)
        self.blocked(d)

    def test_malformed_public_payloads(self):
        self.assertIn("blocked", self.cli("show", None).lower())
        self.assertIn("blocked", self.cli("compare", None, None).lower())
        for key, value in (("stack_names", None), ("stack_names", "old"),
                           ("results", None), ("results", {}), ("results", [None])):
            with self.subTest(key=key, value=value):
                d = fixture()
                d[key] = value
                self.assertIn("blocked", self.cli("show", d).lower())
                self.assertIn("blocked", self.cli("compare", d, d).lower())
        for key, value in (("samples", None), ("stats", None), ("samples", {"old": [None]}),
                           ("stats", {"old": None})):
            d = fixture()
            d["results"][0][key] = value
            self.assertIn("blocked", self.cli("show", d).lower())
            self.assertIn("blocked", self.cli("compare", d, d).lower())
        for key, value in (("median", True), ("n", True)):
            d = fixture(points=1)
            d["results"][0]["stats"]["old"][key] = value
            self.assertIn("blocked", self.cli("show", d).lower())

    def test_remaining_schema(self):
        mutations = [lambda d: d["results"][0]["stats"]["old"].pop("instructions"),
                     lambda d: d.update(sample_info=True), lambda d: d.update(sample_info={"calls": 1}),
                     lambda d: d.update(generator=[])]
        for key, value in (("pre", None), ("prev", 3), ("snap", []), ("cwd", None),
                           ("edits", [None]), ("cmd", [])):
            mutations.append(lambda d, k=key, v=value: d["results"][0].update({k: v}))
        for mutate in mutations:
            d = fixture()
            mutate(d)
            self.assertIn("blocked", self.cli("show", d).lower())
            self.assertIn("blocked", self.cli("compare", d, d).lower())
        d = fixture()
        d["results"][0]["expect_fail"] = True
        d["results"][0]["samples"]["old"][0].update(compile_failed=True, exit=-9)
        self.blocked(d)

    def test_public_environment_identity(self):
        env = {"justrust_rev": "fixture", "snapshots": {"fixture": "pinned"},
               "host": "fixture", "cpu": "fixture", "ncpu": 4}
        for key, value in (("snapshots", {}), ("host", "other"), ("cpu", "other"),
                           ("ncpu", 8), ("ncpu", True), ("execution", "remote"), ("justrust_rev", None)):
            with self.subTest(key=key), tempfile.TemporaryDirectory() as td:
                a, b = Path(td) / "a.json", Path(td) / "b.json"
                a.write_text(json.dumps({"env": env, "synth": fixture()}))
                b.write_text(json.dumps({"env": {**env, key: value}, "synth": fixture()}))
                r = subprocess.run([sys.executable, str(Path(suite.__file__)), "compare", str(a), str(b)],
                                   capture_output=True, text=True)
                self.assertEqual(r.returncode, 0, r.stderr)
                self.assertIn("blocked", r.stdout.lower())

    def test_instruction_counter_coverage(self):
        for missing in (None, "absent"):
            d = fixture(("old", "other"), reps=2)
            for p in d["results"]:
                for name in d["stack_names"]:
                    p["stats"][name]["instructions"] = 100
                    for sample in p["samples"][name]:
                        sample["instructions"] = 100
            if missing is None:
                d["results"][0]["samples"]["other"][1]["instructions"] = None
            else:
                d["results"][0]["samples"]["other"][1].pop("instructions")
            self.assertEqual(run._blocked(d), [])
            self.assertEqual(run.summarize(d)["overall"]["other"]["mean_wall"], 2)
            self.assertIsNone(run.summarize(d)["overall"]["other"]["instructions"])
            text = run.table(d)
            self.assertIn("**expected wait per call**", text)
            self.assertIn("instruction", text.lower())
            self.assertIn("unavailable", text.lower())
            text = run.compare(d, d)
            self.assertNotIn("+0.0%", next(line for line in text.splitlines() if line.startswith("other ")))
            self.assertIn("unavailable", text.lower())
        d = fixture(("old", "other"), reps=2)
        for p in d["results"]:
            for name in d["stack_names"]:
                p["stats"][name]["instructions"] = 100
                for sample in p["samples"][name]:
                    sample["instructions"] = 100
        self.assertIn("1.00", run.table(d))
        self.assertIn("+0.0%", run.compare(d, d))
        env = {"justrust_rev": "fixture", "snapshots": {"fixture": "pinned"},
               "host": "fixture", "cpu": "fixture", "ncpu": 4}
        for other in (None, {}, {k: v for k, v in env.items() if k != "justrust_rev"}):
            with tempfile.TemporaryDirectory() as td:
                path = Path(td) / "missing-env.json"
                path.write_text(json.dumps({"env": other, "synth": fixture()}))
                r = subprocess.run([sys.executable, str(Path(suite.__file__)), "compare", str(path), str(path)],
                                   capture_output=True, text=True)
                self.assertEqual(r.returncode, 0, r.stderr)
                self.assertIn("blocked", r.stdout.lower())


if __name__ == "__main__":
    unittest.main()
