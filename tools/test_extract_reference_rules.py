#!/usr/bin/env python3
"""Archive-backed static extraction tests with isolated temporary fixtures."""
import io
import json
import os
import tarfile
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch
from contextlib import redirect_stdout

from extract_reference_rules import SCOPE, extract, main, parse_markdown, sha256, verify_archive

BASE = Path(__file__).resolve().parent
COMPILER = '48a229ceaefd4985c50990b14116b6d856af0985'
REFERENCE = '86635e30bf861a038dc197d7e16fd09e7e514e7a'


class ParserTests(unittest.TestCase):
    def test_exact_labels_and_original_lines(self):
        rules, headings, _ = parse_markdown('# Heading\nr[foo.bar]\ntext r[not-a-rule]\nr[bad label]\n')
        self.assertEqual(rules, [{'label': 'foo.bar', 'line': 2}])
        self.assertEqual(headings, [{'level': 1, 'title': 'Heading', 'line': 1}])

    def test_fenced_examples_are_not_rules(self):
        rules, headings, _ = parse_markdown('```rust\nr[example]\n# hidden\n```\nr[real]\n')
        self.assertEqual(rules, [{'label': 'real', 'line': 5}])
        self.assertEqual(headings, [])

    def test_longer_tilde_fences(self):
        rules, _, _ = parse_markdown('~~~~\nr[hidden]\n~~~\nr[also-hidden]\n~~~~\nr[shown]\n')
        self.assertEqual([r['label'] for r in rules], ['shown'])

    def test_edition_markers(self):
        _, _, editions = parse_markdown('> [!EDITION-2024]\n> words\n> [!EDITION-2018]\n')
        self.assertEqual(editions, [{'edition': '2024', 'line': 1}, {'edition': '2018', 'line': 3}])


class ExtractTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="reference-rules-")
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name)
        self.root = self.base / 'source'
        (self.root / 'src').mkdir(parents=True)
        (self.root / 'src/SUMMARY.md').write_text('# Index\n- [One](one.md)\n')
        (self.root / 'src/one.md').write_text('# One\nr[one]\n> [!EDITION-2024]\n')
        self.pin = self.base / 'pin.json'
        self.meta = {
            'sha': REFERENCE, 'path': 'src/doc/reference', 'type': 'submodule',
            'submodule_git_url': 'https://github.com/rust-lang/reference.git',
            'url': 'https://api.github.com/repos/rust-lang/rust/contents/src/doc/reference?ref=' + COMPILER,
            'html_url': 'https://github.com/rust-lang/reference/tree/' + REFERENCE,
        }
        self.pin.write_text(json.dumps(self.meta))
        self.inventory = self.base / 'inventory.json'
        self.inv = {'scope': dict(SCOPE), 'sections': [{'id': 'reference-one', 'source_refs': [{'section': 'one.html'}], 'obligations': [{'id': 'reference-one-obligation'}]}]}
        self.inventory.write_text(json.dumps(self.inv))
        self.archive = self.base / 'source.tar'
        self.repack()

    def repack(self, extra=None):
        with tarfile.open(self.archive, 'w') as archive:
            for path in sorted(self.root.rglob('*')):
                archive.add(path, arcname='reference-' + REFERENCE + '/' + path.relative_to(self.root).as_posix(), recursive=False)
            if extra:
                info, data = extra
                archive.addfile(info, io.BytesIO(data) if data is not None else None)
        self.digest = sha256(self.archive.read_bytes())

    def run_extract(self):
        return extract(self.root, self.pin, COMPILER, self.inventory, self.archive, self.digest)

    def run_cli(self, output):
        argv = ['extract_reference_rules.py', '--source-root', str(self.root),
                '--pin', str(self.pin), '--compiler-commit', COMPILER,
                '--inventory', str(self.inventory), '--source-archive', str(self.archive),
                '--archive-sha256', self.digest, '--output', str(output)]
        with patch('sys.argv', argv), redirect_stdout(io.StringIO()):
            main()

    def test_cli_creates_reproducible_new_output(self):
        output = self.base / 'result.json'
        self.run_cli(output)
        self.assertEqual(json.loads(output.read_text()), self.run_extract())

    def test_cli_cannot_overwrite_source_or_prior_evidence(self):
        for output in (self.inventory, self.pin, self.root / 'src/one.md'):
            before = output.read_bytes()
            with self.assertRaises(FileExistsError):
                self.run_cli(output)
            self.assertEqual(output.read_bytes(), before)

    def test_deterministic_verified_bytes_and_no_coverage_upgrade(self):
        result = self.run_extract()
        self.assertEqual(result, self.run_extract())
        self.assertEqual(result['counts']['unique_rule_labels_total'], 1)
        self.assertEqual(result['counts']['rules_with_reviewed_obligation_mapping'], 0)
        self.assertEqual(result['provenance']['archive_verified_local_files'], 2)
        self.assertTrue(result['provenance']['local_source_bytes_match_archive'])
        self.assertEqual(result['provenance']['source_archive_sha256'], self.digest)
        self.assertEqual(result['provenance']['source_archive_url'], 'https://codeload.github.com/rust-lang/reference/tar.gz/' + REFERENCE)
        self.assertEqual(result['provenance']['candidate_inventory_sha256'], sha256(self.inventory.read_bytes()))
        self.assertEqual(result['review_status'], 'unreviewed')

    def test_rejects_floating_compiler_identity(self):
        with self.assertRaisesRegex(ValueError, 'immutable'):
            extract(self.root, self.pin, 'stable', self.inventory, self.archive, self.digest)

    def test_rejects_wrong_official_source(self):
        self.meta['submodule_git_url'] = 'https://github.com/other/reference.git'
        self.pin.write_text(json.dumps(self.meta))
        with self.assertRaisesRegex(ValueError, 'official'):
            self.run_extract()

    def test_rejects_pin_identity_mismatch(self):
        self.meta['url'] += '1'
        self.pin.write_text(json.dumps(self.meta))
        with self.assertRaisesRegex(ValueError, 'compiler commit'):
            self.run_extract()

    def test_rejects_invalid_edition(self):
        self.inv['scope']['edition'] = '2021'
        self.inventory.write_text(json.dumps(self.inv))
        with self.assertRaisesRegex(ValueError, 'scope'):
            self.run_extract()

    def test_rejects_invalid_target(self):
        self.inv['scope']['target'] = 'aarch64-unknown-linux-gnu'
        self.inventory.write_text(json.dumps(self.inv))
        with self.assertRaisesRegex(ValueError, 'scope'):
            self.run_extract()

    def test_rejects_archive_digest_mismatch(self):
        self.digest = '0' * 64
        with self.assertRaisesRegex(ValueError, 'SHA256 mismatch'):
            self.run_extract()

    def test_rejects_local_byte_forgery(self):
        (self.root / 'src/one.md').write_text('r[forged]\n')
        with self.assertRaisesRegex(ValueError, 'bytes differ'):
            self.run_extract()

    def test_rejects_extra_local_file(self):
        (self.root / 'src/extra.md').write_text('r[extra]\n')
        with self.assertRaisesRegex(ValueError, 'file set mismatch'):
            self.run_extract()

    def test_rejects_missing_local_file(self):
        (self.root / 'src/one.md').unlink()
        with self.assertRaisesRegex(ValueError, 'file set mismatch'):
            self.run_extract()

    def test_rejects_symlink_escape(self):
        (self.root / 'src/escape').symlink_to(self.base, target_is_directory=True)
        with self.assertRaisesRegex(ValueError, 'symlink'):
            self.run_extract()

    def test_rejects_archive_symlink(self):
        info = tarfile.TarInfo('reference-' + REFERENCE + '/linked')
        info.type = tarfile.SYMTYPE
        info.linkname = '/outside'
        self.repack((info, None))
        with self.assertRaisesRegex(ValueError, 'symlink or hardlink'):
            self.run_extract()

    def test_rejects_archive_path_escape(self):
        info = tarfile.TarInfo('reference-' + REFERENCE + '/../outside')
        info.size = 1
        self.repack((info, b'x'))
        with self.assertRaisesRegex(ValueError, 'archive path'):
            self.run_extract()

    def test_rejects_duplicate_archive_member(self):
        info = tarfile.TarInfo('reference-' + REFERENCE + '/src/one.md')
        info.size = 1
        self.repack((info, b'x'))
        with self.assertRaisesRegex(ValueError, 'duplicate source archive'):
            self.run_extract()

    def test_rejects_duplicate_labels_including_unlinked(self):
        (self.root / 'src/hidden.md').write_text('r[one]\n')
        self.repack()
        with self.assertRaisesRegex(ValueError, 'duplicate Reference'):
            self.run_extract()

    def test_exposes_unlinked_rule_denominator(self):
        (self.root / 'src/hidden.md').write_text('r[hidden]\n')
        self.repack()
        result = self.run_extract()
        self.assertEqual(result['counts']['linked_page_rule_labels'], 1)
        self.assertEqual(result['counts']['unlinked_page_rule_labels'], 1)
        self.assertEqual(result['counts']['unique_rule_labels_total'], 2)
        self.assertEqual(result['counts']['rules_requiring_mapping_review'], 2)
        self.assertEqual(result['counts']['duplicate_rule_labels'], 0)

    def test_exposes_page_gaps(self):
        (self.root / 'src/SUMMARY.md').write_text('[Two](two.md)\n')
        (self.root / 'src/two.md').write_text('r[two]\n')
        self.repack()
        result = self.run_extract()
        self.assertEqual(result['pinned_pages_without_candidate_section'], ['two.md'])
        self.assertEqual(result['candidate_pages_absent_from_pinned_summary'], ['one.md'])

    def test_rejects_chapter_path_escape(self):
        (self.root / 'src/SUMMARY.md').write_text('[Outside](../outside.md)\n')
        self.repack()
        with self.assertRaisesRegex(ValueError, 'escapes'):
            self.run_extract()


