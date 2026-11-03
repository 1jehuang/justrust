#!/usr/bin/env python3
"""Structural tests with synthetic source, not normative-review evidence."""
import copy
import contextlib
import io
import json
import os
import tempfile
import sys
from types import SimpleNamespace
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

    def test_default_token_bytes_are_unchanged_and_const_module_is_optional(self):
        with mock.patch.dict(sys.modules, {'const_reconciliation_proposals': None}):
            default = build(self.index_path, self.candidate_path, self.root)
            explicit = build(self.index_path, self.candidate_path, self.root, profile='tokens')
        raw = (json.dumps(default, indent=2) + '\n').encode()
        self.assertEqual(digest(raw), '81bde1dfcdd0542a5dda22960f0a7f82047dfec5c3babca08080c7efe260b35a')
        self.assertEqual(default, explicit)

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


class ConstProfileTests(unittest.TestCase):
    """Dummy proposals and source exercise structure, not Rust acceptance."""

    def setUp(self):
        ReconciliationTests.setUp(self)
        self.proposals = SimpleNamespace(
            ALIASES={'const': 'synthetic-const'},
            CLASS_NAMES={'N': 'normative-semantic', 'X': 'cross-reference'},
            ROWS='const-eval.one|N|Synthetic rationale|const|positive|invalid|interaction\n'
                 'const-eval.two|X|Synthetic context|const|positive|invalid|interaction')
        patch = mock.patch.dict(sys.modules, {'const_reconciliation_proposals': self.proposals})
        patch.start()
        self.addCleanup(patch.stop)
        text = 'r[const-eval.one]\nsynthetic one\nr[const-eval.two]\nsynthetic two\n'
        source = self.root / 'src/const_eval.md'
        source.write_text(text)
        candidate = json.loads(self.candidate_path.read_bytes())
        candidate['sections'][0]['obligations'].append({'id': 'synthetic-const'})
        self.candidate_path.write_text(json.dumps(candidate))
        index = json.loads(self.index_path.read_bytes())
        index['provenance']['candidate_inventory_sha256'] = digest(self.candidate_path.read_bytes())
        index['pages'].append({'path': 'src/const_eval.md', 'sha256': digest(source.read_bytes()),
                               'url': 'synthetic-const-url', 'rules': parse_markdown(text)[0]})
        index['pages'][1]['rules'] = [{'label': 'other.' + str(i), 'line': i + 1} for i in range(2847)]
        index['counts']['unique_rule_labels_total'] = 2965
        self.index_path.write_text(json.dumps(index))
        self.data = self.build_const()

    def build_const(self):
        return build(self.index_path, self.candidate_path, self.root, profile='const-eval')

    def verify(self, data):
        return validate(data, self.index_path, self.candidate_path, self.root, profile='const-eval')

    rejected = ReconciliationTests.rejected

    def test_const_selection_global_identity_and_no_credit(self):
        self.assertEqual(self.data, self.build_const())
        self.assertEqual(self.verify(self.data)['global_rule_identities'], 2965)
        self.assertEqual(self.data['counts']['unselected_rule_identities'], 2963)
        self.assertEqual(self.data['counts']['reviewed_rule_rows'], 0)
        self.assertEqual(self.data['counts']['rows_with_exact_executable_evidence'], 0)
        self.assertEqual({row['page'] for row in self.data['rows']}, {'src/const_eval.md'})
        self.assertIn({'page': 'src/tokens.md', 'rule': 'lex.token'}, self.data['unselected_rules'])
        self.assertEqual(self.data['purpose'], 'bounded-const-eval-rule-reconciliation-candidate')
        self.assertNotIn('token', ' '.join(self.data['limitations']))
        self.assertIn('2965-label denominator', self.data['limitations'][1])
        records = reconciliation.load_inputs(self.index_path, self.candidate_path, self.root, 'const-eval')[0]
        self.assertEqual(len(records), 2)

    def test_const_adversarial_rows_bindings_counts_and_scope(self):
        mutations = [
            (lambda d: d['rows'].pop(), 'omitted selected'),
            (lambda d: d['rows'].append(copy.deepcopy(d['rows'][0])), 'duplicate rule'),
            (lambda d: d['rows'][0].update(rule='lex.token'), 'foreign rule'),
            (lambda d: d['rows'][0].update(page='src/tokens.md'), 'foreign rule'),
            (lambda d: d['rows'][0].update(source_block_sha256='0' * 64), 'source block'),
            (lambda d: d['rows'][0].update(line=True), 'source block'),
            (lambda d: d['rows'][0].update(candidate_obligation_ids=['foreign']), 'candidate obligation'),
            (lambda d: d['rows'][0].update(classifications=['index']), 'classifications'),
            (lambda d: d['bindings'].update(rule_index_sha256='0' * 64), 'bindings'),
            (lambda d: d['bindings'].update(candidate_inventory_sha256='0' * 64), 'bindings'),
            (lambda d: d['selected_pages'][0].update(sha256='0' * 64), 'page binding'),
            (lambda d: d['counts'].update(reviewed_rule_rows=1), 'counts or evidence'),
            (lambda d: d['counts'].update(rows_with_exact_executable_evidence=1), 'counts or evidence'),
            (lambda d: d['unselected_rules'].pop(), 'unselected identities'),
            (lambda d: d.update(purpose='bounded-token-rule-reconciliation-candidate'), 'purpose mismatch'),
            (lambda d: d['limitations'].__setitem__(0, 'Token-only scope'), 'scope limitations'),
        ]
        for mutate, error in mutations:
            with self.subTest(error=error):
                self.rejected(mutate, error)

    def test_profiles_cannot_validate_each_others_artifacts(self):
        token = build(self.index_path, self.candidate_path, self.root)
        with self.assertRaisesRegex(ValueError, 'purpose mismatch'):
            self.verify(token)
        with self.assertRaisesRegex(ValueError, 'purpose mismatch'):
            validate(self.data, self.index_path, self.candidate_path, self.root)
        self.assertNotIn('Lexical-invalid', json.dumps(self.data))

    def test_unknown_and_prefix_cli_profiles_are_rejected(self):
        output = self.root / 'must-not-exist.json'
        for profile in ('const', 'token', 'unknown'):
            command = self.command('generate', output)
            command[command.index('const-eval')] = profile
            with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
                main(command)
            self.assertFalse(output.exists())

    def test_const_proposal_completeness_foreign_and_duplicate(self):
        original = self.proposals.ROWS
        for rows, error in ((original.splitlines()[0], 'omitted selected'),
                            (original + '\n' + original.splitlines()[0], 'duplicate rule'),
                            (original.replace('const-eval.one', 'lex.token'), 'foreign proposal'),
                            (original + '|extra', 'seven explicit fields')):
            with self.subTest(error=error), mock.patch.object(self.proposals, 'ROWS', rows):
                with self.assertRaisesRegex(ValueError, error):
                    self.build_const()

    def test_const_changed_inputs_and_duplicate_global_labels_rejected(self):
        for path in (self.index_path, self.candidate_path, self.root / 'src/const_eval.md'):
            before = path.read_bytes()
            path.write_bytes(before + b'\n')
            try:
                with self.assertRaises(ValueError):
                    self.verify(self.data)
            finally:
                path.write_bytes(before)
        index = json.loads(self.index_path.read_bytes())
        index['pages'][1]['rules'][0]['label'] = 'const-eval.one'
        self.index_path.write_text(json.dumps(index))
        with self.assertRaisesRegex(ValueError, 'duplicate index rule label'):
            self.build_const()

    def command(self, action, output):
        return [action, '--profile', 'const-eval', '--rule-index', str(self.index_path),
                '--inventory', str(self.candidate_path), '--source-root', str(self.root),
                '--artifact', str(output)]

    def test_const_cli_generate_validate_exclusive_output(self):
        output = self.root / 'const.json'
        with contextlib.redirect_stdout(io.StringIO()):
            main(self.command('generate', output))
            main(self.command('validate', output))
        raw = output.read_bytes()
        self.assertEqual(json.loads(raw), self.data)
        with self.assertRaises(FileExistsError):
            main(self.command('generate', output))
        self.assertEqual(raw, output.read_bytes())

    def test_const_unsupported_cli_combinations_fail_before_filesystem_access(self):
        output = self.root / 'must-not-exist.json'
        cases = [(action, [flag, str(self.root)]) for action in ('generate', 'validate')
                  for flag in ('--repo-root', '--evidence')]
        for action, extra in cases:
            with self.subTest(action=action, extra=extra), \
                 mock.patch.object(Path, 'read_bytes', side_effect=AssertionError('premature read')), \
                 mock.patch.object(Path, 'read_text', side_effect=AssertionError('premature read')), \
                 mock.patch.object(Path, 'open', side_effect=AssertionError('premature open')):
                with self.assertRaisesRegex(ValueError, 'const-eval profile'):
                    main(self.command(action, output) + extra)
            self.assertFalse(output.exists())

    def test_unknown_api_profile_rejected_before_inputs(self):
        for api, args in ((build, ()), (reconciliation.load_inputs, ()), (validate, (self.data,))):
            with mock.patch.object(Path, 'read_bytes', side_effect=AssertionError('premature read')):
                with self.assertRaisesRegex(ValueError, 'unknown reconciliation profile'):
                    api(*args, self.index_path, self.candidate_path, self.root, profile='foreign')


