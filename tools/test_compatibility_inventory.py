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
        self.assertEqual(expected['proposed_rule_identities'], 594)
        self.assertEqual(expected['rule_identities_without_proposals'], 2371)
        for name, pages, count in (
                ('loops', ['loop-expr'], 60), ('operators', ['operator-expr'], 112),
                ('closures', ['closure-expr'], 17),
                ('calls', ['call-expr', 'method-call-expr'], 23),
                ('aggregates', ['array-expr', 'tuple-expr'], 38),
                ('fields', ['field-expr'], 8)):
            profile = expected['profiles'][name]
            self.assertEqual(profile['selected_pages'],
                             ['src/expressions/' + page + '.md' for page in pages])
            self.assertEqual(profile['proposed_rule_identities'], count)
            self.assertEqual(profile['review_status'], 'unreviewed')
            for field in ('reviewed_rule_rows', 'rows_with_exact_executable_evidence',
                          'completed_rule_rows'):
                self.assertEqual(profile[field], 0)
            self.assertIsNone(profile['full_rust_completion_percent'])
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




class ObligationBacklogIdentityTests(unittest.TestCase):
    """Authored queue identity is deliberately not execution or review evidence."""

    @staticmethod
    def canonical_digest(value):
        import hashlib
        return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":"),
                                         ensure_ascii=False).encode("utf-8")).hexdigest()

    def need_id(self, obligation_id, description):
        return "need-" + self.canonical_digest({"scope": inventory.CONTRACT,
            "obligation_id": obligation_id, "description": description})

    def synthetic(self):
        value = fixture()
        first = value["sections"][0]["obligations"][0]
        first["existing_requirements"] = ["req-two", "req-one"]
        first["test_needs"] = ["Unicode café λ", "Differential type check"]
        second = copy.deepcopy(first)
        second["id"] = "types-zero"
        second["title"] = "Another authored obligation"
        value["sections"][0]["obligations"].append(second)
        return value

    def assert_no_credit(self, report):
        self.assertEqual(report["schema_version"], 1)
        self.assertEqual(report["scope"], inventory.CONTRACT)
        self.assertEqual(report["readiness"], "unreviewed")
        self.assertIsNone(report["full_rust_completion_percent"])
        self.assertEqual(report["reviewed_obligation_count"], 0)
        self.assertEqual(report["executed_test_need_count"], 0)
        self.assertIs(report["execution_evidence_imported"], False)
        self.assertIs(report["normative_inventory_complete"], False)
        self.assertTrue(report["limitations"])
        for row in report["rows"]:
            self.assertEqual(row["review_status"], "unreviewed")
            self.assertEqual(row["direct_exact_test_links"], [])
            self.assertTrue(all(need["status"] == "unmeasured" for need in row["test_needs"]))

    def test_synthetic_exact_schema_identity_and_conservation(self):
        value = self.synthetic()
        report = inventory.obligation_backlog([value], scope_fixture())
        self.assert_no_credit(report)
        self.assertIsNone(report["obligation_id"])
        self.assertEqual((report["known_obligation_count"], report["selected_obligation_count"],
                          report["known_test_need_count"], report["selected_test_need_count"]),
                         (2, 2, 4, 4))
        self.assertEqual([row["obligation_id"] for row in report["rows"]],
                         ["types-one", "types-zero"])
        for row in report["rows"]:
            authored = next(item for item in value["sections"][0]["obligations"]
                            if item["id"] == row["obligation_id"])
            expected_needs = sorted([{"id": self.need_id(authored["id"], text),
                "description": text, "status": "unmeasured"} for text in authored["test_needs"]],
                key=lambda need: need["id"])
            self.assertEqual(row, {"obligation_id": authored["id"], "title": authored["title"],
                "section_id": "types", "section_title": "Types",
                "source_references": [{"source_id": "reference",
                    "url": "https://doc.rust-lang.org/reference/types.html",
                    "version_note": "Synthetic pinned fixture", "section": "types"}],
                "existing_requirements": ["req-one", "req-two"],
                "review_status": "unreviewed", "direct_exact_test_links": [],
                "test_needs": expected_needs,
                "inventory_content_sha256": self.canonical_digest(value)})
        ids = [need["id"] for row in report["rows"] for need in row["test_needs"]]
        self.assertEqual(len(ids), len(set(ids)), "same prose in distinct obligations is not one need")

    def test_reordering_preserves_authored_rows_but_not_inventory_digest(self):
        value = self.synthetic()
        extra = copy.deepcopy(value["sections"][0])
        extra["id"] = "another-section"
        for item in extra["obligations"]:
            item["id"] += "-other"
        value["sections"].append(extra)
        original = inventory.obligation_backlog([value], scope_fixture())
        reordered = copy.deepcopy(value)
        reordered["sections"].reverse()
        for section in reordered["sections"]:
            section["obligations"].reverse()
            for item in section["obligations"]:
                item["test_needs"].reverse()
                item["existing_requirements"].reverse()
        changed = inventory.obligation_backlog([reordered], scope_fixture())
        self.assertNotEqual(self.canonical_digest(value), self.canonical_digest(reordered))
        for report in (original, changed):
            for row in report["rows"]:
                expected = value if report is original else reordered
                self.assertEqual(row.pop("inventory_content_sha256"), self.canonical_digest(expected))
        self.assertEqual(original, changed)
        left, right = fixture(), fixture()
        right["sections"][0]["obligations"][0]["id"] = "different-inventory"
        self.assertEqual(inventory.obligation_backlog([left, right], scope_fixture()),
                         inventory.obligation_backlog([right, left], scope_fixture()))

    def test_identity_changes_only_with_authored_need_inputs(self):
        base = fixture()
        def only_need(value):
            return inventory.obligation_backlog([value], scope_fixture())["rows"][0]["test_needs"][0]["id"]
        identity = only_need(base)
        self.assertEqual(identity, self.need_id("types-one", "Differential type check"))
        for field, replacement in (("id", "types-renamed"), ("test_needs", ["Changed description"])):
            changed = copy.deepcopy(base)
            changed["sections"][0]["obligations"][0][field] = replacement
            self.assertNotEqual(only_need(changed), identity)
        changed = copy.deepcopy(base)
        changed["sections"][0]["obligations"][0]["title"] = "Retitled"
        self.assertEqual(only_need(changed), identity)
        changed["scope"]["edition"] = "2018"
        with self.assertRaises(inventory.InvalidInventory):
            only_need(changed)
        invalid_scope = scope_fixture()
        invalid_scope["scope"]["target"] = "aarch64-unknown-linux-gnu"
        with self.assertRaises(inventory.InvalidInventory):
            inventory.obligation_backlog([base], invalid_scope)

    def test_caller_and_output_mutation_cannot_alias_inputs(self):
        values, scope = [self.synthetic()], scope_fixture()
        before = copy.deepcopy((values, scope))
        report = inventory.obligation_backlog(values, scope)
        pristine = copy.deepcopy(report)
        self.assertEqual((values, scope), before)
        report["scope"]["target"] = "modified-output"
        row = report["rows"][0]
        row["existing_requirements"].append("modified-output")
        row["source_references"][0]["url"] = "modified-output"
        row["test_needs"][0]["description"] = "modified-output"
        self.assertEqual((values, scope), before)
        self.assertEqual(inventory.obligation_backlog(values, scope), pristine)

    def test_validation_of_all_inputs_precedes_exact_filter(self):
        valid = fixture()
        bad_cases = []
        for path in ((), ("sources", 0), ("sections", 0),
                     ("sections", 0, "source_refs", 0), ("sections", 0, "obligations", 0)):
            malformed = copy.deepcopy(valid)
            current = malformed
            for component in path:
                current = current[component]
            current["unexpected"] = "must not be ignored"
            bad_cases.append(malformed)
        duplicate_need = copy.deepcopy(valid)
        duplicate_need["sections"][0]["obligations"][0]["test_needs"] *= 2
        bad_cases.append(duplicate_need)
        unknown_requirement = copy.deepcopy(valid)
        unknown_requirement["sections"][0]["obligations"][0]["existing_requirements"] = ["absent"]
        bad_cases.append(unknown_requirement)
        for malformed in bad_cases:
            malformed["sections"][0]["obligations"][0]["id"] = "not-selected"
            with self.subTest(malformed=malformed):
                with self.assertRaises(inventory.InvalidInventory):
                    inventory.obligation_backlog([valid, malformed], scope_fixture(), obligation_id="types-one")
        for values in ([], [valid, copy.deepcopy(valid)], [valid, None]):
            with self.subTest(values=values):
                with self.assertRaises(inventory.InvalidInventory):
                    inventory.obligation_backlog(values, scope_fixture(), obligation_id="types-one")
        for query in ("", "unknown", "types", " types-one", 1, False, []):
            with self.subTest(query=query):
                with self.assertRaises(inventory.InvalidInventory):
                    inventory.obligation_backlog([valid], scope_fixture(), obligation_id=query)

    def test_actual_queue_conserves_257_obligations_938_needs_and_queries(self):
        root = Path(__file__).resolve().parents[1]
        values = [inventory.load(root / "data" / name) for name in
                  ("compiler-language-inventory.json", "compiler-library-project-inventory.json")]
        scope = inventory.load(root / "data/compiler-compatibility-scope.json")
        report = inventory.obligation_backlog(values, scope)
        self.assert_no_credit(report)
        self.assertEqual((report["known_obligation_count"], report["selected_obligation_count"],
                          report["known_test_need_count"], report["selected_test_need_count"]),
                         (257, 257, 938, 938))
        self.assertEqual(len(report["rows"]), 257)
        self.assertEqual(sum(len(row["test_needs"]) for row in report["rows"]), 938)
        for obligation_id in ("obligation-core-option", "obligation-alloc-vec-string", "obligation-core-iterators"):
            selected = inventory.obligation_backlog(values, scope, obligation_id=obligation_id)
            self.assert_no_credit(selected)
            self.assertEqual(selected["obligation_id"], obligation_id)
            self.assertEqual(selected["known_obligation_count"], 257)
            self.assertEqual(selected["known_test_need_count"], 938)
            self.assertEqual(selected["selected_obligation_count"], 1)
            self.assertEqual(selected["rows"], [row for row in report["rows"]
                                               if row["obligation_id"] == obligation_id])
            self.assertEqual(selected["selected_test_need_count"], len(selected["rows"][0]["test_needs"]))

    def test_cli_backlog_query_and_custom_inventory_validation(self):
        root = Path(__file__).resolve().parents[1]
        def cli(*options):
            return subprocess.run([sys.executable, inventory.__file__, "--root", str(root),
                                   "--contract", str(root / "data/compiler-compatibility-contract.json"), *options],
                                  capture_output=True, text=True, check=False)
        full = cli("--obligation-backlog")
        self.assertEqual(full.returncode, 0, full.stderr)
        report = json.loads(full.stdout)
        self.assert_no_credit(report)
        self.assertEqual(report["selected_obligation_count"], 257)
        for flags in (("--obligation", "obligation-core-option"),
                      ("--obligation-backlog", "--obligation", "obligation-core-option")):
            result = cli(*flags)
            self.assertEqual(result.returncode, 0, result.stderr)
            query = json.loads(result.stdout)
            self.assert_no_credit(query)
            self.assertEqual(query["selected_obligation_count"], 1)
            self.assertEqual(query["rows"][0]["obligation_id"], "obligation-core-option")
        default = cli()
        self.assertEqual(default.returncode, 0, default.stderr)
        summary = json.loads(default.stdout)
        self.assertEqual(summary["counts"]["obligations"], 257)
        self.assertEqual(summary["counts"]["test_needs"], 938)
        self.assertEqual(summary["counts"]["obligations_with_executable_evidence"], 0)
        self.assertIn("obligation_queue", summary)
        with tempfile.TemporaryDirectory() as directory:
            custom = Path(directory) / "inventory.json"
            custom.write_text(json.dumps(fixture()), encoding="utf-8")
            custom_scope = Path(directory) / "scope.json"
            custom_scope.write_text(json.dumps(scope_fixture()), encoding="utf-8")
            result = cli("--obligation-backlog", "--inventory", str(custom),
                         "--scope", str(custom_scope), "--obligation", "types-one")
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(json.loads(result.stdout)["selected_obligation_count"], 1)
            malformed = Path(directory) / "malformed.json"
            malformed.write_text('{"unexpected":true}', encoding="utf-8")
            for options in (("--inventory", str(custom), "--inventory", str(malformed), "--obligation", "types-one"),
                            ("--inventory", str(custom), "--obligation", "not-an-obligation"),
                            ("--inventory", str(custom), "--obligation", "")):
                failed = cli("--scope", str(custom_scope), *options)
                self.assertEqual(failed.returncode, 2, failed.stderr)
                self.assertEqual(failed.stdout, "")
                self.assertIn("error", json.loads(failed.stderr))


    def test_no_authored_needs_is_not_completion_and_missing_root_is_unknown(self):
        value = fixture()
        value['sections'][0]['obligations'][0]['test_needs'] = []
        queue = inventory.summarize([value], scope_fixture())['obligation_queue']
        self.assertEqual(queue['obligations_without_authored_test_needs'], 1)
        self.assertEqual(queue['authored_test_needs'], 0)
        self.assertEqual(queue['reviewed_obligations'], 0)
        report = inventory.obligation_backlog([value], scope_fixture())
        self.assert_no_credit(report)
        self.assertEqual(report['rows'][0]['test_needs'], [])
        self.assertEqual(report['known_obligation_count'], 1)
        with tempfile.TemporaryDirectory() as directory:
            missing = inventory.load_and_summarize(Path(directory))['obligation_queue']
        self.assertEqual(missing['readiness'], 'missing')
        self.assertIsNone(missing['known_obligations'])
        self.assertIsNone(missing['authored_test_needs'])
        self.assertIs(missing['normative_inventory_complete'], False)

    def test_query_does_not_import_witness_or_execution_evidence(self):
        forbidden = AssertionError('source/evidence readers forbidden for authored queue')
        with mock.patch.object(inventory, 'library_witness_readiness', side_effect=forbidden), \
                mock.patch.object(inventory, 'partial_witness_source_readiness', side_effect=forbidden), \
                mock.patch.object(inventory, 'reference_readiness', side_effect=forbidden), \
                mock.patch('subprocess.run', side_effect=forbidden), \
                mock.patch.object(sys, 'stdout', io.StringIO()) as output:
            self.assertEqual(inventory.main(['--obligation', 'obligation-core-option']), 0)
            report = json.loads(output.getvalue())
        self.assert_no_credit(report)
        self.assertEqual(report['selected_obligation_count'], 1)

    def test_cli_incompatible_provenance_flags_reject_before_any_io(self):
        for flag in ("--reference-rules", "--source-archive", "--submodule-proof",
                     "--library-surface-provenance"):
            for action in (("--obligation-backlog",), ("--obligation", "types-one")):
                with self.subTest(flag=flag, action=action), \
                        mock.patch.object(inventory, "load", side_effect=AssertionError("no JSON IO")), \
                        mock.patch.object(inventory, "read_verifier", side_effect=AssertionError("no verifier IO")), \
                        mock.patch.object(sys, "stderr", io.StringIO()) as stderr:
                    with self.assertRaises(SystemExit) as raised:
                        inventory.main([*action, flag, "/missing/provenance", "--inventory", "/missing/inventory",
                                        "--scope", "/missing/scope", "--contract", "/missing/contract",
                                        "--oracle-verifier", "/missing/verifier"])
                    self.assertEqual(raised.exception.code, 2)
                    self.assertIn("cannot override", stderr.getvalue())