class PinnedSourceTests(unittest.TestCase):
    def inputs(self):
        archive = os.environ.get('JR_REFERENCE_ARCHIVE')
        source = os.environ.get('JUSTRUST_WITNESS_REFERENCE_ROOT')
        if not archive:
            self.skipTest('set JR_REFERENCE_ARCHIVE for actual archive and source verification')
        self.assertTrue(archive and source, 'both pinned archive and source root are required')
        return Path(source), Path(archive)

    def test_actual_archive_reproduces_source_and_all_recorded_rule_metadata(self):
        import compatibility_inventory as inventory
        source, archive = self.inputs()
        expected = '580d2250b29ac221aefc647c5e2c3e9d648ba833b6634e41105de2de1316cd91'
        self.assertEqual(verify_archive(source, archive, expected, REFERENCE), 190)
        result = inventory.validate_reference_rules(BASE.parent, source_archive=archive)
        self.assertEqual(result['source_archive_byte_verification'], 'reproduced-from-provided-archive')
        self.assertEqual(result['compiler_submodule_byte_verification'], 'not_reverified_in_this_run')
        self.assertEqual(result['counts']['unique_rule_labels_total'], 2965)
        self.assertEqual(result['counts']['rules_with_reviewed_obligation_mapping'], 0)
        self.assertIsNone(result['full_rust_completion_percent'])

    def test_ci_acquires_before_tests_and_supplies_archive_to_inventory(self):
        workflow = (BASE.parent / '.github/workflows/compiler-conformance.yml').read_text()
        acquisition = workflow.index('python3 tools/acquire_pinned_reference.py --output-dir')
        regressions = workflow.index("python3 -m unittest discover -s tools -p 'test_*.py'")
        self.assertLess(acquisition, regressions)
        self.assertIn("data['readiness'] != 'input-verified'", workflow)
        self.assertIn("data['verified_files'] != 190", workflow)
        for variable in ('JUSTRUST_WITNESS_REFERENCE_ROOT=', 'JR_REFERENCE_ARCHIVE='):
            self.assertLess(workflow.index(variable), regressions)
        self.assertIn('tools/compatibility_inventory.py --source-archive "$JR_REFERENCE_ARCHIVE"', workflow)
        self.assertIn('not semantic review or compiler proof', workflow)


if __name__ == '__main__':
    unittest.main()