class PinnedConstProfileTests(unittest.TestCase):
    def test_actual_pinned_const_profile_if_reference_source_is_supplied(self):
        source = os.environ.get('JUSTRUST_WITNESS_REFERENCE_ROOT')
        if not source:
            self.skipTest('set JUSTRUST_WITNESS_REFERENCE_ROOT for pinned-source structural checks')
        repo = Path(os.environ.get('JUSTRUST_WITNESS_REPO_ROOT', BASE.parent))
        index = repo / 'data/compiler-language-reference-rules.json'
        inventory = repo / 'data/compiler-language-inventory.json'
        data = build(index, inventory, Path(source), profile='const-eval')
        counts = validate(data, index, inventory, Path(source), profile='const-eval')
        self.assertEqual((counts['global_rule_identities'], counts['selected_rule_rows'],
                          counts['unselected_rule_identities']), (2965, 44, 2921))
        self.assertEqual(counts['reviewed_rule_rows'], 0)
        self.assertEqual(counts['rows_with_exact_executable_evidence'], 0)
        self.assertEqual(counts['proposed_classification_counts']['normative-semantic'], 40)
        self.assertEqual(counts['proposed_classification_counts']['index'], 4)
        self.assertEqual(data['purpose'], 'bounded-const-eval-rule-reconciliation-candidate')
        self.assertEqual(data, build(index, inventory, Path(source), profile='const-eval'))
        self.assertNotIn('Lexical-invalid', json.dumps(data))
        self.assertNotIn('src/tokens.md only', json.dumps(data))
        token = build(index, inventory, Path(source))
        self.assertEqual(digest((json.dumps(token, indent=2) + '\n').encode()),
                         'ea7fa27afdf901b91464ead14c54883debd091819c25bf02870e7a434ae61785')


