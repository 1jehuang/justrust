"""Fail-closed CLI eligibility without changing human-report compatibility."""
import copy
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

import suite
from test_synth_reporting import fixture


class StrictComparisonTests(unittest.TestCase):
    def result(self, names=("old",)):
        return {"env": {"justrust_rev": "fixture", "snapshots": {"fixture": "pinned"},
                        "host": "fixture", "cpu": "fixture", "ncpu": 4},
                "synth": fixture(names)}

    def compare(self, a, b, strict=True):
        with tempfile.TemporaryDirectory() as td:
            paths = [Path(td) / name for name in ("a.json", "b.json")]
            for path, data in zip(paths, (a, b)):
                path.write_text(json.dumps(data))
            return subprocess.run([sys.executable, suite.__file__, "compare",
                                   *map(str, paths), *(["--require-valid"] if strict else [])],
                                  capture_output=True, text=True)

    def test_valid_and_positional_stack_names(self):
        a, b = self.result(), self.result(("candidate",))
        result = self.compare(a, b)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertNotIn("blocked", result.stdout.lower())

    def test_invalid_samples_and_identity_fail_closed(self):
        mutations = [
            lambda d: d["synth"].update(partial=True),
            lambda d: d["synth"]["results"].pop(),
            lambda d: d["synth"]["results"][0]["samples"]["old"][0].update(noisy=True),
            lambda d: d["synth"]["results"][0]["samples"]["old"][0].update(exit=101, compile_failed=True),
            lambda d: d["synth"].update(seed=9),
            lambda d: d["synth"]["results"][0].update(cmd=["test"]),
            lambda d: d.pop("env"),
            lambda d: d["env"].update(snapshots={"fixture": "other"}),
            lambda d: d["env"].update(runtime_prerequisites={"policy": "other"}),
        ]
        for mutate in mutations:
            with self.subTest(mutate=mutate):
                a, b = self.result(), self.result()
                mutate(b)
                strict = self.compare(a, b)
                self.assertEqual(strict.returncode, 2, strict.stdout + strict.stderr)
                self.assertIn("blocked", strict.stdout.lower())
                self.assertEqual(self.compare(a, b, strict=False).returncode, 0)

    def test_intended_compile_errors_remain_valid(self):
        a = self.result()
        for point in a["synth"]["results"]:
            point["expect_fail"] = True
            point["samples"]["old"][0].update(compile_failed=True, exit=101)
        result = self.compare(a, copy.deepcopy(a))
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_unsupported_schemas_fail_closed(self):
        for result in ({"env": {}}, {"schema": "native_resident_v1"}):
            outcome = self.compare(result, result)
            self.assertNotEqual(outcome.returncode, 0)
            self.assertIn("supports only synthetic", outcome.stderr)


if __name__ == "__main__":
    unittest.main()
