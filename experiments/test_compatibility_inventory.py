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

    def make_drop_v2(self):
        prefix = "tests/fixtures/drop_normal/"
        source = self.put(prefix + "neutral.rs", "fn main(){}")
        entry = {"path": "neutral.rs", "classification": "unsupported",
                 "sha256": inv.digest(source.read_bytes())}
        manifest = self.put(prefix + "receipts-v2/manifest.json",
                            json.dumps({"schema": 1, "sources": [entry]}))
        stock = self.put(prefix + "receipts-v2/stock.json", json.dumps({
            "schema": 1, "manifest_sha256": inv.digest(manifest.read_bytes()),
            "observations": [{**entry, "compile": {"status": 0}, "run": {"status": 0}}]}))
        return source, manifest, stock

    def test_drop_v2_explicit_label_hashes_and_no_v1_duplicates(self):
        self.make_drop_v2()
        self.put("tests/fixtures/drop_normal/receipts-v1/manifest.json", '{"sources": []}')
        records = inv.inventory(self.root)["records"]
        self.assertEqual(len(records), 1)
        record = records[0]
        self.assertEqual(record["classification"], "legal-unsupported")
        self.assertTrue(record["source_hash_match"])
        self.assertTrue(record["manifest_hash_match"])
        self.assertTrue(record["receipt_record_match"])
        self.assertEqual(record["evidence"], "receipt-record-agrees")
        self.assertEqual(record["native_support"], "unknown; not evaluated")

    def test_drop_v2_source_and_manifest_drift(self):
        source, manifest, stock = self.make_drop_v2()
        source.write_text("changed source")
        self.assertFalse(inv.drop_receipt_records(self.root)[0]["source_hash_match"])
        manifest.write_text(manifest.read_text() + "\n")
        record = inv.drop_receipt_records(self.root)[0]
        self.assertFalse(record["manifest_hash_match"])
        self.assertEqual(record["evidence"], "receipt-record-conflicts")
        data = json.loads(stock.read_text())
        data["observations"][0]["classification"] = "negative"
        stock.write_text(json.dumps(data))
        self.assertFalse(inv.drop_receipt_records(self.root)[0]["receipt_record_match"])

    def test_drop_v2_missing_and_malformed_receipt(self):
        source, manifest, stock = self.make_drop_v2()
        stock.unlink()
        self.assertEqual(inv.drop_receipt_records(self.root)[0]["evidence"], "unavailable")
        source.unlink()
        self.assertFalse(inv.drop_receipt_records(self.root)[0]["source_exists"])
        stock.write_text('{"schema": 1, "observations": null}')
        self.assertEqual(inv.drop_receipt_records(self.root)[0]["evidence"], "receipt-unreadable")
        manifest.write_text('{"schema": 2, "sources": []}')
        self.assertEqual(inv.drop_receipt_records(self.root)[0]["classification"], "unknown")

    def test_drop_v2_unknown_label_duplicate_and_traversal(self):
        _, manifest, _ = self.make_drop_v2()
        data = json.loads(manifest.read_text())
        data["sources"][0]["classification"] = "boundary"
        manifest.write_text(json.dumps(data))
        self.assertEqual(inv.drop_receipt_records(self.root)[0]["classification"], "unknown")
        data["sources"] *= 2
        data["sources"].append({"path": "../outside.rs", "classification": "unsupported"})
        manifest.write_text(json.dumps(data))
        records = inv.drop_receipt_records(self.root)
        self.assertEqual(len(records), 2)
        self.assertEqual(records[0]["classification"], "unknown")
        self.assertIn("duplicate", records[0]["note"])
        self.assertIn("invalid source path", records[1]["note"])

    def test_drop_v2_duplicate_observations_are_not_agreement(self):
        _, _, stock = self.make_drop_v2()
        data = json.loads(stock.read_text())
        data["observations"] *= 2
        stock.write_text(json.dumps(data))
        self.assertEqual(inv.drop_receipt_records(self.root)[0]["evidence"], "receipt-record-duplicate")

    def test_drop_v1_only_has_no_classification_fallback(self):
        self.put("tests/fixtures/drop_normal/unsupported/mystery.rs", "fn main(){}")
        self.put("tests/fixtures/drop_normal/receipts-v1/manifest.json", '{"schema": 1, "sources": []}')
        records = inv.inventory(self.root)["records"]
        self.assertEqual(len(records), 1)
        self.assertEqual(records[0]["classification"], "unknown")
        self.assertEqual(records[0]["locator"], "file")

    def test_drop_selects_numeric_latest_and_missing_receipt(self):
        _, manifest, _ = self.make_drop_v2()
        self.put("tests/fixtures/drop_normal/receipts-v9/manifest.json", "not selected")
        self.put("tests/fixtures/drop_normal/receipts-v10/manifest.json", manifest.read_text())
        self.put("tests/fixtures/drop_normal/receipts-v999oops/manifest.json", "ignore")
        self.put("tests/fixtures/drop_normal/receipts-v0100/manifest.json", "ignore")
        records = inv.drop_receipt_records(self.root)
        self.assertEqual(len(records), 1)
        self.assertIn("receipts-v10/", records[0]["contract"])
        self.assertEqual(records[0]["evidence"], "unavailable")
        self.assertEqual(records[0]["native_support"], "unknown; not evaluated")

    def test_drop_incomplete_latest_never_uses_older(self):
        self.make_drop_v2()
        directory = self.root / "tests/fixtures/drop_normal/receipts-v11"
        directory.mkdir()
        records = inv.drop_receipt_records(self.root)
        self.assertEqual(records[0]["classification"], "unknown")
        self.assertIn("receipts-v11/", records[0]["source_path"])
        self.assertIn("no fallback", records[0]["note"])
        (directory / "manifest.json").write_text('{"schema": 999, "sources": []}')
        self.assertIn("unknown latest", inv.drop_receipt_records(self.root)[0]["note"])

    def test_drop_ignores_symlinked_version_and_rejects_symlinked_receipt(self):
        _, manifest, stock = self.make_drop_v2()
        base = manifest.parent.parent
        (base / "receipts-v99").symlink_to(manifest.parent, target_is_directory=True)
        self.assertIn("receipts-v2/", inv.drop_receipt_records(self.root)[0]["contract"])
        stock.rename(stock.with_name("saved-stock.json"))
        stock.symlink_to(stock.with_name("saved-stock.json"))
        self.assertEqual(inv.drop_receipt_records(self.root)[0]["evidence"], "receipt-symlink-rejected")

    def test_actual_latest_drop_has_one_record_per_source(self):
        root = Path(__file__).resolve().parents[1]
        records = inv.drop_receipt_records(root)
        self.assertTrue(records)
        contract = records[0]["contract"]
        manifest = json.loads((root / contract).read_text())
        self.assertEqual(len(records), len(manifest["sources"]))
        inventory_rows = inv.inventory(root)["records"]
        declared_paths = {r["source_path"] for r in records}
        self.assertTrue(all(sum(row["source_path"] == path for row in inventory_rows) == 1
                            for path in declared_paths))
        self.assertTrue(all(r["source_hash_match"] for r in records))
        self.assertTrue(all(r["native_support"] == "unknown; not evaluated" for r in records))
        self.assertTrue(all(r["evidence"] == "unavailable" or
                            (r["manifest_hash_match"] and r["receipt_record_match"])
                            for r in records))

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
