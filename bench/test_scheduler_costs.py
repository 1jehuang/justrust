"""Offline scheduler costs tests. Run: python3 -m unittest discover -s bench -p test_scheduler_costs.py"""
import hashlib
import json
from pathlib import Path
import tempfile
import unittest

from unittest import mock

import scheduler_costs as costs


def unit(package="exact-name", version="1", wall=1, **extra):
    return dict(package=package, version=version, wall=wall, exit=0, **extra)


class ModelTests(unittest.TestCase):
    def test_max_then_median_exact_names_sorting(self):
        raw, info = costs.aggregate([[unit(wall=1), unit(wall=3), unit("a_b", wall=.001)],
                                     [unit(wall=5), unit("a_b", wall=.002)]])
        self.assertEqual(json.loads(raw), {"a_b": 2, "exact-name": 4000})
        self.assertLess(raw.index(b'a_b'), raw.index(b'exact-name'))
        self.assertEqual(info["eligible_units"], 5)

    def test_ambiguity_even_in_excluded_unit(self):
        bad = unit(version="2")
        bad["exit"] = 1
        raw, info = costs.aggregate([[unit(), bad]])
        self.assertEqual(json.loads(raw), {})
        self.assertEqual(info["ambiguous_packages"], 1)

    def test_exclusions(self):
        rows = [unit(wall=x) for x in [0, -1, float("inf"), float("nan")]]
        rows += [unit(package=""), unit(version=""), unit(package=None),
                 unit(depcache="local"), unit(depcache="hit"), unit(cache_hit=True)]
        failed = unit()
        failed["exit"] = 1
        rows.append(failed)
        raw, info = costs.aggregate([rows + [unit("ok", wall=.00001, depcache="miss")]])
        self.assertEqual(json.loads(raw), {"ok": 1})
        self.assertEqual(info["excluded_units"], len(rows))

    def test_clamp(self):
        raw, info = costs.aggregate([[unit(wall=200)]])
        self.assertEqual(json.loads(raw), {"exact-name": 120000})
        self.assertEqual(info["clamped_packages"], ["exact-name"])

    def test_no_inference_and_distinct_names(self):
        raw, _ = costs.aggregate([[unit("a-b"), unit("a_b", wall=2),
                                  {"crate_name": "inferred", "wall": 3, "exit": 0},
                                  unit("/private/path")]])
        self.assertEqual(json.loads(raw), {"a-b": 1000, "a_b": 2000})

    def test_deterministic_order_and_single_sample(self):
        rows = [unit("z", wall=3), unit("a", wall=.0001)]
        first, _ = costs.aggregate([rows, [unit("z", wall=1)]])
        second, _ = costs.aggregate([[unit("z", wall=1)], rows[::-1]])
        self.assertEqual(first, second)
        self.assertEqual(json.loads(first), {"a": 1, "z": 2000})

    def test_package_limit(self):
        with self.assertRaises(ValueError):
            costs.aggregate([[unit(str(i)) for i in range(1025)]])

    def test_byte_limit(self):
        with self.assertRaises(ValueError):
            costs.aggregate([[unit("x" * 65536)]])


class InputTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.run = "20261014-034928222-288986662"
        self.record = dict(id=self.run, candidate_cargo_selected=True,
                           pinned_compiler_selected=True, toolchain="nightly-test",
                           known_confounds=["uncontrolled concurrent release"], exit=0)
        for suffix, value in [("meta.json", {"id": self.run, "cwd": "/private/path"}),
                              ("summary.json", {"manifest_dir": "/secret/manifest"})]:
            raw = json.dumps(value).encode()
            (self.root / f"{self.run}-{suffix}").write_bytes(raw)
            self.record[suffix.split('.')[0] + "_sha256"] = hashlib.sha256(raw).hexdigest()
        (self.root / f"{self.run}-units.jsonl").write_text(json.dumps(unit(manifest_dir="/private/path")) + "\n")
        self.audit = self.root / "audit.json"
        self.audit.write_text(json.dumps(dict(identities={"cargo": "a" * 64, "rustc": "b" * 64}, records=[self.record])))

    def test_receipt_confounds_hashes_no_paths(self):
        raw, receipt = costs.generate(self.root, self.audit, [self.run])
        text = json.dumps(receipt)
        self.assertNotIn("/private", text)
        self.assertNotIn("/secret", text)
        self.assertNotIn("manifest", text)
        self.assertIn(self.record["known_confounds"][0], text)
        self.assertEqual(receipt["model_sha256"], hashlib.sha256(raw).hexdigest())
        self.assertEqual(receipt["captures"][0]["meta_sha256"], self.record["meta_sha256"])
        self.assertIn("units_sha256", receipt["captures"][0])

    def test_both_hash_mismatches(self):
        for suffix in ["meta.json", "summary.json"]:
            path = self.root / f"{self.run}-{suffix}"
            old = path.read_bytes()
            path.write_bytes(b"{}")
            with self.assertRaisesRegex(ValueError, "hash mismatch"):
                costs.generate(self.root, self.audit, [self.run])
            path.write_bytes(old)

    def test_oversized_input_stat_and_read(self):
        with mock.patch.object(costs, "MAX_INPUT_BYTES", 4):
            with self.assertRaisesRegex(ValueError, "8MiB"):
                costs.read_regular(self.audit)
            # Simulate growth after fstat to exercise the bounded-read guard.
            with mock.patch.object(costs.os, "fstat", return_value=type("Info", (), {
                    "st_mode": 0o100400, "st_size": 0})()):
                with self.assertRaisesRegex(ValueError, "8MiB"):
                    costs.read_regular(self.audit)

    def test_nonobject_audit(self):
        self.audit.write_text("[]")
        with self.assertRaisesRegex(ValueError, "audit must be an object"):
            costs.generate(self.root, self.audit, [self.run])

    def test_symlink_input(self):
        link = self.root / "link"
        link.symlink_to(self.audit)
        with self.assertRaises(ValueError):
            costs.read_regular(link)

    def test_no_overwrite(self):
        out = self.root / "out"
        costs.write_new(out, b"first")
        with self.assertRaises(FileExistsError):
            costs.write_new(out, b"second")
        self.assertEqual(out.read_bytes(), b"first")

    def test_hashes_checked_before_units(self):
        (self.root / f"{self.run}-summary.json").write_bytes(b"changed")
        (self.root / f"{self.run}-units.jsonl").unlink()
        with self.assertRaisesRegex(ValueError, "summary hash mismatch"):
            costs.generate(self.root, self.audit, [self.run])

    def test_ancestor_symlink_and_fifo(self):
        link = self.root / "dirlink"
        link.symlink_to(self.root, target_is_directory=True)
        with self.assertRaises(ValueError):
            costs.read_regular(link / "audit.json")
        fifo = self.root / "fifo"
        import os
        os.mkfifo(fifo)
        with self.assertRaises(ValueError):
            costs.read_regular(fifo)

    def test_duplicate_or_unsafe_run(self):
        for runs in [[self.run, self.run], ["../private"]]:
            with self.assertRaises(ValueError):
                costs.generate(self.root, self.audit, runs)


if __name__ == "__main__":
    unittest.main()
