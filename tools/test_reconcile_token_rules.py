#!/usr/bin/env python3
"""Structural tests with synthetic source, not normative-review evidence."""
import copy
import contextlib
import io
import json
import os
import tempfile
import unittest
from unittest import mock
from pathlib import Path

from extract_reference_rules import SCOPE, parse_markdown
from reconcile_token_rules import build, digest, main, validate
from token_reconciliation_proposals import ALIASES, ROWS
import reconcile_token_rules as reconciliation
from token_reconciliation_proposals import DIRECT_WITNESSES, WITNESS_SOURCE_CONTRACT

BASE = Path(__file__).resolve().parent


class ReconciliationTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / 'src').mkdir()
        labels = [line.split('|')[0] for line in ROWS.strip().splitlines()]
        text = ''.join('r[' + label + ']\nsynthetic fixture block\n' for label in labels)
        source = self.root / 'src/tokens.md'
        source.write_text(text)
        rules, _, _ = parse_markdown(text)
        self.candidate_path = self.root / 'candidate.json'
        candidate = {'scope': SCOPE, 'sections': [{'id': 'synthetic-candidate', 'obligations': [{'id': name} for name in sorted(set(ALIASES.values()))]}]}
        self.candidate_path.write_text(json.dumps(candidate))
        self.index_path = self.root / 'index.json'
        index = {
            'scope': SCOPE,
            'provenance': {'compiler_commit': '1' * 40, 'reference_commit': '2' * 40, 'source_archive_sha256': '3' * 64, 'candidate_inventory_sha256': digest(self.candidate_path.read_bytes())},
            'counts': {'unique_rule_labels_total': len(labels) + 2},
            'pages': [
                {'path': 'src/tokens.md', 'sha256': digest(source.read_bytes()), 'url': 'https://github.com/rust-lang/reference/blob/' + '2' * 40 + '/src/tokens.md', 'rules': rules},
                {'path': 'src/other.md', 'rules': [{'label': 'other.rule', 'line': 1}]},
            ],
            'unlinked_markdown_files': [{'path': 'src/unlinked.md', 'rules': [{'label': 'unlinked.rule', 'line': 1}]}],
        }
        self.index_path.write_text(json.dumps(index))
        self.data = build(self.index_path, self.candidate_path, self.root)

    def verify(self, data):
        return validate(data, self.index_path, self.candidate_path, self.root)

    def rejected(self, mutate, message):
        changed = copy.deepcopy(self.data)
        mutate(changed)
        with self.assertRaisesRegex(ValueError, message):
            self.verify(changed)

    def test_valid_deterministic_candidate_has_no_review_credit(self):
        self.assertEqual(self.data, build(self.index_path, self.candidate_path, self.root))
        counts = self.verify(self.data)
        self.assertEqual(counts['selected_rule_rows'], 115)
        self.assertEqual(counts['global_rule_identities'], 117)
        self.assertEqual(counts['unselected_rule_identities'], 2)
        self.assertEqual(counts['reviewed_rule_rows'], 0)
        self.assertEqual(counts['rows_with_exact_executable_evidence'], 0)

    def test_omitted_selected_rule(self):
        self.rejected(lambda d: d['rows'].pop(), 'omitted selected')

    def test_duplicate_selected_rule(self):
        self.rejected(lambda d: d['rows'].append(copy.deepcopy(d['rows'][0])), 'duplicate rule')

    def test_foreign_rule_label(self):
        self.rejected(lambda d: d['rows'][0].update(rule='other.rule'), 'foreign rule')

    def test_foreign_rule_page(self):
        self.rejected(lambda d: d['rows'][0].update(page='src/other.md'), 'foreign rule')

    def test_omitted_unselected_identity(self):
        self.rejected(lambda d: d['unselected_rules'].pop(), 'unselected identities')

    def test_duplicate_unselected_identity(self):
        self.rejected(lambda d: d['unselected_rules'].append(copy.deepcopy(d['unselected_rules'][0])), 'unselected identities')

    def test_foreign_unselected_identity(self):
        self.rejected(lambda d: d['unselected_rules'][0].update(rule='foreign'), 'unselected identities')

    def test_stale_block_hash(self):
        self.rejected(lambda d: d['rows'][0].update(source_block_sha256='0' * 64), 'source block')

    def test_stale_block_line(self):
        self.rejected(lambda d: d['rows'][0].update(line=2), 'source block')

    def test_bool_line_not_integer(self):
        self.rejected(lambda d: d['rows'][0].update(line=True), 'source block')

    def test_stale_page_hash(self):
        self.rejected(lambda d: d['selected_pages'][0].update(sha256='0' * 64), 'page binding')

    def test_stale_index_binding(self):
        self.rejected(lambda d: d['bindings'].update(rule_index_sha256='0' * 64), 'bindings')

    def test_candidate_source_changed(self):
        self.candidate_path.write_text(self.candidate_path.read_text() + '\n')
        with self.assertRaisesRegex(ValueError, 'candidate binding'):
            self.verify(self.data)

    def test_selected_source_bytes_changed(self):
        (self.root / 'src/tokens.md').write_text('r[lex.token]\nchanged\n')
        with self.assertRaisesRegex(ValueError, 'page hash'):
            self.verify(self.data)

    def test_symlink_source_rejected(self):
        source = self.root / 'src/tokens.md'
        other = self.root / 'tokens-copy.md'
        source.rename(other)
        source.symlink_to(other)
        with self.assertRaisesRegex(ValueError, 'symlink'):
            self.verify(self.data)

    def test_foreign_obligation_id(self):
        self.rejected(lambda d: d['rows'][0].update(candidate_obligation_ids=['source-lexical']), 'candidate obligation')

    def test_duplicate_candidate_links(self):
        self.rejected(lambda d: d['rows'][0]['candidate_obligation_ids'].append(d['rows'][0]['candidate_obligation_ids'][0]), 'candidate obligation')

    def test_missing_rationale(self):
        self.rejected(lambda d: d['rows'][0].update(rationale=' '), 'rationale')

    def test_unknown_classification(self):
        self.rejected(lambda d: d['rows'][0].update(classifications=['supported']), 'classifications')

    def test_duplicate_classification(self):
        self.rejected(lambda d: d['rows'][0].update(classifications=['index','index']), 'classifications')

    def test_heading_cannot_grant_executable_credit(self):
        self.rejected(lambda d: d['rows'][0].update(relationship='proposed-partial-decomposition'), 'context execution')

    def test_reviewed_row_cannot_be_silently_claimed(self):
        self.rejected(lambda d: d['rows'][0].update(review_status='reviewed'), 'reviewed row')

    def test_top_level_reviewed_cannot_be_claimed(self):
        self.rejected(lambda d: d.update(review_status='reviewed'), 'reviewed or foreign')

    def test_execution_evidence_count_cannot_be_claimed(self):
        self.rejected(lambda d: d['counts'].update(rows_with_exact_executable_evidence=1), 'counts or evidence')

    def test_boolean_count_cannot_alias_zero(self):
        self.rejected(lambda d: d['counts'].update(reviewed_rule_rows=False), 'counts or evidence')

    def test_malformed_classification_type(self):
        self.rejected(lambda d: d['rows'][0].update(classifications=[{}]), 'classifications')

    def test_malformed_candidate_link_type(self):
        self.rejected(lambda d: d['rows'][0].update(candidate_obligation_ids=[{}]), 'candidate obligation')

    def test_unknown_pass_field_rejected(self):
        self.rejected(lambda d: d['rows'][0].update(passed=True), 'row fields')

    def test_missing_evidence_class(self):
        self.rejected(lambda d: d['rows'][0]['missing_test_needs'].pop(), 'test-needs')

    def test_missing_evidence_prefix(self):
        self.rejected(lambda d: d['rows'][0]['missing_test_needs'].__setitem__(0, 'ready'), 'evidence classes')

    def test_foreign_scope(self):
        self.rejected(lambda d: d['scope'].update(edition='2021'), 'foreign scope')

    def test_rows_must_be_deterministically_ordered(self):
        self.rejected(lambda d: d['rows'].reverse(), 'row order')

    def command(self, output):
        return ['generate', '--rule-index', str(self.index_path), '--inventory',
                str(self.candidate_path), '--source-root', str(self.root),
                '--artifact', str(output)]

    def test_cli_cannot_overwrite_input_or_prior_artifact(self):
        for output in (self.index_path, self.candidate_path, self.root / 'src/tokens.md'):
            before = output.read_bytes()
            with self.assertRaises(FileExistsError):
                main(self.command(output))
            self.assertEqual(before, output.read_bytes())

    def test_cli_fresh_output_validates_without_review_credit(self):
        output = self.root / 'fresh.json'
        with contextlib.redirect_stdout(io.StringIO()):
            main(self.command(output))
        self.assertEqual(self.verify(json.loads(output.read_text()))['reviewed_rule_rows'], 0)
        before = output.read_bytes()
        with self.assertRaises(FileExistsError):
            main(self.command(output))
        self.assertEqual(before, output.read_bytes())


