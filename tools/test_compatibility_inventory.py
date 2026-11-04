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
from unittest import mock

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
    def test_actual_root_proposals_loader_and_both_cli_paths(self):
        root = Path(__file__).resolve().parents[1]
        report = inventory.load_and_summarize(root)
        self.assertEqual(report['counts']['obligations'], 257)
        self.assertEqual(report['counts']['obligations_with_executable_evidence'], 0)
        self.assertIsNone(report['full_rust_completion_percent'])
        expected = report['rule_proposals']
        self.assertEqual(expected['global_rule_identities'], 2965)
        self.assertEqual(expected['proposed_rule_identities'], 228)
        self.assertEqual(expected['rule_identities_without_proposals'], 2737)
        for options in ([], ['--reference-rules', str(root / inventory.REFERENCE_RULES)]):
            process = subprocess.run([sys.executable, inventory.__file__, '--root', str(root), *options],
                                     capture_output=True, text=True, check=False)
            self.assertEqual(process.returncode, 0, process.stderr)
            result = json.loads(process.stdout)
            self.assertEqual(result['rule_proposals'], expected)
            self.assertEqual(result['counts']['obligations'], 257)
            self.assertEqual(result['counts']['obligations_with_executable_evidence'], 0)

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
            self.assertEqual(result['rule_proposals']['readiness'], 'missing')
            self.assertIsNone(result['rule_proposals']['global_rule_identities'])
            cli = subprocess.run([sys.executable, inventory.__file__, '--root', str(root)],
                                 capture_output=True, text=True, check=False)
            self.assertEqual(cli.returncode, 0, cli.stderr)
            self.assertEqual(json.loads(cli.stdout)['rule_proposals']['readiness'], 'missing')
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
        self.assertEqual(missing["inventory_readiness_blockers"],
                         [inventory.REFERENCE_RULES, inventory.LIBRARY_PROVENANCE, inventory.LIBRARY_BACKLOG])
        reference_path = self.root / inventory.REFERENCE_RULES
        reference_path.write_text(json.dumps(self.data), encoding="utf-8")
        result = inventory.load_and_summarize(self.root)
        self.assertEqual(result["reference_rules"]["counts"]["unique_rule_labels_total"], 2)
        self.assertEqual(result["inventory_readiness_blockers"], [inventory.LIBRARY_PROVENANCE, inventory.LIBRARY_BACKLOG])
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


class LibraryProvenanceTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        (self.root / "data").mkdir()
        (self.root / "tests/support").mkdir(parents=True)
        (self.root / "tools").mkdir()
        (self.root / "data/compiler-compatibility-contract.json").write_text(json.dumps(contract_fixture()), encoding="utf-8")
        (self.root / "tests/support/stock_rustc.rs").write_text(VERIFIER, encoding="utf-8")
        tools = []
        for name in inventory.LIBRARY_TOOLS:
            raw = ("# Synthetic non-executed reproduction tool " + name + "\n").encode()
            (self.root / "tools" / name).write_bytes(raw)
            tools.append({"filename": name, "content_sha256": inventory.sha256(raw)})
        base = "https://doc.rust-lang.org/1.98.1/"
        self.data = {"schema_version": 1, "review_status": "unreviewed", "scope": dict(inventory.CONTRACT),
                     "inventory_kind": "pinned-library-source-and-documentation-input-provenance",
                     "source_identity": {"commit": "a" * 40, "release": "1.98.1",
                         "repository": "https://github.com/rust-lang/rust", "official_documentation_root": base,
                         "root_tree_sha1": "b" * 40, "library_tree_sha1": "c" * 40,
                         "source_index_content_sha256": "d" * 64, "verification": "Synthetic acquisition record"},
                     "artifacts": [{"id": "pinned-html-link-anchor-candidate", "filename": "pinned-html-api-index.json",
                         "content_sha256": "e" * 64, "byte_length": 100, "role": "External observed records"},
                         {"id": "pinned-library-source-path-index", "filename": "library-source-index.json",
                         "content_sha256": "d" * 64, "byte_length": 120, "role": "External source record"}],
                     "observations": {"downloaded_page_records": 8, "literal_index_link_anchor_candidate_records": 10,
                         "missing_detail_page_records": 6, "interpretation": "These observed records are not API counts."},
                     "pages": [{"path": crate + "/" + page, "url": base + crate + "/" + page,
                         "rustdoc_version": "1.98.1 (aaaaaaaaa 2026-09-01)", "content_sha256": "f" * 64,
                         "byte_length": 42} for crate in ("core", "alloc", "std", "proc_macro")
                         for page in ("index.html", "all.html")],
                     "reproduction": {"tools": tools, "working_directory": "Synthetic tools directory",
                         "commands": ["Synthetic non-executed reproduction command"], "requirements": ["External source bytes"]},
                     "limitations": ["Input provenance, not API coverage"], "review_needs": ["API identity and semantics review"]}

    def validate(self, data=inventory._ABSENT):
        return inventory.validate_library_surface_provenance(self.root, self.data if data is inventory._ABSENT else data)

    def reject(self, data):
        with self.assertRaises(inventory.InvalidInventory):
            self.validate(data)

    def test_summary_is_only_input_provenance(self):
        result = self.validate()
        self.assertEqual(result["readiness"], "review-required")
        self.assertEqual(result["observations"]["downloaded_page_records"], 8)
        self.assertEqual(result["external_artifacts"], 2)
        self.assertEqual(result["locally_bound_reproduction_tools"], 3)
        self.assertEqual(result["required_index_pages"], 8)
        self.assertIsNone(result["api_denominator"])
        self.assertIsNone(result["full_rust_completion_percent"])
        self.assertEqual(result["external_source_byte_verification"], "not_reverified_in_this_run")
        self.assertNotIn("supported", result)

    def test_exact_schema_and_no_manual_states(self):
        paths = [(), ("scope",), ("source_identity",), ("artifacts", 0), ("observations",),
                 ("pages", 0), ("reproduction",), ("reproduction", "tools", 0)]
        for path in paths:
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
            for name in ("passing", "support", "execution_status"):
                altered = copy.deepcopy(self.data)
                target = altered
                for part in path:
                    target = target[part]
                target[name] = "passed"
                with self.subTest(path=path, extra=name):
                    self.reject(altered)

    def test_pin_and_official_url_mismatches(self):
        for name, bad in (("release", "1.99.0"), ("commit", "f" * 40),
                          ("repository", "https://github.com/other/rust"),
                          ("official_documentation_root", "https://doc.rust-lang.org/stable/"),
                          ("root_tree_sha1", "main"), ("library_tree_sha1", "A" * 40),
                          ("source_index_content_sha256", "e" * 64), ("verification", "")):
            altered = copy.deepcopy(self.data)
            altered["source_identity"][name] = bad
            with self.subTest(name=name):
                self.reject(altered)
        for url in ("http://doc.rust-lang.org/1.98.1/core/index.html", "https://evil.test/core/index.html",
                    "https://doc.rust-lang.org.evil.test/1.98.1/core/index.html", "https://doc.rust-lang.org/stable/core/index.html"):
            altered = copy.deepcopy(self.data)
            altered["pages"][0]["url"] = url
            self.reject(altered)

    def test_required_eight_pages_and_unique_paths(self):
        for index in range(8):
            altered = copy.deepcopy(self.data)
            altered["pages"].pop(index)
            altered["observations"]["downloaded_page_records"] = 7
            with self.subTest(index=index):
                self.reject(altered)
        altered = copy.deepcopy(self.data)
        altered["pages"].append(copy.deepcopy(altered["pages"][0]))
        altered["observations"]["downloaded_page_records"] = 9
        self.reject(altered)
        for path in ("../core/index.html", "core/../core/index.html", "/core/index.html", "core//index.html",
                     "core/index.html#anchor", "other/index.html", "core/index.txt"):
            altered = copy.deepcopy(self.data)
            altered["pages"][0]["path"] = path
            altered["pages"][0]["url"] = "https://doc.rust-lang.org/1.98.1/" + path
            self.reject(altered)

    def test_rustdoc_version_markers(self):
        for marker in ("1.99.0 (aaaaaaaaa 2026-09-01)", "1.98.1 (bbbbbbbbb 2026-09-01)",
                       "1.98.1 (aaaaaaaaa 2026-02-31)", "1.98.1 (aaaaaaaaa 2026-09-02)", "", 4):
            altered = copy.deepcopy(self.data)
            altered["pages"][0]["rustdoc_version"] = marker
            with self.subTest(marker=marker):
                self.reject(altered)

    def test_hashes_lengths_and_observed_count_types(self):
        for path, field in ((("pages", 0), "content_sha256"), (("artifacts", 0), "content_sha256"),
                            (("pages", 0), "byte_length"), (("artifacts", 0), "byte_length")):
            for bad in (None, "", True, -1, 0, 1.0):
                altered = copy.deepcopy(self.data)
                target = altered
                for part in path:
                    target = target[part]
                target[field] = bad
                self.reject(altered)
        for name in ("downloaded_page_records", "literal_index_link_anchor_candidate_records", "missing_detail_page_records"):
            for bad in (True, -1, 1.0, "1"):
                altered = copy.deepcopy(self.data)
                altered["observations"][name] = bad
                self.reject(altered)
        altered = copy.deepcopy(self.data)
        altered["observations"]["downloaded_page_records"] = 9
        self.reject(altered)
        altered = copy.deepcopy(self.data)
        altered["observations"]["missing_detail_page_records"] = 11
        self.reject(altered)

    def test_artifact_identity_and_hash_linkage(self):
        for artifacts in (self.data["artifacts"][:1], self.data["artifacts"] * 2):
            altered = copy.deepcopy(self.data)
            altered["artifacts"] = artifacts
            self.reject(altered)
        for name, bad in (("id", "unexpected"), ("filename", "../pinned-html-api-index.json")):
            altered = copy.deepcopy(self.data)
            altered["artifacts"][0][name] = bad
            self.reject(altered)
        altered = copy.deepcopy(self.data)
        altered["artifacts"][1]["content_sha256"] = "e" * 64
        self.reject(altered)

    def test_exact_three_tool_bytes_bound(self):
        for tools in (self.data["reproduction"]["tools"][:2], self.data["reproduction"]["tools"] * 2):
            altered = copy.deepcopy(self.data)
            altered["reproduction"]["tools"] = tools
            self.reject(altered)
        altered = copy.deepcopy(self.data)
        altered["reproduction"]["tools"][0]["filename"] = "../acquire_pinned_library_sources.py"
        self.reject(altered)
        altered = copy.deepcopy(self.data)
        altered["reproduction"]["tools"][0]["content_sha256"] = "a" * 64
        self.reject(altered)
        for name in inventory.LIBRARY_TOOLS:
            path = self.root / "tools" / name
            original = path.read_bytes()
            path.write_bytes(original + b"# drift\n")
            self.reject(self.data)
            path.write_bytes(original)
        (self.root / "tools" / inventory.LIBRARY_TOOLS[0]).unlink()
        self.reject(self.data)

    def test_unreviewed_limitations_and_scope(self):
        for name, bad in (("schema_version", True), ("schema_version", 1.0), ("review_status", "reviewed"),
                          ("inventory_kind", "complete-api-inventory"), ("limitations", []),
                          ("limitations", [" "]), ("review_needs", []), ("scope", {"edition": "2021"})):
            altered = copy.deepcopy(self.data)
            altered[name] = bad
            self.reject(altered)
        self.reject(None)

    def test_missing_and_malformed_root_metadata(self):
        result = inventory.library_surface_readiness(self.root)
        self.assertEqual(result["readiness"], "missing")
        self.assertEqual(result["missing_files"], [inventory.LIBRARY_PROVENANCE])
        self.assertIsNone(result["api_denominator"])
        path = self.root / inventory.LIBRARY_PROVENANCE
        for raw in ("{", "null", "[]", '{"schema_version":1,"schema_version":1}'):
            path.write_text(raw, encoding="utf-8")
            with self.assertRaises(inventory.InvalidInventory):
                inventory.library_surface_readiness(self.root)
            with self.assertRaises(inventory.InvalidInventory):
                inventory.load_and_summarize(self.root)

    def test_summary_loader_and_cli(self):
        candidate = fixture()
        extra = fixture()
        extra["sections"][0]["obligations"][0]["id"] = "library-extra"
        for name, value in (("compiler-compatibility-scope.json", scope_fixture()),
                            ("compiler-language-inventory.json", candidate),
                            ("compiler-library-project-inventory.json", extra),
                            ("compiler-library-surface-provenance.json", self.data)):
            (self.root / "data" / name).write_text(json.dumps(value), encoding="utf-8")
        result = inventory.load_and_summarize(self.root)
        self.assertEqual(result["library_surface_inputs"]["observations"]["downloaded_page_records"], 8)
        self.assertEqual(result["inventory_readiness_blockers"], [inventory.REFERENCE_RULES, inventory.LIBRARY_BACKLOG])
        command = [sys.executable, inventory.__file__, "--root", str(self.root)]
        process = subprocess.run(command, capture_output=True, text=True, check=False)
        self.assertEqual(process.returncode, 0, process.stderr)
        self.assertIsNone(json.loads(process.stdout)["library_surface_inputs"]["api_denominator"])
        (self.root / inventory.LIBRARY_PROVENANCE).write_text("null", encoding="utf-8")
        process = subprocess.run(command, capture_output=True, text=True, check=False)
        self.assertEqual(process.returncode, 2)
        self.assertEqual(process.stdout, "")
        self.assertIn("error", json.loads(process.stderr))