class UnsafeProfileTests(unittest.TestCase):
    """Synthetic structural controls, never executable Rust evidence."""

    def setUp(self):
        import unsafe_reconciliation_proposals as proposals
        self.proposals = proposals
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / 'src').mkdir()
        self.candidate_path = self.root / 'candidate.json'
        self.candidate_path.write_text(json.dumps({
            'scope': SCOPE, 'sections': [{'obligations': [
                {'id': name} for name in sorted(set(proposals.ALIASES.values()))]}]}))
        pages = []
        for page, prefix in zip(proposals.SELECTED, ('unsafe', 'safety')):
            labels = [line.split('|')[0] for line in proposals.ROWS.strip().splitlines()
                      if line.split('|')[0].split('.')[0] == prefix]
            text = ''.join('r[' + label + ']\nsynthetic block\n' for label in labels)
            path = self.root / page
            path.write_text(text)
            pages.append({'path': page, 'sha256': digest(path.read_bytes()),
                          'url': 'synthetic-' + prefix, 'rules': parse_markdown(text)[0]})
        pages.append({'path': 'src/other.md', 'rules': [
            {'label': 'other.' + str(i), 'line': i + 1} for i in range(2938)]})
        self.index_path = self.root / 'index.json'
        self.index_path.write_text(json.dumps({
            'scope': SCOPE, 'pages': pages, 'unlinked_markdown_files': [],
            'counts': {'unique_rule_labels_total': 2965},
            'provenance': {'compiler_commit': '1' * 40, 'reference_commit': '2' * 40,
                           'source_archive_sha256': '3' * 64,
                           'candidate_inventory_sha256': digest(self.candidate_path.read_bytes())}}))
        self.data = self.build_unsafe()

    def build_unsafe(self):
        return build(self.index_path, self.candidate_path, self.root, profile='unsafe')

    def verify(self, data):
        return validate(data, self.index_path, self.candidate_path, self.root, profile='unsafe')

    rejected = ReconciliationTests.rejected

    def command(self, action, output):
        return [action, '--profile', 'unsafe', '--rule-index', str(self.index_path),
                '--inventory', str(self.candidate_path), '--source-root', str(self.root),
                '--artifact', str(output)]

    def test_all_labels_on_both_pages_deterministic_and_zero_credit(self):
        self.assertEqual(self.data, self.build_unsafe())
        counts = self.verify(self.data)
        self.assertEqual((counts['global_rule_identities'], counts['selected_rule_rows'],
                          counts['unselected_rule_identities']), (2965, 27, 2938))
        self.assertEqual(counts['reviewed_rule_rows'], 0)
        self.assertEqual(counts['rows_with_exact_executable_evidence'], 0)
        self.assertEqual(counts['proposed_classification_counts']['index'], 5)
        self.assertEqual(counts['proposed_classification_counts']['normative-semantic'], 22)
        self.assertEqual([sum(row['page'] == page for row in self.data['rows'])
                          for page in self.proposals.SELECTED], [16, 11])
        self.assertTrue(all(row['review_status'] == 'unreviewed' for row in self.data['rows']))

    def test_missing_duplicate_foreign_stale_source_and_category_controls(self):
        mutations = [
            (lambda d: d['rows'].pop(), 'omitted selected'),
            (lambda d: d['rows'].append(copy.deepcopy(d['rows'][0])), 'duplicate rule'),
            (lambda d: d['rows'][0].update(rule='other.0'), 'foreign rule'),
            (lambda d: d['rows'][0].update(page='src/unsafety.md'), 'foreign rule'),
            (lambda d: d['rows'][0].update(line=True), 'source block'),
            (lambda d: d['rows'][0].update(end_line=999), 'source block'),
            (lambda d: d['rows'][0].update(source_block_sha256='0' * 64), 'source block'),
            (lambda d: d['selected_pages'][1].update(sha256='0' * 64), 'page binding'),
            (lambda d: d['bindings'].update(rule_index_sha256='0' * 64), 'bindings'),
            (lambda d: d['bindings'].update(candidate_inventory_sha256='0' * 64), 'bindings'),
            (lambda d: d['rows'][0].update(candidate_obligation_ids=['foreign']), 'candidate obligation'),
            (lambda d: d['rows'][0]['candidate_obligation_ids'].append(d['rows'][0]['candidate_obligation_ids'][0]), 'candidate obligation'),
            (lambda d: d['rows'][0].update(classifications=['grammar']), 'classifications'),
            (lambda d: d['rows'][0].update(rationale='invented'), 'authored'),
            (lambda d: d['rows'][0].update(id='invented'), 'row ID'),
            (lambda d: d['rows'][0]['missing_test_needs'].__setitem__(0, 'positive: done'), 'authored'),
            (lambda d: d['rows'][0].update(review_status='reviewed'), 'reviewed row'),
            (lambda d: d['counts'].update(reviewed_rule_rows=1), 'counts or evidence'),
            (lambda d: d['counts'].update(rows_with_exact_executable_evidence=1), 'counts or evidence'),
            (lambda d: d['unselected_rules'].pop(), 'unselected identities'),
            (lambda d: d['unselected_rules'].append(d['unselected_rules'][0]), 'unselected identities'),
            (lambda d: d['unselected_rules'][0].update(rule='foreign'), 'unselected identities'),
            (lambda d: d['limitations'].__setitem__(0, 'Full unsafe support'), 'scope limitations'),
        ]
        for mutate, error in mutations:
            with self.subTest(error=error):
                self.rejected(mutate, error)
        # Even a known category with internally consistent counts/relationship
        # cannot silently reclassify the authored heading as semantic.
        changed = copy.deepcopy(self.data)
        changed['rows'][0].update(classifications=['normative-semantic'],
                                  relationship='proposed-partial-decomposition')
        changed['counts'] = reconciliation.compute_counts(changed['rows'], 2965, 'unsafe')
        with self.assertRaisesRegex(ValueError, 'authored'):
            self.verify(changed)
        changed = copy.deepcopy(self.data)
        changed['rows'][0]['candidate_obligation_ids'] = [self.proposals.ALIASES['union']]
        with self.assertRaisesRegex(ValueError, 'authored'):
            self.verify(changed)

    def test_proposal_completeness_and_identity_controls(self):
        original = self.proposals.ROWS
        for rows, error in ((original.strip().splitlines()[0], 'omitted selected'),
                            (original.strip() + '\n' + original.strip().splitlines()[0], 'duplicate rule'),
                            (original.replace('unsafe|I|', 'foreign|I|'), 'foreign proposal'),
                            (original.strip() + '|extra', 'seven explicit fields')):
            with self.subTest(error=error), mock.patch.object(self.proposals, 'ROWS', rows):
                with self.assertRaisesRegex(ValueError, error):
                    self.build_unsafe()

    def test_changed_sources_index_and_inventory_rejected(self):
        for path in (self.index_path, self.candidate_path,
                     *(self.root / page for page in self.proposals.SELECTED)):
            before = path.read_bytes()
            path.write_bytes(before + b'\n')
            try:
                with self.assertRaises(ValueError):
                    self.verify(self.data)
            finally:
                path.write_bytes(before)
        index = json.loads(self.index_path.read_bytes())
        index['pages'][2]['rules'][0]['label'] = 'unsafe'
        self.index_path.write_text(json.dumps(index))
        with self.assertRaisesRegex(ValueError, 'duplicate index rule label'):
            self.build_unsafe()

    def test_selected_page_missing_duplicate_and_index_line_controls(self):
        original = self.index_path.read_bytes()
        for mutate, error in (
                (lambda i: i['pages'].pop(0), 'index total mismatch'),
                (lambda i: i['pages'].append(copy.deepcopy(i['pages'][0])), 'duplicate index rule identity'),
                (lambda i: i['pages'][1]['rules'][0].update(line=2), 'labels/lines mismatch')):
            index = json.loads(original)
            mutate(index)
            self.index_path.write_text(json.dumps(index))
            try:
                with self.assertRaisesRegex(ValueError, error):
                    self.build_unsafe()
            finally:
                self.index_path.write_bytes(original)

    def test_cli_generate_validate_exclusive_and_cross_profile_controls(self):
        output = self.root / 'unsafe.json'
        with contextlib.redirect_stdout(io.StringIO()):
            main(self.command('generate', output))
            main(self.command('validate', output))
        raw = output.read_bytes()
        self.assertEqual(json.loads(raw), self.data)
        with self.assertRaises(FileExistsError):
            main(self.command('generate', output))
        self.assertEqual(raw, output.read_bytes())
        for profile in ('tokens', 'const-eval'):
            with self.assertRaisesRegex(ValueError, 'purpose mismatch'):
                validate(self.data, self.index_path, self.candidate_path, self.root, profile)
        for profile in ('unsafety', 'unsafe-keyword', 'unknown'):
            command = self.command('generate', self.root / 'absent')
            command[command.index('unsafe')] = profile
            with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
                main(command)

    def test_unsupported_cli_and_witness_api_rejected_before_io(self):
        output = self.root / 'absent'
        actions = ('generate-witness-links', 'build-witness-links',
                   'validate-witness-links', 'assess-witness-links')
        cases = [(action, []) for action in actions]
        cases += [(action, [flag, str(self.root)]) for action in ('generate', 'validate')
                  for flag in ('--repo-root', '--evidence')]
        for action, extra in cases:
            with self.subTest(action=action, extra=extra), \
                 mock.patch.object(Path, 'read_bytes', side_effect=AssertionError('premature read')), \
                 mock.patch.object(Path, 'read_text', side_effect=AssertionError('premature read')), \
                 mock.patch.object(Path, 'open', side_effect=AssertionError('premature open')), \
                 mock.patch.object(Path, 'is_symlink', side_effect=AssertionError('premature stat')):
                with self.assertRaisesRegex(ValueError, 'unsafe profile'):
                    main(self.command(action, output) + extra)
        with mock.patch.object(Path, 'read_bytes', side_effect=AssertionError('premature read')):
            with self.assertRaisesRegex(ValueError, 'unsafe profile'):
                reconciliation.build_witness_links(self.index_path, self.candidate_path,
                                                   self.root, self.root, 'unsafe')
        self.assertFalse(output.exists())