class ObligationWitnessBacklogTests(unittest.TestCase):
    """Partial source attribution must never turn authored needs into credit."""

    @classmethod
    def setUpClass(cls):
        cls.root = Path(inventory.__file__).resolve().parents[1]
        cls.candidates = [inventory.load(cls.root / 'data' / name) for name in
                          ('compiler-language-inventory.json', 'compiler-library-project-inventory.json')]
        cls.scope = inventory.load(cls.root / 'data/compiler-compatibility-scope.json')
        cls.families = [
            ('reference-rule', inventory.partial_witness_source_readiness(cls.root)),
            ('library-documentation', inventory.library_witness_readiness(cls.root)),
        ]
        cls.base = inventory.obligation_backlog(cls.candidates, cls.scope)
        cls.links = {row['obligation_id']: [] for row in cls.base['rows']}
        for kind, family in cls.families:
            for link in family['links']:
                for ident in link['candidate_obligation_ids']:
                    cls.links[ident].append(dict(link, kind=kind))
        for links in cls.links.values():
            links.sort(key=lambda link: (link['kind'], link['suite'], link['test']))
        cls.language_id = next(ident for ident, links in cls.links.items()
                               if any(link['kind'] == 'reference-rule' for link in links))
        cls.unlinked_id = next(ident for ident, links in cls.links.items() if not links)

    def assert_no_credit(self, report):
        ObligationBacklogIdentityTests.assert_no_credit(self, report)
        for row in report['rows']:
            self.assertEqual(row['direct_exact_test_links'], [])
            self.assertEqual(row['review_status'], 'unreviewed')
            self.assertTrue(all(need['status'] == 'unmeasured' for need in row['test_needs']))

    def assert_attribution(self, report):
        self.assert_no_credit(report)
        selected = report['rows']
        self.assertEqual(report['partial_witness_sources'], {
            'readiness': 'review-required', 'known_witness_selectors': 18,
            'selected_witness_selectors': len({(link['suite'], link['test'])
                for row in selected for link in self.links[row['obligation_id']]}),
            'known_obligations_with_partial_witnesses': 14,
            'selected_obligations_with_partial_witnesses': sum(
                bool(self.links[row['obligation_id']]) for row in selected),
            'source_file_byte_verification': 'reverified_in_this_run',
            'source_block_byte_verification': 'not_reverified_in_this_run',
            'documentation_page_byte_verification': 'not_reverified_in_this_run',
            'execution_evidence_imported': False})
        authored = {row['obligation_id']: row for row in self.base['rows']}
        for row in selected:
            self.assertEqual(row, dict(authored[row['obligation_id']],
                                       partial_witness_links=self.links[row['obligation_id']]))
        self.assertEqual(report['known_obligation_count'], 257)
        self.assertEqual(report['known_test_need_count'], 938)
        self.assertEqual(report['selected_obligation_count'], len(selected))
        self.assertEqual(report['selected_test_need_count'], sum(len(row['test_needs']) for row in selected))

    def cli(self, *flags):
        return subprocess.run([sys.executable, inventory.__file__, '--root', str(self.root), *flags],
                              capture_output=True, text=True, check=False)

    def copied_root(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        root = Path(directory.name)
        paths = {'data/compiler-compatibility-contract.json', 'data/compiler-compatibility-scope.json',
                 'data/compiler-language-inventory.json', 'data/compiler-library-project-inventory.json',
                 inventory.REFERENCE_RULES, inventory.LIBRARY_PROVENANCE}
        paths.update(row['path'] for _, family in self.families for row in family['source_files'])
        for name in sorted(paths):
            target = root / name
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes((self.root / name).read_bytes())
        return root

    def test_actual_full_and_exact_queries_conserve_attribution_without_credit(self):
        report = inventory.obligation_witness_backlog(self.root)
        self.assert_attribution(report)
        self.assertEqual(len(report['rows']), 257)
        self.assertEqual(sum(bool(links) for links in self.links.values()), 14)
        self.assertEqual(report['partial_witness_sources']['selected_witness_selectors'], 18)
        for ident in ('obligation-core-option', self.language_id, self.unlinked_id):
            with self.subTest(ident=ident):
                selected = inventory.obligation_witness_backlog(self.root, obligation_id=ident)
                self.assert_attribution(selected)
                self.assertEqual(selected['obligation_id'], ident)
                self.assertEqual(selected['rows'], [row for row in report['rows'] if row['obligation_id'] == ident])
        option = inventory.obligation_witness_backlog(self.root, obligation_id='obligation-core-option')
        self.assertEqual(len(option['rows'][0]['partial_witness_links']), 2)
        self.assertTrue(all(link['kind'] == 'library-documentation'
                            for link in option['rows'][0]['partial_witness_links']))
        unlinked = inventory.obligation_witness_backlog(self.root, obligation_id=self.unlinked_id)
        self.assertEqual(unlinked['rows'][0]['partial_witness_links'], [])
        self.assertEqual(unlinked['partial_witness_sources']['selected_witness_selectors'], 0)

    def test_cli_extended_and_default_query_bytes_remain_separate(self):
        for ident in (None, 'obligation-core-option', self.language_id, self.unlinked_id):
            flags = ['--obligation-backlog'] if ident is None else ['--obligation', ident]
            extended = self.cli(*flags, '--with-partial-witnesses')
            self.assertEqual(extended.returncode, 0, extended.stderr)
            self.assert_attribution(json.loads(extended.stdout))
            expected = inventory.obligation_backlog(self.candidates, self.scope, obligation_id=ident)
            expected['contract_id'] = inventory.load(self.root / 'data/compiler-compatibility-contract.json')['contract_id']
            before = self.cli(*flags)
            self.assertEqual(before.returncode, 0, before.stderr)
            self.assertEqual(before.stdout, json.dumps(expected, indent=2, sort_keys=True) + '\n')
            self.assertNotIn('partial_witness_sources', expected)
            self.assertTrue(all('partial_witness_links' not in row for row in expected['rows']))

    def test_unknown_empty_and_nonexact_queries_have_json_errors_without_stdout(self):
        for query in ('', 'not-an-obligation', ' obligation-core-option', 'obligation-core', False, 7, []):
            with self.subTest(query=query), self.assertRaises(inventory.InvalidInventory):
                inventory.obligation_witness_backlog(self.root, obligation_id=query)
        for query in ('', 'not-an-obligation', ' obligation-core-option', 'obligation-core'):
            result = self.cli('--obligation', query, '--with-partial-witnesses')
            self.assertEqual(result.returncode, 2, result.stderr)
            self.assertEqual(result.stdout, '')
            self.assertTrue(json.loads(result.stderr)['error'])

    def test_actual_root_does_not_read_external_bytes_receipts_or_runner(self):
        import conformance
        import compatibility_scope
        import library_witness_sources
        forbidden = AssertionError('no archives, HTML, execution evidence or runner')
        with mock.patch.object(inventory.tarfile, 'open', side_effect=forbidden), \
                mock.patch.object(inventory, 'reference_readiness', side_effect=forbidden), \
                mock.patch.object(inventory, 'library_surface_readiness', side_effect=forbidden), \
                mock.patch.object(library_witness_sources, 'verify_html', side_effect=forbidden), \
                mock.patch.object(conformance, 'source_identity', side_effect=forbidden), \
                mock.patch.object(conformance, 'discover', side_effect=forbidden), \
                mock.patch.object(compatibility_scope, 'runner_identity', side_effect=forbidden), \
                mock.patch.object(subprocess, 'run', side_effect=forbidden):
            self.assert_attribution(inventory.obligation_witness_backlog(self.root))

    def test_real_missing_and_malformed_inputs_cannot_hide_behind_option_filter(self):
        root = self.copied_root()
        self.assert_attribution(inventory.obligation_witness_backlog(root))
        paths = sorted(str(path.relative_to(root)) for path in root.rglob('*') if path.is_file())
        for name in paths:
            path = root / name
            original = path.read_bytes()
            for mutation in ('missing', 'malformed'):
                try:
                    if mutation == 'missing':
                        path.unlink()
                    else:
                        path.write_bytes(b'{ malformed input\n')
                    with self.subTest(name=name, mutation=mutation), self.assertRaises(inventory.InvalidInventory):
                        inventory.obligation_witness_backlog(root, obligation_id='obligation-core-option')
                finally:
                    path.write_bytes(original)

    def test_real_source_and_metadata_byte_drift_is_rejected_before_filtering(self):
        root = self.copied_root()
        paths = {row['path'] for _, family in self.families for row in family['source_files']}
        paths.update((inventory.REFERENCE_RULES, 'data/compiler-language-inventory.json'))
        for name in sorted(paths):
            path = root / name
            original = path.read_bytes()
            try:
                path.write_bytes(original + b'\n')
                with self.subTest(name=name), self.assertRaises(inventory.InvalidInventory):
                    inventory.obligation_witness_backlog(root, obligation_id='obligation-core-option')
            finally:
                path.write_bytes(original)

    def test_both_readiness_families_are_required_and_foreign_links_checked_globally(self):
        for kind, function in (('reference-rule', 'partial_witness_source_readiness'),
                               ('library-documentation', 'library_witness_readiness')):
            original = next(family for family_kind, family in self.families if family_kind == kind)
            for mutation in ('readiness', 'foreign-link', 'relationship', 'review_status',
                             'source-verification', 'execution-credit', 'empty-obligations'):
                altered = copy.deepcopy(original)
                if mutation == 'readiness':
                    altered['readiness'] = 'missing'
                elif mutation == 'foreign-link':
                    altered['links'][0]['candidate_obligation_ids'].append('foreign-obligation')
                elif mutation == 'source-verification':
                    altered['source_file_byte_verification'] = 'not_reverified_in_this_run'
                elif mutation == 'execution-credit':
                    altered['execution_evidence_imported'] = True
                elif mutation == 'empty-obligations':
                    altered['links'][0]['candidate_obligation_ids'] = []
                elif mutation == 'relationship':
                    altered['links'][0]['relationship'] = 'complete'
                else:
                    altered['links'][0]['review_status'] = 'reviewed'
                with self.subTest(kind=kind, mutation=mutation), \
                        mock.patch.object(inventory, function, return_value=altered), \
                        self.assertRaises(inventory.InvalidInventory):
                    inventory.obligation_witness_backlog(self.root, obligation_id=self.unlinked_id)

    def test_cli_incompatible_combinations_reject_before_any_filesystem_io(self):
        overrides = ('--inventory', '--scope', '--contract', '--oracle-verifier', '--reference-rules',
                     '--source-archive', '--submodule-proof', '--library-surface-provenance')
        combinations = [['--with-partial-witnesses']]
        for action in (['--obligation-backlog'], ['--obligation', 'obligation-core-option']):
            combinations.extend([*action, '--with-partial-witnesses', flag, '/missing'] for flag in overrides)
            combinations.extend([*action, '--with-partial-witnesses', *rule] for rule in
                                (['--rule-backlog'], ['--rule-page', 'src/types.md'],
                                 ['--rule', 'types'], ['--unproposed']))
        forbidden = AssertionError('incompatible flags must reject before filesystem access')
        for flags in combinations:
            with self.subTest(flags=flags), \
                    mock.patch.object(Path, 'open', side_effect=forbidden), \
                    mock.patch.object(Path, 'stat', side_effect=forbidden), \
                    mock.patch.object(inventory, 'load', side_effect=forbidden), \
                    mock.patch.object(inventory, 'read_verifier', side_effect=forbidden), \
                    mock.patch.object(inventory, 'obligation_witness_backlog', side_effect=forbidden), \
                    mock.patch.object(sys, 'stdout', io.StringIO()) as stdout, \
                    mock.patch.object(sys, 'stderr', io.StringIO()) as stderr:
                with self.assertRaises(SystemExit) as raised:
                    inventory.main(['--root', '/missing/root', *flags])
                self.assertEqual(raised.exception.code, 2)
                self.assertEqual(stdout.getvalue(), '')
                self.assertIn('--with-partial-witnesses', stderr.getvalue())


class ObligationWitnessObservationAuthenticationTests(unittest.TestCase):
    """Real receipt parser controls, with no compiler or receipt command execution."""

    @classmethod
    def setUpClass(cls):
        import conformance as c
        import compatibility_scope as s
        cls.c, cls.s = c, s
        cls.root = Path(inventory.__file__).resolve().parents[1]
        cls.source = inventory.obligation_witness_backlog(cls.root)
        cls.scope = inventory.load(cls.root / 'data/compiler-compatibility-scope.json')
        cls.manifest = s.case_manifest(cls.scope, inventory.load(cls.root / c.MANIFEST), c)
        cls.selectors = sorted({(link['suite'], link['test']) for row in cls.source['rows']
                                for link in row['partial_witness_links']})
        cls.case = dict(zip(('suite', 'test'), cls.selectors[0]))
        cls.ident = next(row['obligation_id'] for row in cls.source['rows']
                         if any((link['suite'], link['test']) == cls.selectors[0]
                                for link in row['partial_witness_links']))
        cls.unlinked = next(row['obligation_id'] for row in cls.source['rows']
                            if not row['partial_witness_links'])
        cls.identity = {'revision': 'synthetic-current', 'inputs_sha256': 'abc', 'input_files': 1}
        cls.runner = {'executable': '/stable/justrust', 'sha256': 'client', 'environment': {}}
        cls.statuses = ('passed', 'failed', 'timeout', 'infrastructure', 'unverified',
                        'untested', 'missing', 'stale', 'conflicting')

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='witness-observation-')
        self.addCleanup(self.temp.cleanup)
        self.receipts = Path(self.temp.name)

    def report(self, name='run', code=0, text='justrust: ok: 1 test passed\n', timed_out=False,
               cases=None):
        c, s = self.c, self.s
        directory = self.receipts / name
        directory.mkdir()
        selected = sorted(s.case_id(case) for case in (cases or [self.case]))
        results = []
        for index, entry in enumerate(c.plan(self.manifest, 'full', selected, 'target/test')):
            log = f'{index}.log'
            payload = text.encode()
            (directory / log).write_bytes(payload)
            results.append(dict(entry, returncode=code, timed_out=timed_out,
                                outcome=c.outcome(code, text, 'case', timed_out),
                                tests_passed=c.test_count(text), log=log, log_sha256=c.digest(payload)))
        report = dict(schema_version=1, profile='full', selected=selected, target_dir='target/test',
                      manifest_sha256=c.digest(c.encoded(self.manifest)),
                      scope_sha256=c.digest(c.encoded(self.scope)), source_before=self.identity,
                      source_after=self.identity, runner_before=self.runner, runner_after=self.runner,
                      integration_suites=c.integration_suites(self.root), results=results)
        self.save(directory, report)
        return directory, report

    def save(self, directory, report):
        (directory / 'report.json').write_bytes(self.c.encoded(report))

    def observe(self, reports=(), ident=None):
        forbidden = AssertionError('assessor must not execute receipt commands')
        with mock.patch.object(self.c, 'source_identity', return_value=self.identity), \
                mock.patch.object(self.s, 'runner_identity', return_value=self.runner), \
                mock.patch.object(self.c, 'execute', side_effect=forbidden), \
                mock.patch.object(subprocess, 'run', side_effect=forbidden):
            return inventory.observe_obligation_witnesses(self.root, reports, obligation_id=ident)

    def observations(self, report):
        return [obs for row in report['rows'] for obs in row['partial_witness_observations']]

    def assert_conservation(self, report, imported):
        self.assertIs(report['execution_evidence_imported'], imported)
        self.assertFalse(report['partial_witness_sources']['execution_evidence_imported'])
        self.assertEqual(report['executed_test_need_count'], 0)
        self.assertEqual(report['reviewed_obligation_count'], 0)
        self.assertIsNone(report['full_rust_completion_percent'])
        for row in report['rows']:
            self.assertEqual(row['direct_exact_test_links'], [])
            self.assertEqual(row['review_status'], 'unreviewed')
            self.assertTrue(all(need['status'] == 'unmeasured' for need in row['test_needs']))
            self.assertEqual([(obs['kind'], obs['suite'], obs['test'])
                              for obs in row['partial_witness_observations']],
                             [(link['kind'], link['suite'], link['test'])
                              for link in row['partial_witness_links']])
        summary = report['witness_observations']
        self.assertEqual(set(summary['status_counts']), set(self.statuses))
        self.assertEqual(summary['known_witness_selectors'], 18)
        unique = {(obs['suite'], obs['test']): obs['status'] for obs in self.observations(report)}
        self.assertEqual(summary['selected_witness_selectors'], len(unique))
        self.assertEqual(summary['distinct_linked_selectors_observed'],
                         sum(status != 'untested' for status in unique.values()))
        self.assertEqual(summary['status_counts'],
                         {status: list(unique.values()).count(status) for status in self.statuses})

    def test_no_receipts_never_reads_current_identities_or_executes(self):
        forbidden = AssertionError('source-only observation must not inspect execution identity')
        with mock.patch.object(self.c, 'source_identity', side_effect=forbidden), \
                mock.patch.object(self.s, 'runner_identity', side_effect=forbidden), \
                mock.patch.object(self.c, 'execute', side_effect=forbidden), \
                mock.patch.object(subprocess, 'run', side_effect=forbidden):
            report = inventory.observe_obligation_witnesses(self.root)
        self.assert_conservation(report, False)
        summary = report['witness_observations']
        self.assertIsNone(summary['source_identity'])
        self.assertIsNone(summary['runner_identity'])
        self.assertEqual(summary['evidence_directories'], 0)
        self.assertEqual(summary['status_counts']['untested'], 18)
        self.assertTrue(all(obs['evidence'] == [] for obs in self.observations(report)))

    def test_authentic_status_controls_and_failure_annotation(self):
        controls = [('passed', 0, 'justrust: ok: 1 test passed\n', False),
                    ('failed', 101, 'unsupported valid source\n', False),
                    ('timeout', None, '', True), ('infrastructure', None, '', False),
                    ('unverified', 0, 'justrust: ok: 0 tests passed\n', False)]
        for status, code, text, timed_out in controls:
            directory, _ = self.report(status, code, text, timed_out)
            result = self.observe([directory])
            with self.subTest(status=status):
                self.assert_conservation(result, True)
                self.assertEqual(result['witness_observations']['status_counts'][status], 1)
                observed = [obs for obs in self.observations(result)
                            if (obs['suite'], obs['test']) == self.selectors[0]]
                self.assertTrue(observed)
                for obs in observed:
                    self.assertEqual(obs['status'], status)
                    self.assertEqual(len(obs['evidence']), 1)
                    if status == 'failed':
                        self.assertEqual(obs['failure_annotation'], self.c.failure_annotation(text))
                    else:
                        self.assertNotIn('failure_annotation', obs)

    def test_failure_annotations_are_rederived_and_aggregate_ordinary_failures(self):
        first, receipt = self.report('marked', 101,
                                     '[justrust-conformance-failure:v1] valid-source-rejected\n')
        receipt['results'][0]['failure_annotation'] = 'invalid-source-accepted'
        receipt['failed_case_annotations'] = {'invalid-source-accepted': 999}
        self.save(first, receipt)
        second, _ = self.report('other-marked', 101,
                                '[justrust-conformance-failure:v1] invalid-source-accepted\n')
        for reports, expected in (([first], 'valid-source-rejected'),
                                  ([first, second], 'unannotated')):
            result = self.observe(reports, self.ident)
            self.assert_conservation(result, True)
            for obs in self.observations(result):
                if (obs['suite'], obs['test']) == self.selectors[0]:
                    self.assertEqual(obs['status'], 'failed')
                    self.assertEqual(obs['failure_annotation'], expected)
                    self.assertEqual(obs['evidence'][0]['failure_annotation'], 'valid-source-rejected')

    def test_cli_repeated_evidence_uses_the_authenticated_observer(self):
        first, _ = self.report('cli-passed')
        second, _ = self.report('cli-failed', 101, 'failure')
        args = ['--root', str(self.root), '--obligation', self.ident, '--with-partial-witnesses',
                '--observe-partial-witnesses', '--evidence', str(first), '--evidence', str(second)]
        with mock.patch.object(self.c, 'source_identity', return_value=self.identity), \
                mock.patch.object(self.s, 'runner_identity', return_value=self.runner), \
                mock.patch.object(self.c, 'execute', side_effect=AssertionError('no execution')), \
                mock.patch.object(subprocess, 'run', side_effect=AssertionError('no commands')), \
                mock.patch.object(sys, 'stdout', io.StringIO()) as stdout, \
                mock.patch.object(sys, 'stderr', io.StringIO()) as stderr:
            result = inventory.main(args)
        self.assertIn(result, (0, None))
        self.assertEqual(stderr.getvalue(), '')
        report = json.loads(stdout.getvalue())
        self.assert_conservation(report, True)
        self.assertEqual(report['witness_observations']['evidence_directories'], 2)
        self.assertEqual(report['witness_observations']['status_counts']['conflicting'], 1)

    def test_stale_conflict_and_canonical_directory_deduplication(self):
        stale, receipt = self.report('stale')
        receipt['source_after'] = {'revision': 'old'}
        self.save(stale, receipt)
        passed, _ = self.report('passed')
        failed, _ = self.report('failed', 101, 'failure')
        alias = self.receipts / 'alias'
        alias.symlink_to(passed, target_is_directory=True)
        for reports, expected in (([stale], 'stale'), ([stale, passed], 'passed'),
                                  ([passed, failed], 'conflicting'), ([failed, passed], 'conflicting')):
            with self.subTest(expected=expected, reports=reports):
                result = self.observe(reports, self.ident)
                self.assert_conservation(result, True)
                self.assertEqual(result['witness_observations']['status_counts'][expected], 1)
        result = self.observe([passed, passed / '.'], self.ident)
        with self.assertRaises(ValueError):
            self.observe([alias], self.ident)
        self.assert_conservation(result, True)
        self.assertEqual(result['witness_observations']['evidence_directories'], 1)
        for obs in self.observations(result):
            if obs['status'] == 'passed':
                self.assertEqual(len(obs['evidence']), 1)

    def test_all_receipt_rows_authenticated_even_for_unlinked_filter(self):
        extra = next(dict(suite=gate['suite'], test=gate['test']) for gate in self.manifest['gates']
                     if (gate['suite'], gate['test']) not in self.selectors)
        mutations = {
            'invocation': lambda receipt: receipt['results'][-1]['command'].remove('--exact'),
            'outcome': lambda receipt: receipt['results'][-1].update(outcome='failed'),
            'count': lambda receipt: receipt['results'][-1].update(tests_passed=99),
            'missing': lambda receipt: receipt['results'].pop(),
            'duplicate': lambda receipt: receipt['results'].append(receipt['results'][-1]),
            'manifest': lambda receipt: receipt.update(manifest_sha256='changed'),
            'scope': lambda receipt: receipt.update(scope_sha256='changed'),
            'profile': lambda receipt: receipt.update(profile='smoke'),
            'selection': lambda receipt: receipt.update(selected=[]),
            'unfinalized': lambda receipt: receipt.update(runner_after=None),
        }
        for name, mutate in mutations.items():
            directory, receipt = self.report(name, cases=[self.case, extra])
            mutate(receipt)
            self.save(directory, receipt)
            with self.subTest(name=name), self.assertRaises(self.c.InvalidEvidence):
                self.observe([directory], self.unlinked)
        directory, receipt = self.report('tampered-log', cases=[extra])
        (directory / receipt['results'][0]['log']).write_text('tampered')
        with self.assertRaises(self.c.InvalidEvidence):
            self.observe([directory], self.unlinked)
        directory, receipt = self.report('external-log', cases=[extra])
        log = directory / receipt['results'][0]['log']
        external = self.receipts / 'outside.log'
        external.write_bytes(log.read_bytes())
        log.unlink()
        log.symlink_to(external)
        with self.assertRaises(self.c.InvalidEvidence):
            self.observe([directory], self.unlinked)

    def test_source_validation_precedes_filtering_and_receipts(self):
        original = inventory.obligation_witness_backlog
        with mock.patch.object(inventory, 'obligation_witness_backlog', wraps=original) as source:
            self.observe(ident=self.unlinked)
        self.assertEqual(source.call_args, mock.call(self.root))
        for ident in ('', 'unknown', ' obligation-core-option', False, 7, []):
            with self.subTest(ident=ident), self.assertRaises(inventory.InvalidInventory):
                self.observe(ident=ident)

    def test_cli_incompatible_flags_rejected_before_io(self):
        observe = ['--observe-partial-witnesses']
        action = ['--obligation-backlog', '--with-partial-witnesses', *observe]
        combinations = [observe, [*observe, '--with-partial-witnesses'],
                        ['--obligation-backlog', *observe], ['--evidence', '/missing'],
                        ['--obligation-backlog', '--with-partial-witnesses', '--evidence', '/missing']]
        for flag in ('--inventory', '--scope', '--contract', '--oracle-verifier', '--reference-rules',
                     '--source-archive', '--submodule-proof', '--library-surface-provenance'):
            combinations.append([*action, flag, '/missing'])
        for flags in (['--rule-backlog'], ['--rule-page', 'src/types.md'], ['--rule', 'types'],
                      ['--unproposed']):
            combinations.append([*action, *flags])
        forbidden = AssertionError('argument rejection must precede filesystem and runner access')
        for flags in combinations:
            with self.subTest(flags=flags), \
                    mock.patch.object(Path, 'open', side_effect=forbidden), \
                    mock.patch.object(Path, 'stat', side_effect=forbidden), \
                    mock.patch.object(inventory, 'load', side_effect=forbidden), \
                    mock.patch.object(self.c, 'source_identity', side_effect=forbidden), \
                    mock.patch.object(self.s, 'runner_identity', side_effect=forbidden), \
                    mock.patch.object(sys, 'stdout', io.StringIO()) as stdout, \
                    mock.patch.object(sys, 'stderr', io.StringIO()):
                with self.assertRaises(SystemExit) as raised:
                    inventory.main(['--root', '/missing/root', *flags])
                self.assertEqual(raised.exception.code, 2)
                self.assertEqual(stdout.getvalue(), '')


if __name__ == "__main__":
    unittest.main()
