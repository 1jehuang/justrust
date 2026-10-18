"""Standard-library-only tests. No corpus modules or Rust commands are executed."""
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import compatibility_inventory as inv


class InventoryTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(dir=Path(__file__).resolve().parents[1])
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)

    def put(self, name, text):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)
        return path

    def python_rows(self, text):
        path = self.put("experiments/sample_correctness.py", text)
        return inv.python_records(path, "experiments/sample_correctness.py")

    def test_grouped_dictionary_and_fragment(self):
        rows = self.python_rows("FIXTURES = {'legal_unsupported': [{'name': 'alias', 'source': 'type A=i32;'}]}")
        self.assertEqual(rows[0]["classification"], "legal-unsupported")
        self.assertEqual(rows[0]["source_form"], "fragment-or-unverified-source")

    def test_json_loads_literal_and_full_source_hash(self):
        source = "fn main(){}"
        fixture = [{"name": "a", "group": "legalunsupported", "full_source": source,
                    "full_source_sha256": inv.digest(source.encode())}]
        rows = self.python_rows("import json\nFIXTURES = json.loads(" + repr(json.dumps(fixture)) + ")")
        self.assertTrue(rows[0]["declared_hash_match"])
        self.assertEqual(rows[0]["classification"], "legal-unsupported")

    def test_named_json_constant(self):
        text = json.dumps([{"name": "x", "group": "legal-excluded", "full_source": "x"}])
        rows = self.python_rows("import json\nFROZEN_CORPUS_JSON = " + repr(text) + "\nFIXTURES = json.loads(FROZEN_CORPUS_JSON)")
        self.assertEqual(rows[0]["classification"], "legal-unsupported")

    def test_unknown_group_and_hash_mismatch(self):
        rows = self.python_rows("FIXTURES = [{'name': 'x', 'group': 'future', 'full_source': 'x', 'full_source_sha256': 'bad'}]")
        self.assertEqual(rows[0]["classification"], "unknown")
        self.assertFalse(rows[0]["declared_hash_match"])

    def test_generic_boundary_group_is_unknown(self):
        rows = self.python_rows("FIXTURES = [{'name': 'x', 'group': 'boundary', 'full_source': 'fn main(){}'}]")
        self.assertEqual(rows[0]["classification"], "unknown")
        self.assertEqual(rows[0]["declared_group"], "boundary")

    def test_missing_root_directories_are_empty(self):
        report = inv.inventory(self.root / "missing-root")
        self.assertEqual(report["records"], [])
        self.assertEqual(report["counts"], {})
        self.assertFalse(report["g0_complete"])

    def test_no_execution_and_dynamic_unknown(self):
        rows = self.python_rows("raise RuntimeError('must not execute')\ndef corpus():\n    yield Case('x', 'boundary', generated())")
        self.assertEqual(len(rows), 1)
        self.assertEqual(rows[0]["classification"], "unknown")
        self.assertEqual(rows[0]["locator"], "corpus()")

    def test_nonliteral_call_not_evaluated(self):
        rows = self.python_rows("FIXTURES = __import__('os').system('false')")
        self.assertEqual(rows[0]["classification"], "unknown")

    def test_tsv_receipt_and_hash(self):
        source = self.put("tests/fixtures/leading_zeros/ufcs.rs", "fn main(){}")
        sha = inv.digest(source.read_bytes())
        path = self.put("tests/fixtures/leading_zeros/manifest.tsv", f"ufcs\tlegal-unsupported\taccepted\t{sha}\n")
        self.put("tests/fixtures/leading_zeros/stock-receipt.json", json.dumps({"cases": [
            {"name": "ufcs", "category": "legal-unsupported", "stock": "accepted", "source_sha256": sha}]}))
        record = inv.manifest_records(path, path.relative_to(self.root).as_posix())[0]
        self.assertEqual(record["evidence"], "receipt-record-agrees")
        self.assertTrue(record["source_hash_match"])
        source.write_text("changed")
        self.assertFalse(inv.manifest_records(path, "manifest.tsv")[0]["source_hash_match"])

    def test_missing_source_and_malformed_tsv(self):
        path = self.put("manifest.tsv", "bad\nmissing\tfuture\taccepted\tbad\n")
        rows = inv.manifest_records(path, "manifest.tsv")
        self.assertEqual(rows[0]["classification"], "unknown")
        self.assertFalse(rows[1]["source_exists"])
        self.assertEqual(rows[1]["evidence"], "unavailable")

    def test_receipt_conflict_and_invalid_json(self):
        path = self.put("manifest.tsv", "x\tvalid\taccepted\tbad\n")
        receipt = self.put("stock-receipt.json", '{"cases": [{"name": "x"}]}')
        self.assertEqual(inv.manifest_records(path, "manifest.tsv")[0]["evidence"], "receipt-record-conflicts")
        receipt.write_text("not json")
        self.assertEqual(inv.manifest_records(path, "manifest.tsv")[0]["evidence"], "receipt-unreadable")

    def test_headers_invalid_excluded_is_not_legal(self):
        for stock, expected in (("accept", "legal-unsupported"), ("reject", "declared-invalid")):
            path = self.put("x.rs", f"// stock: {stock}\n// category: excluded\nfn main(){{}}")
            self.assertEqual(inv.header_record(path, "x.rs")["classification"], expected)

    def test_duplicate_headers_unknown(self):
        path = self.put("x.rs", "// stock: accept\n// stock: reject\n// category: excluded\n")
        self.assertEqual(inv.header_record(path, "x.rs")["classification"], "unknown")

    def test_adler_receipt_does_not_imply_native_support(self):
        source = self.put("fixtures/adler2_adapted/cases/empty.rs", "fn main(){}")
        self.put("fixtures/adler2_adapted/stock-evidence.json", json.dumps({"cases": [
            {"name": "empty", "source_sha256": inv.digest(source.read_bytes())}]}))
        record = inv.inventory(self.root)["records"][0]
        self.assertEqual(record["classification"], "unknown")
        self.assertEqual(record["evidence"], "historical-receipt-source-match")

    def test_walk_no_filename_inference_deterministic_and_no_subprocess(self):
        self.put("fixtures/new/unsupported.rs", "fn main(){}")
        self.put("experiments/dynamic_correctness.py", "def corpus():\n return make_cases()\n")
        with patch("subprocess.run", side_effect=AssertionError("no execution")):
            first = inv.inventory(self.root)
            self.assertEqual(first, inv.inventory(self.root))
        self.assertFalse(first["g0_complete"])
        self.assertEqual(first["counts"], {"unknown": 2})

    def test_unknown_manifest_is_not_assumed_tsv(self):
        self.put("fixtures/new/manifest.tsv", "x\tlegal-unsupported\taccepted\tbad\n")
        report = inv.inventory(self.root)
        self.assertEqual(report["counts"], {"unknown": 1})
        self.assertIn("unknown manifest schema", report["records"][0]["note"])

    def test_receipt_null_cases_is_unreadable(self):
        path = self.put("manifest.tsv", "x\tvalid\taccepted\tbad\n")
        self.put("stock-receipt.json", '{"cases": null}')
        self.assertEqual(inv.manifest_records(path, "manifest.tsv")[0]["evidence"], "receipt-unreadable")

    def test_actual_checkout_formats(self):
        root = Path(__file__).resolve().parents[1]
        report = inv.inventory(root)
        records = report["records"]
        for name in ("ufcs", "reference"):
            rows = [r for r in records if r["source_path"] == f"tests/fixtures/leading_zeros/{name}.rs"]
            self.assertEqual(len(rows), 1)
            self.assertEqual(rows[0]["classification"], "legal-unsupported")
            self.assertTrue(rows[0]["source_hash_match"])
            self.assertEqual(rows[0]["evidence"], "receipt-record-agrees")
        excluded = [r for r in records if "slice-contract-rejection/unsupported_" in r["source_path"]]
        self.assertEqual(len(excluded), 6)
        self.assertTrue(all(r["classification"] == "legal-unsupported" for r in excluded))
        self.assertGreater(report["counts"]["unknown"], 0)


if __name__ == "__main__":
    unittest.main()