class PinnedUnsafeProfileTests(unittest.TestCase):
    def test_actual_pinned_generation_and_exact_original_bindings(self):
        source = os.environ.get('JUSTRUST_WITNESS_REFERENCE_ROOT')
        if not source:
            self.skipTest('set JUSTRUST_WITNESS_REFERENCE_ROOT for pinned-source structural checks')
        repo = Path(os.environ.get('JUSTRUST_WITNESS_REPO_ROOT', BASE.parent))
        index = repo / 'data/compiler-language-reference-rules.json'
        inventory = repo / 'data/compiler-language-inventory.json'
        source = Path(source)
        data = build(index, inventory, source, 'unsafe')
        counts = validate(data, index, inventory, source, 'unsafe')
        self.assertEqual((counts['global_rule_identities'], counts['selected_rule_rows'],
                          counts['unselected_rule_identities']), (2965, 27, 2938))
        self.assertEqual(counts['reviewed_rule_rows'], 0)
        self.assertEqual(counts['rows_with_exact_executable_evidence'], 0)
        self.assertEqual(data, build(index, inventory, source, 'unsafe'))
        self.assertEqual(data['bindings']['reference_commit'], '86635e30bf861a038dc197d7e16fd09e7e514e7a')
        # Frozen main5912cf49d legacy artifacts, including JSON formatting.
        # Optional unsafe proposals must not be imported by old profiles.
        with mock.patch.dict(sys.modules, {'unsafe_reconciliation_proposals': None}):
            for profile, expected in (
                    ('tokens', 'ea7fa27afdf901b91464ead14c54883debd091819c25bf02870e7a434ae61785'),
                    ('const-eval', '0ce718c6e2834b6c518ff43f2fed2b5de72fd75eccf01d4611c3217cbdf053c2')):
                legacy = build(index, inventory, source, profile)
                self.assertEqual(digest((json.dumps(legacy, indent=2) + '\n').encode()), expected)
        candidates = {o['id'] for section in json.loads(inventory.read_bytes())['sections']
                      for o in section['obligations']}
        selected = set()
        for page in data['selected_pages']:
            raw = (source / page['path']).read_bytes()
            self.assertEqual(page['sha256'], digest(raw))
            rules = parse_markdown(raw.decode())[0]
            lines = raw.splitlines(keepends=True)
            rows = [row for row in data['rows'] if row['page'] == page['path']]
            self.assertEqual([row['rule'] for row in rows], [rule['label'] for rule in rules])
            for position, (rule, row) in enumerate(zip(rules, rows)):
                end = rules[position + 1]['line'] - 1 if position + 1 < len(rules) else len(lines)
                self.assertEqual((row['line'], row['end_line']), (rule['line'], end))
                self.assertEqual(row['source_block_sha256'], digest(b''.join(lines[rule['line'] - 1:end])))
                self.assertTrue(set(row['candidate_obligation_ids']) <= candidates)
                selected.add((row['page'], row['rule']))
        unselected = {(row['page'], row['rule']) for row in data['unselected_rules']}
        original = json.loads(index.read_bytes())
        universe = {(page['path'], rule['label']) for page in
                    original['pages'] + original['unlinked_markdown_files'] for rule in page['rules']}
        self.assertFalse(selected & unselected)
        self.assertEqual(selected | unselected, universe)
        with tempfile.TemporaryDirectory() as temp:
            artifact = Path(temp) / 'unsafe.json'
            command = ['--profile', 'unsafe', '--rule-index', str(index), '--inventory', str(inventory),
                       '--source-root', str(source), '--artifact', str(artifact)]
            with contextlib.redirect_stdout(io.StringIO()):
                main(['generate', *command])
                main(['validate', *command])
            self.assertEqual(json.loads(artifact.read_bytes()), data)


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