class LibraryBacklogIntegrationTests(unittest.TestCase):
    def setUp(self):
        self.actual_root = Path(__file__).resolve().parents[1]
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        (self.root / "data").mkdir()
        self.provenance = json.loads((self.actual_root / inventory.LIBRARY_PROVENANCE).read_bytes())
        self.backlog = json.loads((self.actual_root / inventory.LIBRARY_BACKLOG).read_bytes())

    def write_backlog(self, value):
        (self.root / inventory.LIBRARY_BACKLOG).write_text(json.dumps(value), encoding="utf-8")

    def test_missing_backlog_never_means_complete_api_inventory(self):
        result = inventory.library_documentation_backlog_readiness(self.root, provenance=self.provenance)
        self.assertEqual(result["readiness"], "missing")
        self.assertEqual(result["missing_files"], [inventory.LIBRARY_BACKLOG])
        self.assertIsNone(result["api_denominator"])
        self.assertIsNone(result["full_rust_completion_percent"])

    def test_present_backlog_without_provenance_is_blocked(self):
        self.write_backlog(self.backlog)
        result = inventory.library_surface_readiness(self.root)
        self.assertEqual(result["readiness"], "missing")
        backlog = result["documentation_backlog"]
        self.assertEqual(backlog["readiness"], "blocked-by-missing-inputs")
        self.assertEqual(backlog["missing_files"], [inventory.LIBRARY_PROVENANCE])
        partial = inventory.load_and_summarize(self.root)
        self.assertEqual(partial["library_surface_inputs"]["documentation_backlog"], backlog)
        self.assertIn(inventory.LIBRARY_PROVENANCE, partial["inventory_readiness_blockers"])

    def test_present_malformed_backlog_rejected_even_in_incomplete_root(self):
        for raw in ("{", "null", "[]", "{}", '{"schema_version":1,"schema_version":1}'):
            (self.root / inventory.LIBRARY_BACKLOG).write_text(raw, encoding="utf-8")
            with self.subTest(raw=raw):
                with self.assertRaises(inventory.InvalidInventory):
                    inventory.load_and_summarize(self.root)
                process = subprocess.run([sys.executable, inventory.__file__, "--root", str(self.root)],
                                         capture_output=True, text=True, check=False)
                self.assertEqual(process.returncode, 2)
                self.assertEqual(process.stdout, "")
                self.assertIn("error", json.loads(process.stderr))

    def test_recorded_metadata_summary_has_no_review_or_execution_credit(self):
        self.write_backlog(self.backlog)
        result = inventory.library_documentation_backlog_readiness(self.root, provenance=self.provenance)
        self.assertEqual(result["readiness"], "review-required")
        self.assertEqual(result["review_status"], "unreviewed")
        self.assertIsNone(result["api_denominator"])
        self.assertIsNone(result["full_rust_completion_percent"])
        self.assertNotIn("supported", result)
        self.assertNotIn("passed", result)
        self.assertEqual(result["external_verification"], "recorded-not-recomputed")
        self.assertIsNone(result["full_rust_denominator"])
        for key in ("review_credit", "execution_credit", "completion_credit"):
            self.assertEqual(result[key], 0)
        counts = result["observations"]["global"]
        self.assertEqual(counts, {"literal_candidate_records": 35236, "unique_candidate_page_paths": 35154,
                                "downloaded_page_records": 40, "downloaded_index_all_pages": 8,
                                "downloaded_other_detail_pages": 32, "missing_unique_paths": 35114,
                                "candidate_records_collapsed_by_page_path": 82})
        self.assertEqual({crate: row["missing_unique_paths"]
                          for crate, row in result["observations"]["per_crate"].items()},
                         {"alloc": 257, "core": 32790, "proc_macro": 0, "std": 2067})
        self.assertEqual(len(result["missing_path_sample"]), 100)
        self.assertTrue(result["missing_path_sample_truncated"])

    def test_partial_root_malformed_companion_provenance_is_structured_error(self):
        self.write_backlog(self.backlog)
        for key, bad in (("source_identity", {}), ("observations", {}), ("observations", None),
                         ("pages", [None]), ("artifacts", None)):
            changed = copy.deepcopy(self.provenance)
            changed[key] = bad
            (self.root / inventory.LIBRARY_PROVENANCE).write_text(json.dumps(changed), encoding="utf-8")
            with self.subTest(key=key, bad=bad):
                with self.assertRaises(inventory.InvalidInventory):
                    inventory.load_and_summarize(self.root)
                process = subprocess.run([sys.executable, inventory.__file__, "--root", str(self.root)],
                                         capture_output=True, text=True, check=False)
                self.assertEqual(process.returncode, 2)
                self.assertEqual(process.stdout, "")
                self.assertIn("error", json.loads(process.stderr))

    def test_stale_provenance_and_manual_review_state_rejected(self):
        self.write_backlog(self.backlog)
        changed = copy.deepcopy(self.provenance)
        changed["artifacts"][0]["content_sha256"] = "f" * 64
        with self.assertRaises(inventory.InvalidInventory):
            inventory.library_documentation_backlog_readiness(self.root, provenance=changed)
        changed = copy.deepcopy(self.backlog)
        changed["review_status"] = "reviewed"
        self.write_backlog(changed)
        with self.assertRaises(inventory.InvalidInventory):
            inventory.library_documentation_backlog_readiness(self.root, provenance=self.provenance)

    def test_actual_root_loader_inventory_clis_and_coverage_agree(self):
        expected = inventory.load_and_summarize(self.actual_root)["library_surface_inputs"]["documentation_backlog"]
        self.assertEqual(expected["readiness"], "review-required")
        self.assertIsNone(expected["api_denominator"])
        for options in ([], ["--reference-rules", str(self.actual_root / inventory.REFERENCE_RULES)]):
            process = subprocess.run([sys.executable, inventory.__file__, "--root", str(self.actual_root), *options],
                                     capture_output=True, text=True, check=False)
            self.assertEqual(process.returncode, 0, process.stderr)
            result = json.loads(process.stdout)
            self.assertEqual(result["library_surface_inputs"]["documentation_backlog"], expected)
            self.assertNotIn(inventory.LIBRARY_BACKLOG, result["inventory_readiness_blockers"])
        process = subprocess.run([sys.executable, str(self.actual_root / "tools/conformance.py"), "coverage", "--json"],
                                 cwd=self.actual_root, capture_output=True, text=True, check=False)
        self.assertEqual(process.returncode, 0, process.stderr)
        result = json.loads(process.stdout)
        self.assertEqual(result["semantic_inventory"]["library_surface_inputs"]["documentation_backlog"], expected)
        process = subprocess.run([sys.executable, str(self.actual_root / "tools/conformance.py"), "coverage"],
                                 cwd=self.actual_root, capture_output=True, text=True, check=False)
        self.assertEqual(process.returncode, 0, process.stderr)
        self.assertIn("40 downloaded page records; 35114 missing literal page records", process.stdout)
        self.assertIn("Backlog: review-required", process.stdout)
        self.assertIn("Not API counts or compatibility coverage. External source bytes not reverified.", process.stdout)