class WitnessLinkTests(unittest.TestCase):
    """Synthetic contracts exercise structure, never normative rule review."""

    def setUp(self):
        ReconciliationTests.setUp(self)
        self.repo = self.root / 'repo'
        (self.repo / 'tests/support').mkdir(parents=True)
        (self.repo / 'data').mkdir()
        self.fixture = self.repo / 'tests/conformance_generated.rs'
        self.fixture.write_text(''.join(
            '#[test]\n' + ('#[ignore = "unmeasured"]\n' if i == 5 else '')
            + 'fn ' + row[0] + '() {}\n' for i, row in enumerate(DIRECT_WITNESSES)))
        for name in ('closure_oracle', 'stock_rustc'):
            (self.repo / ('tests/support/' + name + '.rs')).write_text('// synthetic helper\n')
        cases = [{'suite': 'conformance_generated', 'test': row[0]} for row in DIRECT_WITNESSES]
        self.scope_path = self.repo / 'data/compiler-compatibility-scope.json'
        self.scope_path.write_text(json.dumps({
            'schema_version': 1,
            'scope': {**SCOPE, 'semantic_inventory': 'incomplete'},
            'areas': [{'id': 'synthetic', 'title': 'Synthetic', 'requirements': [{
                'id': 'synthetic-requirement', 'title': 'Synthetic',
                'gates': [{'layer': layer, 'tests': cases} for layer in
                          ('feature', 'rejection', 'interaction', 'end_to_end')],
            }]}],
        }))
        candidate = json.loads(self.candidate_path.read_bytes())
        obligations = candidate['sections'][0]['obligations']
        obligations.extend({'id': 'synthetic-' + str(i)} for i in range(205 - len(obligations)))
        self.candidate_path.write_text(json.dumps(candidate))
        index = json.loads(self.index_path.read_bytes())
        index['provenance']['candidate_inventory_sha256'] = digest(self.candidate_path.read_bytes())
        index['pages'][1]['rules'] = [{'label': 'other.' + str(i), 'line': i + 1} for i in range(2849)]
        index['counts']['unique_rule_labels_total'] = 2965
        self.index_path.write_text(json.dumps(index))
        self.pin = copy.deepcopy(WITNESS_SOURCE_CONTRACT)
        self.pin.update(reference_commit=index['provenance']['reference_commit'],
                        rule_index_sha256=digest(self.index_path.read_bytes()),
                        candidate_inventory_sha256=digest(self.candidate_path.read_bytes()))
        for path in self.pin['files']:
            self.pin['files'][path] = digest((self.repo / path).read_bytes())
        patch = mock.patch.object(reconciliation, 'WITNESS_SOURCE_CONTRACT', self.pin)
        patch.start()
        self.addCleanup(patch.stop)
        self.data = self.build_links()

    def build_links(self):
        return reconciliation.build_witness_links(self.index_path, self.candidate_path, self.root, self.repo)

    def verify(self, data):
        return reconciliation.validate_witness_links(data, self.index_path, self.candidate_path, self.root, self.repo)

    def test_exact_links_preserve_all_missing_needs_and_zero_credit(self):
        self.assertEqual(self.data['reconciliation'], build(self.index_path, self.candidate_path, self.root))
        self.assertEqual(reconciliation.witness_bytes(self.data), reconciliation.witness_bytes(self.build_links()))
        summary = self.verify(self.data)
        self.assertEqual((summary['direct_witness_selectors'], summary['direct_rule_links'],
                          summary['distinct_direct_rule_identities']), (6, 20, 16))
        self.assertEqual((summary['positive_selectors'], summary['semantic_invalid_selectors']), (3, 3))
        self.assertEqual(summary['reviewed_rule_rows'], 0)
        self.assertEqual(summary['rows_with_exact_executable_evidence'], 0)
        self.assertIsNone(summary['full_rust_percent'])
        self.assertIn(DIRECT_WITNESSES[5][0], {link['test'] for link in self.data['links']})
        self.assertNotIn('source_sha256', reconciliation.witness_bytes(self.data).decode())

    def test_strict_adversarial_artifact_mutations(self):
        mutations = {
            'manual pass': lambda d: d['links'][0].update(passed=True),
            'top manual state': lambda d: d.update(state='passed'),
            'deleted link': lambda d: d['links'].pop(),
            'extra link': lambda d: d['links'].append(copy.deepcopy(d['links'][0])),
            'duplicate selector': lambda d: d['links'].__setitem__(1, copy.deepcopy(d['links'][0])),
            'reordered links': lambda d: d['links'].reverse(),
            'deleted rule link': lambda d: d['links'][0]['rule_links'].pop(),
            'reordered rule links': lambda d: d['links'][0]['rule_links'].reverse(),
            'duplicate rule link': lambda d: d['links'][0]['rule_links'].append(copy.deepcopy(d['links'][0]['rule_links'][0])),
            'foreign candidate': lambda d: d['links'][0].update(candidate_obligation_ids=['source-lexical']),
            'foreign selector': lambda d: d['links'][0].update(test='invented'),
            'foreign suite': lambda d: d['links'][0].update(suite='stock_rustc'),
            'foreign selector line': lambda d: d['links'][0].update(selector_line=0),
            'expectation class': lambda d: d['links'][0].update(expectation_class='passed'),
            'source traversal': lambda d: d['links'][0].update(source_file='../tests/conformance_generated.rs'),
            'rule hash': lambda d: d['links'][0]['rule_links'][0].update(source_block_sha256='0' * 64),
            'rule page': lambda d: d['links'][0]['rule_links'][0].update(page='src/other.md'),
            'rule label': lambda d: d['links'][0]['rule_links'][0].update(rule='lex.token'),
            'rule line': lambda d: d['links'][0]['rule_links'][0].update(line=False),
            'rule end line': lambda d: d['links'][0]['rule_links'][0].update(end_line=0),
            'helper pin': lambda d: d['source_files'][1].update(sha256='0' * 64),
            'scope pin': lambda d: d['source_contract']['files'].update({'data/compiler-compatibility-scope.json': '0' * 64}),
            'index pin': lambda d: d['source_contract'].update(rule_index_sha256='0' * 64),
            'candidate pin': lambda d: d['source_contract'].update(candidate_inventory_sha256='0' * 64),
            'scope': lambda d: d['scope'].update(edition='2021'),
            'review': lambda d: d['summary'].update(reviewed_rule_rows=1),
            'evidence': lambda d: d['summary'].update(rows_with_exact_executable_evidence=1),
            'bool zero': lambda d: d['summary'].update(reviewed_rule_rows=False),
            'percent': lambda d: d['summary'].update(full_rust_percent=0),
            'missing needs': lambda d: d['reconciliation']['rows'][0]['missing_test_needs'].pop(),
            'unselected': lambda d: d['reconciliation']['unselected_rules'].pop(),
            'v1 schema confusion': lambda d: d.update(purpose='bounded-token-rule-reconciliation-candidate'),
        }
        for name, mutate in mutations.items():
            with self.subTest(name=name):
                changed = copy.deepcopy(self.data)
                mutate(changed)
                with self.assertRaisesRegex(ValueError, 'exact authored'):
                    self.verify(changed)

    def test_actual_source_file_changes_fail_even_for_build(self):
        for name in self.pin['files']:
            with self.subTest(path=name):
                path = self.repo / name
                before = path.read_bytes()
                path.write_bytes(before + b'\n')
                try:
                    with self.assertRaisesRegex(ValueError, 'source-contract file'):
                        self.build_links()
                finally:
                    path.write_bytes(before)

    def test_discovery_and_scope_validation_are_not_replaced_by_hashes(self):
        before = self.fixture.read_text()
        self.fixture.write_text(before.replace(DIRECT_WITNESSES[0][0], 'foreign_selector'))
        self.pin['files']['tests/conformance_generated.rs'] = digest(self.fixture.read_bytes())
        with self.assertRaisesRegex(ValueError, 'missing exact test'):
            self.build_links()

    def test_selector_absent_from_scope_rejected(self):
        scope = json.loads(self.scope_path.read_bytes())
        for gate in scope['areas'][0]['requirements'][0]['gates']:
            gate['tests'] = gate['tests'][1:]
        self.scope_path.write_text(json.dumps(scope))
        self.pin['files']['data/compiler-compatibility-scope.json'] = digest(self.scope_path.read_bytes())
        with self.assertRaisesRegex(ValueError, 'absent from compatibility scope'):
            self.build_links()

    def test_duplicate_selector_rejected_even_with_updated_synthetic_pin(self):
        self.fixture.write_text(self.fixture.read_text() + '#[test]\nfn ' + DIRECT_WITNESSES[0][0] + '() {}\n')
        self.pin['files']['tests/conformance_generated.rs'] = digest(self.fixture.read_bytes())
        with self.assertRaisesRegex(ValueError, 'duplicate exact top-level selector'):
            self.build_links()

    def test_stale_index_inventory_and_source_block_rejected(self):
        for path in (self.index_path, self.candidate_path, self.root / 'src/tokens.md'):
            with self.subTest(path=str(path)):
                before = path.read_bytes()
                path.write_bytes(before + b'\n')
                try:
                    with self.assertRaises(ValueError):
                        self.build_links()
                finally:
                    path.write_bytes(before)

    def test_unsafe_paths_and_symlinks_rejected(self):
        for path in (self.fixture, self.repo / 'tests/support', self.repo,
                     self.index_path, self.candidate_path, self.root / 'src'):
            with self.subTest(path=str(path)):
                other = path.with_name(path.name + '-real')
                path.rename(other)
                path.symlink_to(other, target_is_directory=other.is_dir())
                try:
                    with self.assertRaisesRegex(ValueError, 'symlink'):
                        self.build_links()
                finally:
                    path.unlink()
                    other.rename(path)
        with self.assertRaisesRegex(ValueError, 'traversal'):
            reconciliation.safe_path(self.repo / '../repo')

    def command(self, action, output):
        return [action, '--rule-index', str(self.index_path), '--inventory', str(self.candidate_path),
                '--source-root', str(self.root), '--repo-root', str(self.repo), '--artifact', str(output)]

    def invoke(self, action, output):
        with contextlib.redirect_stdout(io.StringIO()):
            main(self.command(action, output))

    def test_cli_external_scratch_exclusive_and_canonical(self):
        # Artifact outside both the repository and the reference source root.
        with tempfile.TemporaryDirectory() as external:
            output = Path(external) / 'links.json'
            self.invoke('build-witness-links', output)
            original = output.read_bytes()
            self.invoke('validate-witness-links', output)
            with self.assertRaises(FileExistsError):
                self.invoke('build-witness-links', output)
            self.assertEqual(original, output.read_bytes())
            for raw in (original + b'\n', json.dumps(self.data, sort_keys=True).encode(),
                        original.replace(b'"schema_version": 1,', b'"schema_version": 1, "schema_version": 1,', 1)):
                output.write_bytes(raw)
                with self.assertRaisesRegex(ValueError, 'noncanonical'):
                    self.invoke('validate-witness-links', output)
            output.write_bytes(original)
            alias = Path(external) / 'alias.json'
            alias.symlink_to(output)
            for action in ('build-witness-links', 'validate-witness-links'):
                with self.assertRaisesRegex(ValueError, 'symlink'):
                    self.invoke(action, alias)
            for unsafe in (self.repo / 'output.json', self.root / 'output.json'):
                with self.assertRaisesRegex(ValueError, 'outside repository and reference'):
                    self.invoke('build-witness-links', unsafe)