class WitnessObservationTests(WitnessLinkTests):
    def setUp(self):
        super().setUp()
        import conformance as c
        # Minimal valid ordinary manifest, bound to the synthetic verifier.
        (self.repo / c.MANIFEST).write_text(json.dumps({
            'schema_version': 1,
            'contract': {**SCOPE, 'semantic_inventory': 'incomplete',
                         'oracle': {'verifier': 'tests/support/stock_rustc.rs',
                                    'release': 'synthetic', 'commit': 'synthetic', 'llvm': 'synthetic'}},
            'gates': [{'id': 'format', 'kind': 'fmt', 'profiles': ['full']}],
            'requirements': [{'id': 'partial', 'state': 'partial', 'gates': ['format'],
                              'remaining': ['all semantic review']}], 'known_gaps': [],
        }))

    def assess(self, reports=(), data=None):
        return reconciliation.assess_witness_links(
            self.data if data is None else data, self.index_path, self.candidate_path,
            self.root, self.repo, reports)

    def test_no_evidence_is_untested_and_zero_credit(self):
        import conformance as c
        with mock.patch.object(c, 'source_identity', side_effect=AssertionError('not needed')):
            result = self.assess()
        self.assertEqual(result['summary']['status_counts']['untested'], 6)
        self.assertEqual(result['summary']['distinct_linked_selectors_observed'], 0)
        self.assertEqual(result['summary']['reviewed_rule_rows'], 0)
        self.assertEqual(result['summary']['completed_rules'], 0)
        self.assertIsNone(result['summary']['full_rust_percent'])
        self.assertEqual(result['source_contract_state'], 'partial')

    def test_assess_cli_is_read_only_and_rejects_noncanonical_artifact(self):
        with tempfile.TemporaryDirectory() as external:
            artifact = Path(external) / 'links.json'
            raw = reconciliation.witness_bytes(self.data)
            artifact.write_bytes(raw)
            with contextlib.redirect_stdout(io.StringIO()) as output:
                main(self.command('assess-witness-links', artifact))
            result = json.loads(output.getvalue())
            self.assertEqual(result['summary']['status_counts']['untested'], 6)
            self.assertEqual(artifact.read_bytes(), raw)
            artifact.write_bytes(raw + b'\n')
            with self.assertRaisesRegex(ValueError, 'noncanonical'):
                main(self.command('assess-witness-links', artifact))

    def test_dispatch_current_identity_dedup_exact_filter_and_conflicts(self):
        import conformance as c
        import compatibility_scope as coverage
        key = coverage.case_id(self.data['links'][0])
        first, second = self.root / 'first', self.root / 'second'
        identity, runner = {'actual': 'source'}, {'actual': 'runner'}
        def observed(scope, manifest, directory, current, executable, api, root):
            self.assertIs(api, c)
            self.assertEqual(root, self.repo)
            self.assertEqual(current, identity)
            self.assertEqual(executable, runner)
            self.assertIn(key, {row['id'] for row in manifest['gates']})
            return {key: {'status': 'passed' if directory == first else 'failed'},
                    'unlinked-selector': {'status': 'passed'}}
        with mock.patch.object(c, 'source_identity', return_value=identity) as source, \
             mock.patch.object(coverage, 'runner_identity', return_value=runner) as executable, \
             mock.patch.object(coverage, 'evidence', side_effect=observed) as evidence:
            result = self.assess([first, first, second])
        source.assert_called_once_with(self.repo)
        executable.assert_called_once_with(c)
        self.assertEqual(evidence.call_count, 2)
        self.assertEqual(result['summary']['distinct_linked_selectors_observed'], 1)
        self.assertEqual(result['summary']['status_counts']['conflicting'], 1)
        self.assertEqual(result['summary']['status_counts']['passed'], 0)

    def test_all_nonpass_states_preserved_at_evidence_boundary(self):
        import conformance as c
        import compatibility_scope as coverage
        states = ('failed', 'stale', 'timeout', 'infrastructure', 'unverified', 'passed')
        rows = {coverage.case_id(link): {'status': status}
                for link, status in zip(self.data['links'], states)}
        with mock.patch.object(c, 'source_identity', return_value={}), \
             mock.patch.object(coverage, 'runner_identity', return_value={}), \
             mock.patch.object(coverage, 'evidence', return_value=rows):
            result = self.assess([self.root / 'receipt'])
        self.assertEqual([row['status'] for row in result['links']], list(states))
        self.assertEqual(result['summary']['completed_rules'], 0)

    def test_invalid_artifact_rejected_before_evidence_or_identity(self):
        import conformance as c
        data = copy.deepcopy(self.data)
        data['links'][0]['status'] = 'passed'
        with mock.patch.object(c, 'source_identity', side_effect=AssertionError('too early')):
            with self.assertRaisesRegex(ValueError, 'artifact differs'):
                self.assess([self.root / 'receipt'], data)

    def test_real_evidence_parser_rejects_forged_and_unfinalized_reports(self):
        import conformance as c
        import compatibility_scope as coverage
        directory = self.root / 'receipt'
        directory.mkdir()
        for report in ({'outcome': 'passed'}, {'source_before': {}, 'source_after': {},
                                              'runner_before': {}, 'runner_after': {}}):
            (directory / 'report.json').write_text(json.dumps(report))
            with mock.patch.object(c, 'source_identity', return_value={}), \
                 mock.patch.object(coverage, 'runner_identity', return_value={}):
                with self.assertRaises(c.InvalidEvidence):
                    self.assess([directory])

    def test_real_receipt_current_stale_and_tampered_log(self):
        import conformance as c
        import compatibility_scope as coverage
        directory = self.root / 'ordinary-receipt'
        directory.mkdir()
        scope = c.load_json(self.scope_path)
        base = c.validate(c.load_json(self.repo / c.MANIFEST), self.repo)
        manifest = coverage.case_manifest(scope, base, c)
        key = coverage.case_id(self.data['links'][0])
        entry = c.plan(manifest, 'full', [key], 'scratch-target')[0]
        payload = b'test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n'
        (directory / 'case.log').write_bytes(payload)
        identity, runner = {'actual-source': 1}, {'actual-runner': 1}
        report = {'schema_version': 1, 'profile': 'full', 'selected': [key],
                  'target_dir': 'scratch-target',
                  'scope_sha256': c.digest(c.encoded(scope)),
                  'manifest_sha256': c.digest(c.encoded(manifest)),
                  'source_before': identity, 'source_after': identity,
                  'runner_before': runner, 'runner_after': runner,
                  'integration_suites': c.integration_suites(self.repo),
                  'results': [{**entry, 'log': 'case.log', 'returncode': 0,
                               'timed_out': False, 'outcome': 'passed',
                               'tests_passed': 1, 'log_sha256': c.digest(payload)}]}
        path = directory / 'report.json'
        path.write_text(json.dumps(report))
        with mock.patch.object(c, 'source_identity', return_value=identity), \
             mock.patch.object(coverage, 'runner_identity', return_value=runner):
            self.assertEqual(self.assess([directory])['links'][0]['status'], 'passed')
            failed = self.root / 'failed-receipt'
            failed.mkdir()
            failed_payload = b'[justrust-conformance-failure:v1] semantic-stage-unproven\n'
            (failed / 'case.log').write_bytes(failed_payload)
            failed_report = copy.deepcopy(report)
            failed_report['results'][0].update(returncode=1, outcome='failed', tests_passed=0,
                                                log_sha256=c.digest(failed_payload))
            (failed / 'report.json').write_text(json.dumps(failed_report))
            observed = self.assess([directory, failed])['links'][0]
            self.assertEqual(observed['status'], 'conflicting')
            self.assertEqual([row['failure_annotation'] for row in observed['evidence']
                              if row['status'] == 'failed'], ['semantic-stage-unproven'])
            report['source_before'] = {'old-source': 1}
            path.write_text(json.dumps(report))
            self.assertEqual(self.assess([directory])['links'][0]['status'], 'stale')
            report['source_before'] = identity
            report['runner_after'] = {'old-runner': 1}
            path.write_text(json.dumps(report))
            self.assertEqual(self.assess([directory])['links'][0]['status'], 'stale')
            (directory / 'case.log').write_bytes(b'caller authored success')
            with self.assertRaisesRegex(c.InvalidEvidence, 'inconsistent evidence'):
                self.assess([directory])


