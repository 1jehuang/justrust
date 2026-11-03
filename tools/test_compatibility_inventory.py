#!/usr/bin/env python3
"""Synthetic inventory validation tests. No pending artifacts or compiler needed."""
import copy
import io
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import tarfile
import unittest

import compatibility_inventory as inventory


def fixture():
    return {"schema_version": 1, "scope": dict(inventory.CONTRACT),
            "review_status": "unreviewed", "limitations": ["Not reviewed or exhaustive."],
            "sources": [{"id": "reference", "url": "https://doc.rust-lang.org/reference/types.html",
                         "version_note": "Synthetic pinned fixture"}],
            "sections": [{"id": "types", "title": "Types", "source_refs": [
                {"source": "reference", "section": "types"}], "obligations": [
                    {"id": "types-one", "title": "Type obligation", "existing_requirements": ["req-one"],
                     "test_needs": ["Differential type check"]}]}]}


def scope_fixture():
    return {"schema_version": 1, "scope": dict(inventory.CONTRACT, semantic_inventory="incomplete"),
            "areas": [{"id": "area", "requirements": [{"id": "req-one"}, {"id": "req-two"}]}]}


def contract_fixture():
    return {"schema_version": 1, "contract_id": "rust-2024-linux-x86-64-stock-1-98-1",
            **inventory.CONTRACT, "oracle": {"release": "1.98.1", "commit": "a" * 40,
                "verification": "tests/support/stock_rustc.rs", "upgrade_policy": "New identity required"},
            "dimensions": [{"id": name, "includes": "Synthetic description"} for name in
                           ("language", "libraries", "projects", "execution", "ecosystem")],
            "evidence_policy": {name: "Synthetic policy" for name in
                ("positive", "negative", "undefined_behavior", "unspecified_behavior", "provenance", "graph", "pending")},
            "exclusions": [{"id": name, "reason": "Synthetic reason"} for name in
                           ("nightly-extensions", "other-targets", "rustc-internals", "performance-parity")],
            "readiness": {"inventory_review": "unreviewed", "full_rust_completion_percent": None, "rule": "No percentage"},
            "source_provenance_policy": "Official pinned sources"}


VERIFIER = '\n'.join(f'const {name}: &str = "{value}";' for name, value in {
    "RELEASE": "release: 1.98.1", "COMMIT": "commit-hash: " + "a" * 40,
    "HOST": "host: x86_64-unknown-linux-gnu", "LLVM": "LLVM version: 22.1.8"}.items())