class PartialWitnessIntegrationTests(unittest.TestCase):
    def setUp(self):
        self.actual_root = Path(__file__).resolve().parents[1]
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        (self.root / "data").mkdir()

    def test_actual_partial_attribution_never_completes_inventory_obligations(self):
        result = inventory.load_and_summarize(self.actual_root)
        witnesses = result["partial_witness_sources"]
        self.assertEqual(witnesses["readiness"], "review-required")
        self.assertEqual(witnesses["review_status"], "unreviewed")
        self.assertEqual(witnesses["direct_witness_selectors"], 12)
        self.assertEqual(witnesses["direct_rule_links"], 35)
        self.assertEqual(witnesses["distinct_direct_rule_identities"], 19)
        self.assertEqual(witnesses["candidate_obligations_with_partial_links"], 11)
        self.assertEqual(witnesses["source_file_byte_verification"], "reverified_in_this_run")
        for key in ("source_block_byte_verification", "source_archive_byte_verification"):
            self.assertEqual(witnesses[key], "not_reverified_in_this_run")
        for key in ("reviewed_rule_rows", "completed_rule_rows", "rows_with_exact_executable_evidence"):
            self.assertEqual(witnesses[key], 0)
        self.assertFalse(witnesses["execution_evidence_imported"])
        self.assertIsNone(witnesses["full_rust_completion_percent"])
        self.assertEqual(result["counts"]["obligations"], 257)
        self.assertEqual(result["counts"]["obligations_lacking_direct_exact_test_links"], 257)
        self.assertEqual(result["counts"]["obligations_with_executable_evidence"], 0)

    def test_both_inventory_cli_paths_and_coverage_expose_same_partial_sources(self):
        expected = inventory.partial_witness_source_readiness(self.actual_root)
        for options in ([], ["--reference-rules", str(self.actual_root / inventory.REFERENCE_RULES)]):
            result = subprocess.run([sys.executable, inventory.__file__, "--root", str(self.actual_root), *options],
                                    capture_output=True, text=True, check=False)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(json.loads(result.stdout)["partial_witness_sources"], expected)
        result = subprocess.run([sys.executable, str(self.actual_root / "tools/conformance.py"), "coverage", "--json"],
                                cwd=self.actual_root, capture_output=True, text=True, check=False)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads(result.stdout)["semantic_inventory"]["partial_witness_sources"], expected)
        result = subprocess.run([sys.executable, str(self.actual_root / "tools/conformance.py"), "coverage"],
                                cwd=self.actual_root, capture_output=True, text=True, check=False)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("Unreviewed partial source attribution: 12 exact selectors; 19 rule labels; 11 candidate obligations", result.stdout)
        self.assertIn("Not reviewed coverage or test outcomes. Reference block bytes not reverified.", result.stdout)

    def test_missing_partial_source_inputs_have_no_claimed_credit(self):
        result = inventory.load_and_summarize(self.root)["partial_witness_sources"]
        self.assertEqual(result["readiness"], "missing")
        self.assertTrue(result["missing_files"])
        self.assertIsNone(result["full_rust_completion_percent"])
        self.assertFalse(result["execution_evidence_imported"])

    def test_present_malformed_index_is_not_hidden_by_missing_companions(self):
        for raw in ("null", "[]", "{}", "{", '{"schema_version":1,"schema_version":1}'):
            (self.root / inventory.REFERENCE_RULES).write_text(raw, encoding="utf-8")
            with self.subTest(raw=raw):
                with self.assertRaises(inventory.InvalidInventory):
                    inventory.partial_witness_source_readiness(self.root)
                result = subprocess.run([sys.executable, inventory.__file__, "--root", str(self.root)],
                                        capture_output=True, text=True, check=False)
                self.assertEqual(result.returncode, 2)
                self.assertEqual(result.stdout, "")
                self.assertIn("error", json.loads(result.stderr))

    def test_present_source_drift_is_not_hidden_by_other_missing_sources(self):
        for name in (inventory.REFERENCE_RULES, "data/compiler-language-inventory.json"):
            (self.root / name).write_bytes((self.actual_root / name).read_bytes())
        helper = self.root / "tests/support/closure_oracle.rs"
        helper.parent.mkdir(parents=True)
        helper.write_bytes(b"// changed source, not a missing-input excuse\n")
        with self.assertRaises(inventory.InvalidInventory):
            inventory.partial_witness_source_readiness(self.root)

    def test_source_only_summary_does_not_probe_a_compiler_or_execution_identity(self):
        import conformance
        import compatibility_scope
        with mock.patch.object(subprocess, "run", side_effect=AssertionError("no process probes")), \
                mock.patch.object(conformance, "source_identity", side_effect=AssertionError("no execution identity")), \
                mock.patch.object(conformance, "discover", side_effect=AssertionError("no unrelated whole-suite discovery")), \
                mock.patch.object(compatibility_scope, "validate", side_effect=AssertionError("no unrelated whole-scope validation")), \
                mock.patch.object(compatibility_scope, "runner_identity", side_effect=AssertionError("no runner")):
            result = inventory.partial_witness_source_readiness(self.actual_root)
        self.assertEqual(result["direct_witness_selectors"], 12)
        self.assertFalse(result["execution_evidence_imported"])