class ConstWitnessTests(unittest.TestCase):
    """Real authored selectors, synthetic pins/receipts, no Rust execution."""

    def setUp(self):
        import const_reconciliation_proposals as proposals
        import conformance as c
        ReconciliationTests.setUp(self)
        self.repo = self.root / 'repo'
        (self.repo / 'tests/support').mkdir(parents=True)
        (self.repo / 'data').mkdir()
        self.fixture = self.repo / 'tests/const_source.rs'
        self.fixture.write_bytes((BASE.parent / 'tests/const_source.rs').read_bytes())
        for name in ('closure_oracle', 'stock_rustc'):
            (self.repo / ('tests/support/' + name + '.rs')).write_text('// synthetic helper\n')
        cases = [{'suite': 'const_source', 'test': row[0]} for row in proposals.DIRECT_WITNESSES]
        self.scope_path = self.repo / 'data/compiler-compatibility-scope.json'
        self.scope_path.write_text(json.dumps({
            'schema_version': 1, 'scope': {**SCOPE, 'semantic_inventory': 'incomplete'},
            'areas': [{'id': 'synthetic', 'title': 'Synthetic', 'requirements': [{
                'id': 'synthetic-requirement', 'title': 'Synthetic',
                'gates': [{'layer': layer, 'tests': cases} for layer in
                          ('feature', 'rejection', 'interaction', 'end_to_end')],
            }]}],
        }))
        candidate = json.loads(self.candidate_path.read_bytes())
        obligations = candidate['sections'][0]['obligations']
        obligations.extend({'id': value} for value in proposals.ALIASES.values())
        obligations.extend({'id': 'synthetic-' + str(i)} for i in range(205 - len(obligations)))
        self.candidate_path.write_text(json.dumps(candidate))
        text = ''.join('r[' + line.split('|')[0] + ']\nsynthetic block\n'
                       for line in proposals.ROWS.strip().splitlines())
        source = self.root / 'src/const_eval.md'
        source.write_text(text)
        index = json.loads(self.index_path.read_bytes())
        index['provenance']['candidate_inventory_sha256'] = digest(self.candidate_path.read_bytes())
        index['pages'].append({'path': 'src/const_eval.md', 'sha256': digest(source.read_bytes()),
                               'url': 'synthetic-const-url', 'rules': parse_markdown(text)[0]})
        index['pages'][1]['rules'] = [{'label': 'other.' + str(i), 'line': i + 1} for i in range(2805)]
        index['counts']['unique_rule_labels_total'] = 2965
        self.index_path.write_text(json.dumps(index))
        self.pin = copy.deepcopy(proposals.WITNESS_SOURCE_CONTRACT)
        self.pin.update(reference_commit=index['provenance']['reference_commit'],
                        rule_index_sha256=digest(self.index_path.read_bytes()),
                        candidate_inventory_sha256=digest(self.candidate_path.read_bytes()))
        for path in self.pin['files']:
            self.pin['files'][path] = digest((self.repo / path).read_bytes())
        patch = mock.patch.object(proposals, 'WITNESS_SOURCE_CONTRACT', self.pin)
        patch.start()
        self.addCleanup(patch.stop)
        self.data = self.build_links()
        (self.repo / c.MANIFEST).write_text(json.dumps({
            'schema_version': 1,
            'contract': {**SCOPE, 'semantic_inventory': 'incomplete',
                         'oracle': {'verifier': 'tests/support/stock_rustc.rs',
                                    'release': 'synthetic', 'commit': 'synthetic', 'llvm': 'synthetic'}},
            'gates': [{'id': 'format', 'kind': 'fmt', 'profiles': ['full']}],
            'requirements': [{'id': 'partial', 'state': 'partial', 'gates': ['format'],
                              'remaining': ['all semantic review']}], 'known_gaps': [],
        }))

    def build_links(self):
        return reconciliation.build_witness_links(self.index_path, self.candidate_path,
                                                  self.root, self.repo, profile='const-eval')

    def verify(self, data):
        return reconciliation.validate_witness_links(data, self.index_path, self.candidate_path,
                                                     self.root, self.repo, profile='const-eval')

    def assess(self, reports=(), data=None):
        return reconciliation.assess_witness_links(
            self.data if data is None else data, self.index_path, self.candidate_path,
            self.root, self.repo, reports, profile='const-eval')

    def command(self, action, artifact):
        return [action, '--profile', 'const-eval', '--rule-index', str(self.index_path),
                '--inventory', str(self.candidate_path), '--source-root', str(self.root),
                '--repo-root', str(self.repo), '--artifact', str(artifact)]

    test_strict_adversarial_artifact_mutations = WitnessLinkTests.test_strict_adversarial_artifact_mutations
    test_actual_source_file_changes_fail_even_for_build = WitnessLinkTests.test_actual_source_file_changes_fail_even_for_build
    test_no_evidence_is_untested_and_zero_credit = WitnessObservationTests.test_no_evidence_is_untested_and_zero_credit
    test_assess_cli_is_read_only_and_rejects_noncanonical_artifact = WitnessObservationTests.test_assess_cli_is_read_only_and_rejects_noncanonical_artifact
    test_dispatch_current_identity_dedup_exact_filter_and_conflicts = WitnessObservationTests.test_dispatch_current_identity_dedup_exact_filter_and_conflicts
    test_all_nonpass_states_preserved_at_evidence_boundary = WitnessObservationTests.test_all_nonpass_states_preserved_at_evidence_boundary
    test_invalid_artifact_rejected_before_evidence_or_identity = WitnessObservationTests.test_invalid_artifact_rejected_before_evidence_or_identity
    test_real_evidence_parser_rejects_forged_and_unfinalized_reports = WitnessObservationTests.test_real_evidence_parser_rejects_forged_and_unfinalized_reports
    test_real_receipt_current_stale_and_tampered_log = WitnessObservationTests.test_real_receipt_current_stale_and_tampered_log

    def test_exact_partial_links_and_extracted_sources(self):
        self.assertEqual(self.data['reconciliation'], build(self.index_path, self.candidate_path,
                                                          self.root, profile='const-eval'))
        summary = self.verify(self.data)
        self.assertEqual((summary['direct_witness_selectors'], summary['direct_rule_links'],
                          summary['distinct_direct_rule_identities']), (6, 15, 3))
        for link in self.data['links']:
            labels = {row['rule'] for row in link['rule_links']}
            self.assertNotIn('const-eval.const-expr.if-match', labels)
            self.assertEqual('const-eval.const-expr.error' in labels,
                             link['expectation_class'] == 'semantic-invalid')
            self.assertEqual(link['suite'], 'const_source')
            self.assertEqual(len(link['source_sha256']), 64)
        with self.assertRaises(ValueError):
            reconciliation.validate_witness_links(self.data, self.index_path, self.candidate_path,
                                                  self.root, self.repo)

    def test_strict_source_extraction_guards(self):
        raw = self.fixture.read_bytes()
        for link in self.data['links']:
            selector, expectation = link['test'], link['expectation_class']
            wrong_expect = (raw.replace(b'Expect::Run(', b'Expect::InvalidAfterParse(')
                            if expectation == 'positive' else
                            raw.replace(b'Expect::InvalidAfterParse("E0080")', b'Expect::Run("E0080")'))
            for changed in (raw.replace(b'mod scalar_const_differential;', b'mod impostor;'),
                            wrong_expect, raw.replace(selector.encode(), b'foreign_selector'),
                            raw.replace(b'source: "const ', b'source: include_str!("const ')):
                with self.subTest(selector=selector), self.assertRaises(ValueError):
                    reconciliation.scalar_source_binding(changed, selector, expectation)

    def test_scope_exact_membership_even_after_rebinding_scope_hash(self):
        scope = json.loads(self.scope_path.read_bytes())
        for gate in scope['areas'][0]['requirements'][0]['gates']:
            gate['tests'] = gate['tests'][1:]
        self.scope_path.write_text(json.dumps(scope))
        self.pin['files']['data/compiler-compatibility-scope.json'] = digest(self.scope_path.read_bytes())
        with self.assertRaisesRegex(ValueError, 'exact selector absent'):
            self.build_links()

    def test_const_forged_extracted_hashes_rejected(self):
        for field in ('source_sha256', 'selector_body_sha256'):
            data = copy.deepcopy(self.data)
            data['links'][0][field] = '0' * 64
            with self.assertRaisesRegex(ValueError, 'exact authored'):
                self.verify(data)

    def test_const_stale_index_inventory_page_rejected(self):
        for path in (self.index_path, self.candidate_path, self.root / 'src/const_eval.md'):
            before = path.read_bytes()
            path.write_bytes(before + b'\n')
            try:
                with self.assertRaises(ValueError):
                    self.build_links()
            finally:
                path.write_bytes(before)

    def test_const_duplicate_selector_even_after_rebinding_hash(self):
        selector = self.data['links'][0]['test']
        self.fixture.write_bytes(self.fixture.read_bytes() + ('\n#[test]\nfn ' + selector + '() {}\n').encode())
        self.pin['files']['tests/const_source.rs'] = digest(self.fixture.read_bytes())
        with self.assertRaisesRegex(ValueError, 'duplicate exact top-level selector'):
            self.build_links()

    def test_const_missing_evidence_never_becomes_pass(self):
        import conformance as c
        import compatibility_scope as coverage
        with mock.patch.object(c, 'source_identity', return_value={}), \
             mock.patch.object(coverage, 'runner_identity', return_value={}):
            with self.assertRaises((c.InvalidEvidence, FileNotFoundError)):
                self.assess([self.root / 'missing-receipt'])

    def test_const_cli_build_generate_alias_and_validate(self):
        with tempfile.TemporaryDirectory() as external:
            first, second = Path(external) / 'first.json', Path(external) / 'second.json'
            with contextlib.redirect_stdout(io.StringIO()):
                main(self.command('build-witness-links', first))
                main(self.command('generate-witness-links', second))
                main(self.command('validate-witness-links', first))
            self.assertEqual(first.read_bytes(), second.read_bytes())
            with self.assertRaises(FileExistsError):
                main(self.command('generate-witness-links', first))


