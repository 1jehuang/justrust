#!/usr/bin/env python3
"""Adversarial, read-only-to-checkout tests for six pinned source links."""
import contextlib
import io
import json
import os
from pathlib import Path
import shutil
import tempfile
import unittest
from unittest.mock import patch

import library_witness_sources as witness

ROOT = Path(os.environ.get('JR_COMPAT_LIBRARY_ROOT', Path(__file__).resolve().parent.parent))


class LibraryWitnessSources(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(
            prefix='library-witness-test-', dir=os.environ.get('JCODE_SCRATCH_DIR'))
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name) / 'repo'
        for name in witness.PINS:
            target = self.root / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / name, target)

    def write(self, name, raw):
        (self.root / name).write_bytes(raw)

    def mutate(self, name):
        self.write(name, (self.root / name).read_bytes() + b'\n')

    def result(self):
        return witness.summarize(self.root)

    def test_actual_six_pins_and_exact_expectations(self):
        result = self.result()
        self.assertEqual(result['readiness'], 'review-required')
        self.assertEqual(result['direct_witness_selectors'], 6)
        self.assertEqual(result['positive_selectors'], 3)
        self.assertEqual(result['semantic_invalid_selectors'], 3)
        self.assertEqual(result['direct_documentation_links'], 10)
        self.assertEqual(result['distinct_documentation_references'], 7)
        self.assertEqual(result['documentation_page_count'], 3)
        self.assertEqual(result['candidate_obligations_with_partial_links'], 3)
        self.assertEqual(len(result['source_files']), 6)
        self.assertEqual(result['source_file_byte_verification'], 'reverified_in_this_run')
        for row, link in zip(witness.ROWS, result['links']):
            self.assertEqual(link['test'], row[0])
            self.assertEqual(link['candidate_obligation_ids'], [row[1]])
            self.assertEqual(link['source_sha256'], row[2])
            self.assertEqual(link['source_byte_length'], row[3])
            self.assertEqual(link['oracle_expect'], row[4])
            self.assertEqual(link['expected_observation'], row[5])
            self.assertEqual(link['relationship'], 'partial')
            self.assertTrue(link['rationale'])
            self.assertTrue(link['limitations'])
            self.assertEqual(link['source_file'], witness.FIXTURE)
            expected_lines = (1150, 1178, 1196, 1222, 1242, 1272)
            self.assertEqual(link['selector_line'], expected_lines[witness.ROWS.index(row)])
            self.assertEqual(len(link['selector_body_sha256']), 64)

    def test_no_execution_review_completion_or_denominator(self):
        result = self.result()
        for key in ('reviewed_obligations', 'completed_obligations',
                    'obligations_with_executable_evidence'):
            self.assertEqual(result[key], 0)
        for key in ('api_denominator', 'full_rust_completion_percent'):
            self.assertIsNone(result[key])
        self.assertFalse(result['execution_evidence_imported'])
        for key in ('documentation_page_byte_verification', 'documentation_anchor_verification'):
            self.assertEqual(result[key], 'not_reverified_in_this_run')
        self.assertNotIn('pass', json.dumps(result))

    def test_each_present_pin_drift(self):
        for name in witness.PINS:
            with self.subTest(name=name):
                original = (self.root / name).read_bytes()
                self.mutate(name)
                with self.assertRaisesRegex(ValueError, 'pin drift'):
                    self.result()
                self.write(name, original)

    def test_missing_companion_does_not_hide_present_drift(self):
        for missing in witness.PINS:
            original = (self.root / missing).read_bytes()
            (self.root / missing).unlink()
            for name in witness.PINS:
                if name == missing:
                    continue
                if {name, missing} == {witness.CANDIDATE, witness.PROVENANCE}:
                    # Neither metadata pin remains trusted: unrelated metadata
                    # must return missing rather than speculate about drift.
                    continue
                with self.subTest(missing=missing, drift=name):
                    raw = (self.root / name).read_bytes()
                    self.mutate(name)
                    with self.assertRaisesRegex(ValueError, 'pin drift'):
                        self.result()
                    self.write(name, raw)
            self.write(missing, original)

    def test_each_missing_input_has_zero_attribution(self):
        for name in witness.PINS:
            raw = (self.root / name).read_bytes()
            (self.root / name).unlink()
            result = self.result()
            self.assertEqual(result['readiness'], 'missing')
            self.assertIn(name, result['missing_files'])
            self.assertEqual(result['links'], [])
            self.assertEqual(result['direct_witness_selectors'], 0)
            self.assertEqual(result['source_file_byte_verification'], 'partial')
            self.write(name, raw)

    def test_malformed_metadata_normalized_to_value_error(self):
        for name in (witness.CANDIDATE, witness.PROVENANCE):
            original = (self.root / name).read_bytes()
            for raw in (b'{', b'[]', b'null', b'{}', b'{"schema_version":1,"schema_version":1}',
                        b'{"schema_version":NaN}'):
                with self.subTest(name=name, raw=raw):
                    self.write(name, raw)
                    with self.assertRaises(ValueError):
                        self.result()
            self.write(name, original)

    def test_missing_metadata_still_rejects_malformed_other(self):
        (self.root / witness.CANDIDATE).unlink()
        self.write(witness.PROVENANCE, b'{}')
        with self.assertRaises(ValueError):
            self.result()

    def test_unrelated_metadata_does_not_read_scope_or_tests(self):
        candidate = json.loads((self.root / witness.CANDIDATE).read_bytes())
        candidate['sections'] = candidate['sections'][:1]
        candidate['limitations'] = ['Unrelated tiny legitimate candidate.']
        provenance = json.loads((self.root / witness.PROVENANCE).read_bytes())
        provenance['observations']['interpretation'] = 'Unrelated recorded observation.'
        provenance['pages'].reverse()
        self.write(witness.CANDIDATE, json.dumps(candidate).encode())
        self.write(witness.PROVENANCE, json.dumps(provenance).encode())
        scope = self.root / 'data/compiler-compatibility-scope.json'
        scope.unlink()
        scope.symlink_to('/nonexistent-unrelated-scope')
        shutil.rmtree(self.root / 'tests')
        (self.root / 'tests').symlink_to('/nonexistent-unrelated-tests')
        result = self.result()
        self.assertEqual(result['readiness'], 'missing')
        self.assertEqual(result['source_files'], [])
        self.assertEqual(result['source_file_byte_verification'], 'not_reverified_in_this_run')
        self.assertEqual(result['links'], [])
        self.assertIn('missing_context', result)

    def test_symlink_files_rejected(self):
        for name in witness.PINS:
            with self.subTest(name=name):
                file = self.root / name
                raw = file.read_bytes()
                file.unlink()
                file.symlink_to(ROOT / name)
                with self.assertRaisesRegex(ValueError, 'symlink'):
                    self.result()
                file.unlink()
                file.write_bytes(raw)

    def test_symlink_ancestors_and_root_rejected(self):
        for directory in ('data', 'tests/support'):
            folder = self.root / directory
            moved = folder.with_name(folder.name + '-actual')
            folder.rename(moved)
            folder.symlink_to(moved, target_is_directory=True)
            with self.assertRaisesRegex(ValueError, 'symlink'):
                self.result()
            folder.unlink()
            moved.rename(folder)
        alias = self.root.parent / 'alias'
        alias.symlink_to(self.root, target_is_directory=True)
        with self.assertRaisesRegex(ValueError, 'symlink'):
            witness.summarize(alias)

    def test_missing_root(self):
        result = witness.summarize(self.root / 'absent')
        self.assertEqual(result['readiness'], 'missing')
        self.assertEqual(result['links'], [])
        self.assertEqual(set(result['missing_files']), {witness.CANDIDATE, witness.PROVENANCE})

    def test_unrelated_test_symlink_ignored_no_runner_calls(self):
        (self.root / 'tests/discover.rs').symlink_to('/does-not-exist')
        (self.root / 'tests/conformance.rs').symlink_to('/does-not-exist')
        with patch('subprocess.run', side_effect=AssertionError('runner forbidden')), \
             patch('os.system', side_effect=AssertionError('runner forbidden')):
            self.assertEqual(self.result()['direct_witness_selectors'], 6)

    def fixture_tamper(self, old, new, message):
        raw = (self.root / witness.FIXTURE).read_bytes()
        self.assertIn(old, raw)
        raw = raw.replace(old, new, 1)
        self.write(witness.FIXTURE, raw)
        with patch.dict(witness.PINS, {witness.FIXTURE: witness.sha(raw)}):
            with self.assertRaisesRegex(ValueError, message):
                self.result()

    def test_exact_source_tamper_despite_reauthorized_fixture(self):
        self.fixture_tamper(b'let previous = slot.replace(Err(9));',
                            b'let previous = slot.replace(Err(8));', 'exact source identity')

    def test_expectation_tamper_despite_reauthorized_fixture(self):
        original = (self.root / witness.FIXTURE).read_bytes()
        self.fixture_tamper(b'Expect::Run("replaced\\ndrop 7',
                            b'Expect::Run("replaced\\ndrop 8', 'expectation mismatch')
        self.write(witness.FIXTURE, original)
        tail = b'println!("{}", *borrowed);\n}\nfn main() {}\n"#,\n            expect: '
        self.fixture_tamper(tail + b'Expect::InvalidAfterParse("E0502")',
                            tail + b'Expect::InvalidAfterParse("E0308")', 'expectation mismatch')

    def test_alias_and_nonliteral_case_rejected(self):
        original = (self.root / witness.FIXTURE).read_bytes()
        self.fixture_tamper(b'mod oracle;', b'mod foreign;', 'oracle alias')
        self.write(witness.FIXTURE, original)
        self.fixture_tamper(b'name: "replace_returns_old_nested_owned_value_and_take_empties_slot",\n            source: r#"',
                            b'name: "replace_returns_old_nested_owned_value_and_take_empties_slot",\n            source: concat!(r#"',
                            'single literal Case')

    def test_duplicate_authored_selector_and_foreign_candidate(self):
        rows = list(witness.ROWS)
        rows[-1] = rows[0]
        with patch.object(witness, 'ROWS', tuple(rows)):
            with self.assertRaisesRegex(ValueError, 'duplicate authored selector'):
                self.result()
        rows = list(witness.ROWS)
        row = list(rows[0])
        row[1] = 'obligation-foreign'
        rows[0] = tuple(row)
        with patch.object(witness, 'ROWS', tuple(rows)):
            with self.assertRaisesRegex(ValueError, 'foreign candidate'):
                self.result()
        rows = list(witness.ROWS)
        first, last = list(rows[0]), list(rows[-1])
        first[1], last[1] = last[1], first[1]
        rows[0], rows[-1] = tuple(first), tuple(last)
        with patch.object(witness, 'ROWS', tuple(rows)):
            with self.assertRaisesRegex(ValueError, 'foreign candidate'):
                self.result()
        scope_name = 'data/compiler-compatibility-scope.json'
        scope_raw = (self.root / scope_name).read_bytes()
        bad_scope = scope_raw.replace(witness.ROWS[0][0].encode(), b'foreign_test')
        self.write(scope_name, bad_scope)
        with patch.dict(witness.PINS, {scope_name: witness.sha(bad_scope)}):
            with self.assertRaisesRegex(ValueError, 'scope membership'):
                self.result()
        self.write(scope_name, scope_raw)
        raw = (self.root / witness.FIXTURE).read_bytes()
        raw += ('\nfn ' + witness.ROWS[0][0] + '() {\n}\n').encode()
        self.write(witness.FIXTURE, raw)
        with patch.dict(witness.PINS, {witness.FIXTURE: witness.sha(raw)}):
            with self.assertRaisesRegex(ValueError, 'duplicate authored selector'):
                self.result()

    def test_html_exact_ids_duplicates_missing_version_and_hash(self):
        version = '1.98.1 (48a229cea 2026-09-01)'
        raw = ('<html data-rustdoc-version="' + version + '"><section id="method.nth"></section></html>').encode()
        page = {'path': witness.ITER, 'byte_length': len(raw),
                'content_sha256': witness.sha(raw), 'rustdoc_version': version}
        witness.verify_html(raw, page, {'method.nth'})
        for bad in (raw.replace(b'method.nth', b'method.nthx'),
                    raw.replace(b'</html>', b'<i id="method.nth"></i></html>'),
                    raw.replace(b'1.98.1', b'1.98.2'),
                    raw.replace(b'id="method.nth"', b'id="method.nth" id="method.nth"')):
            with self.subTest(bad=bad):
                modified = dict(page, content_sha256=witness.sha(bad), byte_length=len(bad))
                with self.assertRaises(ValueError):
                    witness.verify_html(bad, modified, {'method.nth'})
        with self.assertRaisesRegex(ValueError, 'byte mismatch'):
            witness.verify_html(raw + b' ', page, {'method.nth'})

    def test_optional_real_html_and_actual_anchor_drift(self):
        html_root = os.environ.get('JR_LIBRARY_WITNESS_HTML_ROOT')
        fixtures = os.environ.get('JR_COMPAT_LIBRARY_FIXTURES')
        if not html_root and not fixtures:
            self.skipTest('JR_LIBRARY_WITNESS_HTML_ROOT or JR_COMPAT_LIBRARY_FIXTURES not provided')
        html = Path(html_root) if html_root else Path(fixtures) / 'pinned-html-final/html'
        result = witness.summarize(self.root, html)
        self.assertEqual(result['documentation_anchor_verification'], 'reverified_in_this_run')
        self.assertEqual(result['documentation_page_byte_verification'], 'reverified_in_this_run')
        page = next(ref for link in result['links'] for ref in link['documentation_references']
                    if ref['path'] == witness.ITER)
        raw = (html / witness.ITER).read_bytes()
        self.assertIn(b'id="method.nth"', raw)
        bad = raw.replace(b'id="method.nth"', b'id="method.nth-drift"', 1)
        # Reauthorize only byte identity to exercise exact anchor enforcement.
        with self.assertRaisesRegex(ValueError, 'anchor'):
            witness.verify_html(bad, dict(page, byte_length=len(bad),
                                content_sha256=witness.sha(bad)), {'method.nth'})

    def test_ci_acquires_and_uses_three_pinned_pages_before_tests(self):
        workflow = (ROOT / '.github/workflows/compiler-conformance.yml').read_text()
        acquire = workflow.index('python3 tools/acquire_pinned_library_pages.py --output-dir')
        export = workflow.index("output.write('JR_LIBRARY_WITNESS_HTML_ROOT='")
        regressions = workflow.index("python3 -m unittest discover -s tools -p 'test_*.py'")
        anchors = workflow.index('python3 tools/library_witness_sources.py --html-root "$JR_LIBRARY_WITNESS_HTML_ROOT"')
        self.assertLess(acquire, export)
        self.assertLess(export, regressions)
        self.assertLess(regressions, anchors)
        self.assertIn("data['verified_pages'] != 3", workflow)
        self.assertIn("data['verified_anchors'] != 7", workflow)
        self.assertIn("data['compatibility_credit'] is not False", workflow)

    def test_optional_html_missing_and_symlink_rejected(self):
        html = self.root.parent / 'html'
        html.mkdir()
        with self.assertRaisesRegex(ValueError, 'missing documentation page'):
            witness.summarize(self.root, html)
        alias = self.root.parent / 'html-alias'
        alias.symlink_to(html, target_is_directory=True)
        with self.assertRaisesRegex(ValueError, 'symlink'):
            witness.summarize(self.root, alias)

    def test_cli_stdout_only_and_no_input_changes(self):
        before = {name: (self.root / name).read_bytes() for name in witness.PINS}
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            self.assertEqual(witness.main(['--root', str(self.root)]), 0)
        self.assertEqual(json.loads(out.getvalue())['readiness'], 'review-required')
        self.assertEqual(before, {name: (self.root / name).read_bytes() for name in witness.PINS})
        self.assertEqual(set(p.relative_to(self.root).as_posix() for p in self.root.rglob('*')
                             if p.is_file()), set(witness.PINS))


if __name__ == '__main__':
    unittest.main()