class FrozenRootWitnessTests(unittest.TestCase):
    def test_actual_frozen_root_if_reference_source_is_supplied(self):
        source = os.environ.get('JUSTRUST_WITNESS_REFERENCE_ROOT')
        if not source:
            self.skipTest('set JUSTRUST_WITNESS_REFERENCE_ROOT for actual pinned-source acceptance')
        repo = Path(os.environ.get('JUSTRUST_WITNESS_REPO_ROOT', BASE.parent))
        index = repo / 'data/compiler-language-reference-rules.json'
        inventory = repo / 'data/compiler-language-inventory.json'
        data = reconciliation.build_witness_links(index, inventory, Path(source), repo)
        self.assertEqual(data['source_contract'], WITNESS_SOURCE_CONTRACT)
        self.assertEqual(reconciliation.validate_witness_links(data, index, inventory, Path(source), repo), data['summary'])
        self.assertEqual(data['reconciliation'], build(index, inventory, Path(source)))
        self.assertEqual(digest((json.dumps(data['reconciliation'], indent=2) + '\n').encode()),
                         'ea7fa27afdf901b91464ead14c54883debd091819c25bf02870e7a434ae61785')
        self.assertEqual([link['test'] for link in data['links']], sorted(row[0] for row in DIRECT_WITNESSES))
        for item in data['source_files']:
            self.assertEqual(item['sha256'], digest((repo / item['path']).read_bytes()))


if __name__ == '__main__':
    unittest.main()