class FrozenRootWitnessTests(unittest.TestCase):
    def test_actual_frozen_const_witnesses_if_reference_source_is_supplied(self):
        source = os.environ.get('JUSTRUST_WITNESS_REFERENCE_ROOT')
        if not source:
            self.skipTest('set JUSTRUST_WITNESS_REFERENCE_ROOT for actual pinned-source acceptance')
        import const_reconciliation_proposals as proposals
        repo = Path(os.environ.get('JUSTRUST_WITNESS_REPO_ROOT', BASE.parent))
        index = repo / 'data/compiler-language-reference-rules.json'
        inventory = repo / 'data/compiler-language-inventory.json'
        data = reconciliation.build_witness_links(index, inventory, Path(source), repo, profile='const-eval')
        self.assertEqual(data['source_contract'], proposals.WITNESS_SOURCE_CONTRACT)
        self.assertEqual(reconciliation.validate_witness_links(data, index, inventory, Path(source),
                                                              repo, profile='const-eval'), data['summary'])
        self.assertEqual(data['reconciliation'], build(index, inventory, Path(source), profile='const-eval'))
        self.assertEqual((data['summary']['direct_witness_selectors'], data['summary']['direct_rule_links'],
                          data['summary']['distinct_direct_rule_identities']), (6, 15, 3))
        observations = reconciliation.assess_witness_links(data, index, inventory, Path(source),
                                                           repo, profile='const-eval')
        self.assertEqual(observations['summary']['status_counts']['untested'], 6)
        self.assertEqual(observations['summary']['completed_rules'], 0)

    def test_actual_frozen_root_if_reference_source_is_supplied(self):
        source = os.environ.get('JUSTRUST_WITNESS_REFERENCE_ROOT')
        if not source:
            self.skipTest('set JUSTRUST_WITNESS_REFERENCE_ROOT for actual pinned-source acceptance')
        repo = Path(os.environ.get('JUSTRUST_WITNESS_REPO_ROOT', BASE.parent))
        index = repo / 'data/compiler-language-reference-rules.json'
        inventory = repo / 'data/compiler-language-inventory.json'
        data = reconciliation.build_witness_links(index, inventory, Path(source), repo)
        self.assertEqual(digest(reconciliation.witness_bytes(data)),
                         '4ef882c8d3f2159e167f0966f9dfdc63b1b8c60d44422d0032f525b67ebd7a47')
        self.assertEqual(data, reconciliation.build_witness_links(index, inventory, Path(source),
                                                                  repo, profile='tokens'))
        observations = reconciliation.assess_witness_links(data, index, inventory, Path(source), repo)
        self.assertEqual(digest(reconciliation.witness_bytes(observations)),
                         '93b2be7215955856e0cdbd83e3aa8847a8e38a05584f67600678c49e0e3348d7')
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