class InventoryTests(unittest.TestCase):
    def reject(self, value):
        with self.assertRaises(inventory.InvalidInventory):
            inventory.summarize([value], scope_fixture())

    def test_summary(self):
        result = inventory.summarize([fixture()], scope_fixture())
        self.assertEqual(result["counts"], {"inventories": 1, "sources": 1, "sections": 1,
                         "obligations": 1, "test_needs": 1, "existing_requirements": 2,
                         "mapped_existing_requirements": 1, "obligations_lacking_direct_exact_test_links": 1,
                         "obligations_with_executable_evidence": 0})
        self.assertIsNone(result["full_rust_completion_percent"])
        self.assertEqual(result["unmapped_existing_requirements"], ["req-two"])
        self.assertEqual(result["review_status"], "unreviewed")
        self.assertNotIn("percentage", result)

    def test_repeat_files_and_global_obligation_ids(self):
        second = fixture()
        with self.assertRaisesRegex(inventory.InvalidInventory, "duplicate id"):
            inventory.summarize([fixture(), second], scope_fixture())
        second["sections"][0]["obligations"][0]["id"] = "types-two"
        self.assertEqual(inventory.summarize([fixture(), second], scope_fixture())["counts"]["obligations"], 2)

    def test_duplicate_ids_at_each_level(self):
        for container in ("sources", "sections"):
            value = fixture()
            value[container].append(copy.deepcopy(value[container][0]))
            with self.subTest(container=container):
                self.reject(value)
        value = fixture()
        value["sections"][0]["obligations"] *= 2
        self.reject(value)

    def test_canonical_ids(self):
        for ident in ("Upper", "has_under", "has space", "trailing-", "a--b", " leading", "1number"):
            value = fixture()
            value["sections"][0]["obligations"][0]["id"] = ident
            with self.subTest(ident=ident):
                self.reject(value)
        value = fixture()
        second = copy.deepcopy(value["sections"][0])
        second["id"] = "other"
        value["sections"].append(second)
        self.reject(value)

    def test_missing_and_extra_fields_at_every_level(self):
        paths = [(), ("scope",), ("sources", 0), ("sections", 0),
                 ("sections", 0, "source_refs", 0), ("sections", 0, "obligations", 0)]
        for path in paths:
            value = fixture()
            obj = value
            for key in path:
                obj = obj[key]
            for key in list(obj):
                altered = copy.deepcopy(value)
                target = altered
                for part in path:
                    target = target[part]
                del target[key]
                with self.subTest(path=path, missing=key):
                    self.reject(altered)
            for key in ("status", "passing", "support", "execution_status", "unexpected"):
                altered = copy.deepcopy(value)
                target = altered
                for part in path:
                    target = target[part]
                target[key] = "passed"
                with self.subTest(path=path, extra=key):
                    self.reject(altered)

    def test_malformed_fields(self):
        for field, bad in (("schema_version", True), ("schema_version", 1.0),
                           ("review_status", "reviewed"), ("limitations", []),
                           ("limitations", [""]), ("limitations", "note"),
                           ("sources", []), ("sections", []), ("scope", [])):
            value = fixture()
            value[field] = bad
            with self.subTest(field=field, bad=bad):
                self.reject(value)
        for field in ("id", "title", "existing_requirements", "test_needs"):
            for bad in (None, 3, {}, ""):
                value = fixture()
                value["sections"][0]["obligations"][0][field] = bad
                with self.subTest(field=field, bad=bad):
                    self.reject(value)

    def test_provenance(self):
        bad_urls = ["http://doc.rust-lang.org/reference/", "https://example.com/", "https://doc.rust-lang.org.evil.test/",
                    "https://evil.doc.rust-lang.org/", "https://user@doc.rust-lang.org/reference/",
                    "https://doc.rust-lang.org:443/", "https://github.com/evil/reference/",
                    "https://doc.rust-lang.org\\@evil.test/", "https://doc.rust-lang.org/\nreference", "https://[bad/"]
        for url in bad_urls:
            value = fixture()
            value["sources"][0]["url"] = url
            with self.subTest(url=url):
                self.reject(value)
        for url in ("https://rust-lang.github.io/unsafe-code-guidelines/", "https://github.com/rust-lang/reference"):
            value = fixture()
            value["sources"][0]["url"] = url
            inventory.summarize([value], scope_fixture())
        for ref in ({"source": "missing", "section": "types"}, {"source": "reference", "section": " "}):
            value = fixture()
            value["sections"][0]["source_refs"] = [ref]
            self.reject(value)
        for field in ("url", "version_note"):
            value = fixture()
            value["sources"][0][field] = ""
            self.reject(value)

    def test_mapping_and_duplicate_strings(self):
        for field, bad in (("existing_requirements", ["unknown"]),
                           ("existing_requirements", ["req-one", "req-one"]),
                           ("existing_requirements", [None]), ("test_needs", [" "]),
                           ("test_needs", ["check", "check"])):
            value = fixture()
            value["sections"][0]["obligations"][0][field] = bad
            self.reject(value)
        value = fixture()
        value["sections"][0]["obligations"][0].update(existing_requirements=[], test_needs=[])
        self.assertEqual(inventory.summarize([value], scope_fixture())["unmapped_existing_requirements"], ["req-one", "req-two"])

    def test_invalid_scope(self):
        cases = [None, {}, dict(scope_fixture(), schema_version=True)]
        duplicate = scope_fixture()
        duplicate["areas"][0]["requirements"] *= 2
        cases.append(duplicate)
        wrong = scope_fixture()
        wrong["scope"]["edition"] = "2021"
        cases.append(wrong)
        for scope in cases:
            with self.subTest(scope=scope), self.assertRaises(inventory.InvalidInventory):
                inventory.summarize([fixture()], scope)

    def test_cli_and_malformed_files(self):
        script = Path(inventory.__file__).resolve()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            scope = root / "scope.json"
            first = root / "first.json"
            second = root / "second.json"
            contract = root / "contract.json"
            verifier = root / "stock_rustc.rs"
            contract.write_text(json.dumps(contract_fixture()), encoding="utf-8")
            verifier.write_text(VERIFIER, encoding="utf-8")
            scope.write_text(json.dumps(scope_fixture()), encoding="utf-8")
            first.write_text(json.dumps(fixture()), encoding="utf-8")
            other = fixture()
            other["sections"][0]["obligations"][0]["id"] = "other"
            second.write_text(json.dumps(other), encoding="utf-8")
            command = [sys.executable, str(script), "--root", str(root), "--scope", str(scope), "--inventory", str(first), "--inventory", str(second),
                       "--contract", str(contract), "--oracle-verifier", str(verifier)]
            result = subprocess.run(command, capture_output=True, text=True, check=False)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(json.loads(result.stdout)["counts"]["inventories"], 2)
            for raw in ('{', '{"schema_version":1,"schema_version":1}', 'NaN', 'null', '[]'):
                first.write_text(raw, encoding="utf-8")
                result = subprocess.run(command, capture_output=True, text=True, check=False)
                self.assertEqual(result.returncode, 2)
                self.assertEqual(result.stdout, "")
                self.assertIn("error", json.loads(result.stderr))
            first.unlink()
            result = subprocess.run(command, capture_output=True, text=True, check=False)
            self.assertEqual(result.returncode, 2)
            self.assertIn("error", json.loads(result.stderr))

    def test_contract_mutations(self):
        inventory.validate_contract(contract_fixture(), VERIFIER)
        for path, value in ((("oracle", "release"), "1.99.0"), (("oracle", "commit"), "b" * 40),
                            (("oracle", "verification"), "other.rs"), (("target",), "other"),
                            (("contract_id",), "other"), (("readiness", "full_rust_completion_percent"), 1),
                            (("readiness", "inventory_review"), "reviewed")):
            contract = contract_fixture()
            obj = contract
            for part in path[:-1]:
                obj = obj[part]
            obj[path[-1]] = value
            with self.subTest(path=path), self.assertRaises(inventory.InvalidInventory):
                inventory.validate_contract(contract, VERIFIER)
        with self.assertRaises(inventory.InvalidInventory):
            inventory.validate_contract(contract_fixture(), VERIFIER.replace("const RELEASE", "// const RELEASE"))

    def test_root_loader_missing_and_present(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            result = inventory.load_and_summarize(root)
            self.assertEqual(result["readiness"], "missing")
            self.assertEqual(len(result["missing_files"]), 5)
            self.assertIsNone(result["full_rust_completion_percent"])
            (root / "data").mkdir()
            (root / "tests/support").mkdir(parents=True)
            for name, value in (("compiler-compatibility-contract.json", contract_fixture()),
                                ("compiler-compatibility-scope.json", scope_fixture()),
                                ("compiler-language-inventory.json", fixture())):
                (root / "data" / name).write_text(json.dumps(value), encoding="utf-8")
            other = fixture()
            other["sections"][0]["obligations"][0]["id"] = "other"
            extra = root / "data/compiler-library-project-inventory.json"
            extra.write_text(json.dumps(other), encoding="utf-8")
            (root / "tests/support/stock_rustc.rs").write_text(VERIFIER, encoding="utf-8")
            result = inventory.load_and_summarize(root)
            self.assertEqual(result["readiness"], "unreviewed")
            self.assertEqual(result["counts"]["obligations_lacking_direct_exact_test_links"], 2)
            self.assertEqual(result["counts"]["obligations_with_executable_evidence"], 0)
            cli = subprocess.run([sys.executable, inventory.__file__, "--root", str(root)],
                                 cwd=root.parent, capture_output=True, text=True, check=False)
            self.assertEqual(cli.returncode, 0, cli.stderr)
            self.assertIsNone(json.loads(cli.stdout)["full_rust_completion_percent"])
            extra.write_text("{", encoding="utf-8")
            with self.assertRaises(inventory.InvalidInventory):
                inventory.load_and_summarize(root)


class ReferenceRulesTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        (self.root / "data").mkdir()
        (self.root / "tests/support").mkdir(parents=True)
        self.candidate = fixture()
        self.candidate["sections"][0]["source_refs"][0]["section"] = "types.html"
        for name, value in (("compiler-compatibility-contract.json", contract_fixture()),
                            ("compiler-compatibility-scope.json", scope_fixture()),
                            ("compiler-language-inventory.json", self.candidate)):
            (self.root / "data" / name).write_text(json.dumps(value), encoding="utf-8")
        (self.root / "tests/support/stock_rustc.rs").write_text(VERIFIER, encoding="utf-8")
        self.reference = "b" * 40
        self.source = {"src/SUMMARY.md": b"# Reference\n[Types](types.md)\n",
                       "src/types.md": b"# Types\nr[type.one]\n> [!EDITION-2024]\n",
                       "src/unlinked.md": b"# Extra\nr[type.unlinked]\n"}
        self.archive = self.root / "reference.tar.gz"
        self.write_archive(list(self.source.items()))
        api = "https://api.github.com/repos/rust-lang/rust/contents/src/doc/reference?ref=" + "a" * 40
        repository = "https://github.com/rust-lang/reference/tree/" + self.reference
        pin = {"sha": self.reference, "path": "src/doc/reference", "type": "submodule",
               "url": api, "html_url": repository, "submodule_git_url": "https://github.com/rust-lang/reference.git"}
        self.pin = self.root / "pin.json"
        self.pin.write_text(json.dumps(pin), encoding="utf-8")
        from extract_reference_rules import parse_markdown
        pages = []
        for path, raw in self.source.items():
            rules, headings, editions = parse_markdown(raw.decode())
            linked = path == "src/types.md"
            pages.append({"path": path, "url": "https://github.com/rust-lang/reference/blob/" + self.reference + "/" + path,
                          "sha256": inventory.sha256(raw), "rules": rules, "headings": headings,
                          "edition_note_markers": editions, "candidate_section": "types" if linked else None,
                          "candidate_obligations": ["types-one"] if linked else [],
                          "rule_obligation_mapping_status": "unreviewed"})
        self.data = {"schema_version": 1, "review_status": "unreviewed", "scope": dict(inventory.CONTRACT),
                     "provenance": {"compiler_commit": "a" * 40, "reference_commit": self.reference,
                        "compiler_submodule_api_url": api, "reference_repository_url": repository,
                        "compiler_submodule_response_sha256": inventory.sha256(self.pin.read_bytes()),
                        "source_archive_url": "https://codeload.github.com/rust-lang/reference/tar.gz/" + self.reference,
                        "source_archive_sha256": inventory.sha256(self.archive.read_bytes()),
                        "archive_verified_local_files": 3, "local_source_bytes_match_archive": True,
                        "summary_sha256": inventory.sha256(self.source["src/SUMMARY.md"]),
                        "candidate_inventory_sha256": inventory.sha256((self.root / "data/compiler-language-inventory.json").read_bytes())},
                     "counts": {"chapter_pages": 1, "linked_page_rule_labels": 1, "unlinked_page_rule_labels": 1,
                        "unique_rule_labels_total": 2, "duplicate_rule_labels": 0, "candidate_pages": 1,
                        "chapter_pages_without_candidate_section": 0, "candidate_pages_absent_from_pinned_summary": 0,
                        "rules_with_reviewed_obligation_mapping": 0, "rules_requiring_mapping_review": 2},
                     "pinned_pages_without_candidate_section": [], "candidate_pages_absent_from_pinned_summary": [],
                     "pages": [p for p in pages if p["path"] == "src/types.md"],
                     "unlinked_markdown_files": [p for p in pages if p["path"] != "src/types.md"],
                     "limitations": ["Synthetic metadata, not semantic or executable coverage."]}

    def write_archive(self, entries):
        with tarfile.open(self.archive, "w:gz") as archive:
            for path, raw in entries:
                member = tarfile.TarInfo("reference-" + self.reference + "/" + path)
                member.size = len(raw)
                archive.addfile(member, io.BytesIO(raw))

    def validate(self, data=None, **kwargs):
        return inventory.validate_reference_rules(self.root, self.data if data is None else data, **kwargs)

    def reject(self, data):
        with self.assertRaises(inventory.InvalidInventory):
            self.validate(data)

    def test_counts_remain_unreviewed_not_coverage(self):
        result = self.validate()
        self.assertEqual(result["counts"]["rules_requiring_mapping_review"], 2)
        self.assertEqual(result["counts"]["rules_with_reviewed_obligation_mapping"], 0)
        self.assertEqual(result["readiness"], "review-required")
        self.assertIsNone(result["full_rust_completion_percent"])
        self.assertEqual(result["source_archive_byte_verification"], "not_reverified_in_this_run")
        self.assertEqual(result["compiler_submodule_byte_verification"], "not_reverified_in_this_run")

    def test_exact_counts_and_types_recomputed(self):
        for name in self.data["counts"]:
            for bad in (True, 0.0, self.data["counts"][name] + 1):
                altered = copy.deepcopy(self.data)
                altered["counts"][name] = bad
                with self.subTest(name=name, bad=bad):
                    self.reject(altered)

    def test_missing_extra_and_manual_pass_fields(self):
        for path in ((), ("provenance",), ("counts",), ("pages", 0), ("pages", 0, "rules", 0)):
            obj = self.data
            for part in path:
                obj = obj[part]
            for name in obj:
                altered = copy.deepcopy(self.data)
                target = altered
                for part in path:
                    target = target[part]
                del target[name]
                with self.subTest(path=path, missing=name):
                    self.reject(altered)
            altered = copy.deepcopy(self.data)
            target = altered
            for part in path:
                target = target[part]
            target["passing"] = True
            self.reject(altered)

    def test_provenance_and_immutable_pins(self):
        for name, bad in (("compiler_commit", "c" * 40), ("reference_commit", "main"),
                          ("compiler_submodule_api_url", "https://example.com/"),
                          ("reference_repository_url", "https://github.com/rust-lang/reference/tree/main"),
                          ("source_archive_url", "https://codeload.github.com/rust-lang/reference/tar.gz/main"),
                          ("candidate_inventory_sha256", "c" * 64), ("summary_sha256", "c" * 64),
                          ("source_archive_sha256", "not-a-hash"), ("archive_verified_local_files", True),
                          ("local_source_bytes_match_archive", "true")):
            altered = copy.deepcopy(self.data)
            altered["provenance"][name] = bad
            with self.subTest(name=name):
                self.reject(altered)
        candidate = self.root / "data/compiler-language-inventory.json"
        candidate.write_bytes(candidate.read_bytes() + b"\n")
        self.reject(self.data)

    def test_page_and_global_label_identities(self):
        for name, bad in (("url", "https://github.com/rust-lang/reference/blob/main/src/types.md"),
                          ("path", "src/../types.md"), ("path", "/src/types.md"),
                          ("sha256", "no"), ("candidate_section", "not-types"),
                          ("candidate_obligations", ["unknown"]), ("candidate_obligations", []),
                          ("rule_obligation_mapping_status", "reviewed")):
            altered = copy.deepcopy(self.data)
            altered["pages"][0][name] = bad
            with self.subTest(name=name, bad=bad):
                self.reject(altered)
        altered = copy.deepcopy(self.data)
        altered["unlinked_markdown_files"][1]["rules"][0]["label"] = "type.one"
        self.reject(altered)
        altered = copy.deepcopy(self.data)
        altered["pages"].append(copy.deepcopy(altered["pages"][0]))
        self.reject(altered)

    def test_missing_reference_is_explicit_blocker(self):
        result = inventory.reference_readiness(self.root)
        self.assertEqual(result["readiness"], "missing")
        self.assertEqual(result["missing_files"], [inventory.REFERENCE_RULES])
        (self.root / inventory.REFERENCE_RULES).write_text("{", encoding="utf-8")
        with self.assertRaises(inventory.InvalidInventory):
            inventory.reference_readiness(self.root)

    def test_root_summary_and_cli_integrate_reference(self):
        extra = fixture()
        extra["sections"][0]["obligations"][0]["id"] = "library-extra"
        (self.root / "data/compiler-library-project-inventory.json").write_text(json.dumps(extra), encoding="utf-8")
        missing = inventory.load_and_summarize(self.root)
        self.assertEqual(missing["inventory_readiness_blockers"], [inventory.REFERENCE_RULES])
        reference_path = self.root / inventory.REFERENCE_RULES
        reference_path.write_text(json.dumps(self.data), encoding="utf-8")
        result = inventory.load_and_summarize(self.root)
        self.assertEqual(result["reference_rules"]["counts"]["unique_rule_labels_total"], 2)
        self.assertEqual(result["inventory_readiness_blockers"], [])
        command = [sys.executable, inventory.__file__, "--root", str(self.root)]
        result = subprocess.run(command, capture_output=True, text=True, check=False)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads(result.stdout)["reference_rules"]["counts"]["rules_with_reviewed_obligation_mapping"], 0)
        reference_path.write_text("null", encoding="utf-8")
        with self.assertRaises(inventory.InvalidInventory):
            inventory.load_and_summarize(self.root)
        result = subprocess.run(command + ["--reference-rules", str(reference_path)], capture_output=True, text=True, check=False)
        self.assertEqual(result.returncode, 2)
        self.assertEqual(result.stdout, "")
        self.assertIn("error", json.loads(result.stderr))

    def test_rule_positions_and_other_schema_states(self):
        for name, bad in (("schema_version", True), ("review_status", "passed"), ("limitations", [])):
            altered = copy.deepcopy(self.data)
            altered[name] = bad
            self.reject(altered)
        for line in (0, -1, True, "2"):
            altered = copy.deepcopy(self.data)
            altered["pages"][0]["rules"][0]["line"] = line
            self.reject(altered)
        altered = copy.deepcopy(self.data)
        altered["pages"][0]["headings"][0]["level"] = 7
        self.reject(altered)
        altered = copy.deepcopy(self.data)
        altered["pages"][0]["edition_note_markers"][0]["edition"] = "next"
        self.reject(altered)
        with self.assertRaises(inventory.InvalidInventory):
            inventory.validate_reference_rules(self.root, None)

    def test_archive_and_pin_reproduction(self):
        result = self.validate(source_archive=self.archive, submodule_proof=self.pin)
        self.assertEqual(result["source_archive_byte_verification"], "reproduced-from-provided-archive")
        self.assertEqual(result["compiler_submodule_byte_verification"], "reproduced-from-provided-response")
        altered = copy.deepcopy(self.data)
        altered["pages"][0]["sha256"] = "c" * 64
        with self.assertRaises(inventory.InvalidInventory):
            self.validate(altered, source_archive=self.archive)
        altered = copy.deepcopy(self.data)
        altered["pages"][0]["rules"][0]["line"] = 3
        with self.assertRaises(inventory.InvalidInventory):
            self.validate(altered, source_archive=self.archive)
        self.pin.write_bytes(self.pin.read_bytes() + b"\n")
        with self.assertRaises(inventory.InvalidInventory):
            self.validate(submodule_proof=self.pin)

    def test_unsafe_duplicate_and_mismatched_archives(self):
        for entries in ([*self.source.items(), ("../escape.md", b"bad")],
                        [*self.source.items(), ("src/types.md", b"duplicate")],
                        [("src/types.md", b"bad")]):
            self.write_archive(entries)
            altered = copy.deepcopy(self.data)
            altered["provenance"]["source_archive_sha256"] = inventory.sha256(self.archive.read_bytes())
            with self.subTest(entries=len(entries)), self.assertRaises(inventory.InvalidInventory):
                self.validate(altered, source_archive=self.archive)
        self.archive.write_bytes(b"bad archive")
        with self.assertRaises(inventory.InvalidInventory):
            self.validate(source_archive=self.archive)


if __name__ == "__main__":
    unittest.main()