class LibraryWitnessIntegrationTests(unittest.TestCase):
    def setUp(self):
        self.actual_root = Path(__file__).resolve().parents[1]
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        (self.root / "data").mkdir()

    def test_partial_library_associations_do_not_complete_inventory_obligations(self):
        report = inventory.load_and_summarize(self.actual_root)
        result = report["library_witness_sources"]
        self.assertEqual(result["readiness"], "review-required")
        self.assertEqual(result["review_status"], "unreviewed")
        for key, count in (("direct_witness_selectors", 6), ("positive_selectors", 3),
                           ("semantic_invalid_selectors", 3), ("direct_documentation_links", 10),
                           ("distinct_documentation_references", 7), ("documentation_page_count", 3),
                           ("candidate_obligations_with_partial_links", 3)):
            self.assertEqual(result[key], count, key)
        self.assertEqual(result["source_file_byte_verification"], "reverified_in_this_run")
        for key in ("documentation_page_byte_verification", "documentation_anchor_verification"):
            self.assertEqual(result[key], "not_reverified_in_this_run")
        for key in ("reviewed_obligations", "completed_obligations", "obligations_with_executable_evidence"):
            self.assertEqual(result[key], 0)
        self.assertFalse(result["execution_evidence_imported"])
        self.assertIsNone(result["full_rust_completion_percent"])
        self.assertIsNone(result["api_denominator"])
        self.assertEqual(report["counts"]["obligations"], 257)
        self.assertEqual(report["counts"]["obligations_lacking_direct_exact_test_links"], 257)
        self.assertEqual(report["counts"]["obligations_with_executable_evidence"], 0)

    def test_both_inventory_cli_paths_and_coverage_report_same_library_sources(self):
        expected = inventory.library_witness_readiness(self.actual_root)
        for options in ([], ["--library-surface-provenance", str(self.actual_root / inventory.LIBRARY_PROVENANCE)]):
            result = subprocess.run([sys.executable, inventory.__file__, "--root", str(self.actual_root), *options],
                                    capture_output=True, text=True, check=False)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(json.loads(result.stdout)["library_witness_sources"], expected)
        command = [sys.executable, str(self.actual_root / "tools/conformance.py"), "coverage"]
        result = subprocess.run([*command, "--json"], cwd=self.actual_root, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads(result.stdout)["semantic_inventory"]["library_witness_sources"], expected)
        result = subprocess.run(command, cwd=self.actual_root, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("Unreviewed library source attribution: 6 exact selectors; 7 documentation references; 3 candidate obligations", result.stdout)
        self.assertIn("Not an API denominator, reviewed coverage or test outcomes. Documentation bytes not reverified.", result.stdout)

    def test_empty_root_has_no_library_attribution_credit(self):
        result = inventory.load_and_summarize(self.root)["library_witness_sources"]
        self.assertEqual(result["readiness"], "missing")
        self.assertEqual(result["direct_witness_selectors"], 0)
        self.assertFalse(result["execution_evidence_imported"])
        self.assertIsNone(result["api_denominator"])

    def test_malformed_present_library_inventory_not_hidden_by_missing_companions(self):
        path = self.root / "data/compiler-library-project-inventory.json"
        for raw in ("null", "[]", "{}", "{", '{"schema_version":1,"schema_version":1}'):
            path.write_text(raw)
            with self.subTest(raw=raw):
                with self.assertRaises(inventory.InvalidInventory):
                    inventory.library_witness_readiness(self.root)
                result = subprocess.run([sys.executable, inventory.__file__, "--root", str(self.root)],
                                        capture_output=True, text=True)
                self.assertEqual(result.returncode, 2)
                self.assertEqual(result.stdout, "")
                self.assertIn("error", json.loads(result.stderr))

    def test_changed_fixture_is_not_hidden_by_other_missing_sources(self):
        for name in ("data/compiler-library-project-inventory.json", inventory.LIBRARY_PROVENANCE):
            (self.root / name).write_bytes((self.actual_root / name).read_bytes())
        fixture = self.root / "tests/conformance_generated.rs"
        fixture.parent.mkdir()
        fixture.write_bytes(b"// changed fixture, not a missing-input excuse\n")
        with self.assertRaises(inventory.InvalidInventory):
            inventory.library_witness_readiness(self.root)

    def test_ordinary_library_summary_has_no_execution_or_unrelated_discovery(self):
        import conformance
        import compatibility_scope
        with mock.patch.object(subprocess, "run", side_effect=AssertionError("no execution")), \
                mock.patch.object(conformance, "source_identity", side_effect=AssertionError("no receipt identity")), \
                mock.patch.object(conformance, "discover", side_effect=AssertionError("no whole-suite scan")), \
                mock.patch.object(compatibility_scope, "validate", side_effect=AssertionError("no whole-scope scan")), \
                mock.patch.object(compatibility_scope, "runner_identity", side_effect=AssertionError("no runner")):
            result = inventory.library_witness_readiness(self.actual_root)
        self.assertEqual(result["direct_witness_selectors"], 6)
        self.assertFalse(result["execution_evidence_imported"])


if __name__ == "__main__":
    unittest.main()
