#!/usr/bin/env python3
"""Structural tests with synthetic source, not normative-review evidence."""
import copy
import contextlib
import io
import json
import os
import subprocess
import shutil
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


class WitnessSourceSummaryTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.repo = self.root / 'repo'
        shutil.copytree(BASE.parent / 'tests', self.repo / 'tests')
        (self.repo / 'data').mkdir()
        for name in ('compiler-compatibility-scope.json', 'compiler-language-inventory.json',
                     'compiler-language-reference-rules.json'):
            shutil.copyfile(BASE.parent / 'data' / name, self.repo / 'data' / name)
        self.index = self.repo / 'data/compiler-language-reference-rules.json'
        self.candidate = self.repo / 'data/compiler-language-inventory.json'

    def summary(self):
        return reconciliation.summarize_witness_sources(self.index, self.candidate, self.repo)

    def test_actual_pins_no_prose_no_execution_or_evidence(self):
        with mock.patch.object(reconciliation, 'build', side_effect=AssertionError('Reference read')), \
                mock.patch.object(subprocess, 'run', side_effect=AssertionError('execution')):
            result = self.summary()
        self.assertEqual(result['readiness'], 'review-required')
        self.assertEqual([result[key] for key in ('direct_witness_selectors', 'direct_rule_links',
                          'distinct_direct_rule_identities', 'candidate_obligations_with_partial_links')],
                         [12, 35, 19, 11])
        self.assertEqual(len(result['candidate_obligation_ids']), 11)
        self.assertEqual(len(result['source_files']), 5)
        self.assertFalse(result['execution_evidence_imported'])
        for row in (result, *result['profiles'].values()):
            self.assertEqual(row['review_status'], 'unreviewed')
            for key in ('reviewed_rule_rows', 'completed_rule_rows', 'rows_with_exact_executable_evidence'):
                self.assertEqual(row[key], 0)
            self.assertIsNone(row['full_rust_completion_percent'])
        self.assertEqual(result['source_file_byte_verification'], 'reverified_in_this_run')
        for key in ('source_block_byte_verification', 'source_archive_byte_verification'):
            self.assertEqual(result[key], 'not_reverified_in_this_run')
        pages = json.loads(self.index.read_bytes())['pages']
        rules = {(page['path'], rule['label']): rule['line'] for page in pages for rule in page['rules']}
        for link in result['links']:
            self.assertTrue(link['rationale'] and link['limitations'])
            for rule in link['rule_links']:
                self.assertEqual(rule['line'], rules[rule['page'], rule['rule']])

    def test_missing_inputs_and_bad_present_hashes(self):
        self.candidate.unlink()
        missing = self.repo / 'tests/support/stock_rustc.rs'
        missing.unlink()
        result = self.summary()
        self.assertEqual(result['readiness'], 'missing')
        self.assertEqual(set(result['missing_files']), {str(self.candidate), str(missing)})
        self.assertIsNone(result['global_rule_identities'])
        (self.repo / 'tests/support/closure_oracle.rs').write_text('bad present file')
        with self.assertRaisesRegex(ValueError, 'source-contract file mismatch'):
            self.summary()

    def test_present_index_pin_checked_without_candidate(self):
        self.candidate.unlink()
        self.index.write_bytes(self.index.read_bytes() + b'\n')
        with self.assertRaisesRegex(ValueError, 'source-contract rule_index_sha256'):
            self.summary()

    def test_unrelated_synthetic_fixture_is_explicitly_missing(self):
        from test_compatibility_inventory import ReferenceRulesTests
        fixture = ReferenceRulesTests('test_missing_reference_is_explicit_blocker')
        fixture.setUp()
        self.addCleanup(fixture.doCleanups)
        index = fixture.root / 'data/compiler-language-reference-rules.json'
        index.write_text(json.dumps(fixture.data))
        with mock.patch.object(reconciliation, 'witness_profile', side_effect=AssertionError('pin claim')):
            result = reconciliation.summarize_witness_sources(
                index, fixture.root / 'data/compiler-language-inventory.json', fixture.root)
        self.assertEqual(result['readiness'], 'missing')
        self.assertTrue(result['missing_selected_pages'])
        self.assertEqual(result['links'], [])
        self.assertEqual(result['source_files'], [])
        self.assertIsNone(result['global_rule_identities'])
        self.assertEqual(result['source_file_byte_verification'], 'not_reverified_in_this_run')

    def test_scope_hash_failure_is_value_error_with_missing_companion(self):
        self.candidate.unlink()
        (self.repo / 'data/compiler-compatibility-scope.json').write_text('{}')
        with self.assertRaisesRegex(ValueError, 'source-contract file mismatch'):
            self.summary()

    def test_present_candidate_pin_checked_without_index(self):
        self.index.unlink()
        self.candidate.write_bytes(self.candidate.read_bytes() + b'\n')
        with self.assertRaisesRegex(ValueError, 'source-contract candidate_inventory_sha256'):
            self.summary()

    def test_symlink_files_and_ancestors_rejected(self):
        source = self.repo / 'tests/support/closure_oracle.rs'
        target = self.root / 'oracle.rs'
        source.rename(target)
        source.symlink_to(target)
        with self.assertRaisesRegex(ValueError, 'symlink'):
            self.summary()
        source.unlink()
        target.rename(source)
        support = self.repo / 'tests/support'
        support.rename(self.root / 'support')
        support.symlink_to(self.root / 'support', target_is_directory=True)
        with self.assertRaisesRegex(ValueError, 'symlink'):
            self.summary()

    def test_authored_shape_duplicates_foreign_and_manual_state(self):
        original = reconciliation.witness_profile
        pin, witnesses, aliases, suite, source = original('tokens')
        first = witnesses[0]
        changes = [first + ('passed',),
                   (first[0], 'passed', *first[2:]),
                   (*first[:2], (first[2][0], first[2][0]), *first[3:]),
                   (*first[:2], ('const-eval.const-expr.evaluation',), *first[3:]),
                   (*first[:3], ('foreign-alias',), *first[4:]),
                   (*first[:3], (first[3][0], first[3][0]), *first[4:]),
                   (*first[:4], '', first[5]), (*first[:5], '')]
        for altered in changes:
            with self.subTest(altered=altered):
                def profile(name):
                    return (pin, (altered, *witnesses[1:]), aliases, suite, source) if name == 'tokens' else original(name)
                with mock.patch.object(reconciliation, 'witness_profile', side_effect=profile):
                    with self.assertRaises(ValueError):
                        self.summary()
        with mock.patch.object(reconciliation, 'witness_profile', side_effect=lambda name:
                (pin, (first, first, *witnesses[2:]), aliases, suite, source) if name == 'tokens' else original(name)):
            with self.assertRaisesRegex(ValueError, 'duplicate direct selector'):
                self.summary()

    def test_discovery_and_scope_controls_even_when_companion_missing(self):
        self.candidate.unlink()
        original = reconciliation.witness_profile
        pin, witnesses, aliases, suite, source = original('tokens')
        path = self.repo / source
        raw = path.read_bytes()
        path.write_bytes(raw + ('\n#[test]\nfn ' + DIRECT_WITNESSES[0][0] + '() {\n}\n').encode())
        changed_pin = copy.deepcopy(pin)
        changed_pin['files'][source] = digest(path.read_bytes())
        with mock.patch.object(reconciliation, 'witness_profile', side_effect=lambda name:
                (changed_pin, witnesses, aliases, suite, source) if name == 'tokens' else original(name)):
            with self.assertRaisesRegex(ValueError, 'duplicate exact top-level selector'):
                self.summary()
        path.write_bytes(raw)
        with mock.patch.object(reconciliation, 'witness_profile', side_effect=lambda name:
                (pin, witnesses, aliases, 'foreign_suite', source) if name == 'tokens' else original(name)):
            with self.assertRaisesRegex(ValueError, 'absent from compatibility scope'):
                self.summary()

    def test_unrelated_files_symlinks_and_discovery_are_not_dependencies(self):
        import conformance
        import compatibility_scope
        expected = self.summary()
        (self.repo / 'tests/unrelated.rs').write_bytes(b'\xff unrelated invalid UTF-8')
        (self.repo / 'tests/unrelated-link.rs').symlink_to(self.root / 'absent.rs')
        with mock.patch.object(conformance, 'discover', side_effect=AssertionError('whole suite')), \
                mock.patch.object(compatibility_scope, 'validate', side_effect=AssertionError('whole scope')):
            self.assertEqual(self.summary(), expected)


class ProposalSummaryTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.index_path = self.root / 'index.json'
        self.candidate_path = self.root / 'candidate.json'
        repo = BASE.parent
        self.index = json.loads((repo / 'data/compiler-language-reference-rules.json').read_bytes())
        self.candidate = json.loads((repo / 'data/compiler-language-inventory.json').read_bytes())
        self.candidate_path.write_bytes((repo / 'data/compiler-language-inventory.json').read_bytes())
        self.index_path.write_bytes((repo / 'data/compiler-language-reference-rules.json').read_bytes())

    def save_index(self):
        self.index_path.write_text(json.dumps(self.index))

    def summary(self):
        return reconciliation.summarize_proposals(self.index_path, self.candidate_path)

    def test_standalone_without_prose_artifacts_or_root(self):
        result = self.summary()
        self.assertEqual(result['global_rule_identities'], 2965)
        self.assertEqual(result['proposed_rule_identities'], 1030)
        self.assertEqual(result['rule_identities_without_proposals'], 1935)
        for name, count in (('tokens', 115), ('const-eval', 44), ('unsafe', 27), ('abi', 24), ('visibility', 18), ('branching', 58), ('destructors', 50), ('loops', 60), ('operators', 112), ('closures', 17), ('calls', 23), ('aggregates', 38), ('fields', 8), ('literals', 87), ('grouped', 6), ('transfers', 14), ('ranges', 10), ('structs', 14), ('blocks', 43), ('expression-core', 46), ('path-expressions', 6), ('never-type', 5), ('coercions', 46), ('subtyping', 12), ('type-expressions', 22), ('primitive-types', 42), ('sequence-types', 19), ('pointer-types', 22), ('function-pointers', 7), ('function-items', 6), ('inferred-types', 4), ('nominal-types', 19), ('type-parameters', 1), ('trait-objects', 10)):
            profile = result['profiles'][name]
            self.assertEqual(profile['proposed_rule_identities'], count)
            self.assertEqual(profile['review_status'], 'unreviewed')
            for key in ('reviewed_rule_rows', 'rows_with_exact_executable_evidence', 'completed_rule_rows'):
                self.assertEqual(profile[key], 0)
                self.assertEqual(result[key], 0)
            self.assertIsNone(profile['full_rust_completion_percent'])
        self.assertIsNone(result['full_rust_completion_percent'])
        for key in ('source_block_byte_verification', 'source_archive_byte_verification'):
            self.assertEqual(result[key], 'not_reverified_in_this_run')
        self.assertIn('not normative semantic coverage', ' '.join(result['limitations']))

    def assert_backlog_matches_metadata(self, result):
        pages = self.index['pages'] + self.index['unlinked_markdown_files']
        selected = {path for name in ('tokens', 'const-eval', 'unsafe', 'abi', 'visibility', 'branching', 'destructors', 'loops', 'operators', 'closures', 'calls', 'aggregates', 'fields', 'literals', 'grouped', 'transfers', 'ranges', 'structs', 'blocks', 'expression-core', 'path-expressions', 'never-type', 'coercions', 'subtyping', 'type-expressions', 'primitive-types', 'sequence-types', 'pointer-types', 'function-pointers', 'function-items', 'inferred-types', 'nominal-types', 'type-parameters', 'trait-objects')
                    for path in reconciliation.profile_inputs(name)[0]}
        proposed = {(page['path'], rule['label']) for page in pages
                    if page['path'] in selected for rule in page['rules']}
        expected_pages, missing = [], {}
        for page in pages:
            identities = {(page['path'], rule['label']) for rule in page['rules']}
            expected_pages.append({
                'page': page['path'], 'global_rule_identities': len(identities),
                'proposed_rule_identities': len(identities & proposed),
                'rule_identities_without_proposals': len(identities - proposed),
            })
            for rule in page['rules']:
                identity = (page['path'], rule['label'])
                if identity not in proposed:
                    missing[identity] = {'page': page['path'], 'rule': rule['label'],
                                         'line': rule['line'], 'page_sha256': page['sha256']}
        backlog = result['backlog']
        expected_pages.sort(key=lambda row: row['page'])
        self.assertEqual(backlog['pages'], expected_pages)
        self.assertEqual(backlog['total_pages'], len(pages))
        self.assertEqual(backlog['pages_without_proposals'], sum(
            row['global_rule_identities'] > 0 and row['proposed_rule_identities'] == 0
            for row in expected_pages))
        self.assertEqual(backlog['pages_with_partial_proposals'], sum(
            0 < row['proposed_rule_identities'] < row['global_rule_identities']
            for row in expected_pages))
        for key in ('global_rule_identities', 'proposed_rule_identities',
                    'rule_identities_without_proposals'):
            self.assertEqual(sum(row[key] for row in backlog['pages']), result[key])
        self.assertEqual(backlog['missing_rule_sample_limit'], 100)
        self.assertIs(backlog['missing_rule_sample_truncated'], len(missing) > 100)
        self.assertEqual(backlog['missing_rule_sample'],
                         [missing[identity] for identity in sorted(missing)[:100]])
        self.assertTrue(all((row['page'], row['rule']) not in proposed
                            for row in backlog['missing_rule_sample']))

    def test_backlog_covers_every_pinned_page_and_exact_missing_metadata(self):
        result = self.summary()
        self.assertEqual((result['global_rule_identities'], result['proposed_rule_identities'],
                          result['rule_identities_without_proposals']), (2965, 1030, 1935))
        self.assert_backlog_matches_metadata(result)
        self.assertEqual(result, self.summary())
        self.assertTrue(any(not page['rules'] for page in self.index['pages']))
        self.assertTrue(any(not page['rules'] for page in self.index['unlinked_markdown_files']))
        self.assertEqual(result['backlog']['pages_with_partial_proposals'], 0)
        self.assertEqual(len(result['backlog']['missing_rule_sample']), 100)

    def test_backlog_helper_boundaries_partial_pages_and_deterministic_order(self):
        for count in (0, 100, 101):
            with self.subTest(missing=count):
                pages = [
                    {'path': 'src/zero.md', 'sha256': '0' * 64, 'rules': []},
                    {'path': 'src/full.md', 'sha256': '1' * 64,
                     'rules': [{'label': 'full', 'line': 9}]},
                    {'path': 'src/partial.md', 'sha256': '2' * 64,
                     'rules': [{'label': 'proposed', 'line': 3}] + [
                         {'label': 'missing.' + str(i).zfill(3), 'line': i + 10}
                         for i in range(count)]},
                ]
                proposed = {('src/full.md', 'full'), ('src/partial.md', 'proposed')}
                backlog = reconciliation.proposal_backlog(pages, proposed)
                reordered = copy.deepcopy(pages[::-1])
                for page in reordered:
                    page['rules'].reverse()
                self.assertEqual(backlog, reconciliation.proposal_backlog(reordered, proposed))
                self.assertEqual(backlog['pages'], [
                    {'page': 'src/full.md', 'global_rule_identities': 1,
                     'proposed_rule_identities': 1, 'rule_identities_without_proposals': 0},
                    {'page': 'src/partial.md', 'global_rule_identities': count + 1,
                     'proposed_rule_identities': 1, 'rule_identities_without_proposals': count},
                    {'page': 'src/zero.md', 'global_rule_identities': 0,
                     'proposed_rule_identities': 0, 'rule_identities_without_proposals': 0},
                ])
                self.assertEqual(backlog['total_pages'], 3)
                self.assertEqual(backlog['pages_without_proposals'], 0)
                self.assertEqual(backlog['pages_with_partial_proposals'], int(count > 0))
                self.assertEqual(backlog['missing_rule_sample_limit'], 100)
                self.assertIs(backlog['missing_rule_sample_truncated'], count > 100)
                self.assertEqual(backlog['missing_rule_sample'], [
                    {'page': 'src/partial.md', 'rule': 'missing.' + str(i).zfill(3),
                     'line': i + 10, 'page_sha256': '2' * 64}
                    for i in range(min(count, 100))])

    def test_backlog_absent_when_either_or_both_inputs_are_missing(self):
        index_bytes = self.index_path.read_bytes()
        candidate_bytes = self.candidate_path.read_bytes()
        for missing in ((self.index_path,), (self.candidate_path,),
                        (self.index_path, self.candidate_path)):
            with self.subTest(missing=[path.name for path in missing]):
                self.index_path.write_bytes(index_bytes)
                self.candidate_path.write_bytes(candidate_bytes)
                for path in missing:
                    path.unlink()
                result = self.summary()
                self.assertEqual(result['readiness'], 'missing')
                self.assertNotIn('backlog', result)

    def test_each_authored_module_rejects_duplicate_omitted_foreign_and_bad_fields(self):
        import compatibility_inventory as inventory
        original = reconciliation.profile_inputs
        for profile in ('tokens', 'const-eval', 'unsafe', 'abi', 'visibility', 'branching', 'destructors'):
            selected, aliases, classes, authored = original(profile)
            lines = authored.strip().splitlines()
            fields = lines[0].split('|')
            mutations = [lines + [lines[0]], lines[1:]]
            for position, value in ((0, 'foreign.rule'), (1, 'Z'), (1, 'N N'),
                                    (3, 'foreign-alias'), (3, fields[3].split()[0] + ' ' + fields[3].split()[0])):
                changed = list(fields)
                changed[position] = value
                mutations.append(['|'.join(changed)] + lines[1:])
            for position in range(7):
                changed = list(fields)
                changed[position] = ' '
                mutations.append(['|'.join(changed)] + lines[1:])
            for mutation in mutations:
                def inputs(name):
                    return (selected, aliases, classes, '\n'.join(mutation)) if name == profile else original(name)
                with self.subTest(profile=profile, row=mutation[0]), mock.patch.object(
                        reconciliation, 'profile_inputs', side_effect=inputs), self.assertRaises(inventory.InvalidInventory):
                    self.summary()
            for changed_aliases, changed_classes in ((dict(aliases, unused='foreign-id'), classes),
                                                       (aliases, dict(classes, Z='executable'))):
                def inputs(name):
                    return (selected, changed_aliases, changed_classes, authored) if name == profile else original(name)
                with mock.patch.object(reconciliation, 'profile_inputs', side_effect=inputs), self.assertRaises(inventory.InvalidInventory):
                    self.summary()

    def test_unique_union_not_sum_of_overlapping_profiles(self):
        original = reconciliation.profile_inputs
        token = next(page for page in self.index['pages'] if page['path'] == 'src/tokens.md')
        # Move labels to an unselected page, preserving the synthetic overlap contract.
        other = next(page for page in self.index['pages'] if page['path'] == 'src/type-layout.md')
        moved, token['rules'] = token['rules'][:44], token['rules'][44:]
        start = max([rule['line'] for rule in other['rules']] + [0])
        other['rules'].extend({'label': row['label'], 'line': start + i + 1} for i, row in enumerate(moved))
        self.save_index()
        selected = ('src/tokens.md', 'src/const_eval.md')
        labels = [rule['label'] for page in self.index['pages'] if page['path'] in selected for rule in page['rules']]
        aliases = original('tokens')[1]
        authored = '\n'.join(label + '|N|Synthetic rationale|lit|Positive need|Invalid need|Interaction need' for label in labels)
        def inputs(profile):
            return (selected, aliases, original(profile)[2], authored) if profile == 'tokens' else original(profile)
        pin = dict(WITNESS_SOURCE_CONTRACT, rule_index_sha256=digest(self.index_path.read_bytes()))
        with mock.patch.object(reconciliation, 'profile_inputs', side_effect=inputs), mock.patch.object(
                reconciliation, 'WITNESS_SOURCE_CONTRACT', pin):
            result = self.summary()
            self.assert_backlog_matches_metadata(result)
            self.assertEqual(sum(row['proposed_rule_identities'] for row in result['backlog']['pages']), 986)
        self.assertEqual(result['proposed_rule_identities'], 986)
        self.assertEqual(result['rule_identities_without_proposals'], 1979)

    def test_malformed_metadata_binding_counts_and_false_credit(self):
        import compatibility_inventory as inventory
        baseline = copy.deepcopy(self.index)
        mutations = []
        for key, value in (('unique_rule_labels_total', 2964), ('rules_with_reviewed_obligation_mapping', 1),
                           ('duplicate_rule_labels', True)):
            changed = copy.deepcopy(baseline)
            changed['counts'][key] = value
            mutations.append(changed)
        changed = copy.deepcopy(baseline)
        changed['pages'].append(copy.deepcopy(changed['pages'][0]))
        mutations.append(changed)
        changed = copy.deepcopy(baseline)
        changed['pages'][0]['rules'].append(copy.deepcopy(changed['pages'][0]['rules'][0]))
        mutations.append(changed)
        changed = copy.deepcopy(baseline)
        changed['review_status'] = 'reviewed'
        mutations.append(changed)
        for changed in mutations:
            self.index = changed
            self.save_index()
            with self.assertRaises(inventory.InvalidInventory):
                self.summary()
        self.index = baseline
        self.save_index()
        for raw in ('null', '[]', '{', '{"sections":[],"sections":[]}'):
            self.candidate_path.write_text(raw)
            with self.assertRaises(inventory.InvalidInventory):
                self.summary()

    def test_missing_inputs_do_not_invent_denominators_and_present_errors_survive(self):
        import compatibility_inventory as inventory
        self.index_path.unlink()
        result = self.summary()
        self.assertEqual(result['readiness'], 'missing')
        self.assertIsNone(result['global_rule_identities'])
        self.candidate_path.write_text('null')
        with self.assertRaises(inventory.InvalidInventory):
            self.summary()
        self.candidate_path.unlink()
        result = self.summary()
        self.assertEqual(len(result['missing_files']), 2)
        self.index_path.write_text('null')
        with self.assertRaises(inventory.InvalidInventory):
            self.summary()

    def test_ordinary_cli_normalizes_invalid_proposal_inputs(self):
        data = self.root / 'data'
        data.mkdir()
        (data / 'compiler-language-inventory.json').write_bytes(self.candidate_path.read_bytes())
        (data / 'compiler-language-reference-rules.json').write_text('null')
        process = subprocess.run([sys.executable, str(BASE / 'compatibility_inventory.py'),
                                  '--root', str(self.root)], capture_output=True, text=True, check=False)
        self.assertEqual(process.returncode, 2)
        self.assertEqual(process.stdout, '')
        self.assertIn('rule_proposals', json.loads(process.stderr)['error'])

    def test_self_consistent_foreign_pin_and_index_bytes_rejected(self):
        import compatibility_inventory as inventory
        old = self.index['provenance']['reference_commit']
        foreign = 'a' * 40
        for name in ('reference_commit', 'reference_repository_url', 'source_archive_url'):
            self.index['provenance'][name] = self.index['provenance'][name].replace(old, foreign)
        for page in self.index['pages'] + self.index['unlinked_markdown_files']:
            page['url'] = page['url'].replace(old, foreign)
        self.save_index()
        with self.assertRaisesRegex(inventory.InvalidInventory, 'source-contract reference_commit'):
            self.summary()
        self.index_path.write_bytes((BASE.parent / 'data/compiler-language-reference-rules.json').read_bytes() + b' ')
        with self.assertRaisesRegex(inventory.InvalidInventory, 'source-contract rule_index_sha256'):
            self.summary()


class RuleBacklogQueryTests(unittest.TestCase):
    """Pinned metadata navigation is not source verification or review credit."""
    setUp = ProposalSummaryTests.setUp

    def query(self, **selection):
        return reconciliation.query_rule_backlog(self.index_path, self.candidate_path, **selection)

    def expected(self, *, page=None, rule=None, missing_only=False):
        memberships = {}
        for profile, _ in reconciliation.PROPOSAL_COUNTS:
            for path in reconciliation.profile_inputs(profile)[0]:
                memberships.setdefault(path, []).append(profile)
        pages, rows = [], []
        for linked, entries in ((True, self.index['pages']),
                                (False, self.index['unlinked_markdown_files'])):
            for entry in entries:
                if page is not None and entry['path'] != page:
                    continue
                chosen = [item for item in entry['rules']
                          if (rule is None or item['label'] == rule)
                          and (not missing_only or entry['path'] not in memberships)]
                if rule is not None and not any(item['label'] == rule for item in entry['rules']):
                    continue
                pages.append({'page': entry['path'], 'page_sha256': entry['sha256'],
                              'linked': linked, 'rule_identities': len(entry['rules']),
                              'selected_rule_identities': len(chosen)})
                for item in chosen:
                    profiles = sorted(memberships.get(entry['path'], []))
                    rows.append({'page': entry['path'], 'rule': item['label'],
                                 'line': item['line'], 'page_sha256': entry['sha256'],
                                 'linked': linked, 'proposal_profiles': profiles,
                                 'proposal_status': 'authored-unreviewed' if profiles else 'missing',
                                 'review_status': 'unreviewed'})
        return sorted(pages, key=lambda row: row['page']), sorted(rows, key=lambda row: (row['page'], row['rule']))

    def assert_query(self, result, **selection):
        selection = {
            'page': selection.get('page'), 'rule': selection.get('rule'),
            'missing_only': selection.get('missing_only', False)}
        pages, rows = self.expected(**selection)
        self.assertEqual(result['scope'], dict(SCOPE))
        self.assertEqual(result['bindings'], {name: WITNESS_SOURCE_CONTRACT[name] for name in (
            'reference_commit', 'rule_index_sha256', 'candidate_inventory_sha256')})
        self.assertEqual(result['selection'], selection)
        self.assertEqual(result['pages'], pages)
        self.assertEqual(result['rows'], rows)
        for row in (*result['pages'], *result['rows']):
            self.assertIs(type(row['linked']), bool)
        self.assertEqual(result['selected_page_count'], len(pages))
        self.assertEqual(result['selected_rule_identities'], len(rows))
        proposed = sum(bool(row['proposal_profiles']) for row in rows)
        self.assertEqual(result['selected_proposed_rule_identities'], proposed)
        self.assertEqual(result['selected_rule_identities_without_proposals'], len(rows) - proposed)
        self.assertEqual(sum(row['selected_rule_identities'] for row in pages), len(rows))
        for key, value in (('global_rule_identities', 2965), ('proposed_rule_identities', 1030),
                           ('rule_identities_without_proposals', 1935), ('total_pages', 122),
                           ('readiness', 'review-required'), ('review_status', 'unreviewed')):
            self.assertEqual(result[key], value)
        for key in ('reviewed_rule_rows', 'completed_rule_rows', 'rows_with_exact_executable_evidence'):
            self.assertEqual(result[key], 0)
        self.assertIsNone(result['full_rust_completion_percent'])
        self.assertIs(result['normative_inventory_complete'], False)
        for key in ('source_block_byte_verification', 'source_archive_byte_verification'):
            self.assertEqual(result[key], 'not_reverified_in_this_run')
        self.assertTrue(result['limitations'])

    def test_complete_conservation_exact_rows_and_determinism(self):
        result = self.query()
        self.assert_query(result)
        self.assertEqual(len(result['rows']), 2965)
        self.assertEqual(len(result['pages']), 122)
        self.assertEqual(result, self.query())
        self.assertEqual(sum(not row['linked'] for row in result['rows']), 9)
        self.assertEqual(json.dumps(result, sort_keys=True), json.dumps(self.query(), sort_keys=True))
        original = self.index_path.read_bytes()
        self.assertEqual(digest(original), WITNESS_SOURCE_CONTRACT['rule_index_sha256'])

    def test_exact_page_label_combination_and_textual_unlinked_labels(self):
        for page in ('src/tokens.md', 'src/const_eval.md'):
            with self.subTest(page=page):
                self.assert_query(self.query(page=page), page=page)
        unlinked = [page for page in self.index['unlinked_markdown_files'] if page['rules']]
        for page in unlinked:
            for item in page['rules']:
                for selection in ({'rule': item['label']}, {'page': page['path']},
                                  {'page': page['path'], 'rule': item['label']}):
                    with self.subTest(selection=selection):
                        result = self.query(**selection)
                        self.assert_query(result, **selection)
                        self.assertTrue(all(not row['linked'] for row in result['rows']))
        label = self.query(page='src/tokens.md')['rows'][0]['rule']
        self.assert_query(self.query(rule=label), rule=label)
        self.assert_query(self.query(page='src/tokens.md', rule=label), page='src/tokens.md', rule=label)

    def test_missing_only_preserves_known_fully_proposed_and_zero_rule_pages(self):
        result = self.query(missing_only=True)
        self.assert_query(result, missing_only=True)
        self.assertEqual(len(result['rows']), 1935)
        self.assertEqual(len(result['pages']), 122)
        for selection in ({'page': 'src/tokens.md', 'missing_only': True},
                          {'page': 'src/SUMMARY.md'},
                          {'page': 'src/SUMMARY.md', 'missing_only': True}):
            with self.subTest(selection=selection):
                result = self.query(**selection)
                self.assert_query(result, **selection)
                self.assertEqual(result['rows'], [])
                self.assertEqual(result['selected_page_count'], 1)
        label = self.query(page='src/tokens.md')['rows'][0]['rule']
        self.assert_query(self.query(rule=label, missing_only=True), rule=label, missing_only=True)

    def test_unknown_empty_nonstring_and_nonboolean_filters_are_invalid(self):
        import compatibility_inventory as inventory
        label = self.query(page='src/tokens.md')['rows'][0]['rule']
        selections = [{'page': value} for value in ('', ' ', 'src/token', 'tokens.md',
                       'src/TOKENS.md', '../src/tokens.md', 0, False, [], {})]
        selections += [{'rule': value} for value in ('', ' ', 'foreign.rule', label[:-1], 0, False, [], {})]
        selections += [{'missing_only': value} for value in (None, 0, 1, 'true', [], {})]
        selections += [{'page': 'src/const_eval.md', 'rule': label}]
        for selection in selections:
            with self.subTest(selection=selection), self.assertRaises(inventory.InvalidInventory):
                self.query(**selection)

    def test_missing_inputs_and_no_pinned_context_fail_closed(self):
        import compatibility_inventory as inventory
        for missing in (self.index_path, self.candidate_path):
            raw = missing.read_bytes()
            missing.unlink()
            with self.subTest(path=missing), self.assertRaises(inventory.InvalidInventory):
                self.query()
            missing.write_bytes(raw)
        with mock.patch.object(reconciliation, 'summarize_proposals', return_value={
                'readiness': 'missing', 'missing_files': [], 'missing_selected_pages': ['src/tokens.md']}):
            with self.assertRaises(inventory.InvalidInventory):
                self.query()

    def test_index_candidate_pin_drift_and_malformed_json_rejected(self):
        import compatibility_inventory as inventory
        for path in (self.index_path, self.candidate_path):
            original = path.read_bytes()
            for raw in (original + b'\n', b'null', b'[]', b'{', b'{"x":0,"x":1}'):
                with self.subTest(path=path, raw=raw[:15]):
                    path.write_bytes(raw)
                    with self.assertRaises(inventory.InvalidInventory):
                        self.query()
            path.write_bytes(original)
        changed = copy.deepcopy(self.index)
        changed['counts']['unique_rule_labels_total'] = 2964
        self.index_path.write_text(json.dumps(changed))
        with self.assertRaises(inventory.InvalidInventory):
            self.query()

    def test_all_authored_modules_revalidated_even_for_unrelated_page(self):
        import compatibility_inventory as inventory
        original = reconciliation.profile_inputs
        for profile, _ in reconciliation.PROPOSAL_COUNTS:
            selected, aliases, classes, authored = original(profile)
            lines = authored.strip().splitlines()
            for mutation in ('\n'.join(lines[1:]), authored + '\n' + lines[0],
                             authored.replace(lines[0].split('|')[0], 'foreign.rule', 1)):
                def inputs(name):
                    return (selected, aliases, classes, mutation) if name == profile else original(name)
                with self.subTest(profile=profile), mock.patch.object(
                        reconciliation, 'profile_inputs', side_effect=inputs), self.assertRaises(inventory.InvalidInventory):
                    self.query(page='src/SUMMARY.md')
        with mock.patch.object(reconciliation, 'profile_inputs', side_effect=ImportError('missing proposal module')):
            with self.assertRaises(inventory.InvalidInventory):
                self.query()

    def test_second_index_read_cannot_substitute_unvalidated_metadata(self):
        import compatibility_inventory as inventory
        original = reconciliation.summarize_proposals
        def summarize_then_change(*args, **kwargs):
            result = original(*args, **kwargs)
            altered = copy.deepcopy(self.index)
            altered['pages'][0]['rules'][0]['line'] += 1
            self.index_path.write_text(json.dumps(altered))
            return result
        with mock.patch.object(reconciliation, 'summarize_proposals', side_effect=summarize_then_change):
            with self.assertRaises(inventory.InvalidInventory):
                self.query()

    def test_metadata_only_without_source_archive_receipts_or_execution(self):
        import compatibility_inventory as inventory
        expected = self.query()
        allowed = {self.index_path.resolve(), self.candidate_path.resolve()}
        original_bytes, original_text = Path.read_bytes, Path.read_text
        def read_bytes(path):
            self.assertIn(path.resolve(), allowed, 'unexpected nonmetadata read')
            return original_bytes(path)
        def read_text(path, *args, **kwargs):
            self.assertIn(path.resolve(), allowed, 'unexpected nonmetadata read')
            return original_text(path, *args, **kwargs)
        with contextlib.ExitStack() as stack:
            for module, name in ((reconciliation, 'build'), (reconciliation, 'validate'),
                                 (reconciliation, 'summarize_witness_sources'),
                                 (inventory, 'verify_reference_archive'), (inventory, 'read_verifier'),
                                 (subprocess, 'run')):
                stack.enter_context(mock.patch.object(module, name, side_effect=AssertionError(name)))
            stack.enter_context(mock.patch.object(Path, 'read_bytes', read_bytes))
            stack.enter_context(mock.patch.object(Path, 'read_text', read_text))
            self.assertEqual(self.query(), expected)

    def cli(self, args):
        import compatibility_inventory as inventory
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            try:
                status = inventory.main(['--root', str(self.root), *args])
            except SystemExit as exc:
                status = exc.code
        return status, out.getvalue(), err.getvalue()

    def cli_inputs(self):
        data = self.root / 'data'
        data.mkdir(exist_ok=True)
        shutil.copyfile(self.index_path, data / 'compiler-language-reference-rules.json')
        shutil.copyfile(self.candidate_path, data / 'compiler-language-inventory.json')

    def test_cli_default_language_candidate_and_explicit_single_override(self):
        self.cli_inputs()
        label = self.query(page='src/tokens.md')['rows'][0]['rule']
        for args, selection in ((['--rule-backlog'], {}),
                (['--rule-page', 'src/SUMMARY.md'], {'page': 'src/SUMMARY.md'}),
                (['--rule', label], {'rule': label}),
                (['--rule-page', 'src/tokens.md', '--rule', label], {'page': 'src/tokens.md', 'rule': label}),
                (['--rule-backlog', '--unproposed'], {'missing_only': True}),
                (['--rule-page', 'src/tokens.md', '--unproposed'], {'page': 'src/tokens.md', 'missing_only': True})):
            for overrides in ([], ['--reference-rules', str(self.index_path), '--inventory', str(self.candidate_path)]):
                with self.subTest(args=args, overrides=overrides):
                    status, out, err = self.cli(args + overrides)
                    self.assertEqual((status, err), (0, ''))
                    self.assert_query(json.loads(out), **selection)

    def test_cli_unknown_and_empty_filters_no_stdout(self):
        self.cli_inputs()
        for args in (['--rule', 'foreign.rule'], ['--rule', ''], ['--rule-page', 'src/unknown.md'],
                     ['--rule-page', ''], ['--rule-page', 'src/SUMMARY.md', '--rule', 'foreign.rule']):
            with self.subTest(args=args):
                status, out, err = self.cli(args)
                self.assertEqual(status, 2)
                self.assertEqual(out, '')
                self.assertTrue(err)

    def test_cli_conflicts_rejected_before_any_metadata_io(self):
        import compatibility_inventory as inventory
        conflicts = [['--unproposed']]
        for query in (['--rule-backlog'], ['--rule-page', 'src/tokens.md'], ['--rule', 'token']):
            for incompatible in ('--scope', '--contract', '--oracle-verifier', '--source-archive',
                                 '--submodule-proof', '--library-surface-provenance', '--obligation'):
                conflicts.append(query + [incompatible, 'absent'])
            conflicts.append(query + ['--obligation-backlog'])
            conflicts.append(query + ['--inventory', 'one', '--inventory', 'two'])
        for args in conflicts:
            with self.subTest(args=args), mock.patch.object(inventory, 'load', side_effect=AssertionError('metadata IO')), \
                    mock.patch.object(Path, 'read_bytes', side_effect=AssertionError('byte IO')), \
                    mock.patch.object(Path, 'read_text', side_effect=AssertionError('text IO')):
                status, out, err = self.cli(args)
                self.assertEqual(status, 2)
                self.assertEqual(out, '')
                self.assertTrue(err)

    def test_cli_script_import_boundary_reports_structured_errors(self):
        self.cli_inputs()
        for args in (['--rule', 'foreign.rule'], ['--rule-page', 'src/SUMMARY.md', '--rule', 'foreign.rule']):
            process = subprocess.run([sys.executable, str(BASE / 'compatibility_inventory.py'),
                                      '--root', str(self.root), *args],
                                     capture_output=True, text=True, check=False)
            self.assertEqual(process.returncode, 2)
            self.assertEqual(process.stdout, '')
            self.assertIsInstance(json.loads(process.stderr)['error'], str)
            self.assertNotIn('Traceback', process.stderr)
        (self.root / 'data/compiler-language-inventory.json').write_text('null')
        process = subprocess.run([sys.executable, str(BASE / 'compatibility_inventory.py'),
                                  '--root', str(self.root), '--rule-backlog'],
                                 capture_output=True, text=True, check=False)
        self.assertEqual(process.returncode, 2)
        self.assertEqual(process.stdout, '')
        self.assertIsInstance(json.loads(process.stderr)['error'], str)


    def expected_authored_proposals(self):
        expected = {}
        for profile, _ in reconciliation.PROPOSAL_COUNTS:
            selected, aliases, classes, authored = reconciliation.profile_inputs(profile)
            pages = {item['label']: entry['path'] for entry in self.index['pages'] +
                     self.index['unlinked_markdown_files'] if entry['path'] in selected
                     for item in entry['rules']}
            for line in authored.strip().splitlines():
                label, codes, rationale, links, positive, invalid, interaction = line.split('|')
                classifications = sorted(classes[code] for code in codes.split())
                expected.setdefault((pages[label], label), []).append({
                    'profile': profile, 'classifications': classifications,
                    'rationale': rationale,
                    'candidate_obligation_ids': sorted(aliases[name] for name in links.split()),
                    'relationship': ('proposed-partial-decomposition' if
                                     {'normative-semantic', 'grammar'} & set(classifications)
                                     else 'context-only'),
                    'review_status': 'unreviewed',
                    'missing_test_needs': {'positive': positive, 'invalid': invalid,
                                           'interaction': interaction}})
        return {identity: sorted(rows, key=lambda row: row['profile'])
                for identity, rows in expected.items()}

    def assert_proposal_query(self, result, **selection):
        expected = self.expected_authored_proposals()
        self.assertIs(result['include_proposals'], True)
        self.assertEqual(result['selected_authored_proposal_count'], sum(
            len(row['authored_proposals']) for row in result['rows']))
        ordinary = copy.deepcopy(result)
        ordinary.pop('include_proposals')
        ordinary.pop('selected_authored_proposal_count')
        for row in ordinary['rows']:
            details = row.pop('authored_proposals')
            self.assertEqual(details, expected.get((row['page'], row['rule']), []))
            self.assertEqual([detail['profile'] for detail in details], row['proposal_profiles'])
        # An additional opt-in warning must not mask any original limitations.
        baseline = self.query(**selection)
        self.assertTrue(set(baseline['limitations']) <= set(ordinary['limitations']))
        additions = set(ordinary['limitations']) - set(baseline['limitations'])
        self.assertTrue(additions, 'authored needs must not replace full profile requirements')
        warning = ' '.join(additions).lower()
        for word in ('profile', 'generate', 'validate', 'end-to-end'):
            self.assertIn(word, warning)
        ordinary['limitations'] = baseline['limitations']
        self.assertEqual(ordinary, baseline)
        self.assert_query(ordinary, **selection)

    def test_proposal_details_all_profiles_exact_authored_text_and_no_credit(self):
        result = self.query(include_proposals=True)
        self.assert_proposal_query(result)
        expected_count = sum(count for _, count in reconciliation.PROPOSAL_COUNTS)
        self.assertEqual(result['selected_authored_proposal_count'], expected_count)
        self.assertEqual({detail['profile'] for row in result['rows']
                          for detail in row['authored_proposals']},
                         {profile for profile, _ in reconciliation.PROPOSAL_COUNTS})
        self.assertEqual(result, self.query(include_proposals=True))

    def test_proposal_details_count_overlapping_profile_rows_not_unique_rules(self):
        original = reconciliation.profile_inputs
        counts = (*reconciliation.PROPOSAL_COUNTS, ('zz_tokens_overlap', 115))
        def inputs(profile):
            return original('tokens' if profile == 'zz_tokens_overlap' else profile)
        with mock.patch.object(reconciliation, 'PROPOSAL_COUNTS', counts), mock.patch.object(
                reconciliation, 'profile_inputs', side_effect=inputs):
            result = self.query(include_proposals=True)
            self.assert_proposal_query(result)
            self.assertEqual(result['selected_proposed_rule_identities'], 1030)
            self.assertEqual(result['selected_authored_proposal_count'], 1150)
            result = self.query(page='src/tokens.md', include_proposals=True)
            self.assertEqual(result['selected_proposed_rule_identities'], 115)
            self.assertEqual(result['selected_authored_proposal_count'], 230)
            self.assertTrue(all([detail['profile'] for detail in row['authored_proposals']] ==
                                ['tokens', 'zz_tokens_overlap'] for row in result['rows']))

    def test_existing_return_overlap_keeps_two_proposals_without_double_counting_rules(self):
        result = self.query(page='src/expressions/return-expr.md', include_proposals=True)
        self.assert_proposal_query(result, page='src/expressions/return-expr.md')
        self.assertEqual(result['selected_proposed_rule_identities'], 5)
        self.assertEqual(result['selected_authored_proposal_count'], 10)
        self.assertEqual(len(result['rows']), 5)
        for row in result['rows']:
            self.assertEqual([p['profile'] for p in row['authored_proposals']],
                             ['branching', 'transfers'])
            self.assertTrue(all(p['review_status'] == 'unreviewed'
                                for p in row['authored_proposals']))

    def test_proposal_details_explicit_false_preserves_default_bytes_and_schema(self):
        baseline = self.query()
        explicit = self.query(include_proposals=False)
        self.assertEqual(json.dumps(baseline, indent=2), json.dumps(explicit, indent=2))
        self.assertNotIn('include_proposals', baseline)
        self.assertNotIn('selected_authored_proposal_count', baseline)
        self.assertTrue(all('authored_proposals' not in row for row in baseline['rows']))

    def test_proposal_details_exact_page_rule_and_missing_filters(self):
        details = self.expected_authored_proposals()
        # Every authored profile is exercised through exact page and rule selection.
        representatives = {}
        for identity, proposals in details.items():
            for proposal in proposals:
                representatives.setdefault(proposal['profile'], identity)
        selections = []
        for page, rule in representatives.values():
            selections.extend(({'page': page}, {'rule': rule}, {'page': page, 'rule': rule}))
        selections.extend(({'missing_only': True}, {'page': 'src/SUMMARY.md'},
                           {'page': 'src/tokens.md', 'missing_only': True}))
        for selection in selections:
            with self.subTest(selection=selection):
                result = self.query(include_proposals=True, **selection)
                self.assert_proposal_query(result, **selection)
                if selection.get('missing_only'):
                    self.assertEqual(result['selected_authored_proposal_count'], 0)
                    self.assertTrue(all(row['authored_proposals'] == [] for row in result['rows']))

    def test_proposal_details_strict_boolean_rejected_before_io(self):
        import compatibility_inventory as inventory
        for value in (None, 0, 1, '', 'true', [], {}, (True,)):
            with self.subTest(value=value), contextlib.ExitStack() as stack:
                stack.enter_context(mock.patch.object(reconciliation, 'summarize_proposals',
                                   side_effect=AssertionError('summary before flag validation')))
                stack.enter_context(mock.patch.object(Path, 'read_bytes', side_effect=AssertionError('byte IO')))
                stack.enter_context(mock.patch.object(Path, 'read_text', side_effect=AssertionError('text IO')))
                with self.assertRaises(inventory.InvalidInventory):
                    self.query(include_proposals=value)

    def test_proposal_details_validate_all_authored_modules_before_filtering(self):
        import compatibility_inventory as inventory
        original = reconciliation.profile_inputs
        for profile, _ in reconciliation.PROPOSAL_COUNTS:
            selected, aliases, classes, authored = original(profile)
            lines = authored.strip().splitlines()
            fields = lines[0].split('|')
            mutations = ['\n'.join(lines[1:]), authored + '\n' + lines[0],
                         authored.replace(fields[0], 'foreign.rule', 1)]
            for column, replacement in ((1, 'UNKNOWN'), (3, 'foreign-alias'), (4, ''), (6, '')):
                malformed = list(fields)
                malformed[column] = replacement
                mutations.append('\n'.join(['|'.join(malformed), *lines[1:]]))
            for mutation in mutations:
                def inputs(name):
                    return (selected, aliases, classes, mutation) if name == profile else original(name)
                with self.subTest(profile=profile, mutation=mutation[:60]), mock.patch.object(
                        reconciliation, 'profile_inputs', side_effect=inputs):
                    for selection in ({'page': 'src/SUMMARY.md'}, {'missing_only': True}):
                        with self.assertRaises(inventory.InvalidInventory):
                            self.query(include_proposals=True, **selection)

    def test_proposal_details_metadata_only_no_source_receipts_or_execution(self):
        import compatibility_inventory as inventory
        expected = self.query(include_proposals=True)
        allowed = {self.index_path.resolve(), self.candidate_path.resolve()}
        original_bytes, original_text = Path.read_bytes, Path.read_text
        def read_bytes(path):
            self.assertIn(path.resolve(), allowed, 'unexpected nonmetadata read')
            return original_bytes(path)
        def read_text(path, *args, **kwargs):
            self.assertIn(path.resolve(), allowed, 'unexpected nonmetadata read')
            return original_text(path, *args, **kwargs)
        with contextlib.ExitStack() as stack:
            for module, name in ((reconciliation, 'build'), (reconciliation, 'validate'),
                                 (reconciliation, 'summarize_witness_sources'),
                                 (inventory, 'verify_reference_archive'), (inventory, 'read_verifier'),
                                 (subprocess, 'run')):
                stack.enter_context(mock.patch.object(module, name, side_effect=AssertionError(name)))
            stack.enter_context(mock.patch.object(Path, 'read_bytes', read_bytes))
            stack.enter_context(mock.patch.object(Path, 'read_text', read_text))
            self.assertEqual(self.query(include_proposals=True), expected)

    def test_cli_with_rule_proposals_exact_api_and_actual_script(self):
        self.cli_inputs()
        label = self.query(page='src/tokens.md')['rows'][0]['rule']
        for args, selection in ((['--rule-backlog'], {}),
                (['--rule-page', 'src/tokens.md'], {'page': 'src/tokens.md'}),
                (['--rule', label], {'rule': label}),
                (['--rule-page', 'src/tokens.md', '--rule', label],
                 {'page': 'src/tokens.md', 'rule': label}),
                (['--rule-backlog', '--unproposed'], {'missing_only': True})):
            with self.subTest(args=args):
                expected = self.query(include_proposals=True, **selection)
                status, out, err = self.cli([*args, '--with-rule-proposals'])
                self.assertEqual((status, err), (0, ''))
                self.assertEqual(json.loads(out), expected)
                process = subprocess.run([sys.executable, str(BASE / 'compatibility_inventory.py'),
                                          '--root', str(self.root), *args, '--with-rule-proposals'],
                                         capture_output=True, text=True, check=False)
                self.assertEqual((process.returncode, process.stderr), (0, ''))
                self.assertEqual(process.stdout, out)

    def test_cli_with_rule_proposals_conflicts_before_any_io(self):
        import compatibility_inventory as inventory
        conflicts = [[], ['--unproposed'], ['--obligation-backlog'], ['--obligation', 'absent'],
                     ['--with-partial-witnesses'], ['--observe-partial-witnesses']]
        for query in (['--rule-backlog'], ['--rule-page', 'src/tokens.md'], ['--rule', 'token']):
            for flag in ('--scope', '--contract', '--oracle-verifier', '--source-archive',
                         '--submodule-proof', '--library-surface-provenance', '--obligation',
                         '--inventory', '--reference-rules', '--evidence'):
                conflicts.append([*query, flag, 'absent'])
            for flag in ('--obligation-backlog', '--with-partial-witnesses', '--observe-partial-witnesses'):
                conflicts.append([*query, flag])
        for args in conflicts:
            with self.subTest(args=args), contextlib.ExitStack() as stack:
                for module, name in ((inventory, 'load'), (reconciliation, 'query_rule_backlog'),
                                     (Path, 'read_bytes'), (Path, 'read_text'), (Path, 'is_file')):
                    stack.enter_context(mock.patch.object(module, name, side_effect=AssertionError('IO: ' + name)))
                status, out, err = self.cli([*args, '--with-rule-proposals'])
                self.assertEqual(status, 2)
                self.assertEqual(out, '')
                self.assertTrue(err)


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


class ABIProfileTests(unittest.TestCase):
    """Synthetic source-only controls, never compiler execution evidence."""

    def setUp(self):
        import abi_reconciliation_proposals as proposals
        self.proposals = proposals
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / 'src').mkdir()
        self.candidate_path = self.root / 'candidate.json'
        self.candidate_path.write_text(json.dumps({
            'scope': SCOPE, 'sections': [{'obligations': [
                {'id': name} for name in sorted(set(proposals.ALIASES.values()))]}]}))
        labels = [line.split('|')[0] for line in proposals.ROWS.strip().splitlines()]
        text = ''.join('r[' + label + ']\nsynthetic ABI block\n' for label in labels)
        source = self.root / 'src/abi.md'
        source.write_text(text)
        self.index_path = self.root / 'index.json'
        self.index_path.write_text(json.dumps({
            'scope': SCOPE, 'pages': [
                {'path': 'src/abi.md', 'sha256': digest(source.read_bytes()),
                 'url': 'synthetic-abi', 'rules': parse_markdown(text)[0]},
                {'path': 'src/other.md', 'rules': [
                    {'label': 'other.' + str(i), 'line': i + 1} for i in range(2941)]}],
            'unlinked_markdown_files': [], 'counts': {'unique_rule_labels_total': 2965},
            'provenance': {'compiler_commit': '1' * 40, 'reference_commit': '2' * 40,
                           'source_archive_sha256': '3' * 64,
                           'candidate_inventory_sha256': digest(self.candidate_path.read_bytes())}}))
        self.data = self.build_abi()

    def build_abi(self):
        return build(self.index_path, self.candidate_path, self.root, profile='abi')

    def verify(self, data):
        return validate(data, self.index_path, self.candidate_path, self.root, profile='abi')

    rejected = ReconciliationTests.rejected

    def command(self, action, output):
        return [action, '--profile', 'abi', '--rule-index', str(self.index_path),
                '--inventory', str(self.candidate_path), '--source-root', str(self.root),
                '--artifact', str(output)]

    def test_all_24_labels_deterministic_full_universe_and_zero_credit(self):
        self.assertEqual(self.proposals.SELECTED, ('src/abi.md',))
        self.assertEqual(self.proposals.ALIASES, {'abi': 'reference-abi-abi-compatibility'})
        labels = [line.split('|')[0] for line in self.proposals.ROWS.strip().splitlines()]
        self.assertEqual(len(labels), 24)
        self.assertEqual(len(set(labels)), 24)
        self.assertEqual(self.data, self.build_abi())
        counts = self.verify(self.data)
        self.assertEqual((counts['selected_rule_rows'], counts['unselected_rule_identities'],
                          counts['global_rule_identities']), (24, 2941, 2965))
        self.assertEqual(counts['reviewed_rule_rows'], 0)
        self.assertEqual(counts['rows_with_exact_executable_evidence'], 0)
        self.assertEqual(counts['proposed_classification_counts'], {
            'normative-semantic': 15, 'cross-reference': 13, 'index': 6,
            'grammar': 3, 'edition-context': 3})
        self.assertEqual(self.data['purpose'], 'bounded-abi-rule-reconciliation-candidate')
        self.assertEqual([row['rule'] for row in self.data['rows']], labels)
        self.assertEqual({row['page'] for row in self.data['rows']}, {'src/abi.md'})
        self.assertTrue(all(row['review_status'] == 'unreviewed' for row in self.data['rows']))
        linked = {name for row in self.data['rows'] for name in row['candidate_obligation_ids']}
        self.assertEqual(linked, set(self.proposals.ALIASES.values()))
        raw = (self.root / 'src/abi.md').read_bytes()
        self.assertEqual(self.data['selected_pages'][0]['sha256'], digest(raw))
        rules = parse_markdown(raw.decode())[0]
        lines = raw.splitlines(keepends=True)
        for position, (rule, row) in enumerate(zip(rules, self.data['rows'])):
            end = rules[position + 1]['line'] - 1 if position + 1 < len(rules) else len(lines)
            self.assertEqual((row['line'], row['end_line']), (rule['line'], end))
            self.assertEqual(row['source_block_sha256'], digest(b''.join(lines[rule['line'] - 1:end])))

    def test_missing_duplicate_foreign_stale_and_false_credit_controls(self):
        mutations = [
            (lambda d: d['rows'].pop(), 'omitted selected'),
            (lambda d: d['rows'].append(copy.deepcopy(d['rows'][0])), 'duplicate rule'),
            (lambda d: d['rows'][0].update(rule='other.0'), 'foreign rule'),
            (lambda d: d['rows'][0].update(page='src/other.md'), 'foreign rule'),
            (lambda d: d['rows'][0].update(line=True), 'source block'),
            (lambda d: d['rows'][0].update(end_line=999), 'source block'),
            (lambda d: d['rows'][0].update(source_block_sha256='0' * 64), 'source block'),
            (lambda d: d['selected_pages'][0].update(sha256='0' * 64), 'page binding'),
            (lambda d: d['bindings'].update(rule_index_sha256='0' * 64), 'bindings'),
            (lambda d: d['bindings'].update(candidate_inventory_sha256='0' * 64), 'bindings'),
            (lambda d: d['rows'][0].update(candidate_obligation_ids=['foreign']), 'candidate obligation'),
            (lambda d: d['rows'][0]['candidate_obligation_ids'].append(d['rows'][0]['candidate_obligation_ids'][0]), 'candidate obligation'),
            (lambda d: d['rows'][0].update(classifications=['foreign']), 'classifications'),
            (lambda d: d['rows'][0].update(rationale='invented'), 'authored'),
            (lambda d: d['rows'][0].update(id='invented'), 'row ID'),
            (lambda d: d['rows'][0]['missing_test_needs'].__setitem__(0, 'positive: done'), 'authored'),
            (lambda d: d['rows'][0]['missing_test_needs'].pop(), 'test-needs'),
            (lambda d: d['rows'][0].update(review_status='reviewed'), 'reviewed row'),
            (lambda d: d.update(review_status='reviewed'), 'reviewed or foreign'),
            (lambda d: d['counts'].update(reviewed_rule_rows=1), 'counts or evidence'),
            (lambda d: d['counts'].update(reviewed_rule_rows=False), 'counts or evidence'),
            (lambda d: d['counts'].update(rows_with_exact_executable_evidence=1), 'counts or evidence'),
            (lambda d: d['counts'].update(global_rule_identities=24), 'counts or evidence'),
            (lambda d: d['unselected_rules'].pop(), 'unselected identities'),
            (lambda d: d['unselected_rules'].append(d['unselected_rules'][0]), 'unselected identities'),
            (lambda d: d['unselected_rules'][0].update(rule='foreign'), 'unselected identities'),
            (lambda d: d['limitations'].__setitem__(0, 'Full ABI support'), 'scope limitations'),
        ]
        for mutate, error in mutations:
            with self.subTest(error=error):
                self.rejected(mutate, error)
        changed = copy.deepcopy(self.data)
        changed['rows'][0].update(classifications=['normative-semantic'],
                                  relationship='proposed-partial-decomposition')
        changed['counts'] = reconciliation.compute_counts(changed['rows'], 2965, 'abi')
        with self.assertRaisesRegex(ValueError, 'authored'):
            self.verify(changed)

    def test_module_omission_duplicate_foreign_and_field_controls(self):
        original = self.proposals.ROWS.strip().splitlines()
        foreign = original[0].split('|')
        foreign[0] = 'foreign.rule'
        for rows, error in ((original[1:], 'omitted selected'),
                            (original + [original[0]], 'duplicate rule'),
                            (['|'.join(foreign)] + original[1:], 'foreign proposal'),
                            (original + ['extra'], 'seven explicit fields')):
            with self.subTest(error=error), mock.patch.object(self.proposals, 'ROWS', '\n'.join(rows)):
                with self.assertRaisesRegex(ValueError, error):
                    self.build_abi()

    def test_missing_input_files_are_not_silently_accepted(self):
        for path in (self.index_path, self.candidate_path, self.root / 'src/abi.md'):
            parked = path.with_suffix(path.suffix + '.parked')
            path.rename(parked)
            try:
                with self.subTest(path=path.name), self.assertRaises(FileNotFoundError):
                    self.build_abi()
            finally:
                parked.rename(path)

    def test_changed_input_bytes_and_index_controls(self):
        for path in (self.index_path, self.candidate_path, self.root / 'src/abi.md'):
            before = path.read_bytes()
            path.write_bytes(before + b'\n')
            try:
                with self.subTest(path=path.name), self.assertRaises(ValueError):
                    self.verify(self.data)
            finally:
                path.write_bytes(before)
        original = self.index_path.read_bytes()
        for mutate, error in (
                (lambda i: i['pages'].pop(0), 'index total mismatch'),
                (lambda i: i['pages'].append(copy.deepcopy(i['pages'][0])), 'duplicate index rule identity'),
                (lambda i: i['pages'][0]['rules'][0].update(line=2), 'labels/lines mismatch'),
                (lambda i: (i['pages'][1]['rules'].pop(),
                            i['counts'].update(unique_rule_labels_total=2964)), 'pinned denominator'),
                (lambda i: i['pages'][1]['rules'][0].update(label=i['pages'][0]['rules'][0]['label']), 'duplicate index rule label')):
            index = json.loads(original)
            mutate(index)
            self.index_path.write_text(json.dumps(index))
            try:
                with self.subTest(error=error), self.assertRaisesRegex(ValueError, error):
                    self.build_abi()
            finally:
                self.index_path.write_bytes(original)

    def test_cli_generate_validate_exclusive_and_cross_profile_controls(self):
        output = self.root / 'abi.json'
        with contextlib.redirect_stdout(io.StringIO()):
            main(self.command('generate', output))
            main(self.command('validate', output))
        raw = output.read_bytes()
        self.assertEqual(json.loads(raw), self.data)
        for target in (output, self.index_path, self.candidate_path, self.root / 'src/abi.md'):
            before = target.read_bytes()
            with self.assertRaises(FileExistsError):
                main(self.command('generate', target))
            self.assertEqual(target.read_bytes(), before)
        for profile in ('tokens', 'const-eval', 'unsafe'):
            with self.subTest(profile=profile), self.assertRaisesRegex(ValueError, 'purpose mismatch'):
                validate(self.data, self.index_path, self.candidate_path, self.root, profile)

    def test_unsupported_witness_and_evidence_combinations_reject_before_io(self):
        output = self.root / 'absent'
        actions = ('generate-witness-links', 'build-witness-links',
                   'validate-witness-links', 'assess-witness-links')
        extras = ([], ['--repo-root', str(self.root)], ['--evidence', str(self.root)],
                  ['--repo-root', str(self.root), '--evidence', str(self.root)])
        cases = [(action, extra) for action in actions for extra in extras]
        cases += [(action, extra) for action in ('generate', 'validate') for extra in extras[1:]]
        for action, extra in cases:
            with self.subTest(action=action, extra=extra), \
                 mock.patch.object(Path, 'read_bytes', side_effect=AssertionError('premature read')), \
                 mock.patch.object(Path, 'read_text', side_effect=AssertionError('premature read')), \
                 mock.patch.object(Path, 'open', side_effect=AssertionError('premature open')), \
                 mock.patch.object(Path, 'is_symlink', side_effect=AssertionError('premature stat')):
                with self.assertRaisesRegex(ValueError, 'abi profile'):
                    main(self.command(action, output) + extra)
        with mock.patch.object(Path, 'read_bytes', side_effect=AssertionError('premature read')), \
             mock.patch.object(Path, 'read_text', side_effect=AssertionError('premature read')), \
             mock.patch.object(Path, 'open', side_effect=AssertionError('premature open')), \
             mock.patch.object(Path, 'is_symlink', side_effect=AssertionError('premature stat')):
            with self.assertRaisesRegex(ValueError, 'abi profile'):
                reconciliation.build_witness_links(self.index_path, self.candidate_path,
                                                   self.root, self.root, 'abi')
        self.assertFalse(output.exists())


class PinnedABIProfileTests(unittest.TestCase):
    def test_actual_24_labels_independent_blocks_and_unchanged_legacy_artifacts(self):
        source = os.environ.get('JUSTRUST_WITNESS_REFERENCE_ROOT')
        if not source:
            self.skipTest('set JUSTRUST_WITNESS_REFERENCE_ROOT for pinned-source structural checks')
        import abi_reconciliation_proposals as proposals
        repo = Path(os.environ.get('JUSTRUST_WITNESS_REPO_ROOT', BASE.parent))
        index = repo / 'data/compiler-language-reference-rules.json'
        inventory = repo / 'data/compiler-language-inventory.json'
        source = Path(source)
        data = build(index, inventory, source, 'abi')
        counts = validate(data, index, inventory, source, 'abi')
        self.assertEqual((counts['selected_rule_rows'], counts['unselected_rule_identities'],
                          counts['global_rule_identities']), (24, 2941, 2965))
        self.assertEqual(counts['reviewed_rule_rows'], 0)
        self.assertEqual(counts['rows_with_exact_executable_evidence'], 0)
        self.assertEqual(data, build(index, inventory, source, 'abi'))
        self.assertEqual(data['bindings']['reference_commit'], '86635e30bf861a038dc197d7e16fd09e7e514e7a')
        self.assertEqual([page['path'] for page in data['selected_pages']], ['src/abi.md'])
        raw = (source / 'src/abi.md').read_bytes()
        self.assertEqual(data['selected_pages'][0]['sha256'], digest(raw))
        rules = parse_markdown(raw.decode())[0]
        self.assertEqual(len(rules), 24)
        self.assertEqual([rule['label'] for rule in rules],
                         [line.split('|')[0] for line in proposals.ROWS.strip().splitlines()])
        self.assertEqual([row['rule'] for row in data['rows']], [rule['label'] for rule in rules])
        lines = raw.splitlines(keepends=True)
        for position, (rule, row) in enumerate(zip(rules, data['rows'])):
            end = rules[position + 1]['line'] - 1 if position + 1 < len(rules) else len(lines)
            self.assertEqual((row['page'], row['line'], row['end_line']), ('src/abi.md', rule['line'], end))
            self.assertEqual(row['source_block_sha256'], digest(b''.join(lines[rule['line'] - 1:end])))
        original = json.loads(index.read_bytes())
        universe = {(page['path'], rule['label']) for page in
                    original['pages'] + original['unlinked_markdown_files'] for rule in page['rules']}
        selected = {(row['page'], row['rule']) for row in data['rows']}
        unselected = {(row['page'], row['rule']) for row in data['unselected_rules']}
        self.assertFalse(selected & unselected)
        self.assertEqual(selected | unselected, universe)
        with mock.patch.dict(sys.modules, {'abi_reconciliation_proposals': None}):
            for profile, expected in (
                    ('tokens', 'ea7fa27afdf901b91464ead14c54883debd091819c25bf02870e7a434ae61785'),
                    ('const-eval', '0ce718c6e2834b6c518ff43f2fed2b5de72fd75eccf01d4611c3217cbdf053c2'),
                    ('unsafe', 'bc752efc1f0172cd85dad9399a1309c06489b3b0e4d4f74884c8bc1ef58ebb58')):
                legacy = build(index, inventory, source, profile)
                self.assertEqual(digest((json.dumps(legacy, indent=2) + '\n').encode()), expected)
        with tempfile.TemporaryDirectory() as temp:
            artifact = Path(temp) / 'abi.json'
            command = ['--profile', 'abi', '--rule-index', str(index), '--inventory', str(inventory),
                       '--source-root', str(source), '--artifact', str(artifact)]
            with contextlib.redirect_stdout(io.StringIO()):
                main(['generate', *command])
                main(['validate', *command])
            self.assertEqual(artifact.read_bytes(), (json.dumps(data, indent=2) + '\n').encode())


class VisibilityProfileTests(unittest.TestCase):
    """Synthetic source-only controls, never compiler execution evidence."""

    def setUp(self):
        import visibility_reconciliation_proposals as proposals
        self.proposals = proposals
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / 'src').mkdir()
        self.candidate_path = self.root / 'candidate.json'
        self.candidate_path.write_text(json.dumps({
            'scope': SCOPE, 'sections': [{'obligations': [
                {'id': name} for name in sorted(set(proposals.ALIASES.values()))]}]}))
        labels = [line.split('|')[0] for line in proposals.ROWS.strip().splitlines()]
        text = ''.join('r[' + label + ']\nsynthetic Visibility block\n' for label in labels)
        source = self.root / 'src/visibility-and-privacy.md'
        source.write_text(text)
        self.index_path = self.root / 'index.json'
        self.index_path.write_text(json.dumps({
            'scope': SCOPE, 'pages': [
                {'path': 'src/visibility-and-privacy.md', 'sha256': digest(source.read_bytes()),
                 'url': 'synthetic-visibility', 'rules': parse_markdown(text)[0]},
                {'path': 'src/other.md', 'rules': [
                    {'label': 'other.' + str(i), 'line': i + 1} for i in range(2947)]}],
            'unlinked_markdown_files': [], 'counts': {'unique_rule_labels_total': 2965},
            'provenance': {'compiler_commit': '1' * 40, 'reference_commit': '2' * 40,
                           'source_archive_sha256': '3' * 64,
                           'candidate_inventory_sha256': digest(self.candidate_path.read_bytes())}}))
        self.data = self.build_visibility()

    def build_visibility(self):
        return build(self.index_path, self.candidate_path, self.root, profile='visibility')

    def verify(self, data):
        return validate(data, self.index_path, self.candidate_path, self.root, profile='visibility')

    rejected = ReconciliationTests.rejected

    def command(self, action, output):
        return [action, '--profile', 'visibility', '--rule-index', str(self.index_path),
                '--inventory', str(self.candidate_path), '--source-root', str(self.root),
                '--artifact', str(output)]

    def test_all_18_labels_deterministic_full_universe_and_zero_credit(self):
        self.assertEqual(self.proposals.SELECTED, ('src/visibility-and-privacy.md',))
        labels = [line.split('|')[0] for line in self.proposals.ROWS.strip().splitlines()]
        self.assertEqual(len(labels), 18)
        self.assertEqual(len(set(labels)), 18)
        self.assertEqual(self.data, self.build_visibility())
        counts = self.verify(self.data)
        self.assertEqual((counts['selected_rule_rows'], counts['unselected_rule_identities'],
                          counts['global_rule_identities']), (18, 2947, 2965))
        self.assertEqual(counts['reviewed_rule_rows'], 0)
        self.assertEqual(counts['rows_with_exact_executable_evidence'], 0)
        self.assertEqual(self.data['purpose'], 'bounded-visibility-rule-reconciliation-candidate')
        self.assertEqual([row['rule'] for row in self.data['rows']], labels)
        self.assertEqual({row['page'] for row in self.data['rows']}, {'src/visibility-and-privacy.md'})
        self.assertTrue(all(row['review_status'] == 'unreviewed' for row in self.data['rows']))
        linked = {name for row in self.data['rows'] for name in row['candidate_obligation_ids']}
        self.assertEqual(linked, set(self.proposals.ALIASES.values()))
        raw = (self.root / 'src/visibility-and-privacy.md').read_bytes()
        self.assertEqual(self.data['selected_pages'][0]['sha256'], digest(raw))
        rules = parse_markdown(raw.decode())[0]
        lines = raw.splitlines(keepends=True)
        for position, (rule, row) in enumerate(zip(rules, self.data['rows'])):
            end = rules[position + 1]['line'] - 1 if position + 1 < len(rules) else len(lines)
            self.assertEqual((row['line'], row['end_line']), (rule['line'], end))
            self.assertEqual(row['source_block_sha256'], digest(b''.join(lines[rule['line'] - 1:end])))

    def test_every_authored_field_is_preserved_exactly(self):
        authored = [line.split('|') for line in self.proposals.ROWS.strip().splitlines()]
        for fields, row in zip(authored, self.data['rows']):
            label, classes, rationale, aliases, positive, invalid, interaction = fields
            with self.subTest(rule=label):
                classifications = [self.proposals.CLASS_NAMES[c] for c in classes.split()]
                self.assertEqual(row['rule'], label)
                self.assertEqual(row['classifications'], classifications)
                self.assertEqual(row['rationale'], rationale)
                self.assertEqual(row['candidate_obligation_ids'],
                                 [self.proposals.ALIASES[a] for a in aliases.split()])
                self.assertEqual(row['relationship'], 'proposed-partial-decomposition'
                                 if {'normative-semantic', 'grammar'} & set(classifications)
                                 else 'context-only')
                self.assertEqual(row['missing_test_needs'], [
                    'positive: Missing direct exact test link. ' + positive,
                    'invalid: Missing direct exact test link. ' + invalid,
                    'interaction: Missing direct exact test link. ' + interaction,
                    self.proposals.END_TO_END])
        self.assertEqual(self.data['limitations'], list(self.proposals.LIMITATIONS))

    def test_missing_duplicate_foreign_stale_and_false_credit_controls(self):
        mutations = [
            (lambda d: d['rows'].pop(), 'omitted selected'),
            (lambda d: d['rows'].reverse(), 'nondeterministic row order'),
            (lambda d: d['rows'].append(copy.deepcopy(d['rows'][0])), 'duplicate rule'),
            (lambda d: d['rows'][0].update(rule='other.0'), 'foreign rule'),
            (lambda d: d['rows'][0].update(page='src/other.md'), 'foreign rule'),
            (lambda d: d['rows'][0].update(line=True), 'source block'),
            (lambda d: d['rows'][0].update(end_line=999), 'source block'),
            (lambda d: d['rows'][0].update(source_block_sha256='0' * 64), 'source block'),
            (lambda d: d['selected_pages'][0].update(sha256='0' * 64), 'page binding'),
            (lambda d: d['bindings'].update(rule_index_sha256='0' * 64), 'bindings'),
            (lambda d: d['bindings'].update(reference_commit='0' * 40), 'bindings'),
            (lambda d: d['bindings'].update(candidate_inventory_sha256='0' * 64), 'bindings'),
            (lambda d: d['rows'][0].update(candidate_obligation_ids=['foreign']), 'candidate obligation'),
            (lambda d: d['rows'][0].update(candidate_obligation_ids=[]), 'candidate obligation'),
            (lambda d: d['rows'][0]['candidate_obligation_ids'].append(d['rows'][0]['candidate_obligation_ids'][0]), 'candidate obligation'),
            (lambda d: d['rows'][0].update(classifications=['foreign']), 'classifications'),
            (lambda d: d['rows'][0].update(rationale='invented'), 'authored'),
            (lambda d: d['rows'][0].update(id='invented'), 'row ID'),
            (lambda d: d['rows'][0]['missing_test_needs'].__setitem__(0, 'positive: done'), 'authored'),
            (lambda d: d['rows'][0]['missing_test_needs'].pop(), 'test-needs'),
            (lambda d: d['rows'][0].update(review_status='reviewed'), 'reviewed row'),
            (lambda d: d.update(review_status='reviewed'), 'reviewed or foreign'),
            (lambda d: d['counts'].update(reviewed_rule_rows=1), 'counts or evidence'),
            (lambda d: d['counts'].update(reviewed_rule_rows=False), 'counts or evidence'),
            (lambda d: d['counts'].update(rows_with_exact_executable_evidence=1), 'counts or evidence'),
            (lambda d: d['counts'].update(global_rule_identities=18), 'counts or evidence'),
            (lambda d: d['unselected_rules'].pop(), 'unselected identities'),
            (lambda d: d['unselected_rules'].reverse(), 'unselected identities'),
            (lambda d: d['unselected_rules'].append(d['unselected_rules'][0]), 'unselected identities'),
            (lambda d: d['unselected_rules'][0].update(rule='foreign'), 'unselected identities'),
            (lambda d: d['limitations'].__setitem__(0, 'Full Visibility support'), 'scope limitations'),
        ]
        for mutate, error in mutations:
            with self.subTest(error=error):
                self.rejected(mutate, error)
        changed = copy.deepcopy(self.data)
        changed['rows'][0].update(classifications=['normative-semantic'],
                                  relationship='proposed-partial-decomposition')
        changed['counts'] = reconciliation.compute_counts(changed['rows'], 2965, 'visibility')
        with self.assertRaisesRegex(ValueError, 'authored'):
            self.verify(changed)

    def test_module_omission_duplicate_foreign_and_field_controls(self):
        original = self.proposals.ROWS.strip().splitlines()
        foreign = original[0].split('|')
        foreign[0] = 'foreign.rule'
        for rows, error in ((original[1:], 'omitted selected'),
                            (original + [original[0]], 'duplicate rule'),
                            (['|'.join(foreign)] + original[1:], 'foreign proposal'),
                            (original + ['extra'], 'seven explicit fields')):
            with self.subTest(error=error), mock.patch.object(self.proposals, 'ROWS', '\n'.join(rows)):
                with self.assertRaisesRegex(ValueError, error):
                    self.build_visibility()

    def test_missing_input_files_are_not_silently_accepted(self):
        for path in (self.index_path, self.candidate_path, self.root / 'src/visibility-and-privacy.md'):
            parked = path.with_suffix(path.suffix + '.parked')
            path.rename(parked)
            try:
                with self.subTest(path=path.name), self.assertRaises(FileNotFoundError):
                    self.build_visibility()
            finally:
                parked.rename(path)

    def test_changed_input_bytes_and_index_controls(self):
        for path in (self.index_path, self.candidate_path, self.root / 'src/visibility-and-privacy.md'):
            before = path.read_bytes()
            path.write_bytes(before + b'\n')
            try:
                with self.subTest(path=path.name), self.assertRaises(ValueError):
                    self.verify(self.data)
            finally:
                path.write_bytes(before)
        original = self.index_path.read_bytes()
        for mutate, error in (
                (lambda i: i['pages'].pop(0), 'index total mismatch'),
                (lambda i: i['pages'].append(copy.deepcopy(i['pages'][0])), 'duplicate index rule identity'),
                (lambda i: i['pages'][0]['rules'][0].update(line=2), 'labels/lines mismatch'),
                (lambda i: (i['pages'][1]['rules'].pop(),
                            i['counts'].update(unique_rule_labels_total=2964)), 'pinned denominator'),
                (lambda i: i['pages'][1]['rules'][0].update(label=i['pages'][0]['rules'][0]['label']), 'duplicate index rule label')):
            index = json.loads(original)
            mutate(index)
            self.index_path.write_text(json.dumps(index))
            try:
                with self.subTest(error=error), self.assertRaisesRegex(ValueError, error):
                    self.build_visibility()
            finally:
                self.index_path.write_bytes(original)

    def test_cli_generate_validate_exclusive_and_cross_profile_controls(self):
        output = self.root / 'visibility.json'
        with contextlib.redirect_stdout(io.StringIO()):
            main(self.command('generate', output))
            main(self.command('validate', output))
        raw = output.read_bytes()
        self.assertEqual(json.loads(raw), self.data)
        for target in (output, self.index_path, self.candidate_path, self.root / 'src/visibility-and-privacy.md'):
            before = target.read_bytes()
            with self.assertRaises(FileExistsError):
                main(self.command('generate', target))
            self.assertEqual(target.read_bytes(), before)
        for profile in ('tokens', 'const-eval', 'unsafe', 'abi'):
            with self.subTest(profile=profile), self.assertRaisesRegex(ValueError, 'purpose mismatch'):
                validate(self.data, self.index_path, self.candidate_path, self.root, profile)

    def test_unsupported_witness_and_evidence_combinations_reject_before_io(self):
        output = self.root / 'absent'
        actions = ('generate-witness-links', 'build-witness-links',
                   'validate-witness-links', 'assess-witness-links')
        extras = ([], ['--repo-root', str(self.root)], ['--evidence', str(self.root)],
                  ['--repo-root', str(self.root), '--evidence', str(self.root)])
        cases = [(action, extra) for action in actions for extra in extras]
        cases += [(action, extra) for action in ('generate', 'validate') for extra in extras[1:]]
        for action, extra in cases:
            with self.subTest(action=action, extra=extra), \
                 mock.patch.object(Path, 'read_bytes', side_effect=AssertionError('premature read')), \
                 mock.patch.object(Path, 'read_text', side_effect=AssertionError('premature read')), \
                 mock.patch.object(Path, 'open', side_effect=AssertionError('premature open')), \
                 mock.patch.object(Path, 'is_symlink', side_effect=AssertionError('premature stat')):
                with self.assertRaisesRegex(ValueError, 'visibility profile'):
                    main(self.command(action, output) + extra)
        with mock.patch.object(Path, 'read_bytes', side_effect=AssertionError('premature read')), \
             mock.patch.object(Path, 'read_text', side_effect=AssertionError('premature read')), \
             mock.patch.object(Path, 'open', side_effect=AssertionError('premature open')), \
             mock.patch.object(Path, 'is_symlink', side_effect=AssertionError('premature stat')):
            calls = (
                lambda: reconciliation.build_witness_links(
                    self.index_path, self.candidate_path, self.root, self.root, 'visibility'),
                lambda: reconciliation.validate_witness_links(
                    {}, self.index_path, self.candidate_path, self.root, self.root, 'visibility'),
                lambda: reconciliation.assess_witness_links(
                    {}, self.index_path, self.candidate_path, self.root, self.root, (), 'visibility'),
            )
            for call in calls:
                with self.subTest(api=call), self.assertRaisesRegex(ValueError, 'visibility profile'):
                    call()
        self.assertFalse(output.exists())


class PinnedVisibilityProfileTests(unittest.TestCase):
    def test_actual_18_labels_independent_blocks_and_unchanged_legacy_artifacts(self):
        source = os.environ.get('JUSTRUST_WITNESS_REFERENCE_ROOT')
        if not source:
            self.skipTest('set JUSTRUST_WITNESS_REFERENCE_ROOT for pinned-source structural checks')
        import visibility_reconciliation_proposals as proposals
        repo = Path(os.environ.get('JUSTRUST_WITNESS_REPO_ROOT', BASE.parent))
        index = repo / 'data/compiler-language-reference-rules.json'
        inventory = repo / 'data/compiler-language-inventory.json'
        source = Path(source)
        data = build(index, inventory, source, 'visibility')
        counts = validate(data, index, inventory, source, 'visibility')
        self.assertEqual((counts['selected_rule_rows'], counts['unselected_rule_identities'],
                          counts['global_rule_identities']), (18, 2947, 2965))
        self.assertEqual(counts['reviewed_rule_rows'], 0)
        self.assertEqual(counts['rows_with_exact_executable_evidence'], 0)
        self.assertEqual(data, build(index, inventory, source, 'visibility'))
        self.assertEqual(data['bindings']['reference_commit'], '86635e30bf861a038dc197d7e16fd09e7e514e7a')
        self.assertEqual([page['path'] for page in data['selected_pages']], ['src/visibility-and-privacy.md'])
        raw = (source / 'src/visibility-and-privacy.md').read_bytes()
        self.assertEqual(data['selected_pages'][0]['sha256'], digest(raw))
        rules = parse_markdown(raw.decode())[0]
        self.assertEqual([rule['label'] for rule in rules], [
            'vis', 'vis.syntax', 'vis.intro', 'vis.name-hierarchy', 'vis.privacy',
            'vis.default', 'vis.access', 'vis.use', 'vis.scoped', 'vis.scoped.intro',
            'vis.scoped.in', 'vis.scoped.crate', 'vis.scoped.super', 'vis.scoped.self',
            'vis.scoped.edition2018', 'vis.reexports', 'vis.reexports.intro',
            'vis.reexports.private-item'])
        self.assertEqual([rule['label'] for rule in rules],
                         [line.split('|')[0] for line in proposals.ROWS.strip().splitlines()])
        self.assertEqual([row['rule'] for row in data['rows']], [rule['label'] for rule in rules])
        lines = raw.splitlines(keepends=True)
        for position, (rule, row) in enumerate(zip(rules, data['rows'])):
            end = rules[position + 1]['line'] - 1 if position + 1 < len(rules) else len(lines)
            self.assertEqual((row['page'], row['line'], row['end_line']), ('src/visibility-and-privacy.md', rule['line'], end))
            self.assertEqual(row['source_block_sha256'], digest(b''.join(lines[rule['line'] - 1:end])))
        original = json.loads(index.read_bytes())
        universe = {(page['path'], rule['label']) for page in
                    original['pages'] + original['unlinked_markdown_files'] for rule in page['rules']}
        selected = {(row['page'], row['rule']) for row in data['rows']}
        unselected = {(row['page'], row['rule']) for row in data['unselected_rules']}
        self.assertFalse(selected & unselected)
        self.assertEqual(selected | unselected, universe)
        with mock.patch.dict(sys.modules, {'visibility_reconciliation_proposals': None}):
            for profile, expected in (
                    ('tokens', 'ea7fa27afdf901b91464ead14c54883debd091819c25bf02870e7a434ae61785'),
                    ('const-eval', '0ce718c6e2834b6c518ff43f2fed2b5de72fd75eccf01d4611c3217cbdf053c2'),
                    ('unsafe', 'bc752efc1f0172cd85dad9399a1309c06489b3b0e4d4f74884c8bc1ef58ebb58'),
                    ('abi', 'c4e561a9a78002281c34cbc35fe7fcaee3f29e82c659b718844504514ac1d317')):
                legacy = build(index, inventory, source, profile)
                self.assertEqual(digest((json.dumps(legacy, indent=2) + '\n').encode()), expected)
        for profile in ('tokens', 'const-eval', 'unsafe', 'abi'):
            legacy = build(index, inventory, source, profile)
            with self.subTest(profile=profile), self.assertRaisesRegex(ValueError, 'purpose mismatch'):
                validate(legacy, index, inventory, source, 'visibility')
        with tempfile.TemporaryDirectory() as temp:
            artifact = Path(temp) / 'visibility.json'
            command = ['--profile', 'visibility', '--rule-index', str(index), '--inventory', str(inventory),
                       '--source-root', str(source), '--artifact', str(artifact)]
            with contextlib.redirect_stdout(io.StringIO()):
                main(['generate', *command])
                main(['validate', *command])
            self.assertEqual(artifact.read_bytes(), (json.dumps(data, indent=2) + '\n').encode())


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
                         'cbd26b9353c21759910f4944ff2db793a92f1e120947680b67bbb2abf8e1d86d')
        self.assertEqual(data, reconciliation.build_witness_links(index, inventory, Path(source),
                                                                  repo, profile='tokens'))
        observations = reconciliation.assess_witness_links(data, index, inventory, Path(source), repo)
        self.assertEqual(digest(reconciliation.witness_bytes(observations)),
                         'c3fa23801dfbd4f7fe35f68f3fb86693b37666b38be363581bd5bdeed3089273')
        self.assertEqual(data['source_contract'], WITNESS_SOURCE_CONTRACT)
        self.assertEqual(reconciliation.validate_witness_links(data, index, inventory, Path(source), repo), data['summary'])
        self.assertEqual(data['reconciliation'], build(index, inventory, Path(source)))
        self.assertEqual(digest((json.dumps(data['reconciliation'], indent=2) + '\n').encode()),
                         'ea7fa27afdf901b91464ead14c54883debd091819c25bf02870e7a434ae61785')
        self.assertEqual([link['test'] for link in data['links']], sorted(row[0] for row in DIRECT_WITNESSES))
        for item in data['source_files']:
            self.assertEqual(item['sha256'], digest((repo / item['path']).read_bytes()))


class PinnedBranchingProfileTests(unittest.TestCase):
    def setUp(self):
        source = os.environ.get('JUSTRUST_WITNESS_REFERENCE_ROOT')
        if not source:
            self.skipTest('set JUSTRUST_WITNESS_REFERENCE_ROOT for pinned-source structural checks')
        import branching_reconciliation_proposals as proposals
        self.proposals = proposals
        self.source = Path(source)
        repo = Path(os.environ.get('JUSTRUST_WITNESS_REPO_ROOT', BASE.parent))
        self.index = repo / 'data/compiler-language-reference-rules.json'
        self.inventory = repo / 'data/compiler-language-inventory.json'

    def build_branching(self):
        return build(self.index, self.inventory, self.source, 'branching')

    def test_actual_58_labels_hashes_blocks_and_zero_credit(self):
        data = self.build_branching()
        counts = validate(data, self.index, self.inventory, self.source, 'branching')
        self.assertEqual((counts['selected_rule_rows'], counts['unselected_rule_identities'],
                          counts['global_rule_identities']), (58, 2907, 2965))
        self.assertEqual(counts['reviewed_rule_rows'], 0)
        self.assertEqual(counts['rows_with_exact_executable_evidence'], 0)
        self.assertEqual(data, self.build_branching())
        self.assertEqual([p['path'] for p in data['selected_pages']], list(self.proposals.SELECTED))
        labels = []
        for page, count in zip(self.proposals.SELECTED, (19, 34, 5)):
            raw = (self.source / page).read_bytes()
            self.assertEqual(digest(raw), self.proposals.PAGE_SHA256[page])
            rules = parse_markdown(raw.decode())[0]
            self.assertEqual(len(rules), count)
            labels.extend(rule['label'] for rule in rules)
            lines = raw.splitlines(keepends=True)
            rows = [row for row in data['rows'] if row['page'] == page]
            self.assertEqual(len(rows), count)
            for position, (rule, row) in enumerate(zip(rules, rows)):
                end = rules[position + 1]['line'] - 1 if position + 1 < count else len(lines)
                self.assertEqual((row['rule'], row['line'], row['end_line']),
                                 (rule['label'], rule['line'], end))
                self.assertEqual(row['source_block_sha256'],
                                 digest(b''.join(lines[rule['line'] - 1:end])))
        proposal_rows = [line.split('|') for line in self.proposals.ROWS.strip().splitlines()]
        self.assertEqual(labels, [row[0] for row in proposal_rows])
        self.assertTrue(all(len(row) == 7 for row in proposal_rows))
        self.assertTrue(all(need.startswith('Missing ') for row in proposal_rows for need in row[4:]))
        original = json.loads(self.index.read_bytes())
        universe = {(page['path'], rule['label']) for page in
                    original['pages'] + original['unlinked_markdown_files'] for rule in page['rules']}
        selected = {(row['page'], row['rule']) for row in data['rows']}
        unselected = {(row['page'], row['rule']) for row in data['unselected_rules']}
        self.assertFalse(selected & unselected)
        self.assertEqual(selected | unselected, universe)

    def test_missing_duplicate_foreign_proposals_rejected(self):
        original = self.proposals.ROWS.strip().splitlines()
        foreign = original[0].split('|')
        foreign[0] = 'foreign.branching'
        for rows in (original[:-1], original + [original[0]], ['|'.join(foreign)] + original[1:]):
            with self.subTest(rows=len(rows)), mock.patch.object(self.proposals, 'ROWS', '\n'.join(rows)):
                with self.assertRaises(ValueError):
                    self.build_branching()

    def test_line_and_block_tampering_rejected(self):
        original = self.build_branching()
        for field, value in (('line', 99999), ('end_line', 99999), ('source_block_sha256', '0' * 64)):
            data = copy.deepcopy(original)
            data['rows'][0][field] = value
            with self.subTest(field=field), self.assertRaises(ValueError):
                validate(data, self.index, self.inventory, self.source, 'branching')

    def test_legacy_token_const_bytes_unchanged(self):
        for profile, expected in (
                ('tokens', 'ea7fa27afdf901b91464ead14c54883debd091819c25bf02870e7a434ae61785'),
                ('const-eval', '0ce718c6e2834b6c518ff43f2fed2b5de72fd75eccf01d4611c3217cbdf053c2')):
            legacy = build(self.index, self.inventory, self.source, profile)
            self.assertEqual(digest((json.dumps(legacy, indent=2) + '\n').encode()), expected)

    def test_source_only_actions_rejected_before_io(self):
        args = ['--profile', 'branching', '--rule-index', '/absent/index',
                '--inventory', '/absent/inventory', '--source-root', '/absent/source',
                '--artifact', '/absent/artifact']
        for action in ('generate-witness-links', 'build-witness-links',
                       'validate-witness-links', 'assess-witness-links'):
            with self.subTest(action=action), \
                    mock.patch.object(Path, 'read_bytes', side_effect=AssertionError('unexpected IO')), \
                    mock.patch.object(Path, 'read_text', side_effect=AssertionError('unexpected IO')), \
                    mock.patch.object(Path, 'open', side_effect=AssertionError('unexpected IO')):
                with self.assertRaisesRegex(ValueError, 'generate/validate only'):
                    main([action, *args])


class PinnedDestructorsProfileTests(unittest.TestCase):
    """Actual pinned source boundaries, not Rust execution or review evidence."""

    def setUp(self):
        source = os.environ.get('JUSTRUST_WITNESS_REFERENCE_ROOT')
        if not source:
            self.skipTest('set JUSTRUST_WITNESS_REFERENCE_ROOT for pinned-source structural checks')
        import destructors_reconciliation_proposals as proposals
        self.proposals = proposals
        self.source = Path(source)
        repo = Path(os.environ.get('JUSTRUST_WITNESS_REPO_ROOT', BASE.parent))
        self.index = repo / 'data/compiler-language-reference-rules.json'
        self.inventory = repo / 'data/compiler-language-inventory.json'

    def build_destructors(self):
        return build(self.index, self.inventory, self.source, 'destructors')

    def verify(self, data):
        return validate(data, self.index, self.inventory, self.source, 'destructors')

    def test_exact_50_source_rows_page_blocks_full_union_and_zero_credit(self):
        data = self.build_destructors()
        counts = self.verify(data)
        self.assertEqual((counts['selected_rule_rows'], counts['unselected_rule_identities'],
                          counts['global_rule_identities']), (50, 2915, 2965))
        self.assertEqual(counts['reviewed_rule_rows'], 0)
        self.assertEqual(counts['rows_with_exact_executable_evidence'], 0)
        self.assertEqual(data, self.build_destructors())
        self.assertEqual(data['purpose'], 'bounded-destructors-rule-reconciliation-candidate')
        self.assertEqual(data['bindings']['reference_commit'],
                         '86635e30bf861a038dc197d7e16fd09e7e514e7a')
        self.assertEqual(self.proposals.SELECTED, ('src/destructors.md',))
        self.assertEqual([page['path'] for page in data['selected_pages']], ['src/destructors.md'])
        raw = (self.source / 'src/destructors.md').read_bytes()
        expected_hash = 'dc924a9f95b445d080d06c82a0ec3610989beb2b4c72ec1298d8b8fbe6899ef3'
        self.assertEqual(digest(raw), expected_hash)
        self.assertEqual(data['selected_pages'][0]['sha256'], expected_hash)
        lines = raw.splitlines(keepends=True)
        self.assertEqual(len(lines), 707)
        rules = parse_markdown(raw.decode())[0]
        self.assertEqual(len(rules), 50)
        self.assertEqual(len({rule['label'] for rule in rules}), 50)
        self.assertEqual([row['rule'] for row in data['rows']], [rule['label'] for rule in rules])
        authored = [line.split('|') for line in self.proposals.ROWS.strip().splitlines()]
        self.assertEqual([row[0] for row in authored], [rule['label'] for rule in rules])
        self.assertTrue(all(len(row) == 7 for row in authored))
        self.assertTrue(all(need.startswith('Missing ') for row in authored for need in row[4:]))
        candidates = {item['id'] for section in json.loads(self.inventory.read_bytes())['sections']
                      for item in section['obligations']}
        for position, (rule, row) in enumerate(zip(rules, data['rows'])):
            with self.subTest(rule=rule['label']):
                end = rules[position + 1]['line'] - 1 if position + 1 < len(rules) else len(lines)
                self.assertEqual((row['page'], row['line'], row['end_line']),
                                 ('src/destructors.md', rule['line'], end))
                self.assertEqual(row['source_block_sha256'],
                                 digest(b''.join(lines[rule['line'] - 1:end])))
                self.assertEqual(row['review_status'], 'unreviewed')
                self.assertTrue(row['candidate_obligation_ids'])
                self.assertTrue(set(row['candidate_obligation_ids']) <= candidates)
        original = json.loads(self.index.read_bytes())
        universe = {(page['path'], rule['label']) for page in
                    original['pages'] + original['unlinked_markdown_files'] for rule in page['rules']}
        selected = {(row['page'], row['rule']) for row in data['rows']}
        unselected = {(row['page'], row['rule']) for row in data['unselected_rules']}
        self.assertEqual((len(selected), len(unselected), len(universe)), (50, 2915, 2965))
        self.assertEqual(len(data['unselected_rules']), 2915)
        self.assertFalse(selected & unselected)
        self.assertEqual(selected | unselected, universe)

    def test_missing_duplicate_foreign_and_malformed_proposals_rejected(self):
        valid = self.build_destructors()
        original = self.proposals.ROWS.strip().splitlines()
        foreign = original[0].split('|')
        foreign[0] = 'foreign.destructors'
        cases = [original[:-1], original + [original[0]],
                 ['|'.join(foreign)] + original[1:], original + ['extra']]
        for position in range(7):
            empty = original[0].split('|')
            empty[position] = ' '
            cases.append(['|'.join(empty)] + original[1:])
        for number, rows in enumerate(cases):
            with self.subTest(mutation=number), mock.patch.object(self.proposals, 'ROWS', '\n'.join(rows)):
                with self.assertRaises(ValueError):
                    self.build_destructors()
                with self.assertRaises(ValueError):
                    self.verify(valid)

    def test_artifact_identity_line_block_binding_and_false_credit_forgery_rejected(self):
        original = self.build_destructors()
        mutations = [
            lambda d: d['rows'].pop(),
            lambda d: d['rows'].append(copy.deepcopy(d['rows'][0])),
            lambda d: d['rows'][0].update(rule='foreign.destructors'),
            lambda d: d['rows'][0].update(page='src/tokens.md'),
            lambda d: d['rows'][0].update(line=True),
            lambda d: d['rows'][0].update(line=99999),
            lambda d: d['rows'][0].update(end_line=99999),
            lambda d: d['rows'][0].update(source_block_sha256='0' * 64),
            lambda d: d['selected_pages'][0].update(sha256='0' * 64),
            lambda d: d['bindings'].update(reference_commit='0' * 40),
            lambda d: d['bindings'].update(rule_index_sha256='0' * 64),
            lambda d: d['rows'][0].update(rationale='invented'),
            lambda d: d['rows'][0].update(review_status='reviewed'),
            lambda d: d['counts'].update(reviewed_rule_rows=1),
            lambda d: d['counts'].update(rows_with_exact_executable_evidence=1),
            lambda d: d['unselected_rules'].pop(),
            lambda d: d['unselected_rules'].append(copy.deepcopy(d['unselected_rules'][0])),
            lambda d: d['unselected_rules'][0].update(rule='foreign.destructors'),
        ]
        for number, mutate in enumerate(mutations):
            changed = copy.deepcopy(original)
            mutate(changed)
            with self.subTest(mutation=number), self.assertRaises(ValueError):
                self.verify(changed)

    def test_source_and_index_line_tampering_rejected_without_shared_writes(self):
        original = self.build_destructors()
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / 'src').mkdir()
            source = root / 'src/destructors.md'
            raw = (self.source / 'src/destructors.md').read_bytes()
            source.write_bytes(raw + b'\n')
            with self.assertRaisesRegex(ValueError, 'page hash'):
                validate(original, self.index, self.inventory, root, 'destructors')
            source.write_bytes(raw)
            index = json.loads(self.index.read_bytes())
            page = next(page for page in index['pages'] if page['path'] == 'src/destructors.md')
            page['rules'][0]['line'] += 1
            altered_index = root / 'index.json'
            altered_index.write_text(json.dumps(index))
            with self.assertRaisesRegex(ValueError, 'labels/lines mismatch'):
                build(altered_index, self.inventory, root, 'destructors')

    def test_legacy_token_const_bytes_unchanged_without_optional_destructor_module(self):
        with mock.patch.dict(sys.modules, {'destructors_reconciliation_proposals': None}):
            for profile, expected in (
                    ('tokens', 'ea7fa27afdf901b91464ead14c54883debd091819c25bf02870e7a434ae61785'),
                    ('const-eval', '0ce718c6e2834b6c518ff43f2fed2b5de72fd75eccf01d4611c3217cbdf053c2')):
                with self.subTest(profile=profile):
                    legacy = build(self.index, self.inventory, self.source, profile)
                    self.assertEqual(digest((json.dumps(legacy, indent=2) + '\n').encode()), expected)

    def test_source_only_cli_and_witness_api_rejected_before_io(self):
        args = ['--profile', 'destructors', '--rule-index', '/absent/index',
                '--inventory', '/absent/inventory', '--source-root', '/absent/source',
                '--artifact', '/absent/artifact']
        actions = ('generate-witness-links', 'build-witness-links',
                   'validate-witness-links', 'assess-witness-links')
        extras = ([], ['--repo-root', '/absent/repo'], ['--evidence', '/absent/evidence'],
                  ['--repo-root', '/absent/repo', '--evidence', '/absent/evidence'])
        cases = [(action, extra) for action in actions for extra in extras]
        cases += [(action, extra) for action in ('generate', 'validate') for extra in extras[1:]]
        with contextlib.ExitStack() as stack:
            for name in ('read_bytes', 'read_text', 'open', 'is_symlink', 'is_file'):
                stack.enter_context(mock.patch.object(Path, name, side_effect=AssertionError('unexpected IO')))
            for action, extra in cases:
                with self.subTest(action=action, extra=extra), self.assertRaises(ValueError):
                    main([action, *args, *extra])
            with self.assertRaises(ValueError):
                reconciliation.build_witness_links(self.index, self.inventory, self.source,
                                                   self.source, 'destructors')

    def test_cli_exact_generate_validate_bytes_exclusive_output_and_cross_profile(self):
        data = self.build_destructors()
        with tempfile.TemporaryDirectory() as temp:
            artifact = Path(temp) / 'destructors.json'
            args = ['--profile', 'destructors', '--rule-index', str(self.index),
                    '--inventory', str(self.inventory), '--source-root', str(self.source),
                    '--artifact', str(artifact)]
            with contextlib.redirect_stdout(io.StringIO()):
                main(['generate', *args])
                main(['validate', *args])
            raw = artifact.read_bytes()
            self.assertEqual(raw, (json.dumps(data, indent=2) + '\n').encode())
            with self.assertRaises(FileExistsError):
                main(['generate', *args])
            self.assertEqual(artifact.read_bytes(), raw)
        for profile in ('tokens', 'const-eval', 'branching'):
            with self.subTest(profile=profile), self.assertRaisesRegex(ValueError, 'purpose mismatch'):
                validate(data, self.index, self.inventory, self.source, profile)



class PinnedLoopsProfileTests(unittest.TestCase):
    """Actual pinned source boundaries, not Rust execution or review evidence."""

    def setUp(self):
        source = os.environ.get('JUSTRUST_WITNESS_REFERENCE_ROOT')
        if not source:
            self.skipTest('set JUSTRUST_WITNESS_REFERENCE_ROOT for pinned-source structural checks')
        self.source = Path(source)
        repo = Path(os.environ.get('JUSTRUST_WITNESS_REPO_ROOT', BASE.parent))
        self.index = repo / 'data/compiler-language-reference-rules.json'
        self.inventory = repo / 'data/compiler-language-inventory.json'

    def build_loops(self):
        import loops_reconciliation_proposals as proposals
        self.proposals = proposals
        return build(self.index, self.inventory, self.source, 'loops')

    def verify(self, data):
        return validate(data, self.index, self.inventory, self.source, 'loops')

    def test_exact_60_source_rows_page_blocks_full_union_and_zero_credit(self):
        data = self.build_loops()
        counts = self.verify(data)
        self.assertEqual((counts['selected_rule_rows'], counts['unselected_rule_identities'],
                          counts['global_rule_identities']), (60, 2905, 2965))
        self.assertEqual(counts['reviewed_rule_rows'], 0)
        self.assertEqual(counts['rows_with_exact_executable_evidence'], 0)
        self.assertEqual(data, self.build_loops())
        self.assertEqual(data['purpose'], 'bounded-loops-rule-reconciliation-candidate')
        self.assertEqual(data['bindings']['reference_commit'],
                         '86635e30bf861a038dc197d7e16fd09e7e514e7a')
        self.assertEqual(self.proposals.SELECTED, ('src/expressions/loop-expr.md',))
        self.assertEqual([page['path'] for page in data['selected_pages']], ['src/expressions/loop-expr.md'])
        raw = (self.source / 'src/expressions/loop-expr.md').read_bytes()
        expected_hash = 'f43315c71c4ab3bd5f564d6cda444c405fbb9c68a9a195e3ddb71b5c99e1d06c'
        self.assertEqual(self.proposals.PAGE_SHA256, {'src/expressions/loop-expr.md': expected_hash})
        self.assertEqual(digest(raw), expected_hash)
        self.assertEqual(data['selected_pages'][0]['sha256'], expected_hash)
        lines = raw.splitlines(keepends=True)
        self.assertEqual(len(lines), 461)
        rules = parse_markdown(raw.decode())[0]
        self.assertEqual(len(rules), 60)
        expected_labels = (
            'expr.loop',
            'expr.loop.syntax',
            'expr.loop.intro',
            'expr.loop.break-label',
            'expr.loop.continue-label',
            'expr.loop.explicit-result',
            'expr.loop.infinite',
            'expr.loop.infinite.syntax',
            'expr.loop.infinite.intro',
            'expr.loop.infinite.diverging',
            'expr.loop.infinite.break',
            'expr.loop.while',
            'expr.loop.while.syntax',
            'expr.loop.while.intro',
            'expr.loop.while.condition',
            'expr.loop.while.repeat',
            'expr.loop.while.exit',
            'expr.loop.while.eval',
            'expr.loop.while.let',
            'expr.loop.while.let.intro',
            'expr.loop.while.let.desugar',
            'expr.loop.while.let.or-pattern',
            'expr.loop.while.chains',
            'expr.loop.while.chains.intro',
            'expr.loop.for',
            'expr.loop.for.syntax',
            'expr.loop.for.intro',
            'expr.loop.for.condition',
            'expr.loop.for.desugar',
            'expr.loop.for.lang-items',
            'expr.loop.label',
            'expr.loop.label.syntax',
            'expr.loop.label.intro',
            'expr.loop.label.control-flow',
            'expr.loop.label.ref',
            'expr.loop.break',
            'expr.loop.break.syntax',
            'expr.loop.break.intro',
            'expr.loop.break.diverging',
            'expr.loop.break.label',
            'expr.loop.break.value',
            'expr.loop.break-value.implicit-value',
            'expr.loop.block-labels',
            'expr.loop.block-labels.syntax',
            'expr.loop.block-labels.intro',
            'expr.loop.block-labels.break',
            'expr.loop.block-labels.label-required',
            'expr.loop.block-labels.type',
            'expr.loop.continue',
            'expr.loop.continue.syntax',
            'expr.loop.continue.intro',
            'expr.loop.continue.diverging',
            'expr.loop.continue.while',
            'expr.loop.continue.for',
            'expr.loop.continue.label',
            'expr.loop.continue.in-loop-only',
            'expr.loop.break-value',
            'expr.loop.break-value.intro',
            'expr.loop.break-value.type',
            'expr.loop.break-value.diverging',
        )
        self.assertEqual(tuple(rule['label'] for rule in rules), expected_labels)
        self.assertEqual(len({rule['label'] for rule in rules}), 60)
        self.assertEqual(len({row['id'] for row in data['rows']}), 60)
        by_label = {row['rule']: row for row in data['rows']}
        for left, right in (('expr.loop.break.value', 'expr.loop.break-value'),
                            ('expr.loop.break.label', 'expr.loop.break-label'),
                            ('expr.loop.continue.label', 'expr.loop.continue-label')):
            with self.subTest(left=left, right=right):
                self.assertNotEqual(by_label[left]['id'], by_label[right]['id'])
        self.assertEqual([row['rule'] for row in data['rows']], [rule['label'] for rule in rules])
        authored = [line.split('|') for line in self.proposals.ROWS.strip().splitlines()]
        self.assertEqual([row[0] for row in authored], [rule['label'] for rule in rules])
        self.assertTrue(all(len(row) == 7 for row in authored))
        self.assertTrue(all(need.startswith('Missing ') for row in authored for need in row[4:]))
        candidates = {item['id'] for section in json.loads(self.inventory.read_bytes())['sections']
                      for item in section['obligations']}
        for position, (rule, row) in enumerate(zip(rules, data['rows'])):
            with self.subTest(rule=rule['label']):
                end = rules[position + 1]['line'] - 1 if position + 1 < len(rules) else len(lines)
                self.assertEqual((row['page'], row['line'], row['end_line']),
                                 ('src/expressions/loop-expr.md', rule['line'], end))
                self.assertEqual(row['source_block_sha256'],
                                 digest(b''.join(lines[rule['line'] - 1:end])))
                self.assertEqual(bytes.fromhex(row['id'].removeprefix('reconcile-loops-')).decode('utf-8'),
                                 rule['label'])
                self.assertTrue(row['id'].startswith('reconcile-loops-'))
                self.assertEqual(row['review_status'], 'unreviewed')
                self.assertTrue(row['candidate_obligation_ids'])
                self.assertTrue(set(row['candidate_obligation_ids']) <= candidates)
        original = json.loads(self.index.read_bytes())
        universe = {(page['path'], rule['label']) for page in
                    original['pages'] + original['unlinked_markdown_files'] for rule in page['rules']}
        selected = {(row['page'], row['rule']) for row in data['rows']}
        unselected = {(row['page'], row['rule']) for row in data['unselected_rules']}
        self.assertEqual((len(selected), len(unselected), len(universe)), (60, 2905, 2965))
        self.assertEqual(len(data['unselected_rules']), 2905)
        self.assertFalse(selected & unselected)
        self.assertEqual(selected | unselected, universe)

    def test_missing_duplicate_foreign_and_malformed_proposals_rejected(self):
        valid = self.build_loops()
        original = self.proposals.ROWS.strip().splitlines()
        foreign = original[0].split('|')
        foreign[0] = 'foreign.loops'
        cases = [[], original[:-1], original + [original[0]],
                 ['|'.join(foreign)] + original[1:], original + ['extra'],
                 ['|'.join(original[0].split('|')[:-1])] + original[1:],
                 [original[0] + '|extra'] + original[1:]]
        for position in range(7):
            for blank in ('', ' ', '\t'):
                empty = original[0].split('|')
                empty[position] = blank
                cases.append(['|'.join(empty)] + original[1:])
        for number, rows in enumerate(cases):
            with self.subTest(mutation=number), mock.patch.object(self.proposals, 'ROWS', '\n'.join(rows)):
                with self.assertRaises(ValueError):
                    self.build_loops()
                with self.assertRaises(ValueError):
                    self.verify(valid)

    def test_artifact_identity_line_block_binding_and_false_credit_forgery_rejected(self):
        original = self.build_loops()
        def swap_ids(data):
            first, second = data['rows'][:2]
            first['id'], second['id'] = second['id'], first['id']

        mutations = [
            swap_ids,
            lambda d: d['rows'][0].update(id='reconcile-expr-loop'),
            lambda d: d['rows'].reverse(),
            lambda d: d['rows'].pop(),
            lambda d: d['rows'].append(copy.deepcopy(d['rows'][0])),
            lambda d: d['rows'][0].update(id='reconcile-forged-loop'),
            lambda d: d['rows'][0].update(rule='foreign.loops'),
            lambda d: d['rows'][0].update(page='src/tokens.md'),
            lambda d: d['rows'][0].update(line=True),
            lambda d: d['rows'][0].update(line=99999),
            lambda d: d['rows'][0].update(end_line=True),
            lambda d: d['rows'][0].update(end_line=99999),
            lambda d: d['rows'][0].update(source_block_sha256='0' * 64),
            lambda d: d['selected_pages'].pop(),
            lambda d: d['selected_pages'].append(copy.deepcopy(d['selected_pages'][0])),
            lambda d: d['selected_pages'][0].update(path='src/tokens.md'),
            lambda d: d['selected_pages'][0].update(sha256='0' * 64),
            lambda d: d['bindings'].update(reference_commit='0' * 40),
            lambda d: d['bindings'].update(rule_index_sha256='0' * 64),
            lambda d: d['rows'][0].update(candidate_obligation_ids=['foreign.obligation']),
            lambda d: d['rows'][0].update(rationale='invented'),
            lambda d: d.update(review_status='reviewed'),
            lambda d: d['rows'][0].update(missing_test_needs=[]),
            lambda d: d['rows'][0].update(missing_test_needs=[' '] * 4),
            lambda d: d['rows'][0]['missing_test_needs'].__setitem__(0, 'positive: implemented'),
            lambda d: d['rows'][0].update(review_status='reviewed'),
            lambda d: d['counts'].update(reviewed_rule_rows=1),
            lambda d: d['counts'].update(rows_with_exact_executable_evidence=1),
            lambda d: d['unselected_rules'].reverse(),
            lambda d: d['unselected_rules'].pop(),
            lambda d: d['unselected_rules'].append(copy.deepcopy(d['unselected_rules'][0])),
            lambda d: d['unselected_rules'][0].update(rule='foreign.loops'),
        ]
        for number, mutate in enumerate(mutations):
            changed = copy.deepcopy(original)
            mutate(changed)
            with self.subTest(mutation=number), self.assertRaises(ValueError):
                self.verify(changed)

    def test_source_and_index_line_tampering_rejected_without_shared_writes(self):
        original = self.build_loops()
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / 'src/expressions').mkdir(parents=True)
            source = root / 'src/expressions/loop-expr.md'
            raw = (self.source / 'src/expressions/loop-expr.md').read_bytes()
            source.write_bytes(raw + b'\n')
            with self.assertRaisesRegex(ValueError, 'page hash'):
                validate(original, self.index, self.inventory, root, 'loops')
            source.write_bytes(raw)
            index = json.loads(self.index.read_bytes())
            page = next(page for page in index['pages'] if page['path'] == 'src/expressions/loop-expr.md')
            page['rules'][0]['line'] += 1
            altered_index = root / 'index.json'
            altered_index.write_text(json.dumps(index))
            with self.assertRaisesRegex(ValueError, 'labels/lines mismatch'):
                build(altered_index, self.inventory, root, 'loops')

    def test_legacy_profile_bytes_unchanged_without_optional_loops_module(self):
        with mock.patch.dict(sys.modules, {'loops_reconciliation_proposals': None}):
            for profile, expected in (
                    ('tokens', 'ea7fa27afdf901b91464ead14c54883debd091819c25bf02870e7a434ae61785'),
                    ('const-eval', '0ce718c6e2834b6c518ff43f2fed2b5de72fd75eccf01d4611c3217cbdf053c2'),
                    ('unsafe', 'bc752efc1f0172cd85dad9399a1309c06489b3b0e4d4f74884c8bc1ef58ebb58'),
                    ('abi', 'c4e561a9a78002281c34cbc35fe7fcaee3f29e82c659b718844504514ac1d317'),
                    ('visibility', '49883324b912c7f107d3573d6b95a3b4cc52b3d97253653eb39dfee6af9cbd58'),
                    ('branching', 'b08afed11ff9bd56c0b15f31bbaa3109105181bde417f0565ff1082cf340e74a'),
                    ('destructors', 'af8e7a87b177e4c72efd7f0dddfe26c2a0390d710377a507f9765f3d1b9d188e')):
                with self.subTest(profile=profile):
                    legacy = build(self.index, self.inventory, self.source, profile)
                    self.assertEqual(digest((json.dumps(legacy, indent=2) + '\n').encode()), expected)

    def test_source_only_cli_and_witness_api_rejected_before_io(self):
        args = ['--profile', 'loops', '--rule-index', '/absent/index',
                '--inventory', '/absent/inventory', '--source-root', '/absent/source',
                '--artifact', '/absent/artifact']
        actions = ('generate-witness-links', 'build-witness-links',
                   'validate-witness-links', 'assess-witness-links')
        extras = ([], ['--repo-root', '/absent/repo'], ['--evidence', '/absent/evidence'],
                  ['--repo-root', '/absent/repo', '--evidence', '/absent/evidence'])
        cases = [(action, extra) for action in actions for extra in extras]
        cases += [(action, extra) for action in ('generate', 'validate') for extra in extras[1:]]
        with contextlib.ExitStack() as stack:
            for name in ('read_bytes', 'read_text', 'open', 'is_symlink', 'is_file',
                         'write_bytes', 'write_text', 'mkdir'):
                stack.enter_context(mock.patch.object(Path, name, side_effect=AssertionError('unexpected IO')))
            stack.enter_context(mock.patch.object(subprocess, 'run', side_effect=AssertionError('execution')))
            for action, extra in cases:
                with self.subTest(action=action, extra=extra), self.assertRaises(ValueError):
                    main([action, *args, *extra])
            with self.assertRaises(ValueError):
                reconciliation.build_witness_links(self.index, self.inventory, self.source,
                                                   self.source, 'loops')

    def test_cli_exact_generate_validate_bytes_exclusive_output_and_cross_profile(self):
        data = self.build_loops()
        with tempfile.TemporaryDirectory() as temp:
            artifact = Path(temp) / 'loops.json'
            args = ['--profile', 'loops', '--rule-index', str(self.index),
                    '--inventory', str(self.inventory), '--source-root', str(self.source),
                    '--artifact', str(artifact)]
            with contextlib.redirect_stdout(io.StringIO()):
                main(['generate', *args])
                main(['validate', *args])
            raw = artifact.read_bytes()
            self.assertEqual(raw, (json.dumps(data, indent=2) + '\n').encode())
            with self.assertRaises(FileExistsError):
                main(['generate', *args])
            self.assertEqual(artifact.read_bytes(), raw)
            forged = copy.deepcopy(data)
            forged['rows'][0]['review_status'] = 'reviewed'
            artifact.write_text(json.dumps(forged, indent=2) + '\n')
            with self.assertRaises(ValueError):
                main(['validate', *args])
        for profile in ('tokens', 'const-eval', 'unsafe', 'abi', 'visibility', 'branching', 'destructors'):
            with self.subTest(profile=profile), self.assertRaisesRegex(ValueError, 'purpose mismatch'):
                validate(data, self.index, self.inventory, self.source, profile)

class PinnedOperatorsProfileTests(unittest.TestCase):
    """Independent pinned-source checks, never Rust execution or review credit."""

    def setUp(self):
        source = os.environ.get('JUSTRUST_WITNESS_REFERENCE_ROOT')
        if not source:
            self.skipTest('set JUSTRUST_WITNESS_REFERENCE_ROOT for pinned-source structural checks')
        self.source = Path(source)
        repo = Path(os.environ.get('JUSTRUST_WITNESS_REPO_ROOT', BASE.parent))
        self.index = repo / 'data/compiler-language-reference-rules.json'
        self.inventory = repo / 'data/compiler-language-inventory.json'

    def artifact(self):
        return build(self.index, self.inventory, self.source, 'operators')

    def verify(self, data, source=None):
        return validate(data, self.index, self.inventory,
                        self.source if source is None else source, 'operators')

    def test_actual_source_hash_labels_blocks_ids_partition_and_zero_credit(self):
        import hashlib
        import re
        data = self.artifact()
        counts = self.verify(data)
        self.assertEqual([counts[key] for key in ('selected_rule_rows',
                         'unselected_rule_identities', 'global_rule_identities')], [112, 2853, 2965])
        for key in ('reviewed_rule_rows', 'rows_with_exact_executable_evidence'):
            self.assertEqual(counts[key], 0)
        page = 'src/expressions/operator-expr.md'
        raw = (self.source / page).read_bytes()
        sha = lambda value: hashlib.sha256(value).hexdigest()
        self.assertEqual(sha(raw), 'a3a37a4182f5993bf20a35eee1e73364f85bf79df189c7b45cab63b258577032')
        self.assertEqual([(p['path'], p['sha256']) for p in data['selected_pages']], [(page, sha(raw))])
        lines = raw.splitlines(keepends=True)
        # Do not use the production Markdown parser or authored proposal labels as the oracle.
        markers = [(number, match.group(1).decode()) for number, line in enumerate(lines, 1)
                   if (match := re.fullmatch(rb'r\[([^\]\r\n]+)\]\r?\n?', line))]
        self.assertEqual(len(markers), 112)
        self.assertEqual(len({label for _, label in markers}), 112)
        self.assertEqual([row['rule'] for row in data['rows']], [label for _, label in markers])
        self.assertEqual(len({row['id'] for row in data['rows']}), 112)
        for position, (row, (start, label)) in enumerate(zip(data['rows'], markers)):
            with self.subTest(label=label):
                end = markers[position + 1][0] - 1 if position + 1 < len(markers) else len(lines)
                self.assertEqual((row['page'], row['line'], row['end_line']), (page, start, end))
                self.assertEqual(row['source_block_sha256'], sha(b''.join(lines[start - 1:end])))
                self.assertEqual(row['id'], 'reconcile-operators-' + label.encode().hex())
                self.assertEqual(bytes.fromhex(row['id'][20:]).decode(), label)
                self.assertEqual(row['review_status'], 'unreviewed')
                self.assertTrue(all(need.startswith(('positive: Missing ', 'invalid: Missing ',
                                                    'interaction: Missing ', 'end-to-end: Missing '))
                                    for need in row['missing_test_needs']))
        index = json.loads(self.index.read_bytes())
        universe = {(p['path'], r['label']) for p in index['pages'] + index['unlinked_markdown_files']
                    for r in p['rules']}
        selected = {(row['page'], row['rule']) for row in data['rows']}
        other = {(row['page'], row['rule']) for row in data['unselected_rules']}
        self.assertEqual((len(selected), len(other), len(universe)), (112, 2853, 2965))
        self.assertEqual(len(data['unselected_rules']), 2853)
        self.assertFalse(selected & other)
        self.assertEqual(selected | other, universe)
        self.assertEqual(data, self.artifact())

    def test_missing_duplicate_foreign_authored_labels_rejected(self):
        import operators_reconciliation_proposals as proposals
        valid = self.artifact()
        rows = proposals.ROWS.strip().splitlines()
        foreign = rows[0].split('|')
        foreign[0] = 'expr.loop'
        for changed in (rows[:-1], rows + rows[:1], ['|'.join(foreign)] + rows[1:]):
            with self.subTest(first=changed[0], length=len(changed)), \
                    mock.patch.object(proposals, 'ROWS', '\n'.join(changed)):
                with self.assertRaises(ValueError):
                    self.artifact()
                with self.assertRaises(ValueError):
                    self.verify(valid)

    def test_forged_rows_source_classifications_needs_ids_and_credits_rejected(self):
        original = self.artifact()
        mutations = [
            lambda d: d['rows'][0].update(page='src/tokens.md'),
            lambda d: d['rows'][0].update(rule='expr.loop'),
            lambda d: d['rows'][0].update(line=True),
            lambda d: d['rows'][0].update(end_line=99999),
            lambda d: d['rows'][0].update(source_block_sha256='0' * 64),
            lambda d: d['rows'][0].update(classifications=['normative-semantic']),
            lambda d: d['rows'][0].update(rationale='invented interpretation'),
            lambda d: d['rows'][0].update(candidate_obligation_ids=['foreign']),
            lambda d: d['rows'][0].update(missing_test_needs=[]),
            lambda d: d['rows'][0]['missing_test_needs'].__setitem__(0, 'positive: implemented'),
            lambda d: d['rows'][0].update(id=d['rows'][1]['id']),
            lambda d: d['rows'][0].update(id='reconcile-operators-expr-operator'),
            lambda d: d['rows'][0].update(review_status='reviewed'),
            lambda d: d['counts'].update(reviewed_rule_rows=1),
            lambda d: d['counts'].update(rows_with_exact_executable_evidence=1),
            lambda d: d['selected_pages'][0].update(sha256='0' * 64),
            lambda d: d['bindings'].update(reference_commit='0' * 40),
            lambda d: d['unselected_rules'].pop(),
            lambda d: d['unselected_rules'].append(copy.deepcopy(d['unselected_rules'][0])),
            lambda d: d['unselected_rules'][0].update(rule=d['rows'][0]['rule']),
        ]
        for number, mutate in enumerate(mutations):
            changed = copy.deepcopy(original)
            mutate(changed)
            with self.subTest(mutation=number), self.assertRaises(ValueError):
                self.verify(changed)

    def test_stale_source_and_symlink_rejected(self):
        data = self.artifact()
        page = 'src/expressions/operator-expr.md'
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            target = root / page
            target.parent.mkdir(parents=True)
            target.write_bytes((self.source / page).read_bytes() + b'\n')
            with self.assertRaises(ValueError):
                self.verify(data, root)
            target.unlink()
            target.symlink_to(self.source / page)
            with self.assertRaises(ValueError):
                self.verify(data, root)

    def test_source_only_unsupported_cli_and_api_fail_before_io(self):
        args = ['--profile', 'operators', '--rule-index', '/absent/index',
                '--inventory', '/absent/inventory', '--source-root', '/absent/source',
                '--artifact', '/absent/artifact']
        extras = ([], ['--repo-root', '/absent/repo'], ['--evidence', '/absent/evidence'],
                  ['--repo-root', '/absent/repo', '--evidence', '/absent/evidence'])
        actions = ('generate-witness-links', 'build-witness-links',
                   'validate-witness-links', 'assess-witness-links')
        cases = [(action, extra) for action in actions for extra in extras]
        cases += [(action, extra) for action in ('generate', 'validate') for extra in extras[1:]]
        with contextlib.ExitStack() as stack:
            for name in ('read_bytes', 'read_text', 'open', 'is_symlink', 'is_file',
                         'write_bytes', 'write_text', 'mkdir'):
                stack.enter_context(mock.patch.object(Path, name, side_effect=AssertionError('IO')))
            stack.enter_context(mock.patch.object(subprocess, 'run', side_effect=AssertionError('execution')))
            for action, extra in cases:
                with self.subTest(action=action, extra=extra), self.assertRaises(ValueError):
                    main([action, *args, *extra])
            calls = (
                lambda: reconciliation.build_witness_links(
                    self.index, self.inventory, self.source, self.source, 'operators'),
                lambda: reconciliation.validate_witness_links(
                    {}, self.index, self.inventory, self.source, self.source, 'operators'),
                lambda: reconciliation.assess_witness_links(
                    {}, self.index, self.inventory, self.source, self.source, (), 'operators'),
            )
            for call in calls:
                with self.subTest(api=call), self.assertRaises(ValueError):
                    call()

    def test_cli_generate_validate_exclusive_output(self):
        data = self.artifact()
        with tempfile.TemporaryDirectory() as temp:
            artifact = Path(temp) / 'operators.json'
            args = ['--profile', 'operators', '--rule-index', str(self.index),
                    '--inventory', str(self.inventory), '--source-root', str(self.source),
                    '--artifact', str(artifact)]
            with contextlib.redirect_stdout(io.StringIO()):
                main(['generate', *args])
                main(['validate', *args])
            raw = artifact.read_bytes()
            self.assertEqual(raw, (json.dumps(data, indent=2) + '\n').encode())
            with self.assertRaises(FileExistsError):
                main(['generate', *args])
            self.assertEqual(artifact.read_bytes(), raw)
            data['rows'][0]['review_status'] = 'reviewed'
            artifact.write_text(json.dumps(data))
            with self.assertRaises(ValueError):
                main(['validate', *args])

    def test_all_eight_prior_profile_bytes_unchanged_without_operators_module(self):
        expected = (
            ('tokens', 'ea7fa27afdf901b91464ead14c54883debd091819c25bf02870e7a434ae61785'),
            ('const-eval', '0ce718c6e2834b6c518ff43f2fed2b5de72fd75eccf01d4611c3217cbdf053c2'),
            ('unsafe', 'bc752efc1f0172cd85dad9399a1309c06489b3b0e4d4f74884c8bc1ef58ebb58'),
            ('abi', 'c4e561a9a78002281c34cbc35fe7fcaee3f29e82c659b718844504514ac1d317'),
            ('visibility', '49883324b912c7f107d3573d6b95a3b4cc52b3d97253653eb39dfee6af9cbd58'),
            ('branching', 'b08afed11ff9bd56c0b15f31bbaa3109105181bde417f0565ff1082cf340e74a'),
            ('destructors', 'af8e7a87b177e4c72efd7f0dddfe26c2a0390d710377a507f9765f3d1b9d188e'),
            ('loops', '4a77bd5239c74bde49e8ba7933248484acd773c4f08239ce6f1fce23efb2261a'))
        with mock.patch.dict(sys.modules, {'operators_reconciliation_proposals': None}):
            for profile, sha in expected:
                with self.subTest(profile=profile):
                    data = build(self.index, self.inventory, self.source, profile)
                    self.assertEqual(digest((json.dumps(data, indent=2) + '\n').encode()), sha)




class PinnedClosuresProfileTests(unittest.TestCase):
    """Independent pinned-source checks, not Rust execution or review credit."""

    PAGE = 'src/expressions/closure-expr.md'
    PAGE_SHA256 = 'c6b2cfe69852a0dde9e91a37c69b1885ff680af6e10120f7077d855a1f355813'
    LABELS = (
        'expr.closure', 'expr.closure.syntax', 'expr.closure.intro',
        'expr.closure.param-type', 'expr.closure.explicit-type-body',
        'expr.closure.parameter-restriction', 'expr.closure.unique-type',
        'expr.closure.captures', 'expr.closure.capture-inference',
        'expr.closure.capture-mut-ref', 'expr.closure.capture-move',
        'expr.closure.trait-impl', 'expr.closure.async',
        'expr.closure.async.intro', 'expr.closure.async.future',
        'expr.closure.async.edition2018', 'expr.closure.param-attributes',
    )

    def setUp(self):
        source = os.environ.get('JUSTRUST_WITNESS_REFERENCE_ROOT')
        if not source:
            self.skipTest('set JUSTRUST_WITNESS_REFERENCE_ROOT for pinned-source structural checks')
        self.source = Path(source)
        repo = Path(os.environ.get('JUSTRUST_WITNESS_REPO_ROOT', BASE.parent))
        self.index = repo / 'data/compiler-language-reference-rules.json'
        self.inventory = repo / 'data/compiler-language-inventory.json'

    def artifact(self):
        return build(self.index, self.inventory, self.source, 'closures')

    def verify(self, data, source=None):
        return validate(data, self.index, self.inventory,
                        self.source if source is None else source, 'closures')

    def test_actual_source_hash_labels_blocks_ids_partition_and_zero_credit(self):
        import hashlib
        import re
        data = self.artifact()
        counts = self.verify(data)
        self.assertEqual([counts[key] for key in ('selected_rule_rows',
                         'unselected_rule_identities', 'global_rule_identities')], [17, 2948, 2965])
        for key in ('reviewed_rule_rows', 'rows_with_exact_executable_evidence'):
            self.assertEqual(counts[key], 0)
        raw = (self.source / self.PAGE).read_bytes()
        sha = lambda value: hashlib.sha256(value).hexdigest()
        self.assertEqual(sha(raw), self.PAGE_SHA256)
        self.assertEqual([(p['path'], p['sha256']) for p in data['selected_pages']],
                         [(self.PAGE, self.PAGE_SHA256)])
        lines = raw.splitlines(keepends=True)
        # The oracle deliberately does not call the production Markdown parser.
        markers = [(number, match.group(1).decode()) for number, line in enumerate(lines, 1)
                   if (match := re.fullmatch(rb'r\[([^\]\r\n]+)\]\r?\n?', line))]
        self.assertEqual(tuple(label for _, label in markers), self.LABELS)
        self.assertEqual(tuple(row['rule'] for row in data['rows']), self.LABELS)
        self.assertEqual(len({row['id'] for row in data['rows']}), 17)
        prefix = 'reconcile-closures-'
        for position, (row, (start, label)) in enumerate(zip(data['rows'], markers)):
            with self.subTest(label=label):
                end = markers[position + 1][0] - 1 if position + 1 < len(markers) else len(lines)
                self.assertEqual((row['page'], row['line'], row['end_line']), (self.PAGE, start, end))
                self.assertEqual(row['source_block_sha256'], sha(b''.join(lines[start - 1:end])))
                self.assertEqual(row['id'], prefix + label.encode().hex())
                self.assertEqual(bytes.fromhex(row['id'][len(prefix):]).decode(), label)
                self.assertEqual(row['review_status'], 'unreviewed')
                self.assertTrue(row['missing_test_needs'])
                self.assertTrue(all(need.startswith(('positive: Missing ', 'invalid: Missing ',
                                                    'interaction: Missing ', 'end-to-end: Missing '))
                                    for need in row['missing_test_needs']))
        index = json.loads(self.index.read_bytes())
        universe = {(p['path'], r['label']) for p in index['pages'] + index['unlinked_markdown_files']
                    for r in p['rules']}
        selected = {(row['page'], row['rule']) for row in data['rows']}
        other = {(row['page'], row['rule']) for row in data['unselected_rules']}
        self.assertEqual((len(selected), len(other), len(universe)), (17, 2948, 2965))
        self.assertEqual(len(data['unselected_rules']), 2948)
        self.assertFalse(selected & other)
        self.assertEqual(selected | other, universe)
        self.assertEqual(data, self.artifact())

    def test_missing_duplicate_foreign_authored_labels_rejected(self):
        import closures_reconciliation_proposals as proposals
        valid = self.artifact()
        rows = proposals.ROWS.strip().splitlines()
        foreign = rows[0].split('|')
        foreign[0] = 'expr.loop'
        for changed in (rows[:-1], rows + rows[:1], ['|'.join(foreign)] + rows[1:]):
            with self.subTest(length=len(changed)), \
                    mock.patch.object(proposals, 'ROWS', '\n'.join(changed)):
                with self.assertRaises(ValueError):
                    self.artifact()
                with self.assertRaises(ValueError):
                    self.verify(valid)

    def test_forged_rows_partition_bindings_and_credits_rejected(self):
        original = self.artifact()
        mutations = [
            lambda d: d['rows'].pop(),
            lambda d: d['rows'].append(copy.deepcopy(d['rows'][0])),
            lambda d: d['rows'][0].update(page='src/tokens.md'),
            lambda d: d['rows'][0].update(rule='expr.loop'),
            lambda d: d['rows'][0].update(line=True),
            lambda d: d['rows'][0].update(end_line=99999),
            lambda d: d['rows'][0].update(source_block_sha256='0' * 64),
            lambda d: d['rows'][0].update(classifications=['normative-semantic']),
            lambda d: d['rows'][0].update(rationale='invented interpretation'),
            lambda d: d['rows'][0].update(candidate_obligation_ids=['foreign']),
            lambda d: d['rows'][0].update(missing_test_needs=[]),
            lambda d: d['rows'][0]['missing_test_needs'].__setitem__(0, 'positive: implemented'),
            lambda d: d['rows'][0].update(id=d['rows'][1]['id']),
            lambda d: d['rows'][0].update(id='reconcile-closures-expr-closure'),
            lambda d: d['rows'][0].update(review_status='reviewed'),
            lambda d: d['counts'].update(reviewed_rule_rows=1),
            lambda d: d['counts'].update(rows_with_exact_executable_evidence=1),
            lambda d: d['selected_pages'][0].update(sha256='0' * 64),
            lambda d: d['bindings'].update(reference_commit='0' * 40),
            lambda d: d['unselected_rules'].pop(),
            lambda d: d['unselected_rules'].append(copy.deepcopy(d['unselected_rules'][0])),
            lambda d: d['unselected_rules'][0].update(page=d['rows'][0]['page'],
                                                     rule=d['rows'][0]['rule']),
        ]
        for number, mutate in enumerate(mutations):
            changed = copy.deepcopy(original)
            mutate(changed)
            with self.subTest(mutation=number), self.assertRaises(ValueError):
                self.verify(changed)

    def test_stale_source_and_symlink_rejected(self):
        data = self.artifact()
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            target = root / self.PAGE
            target.parent.mkdir(parents=True)
            target.write_bytes((self.source / self.PAGE).read_bytes() + b'\n')
            with self.assertRaises(ValueError):
                self.verify(data, root)
            target.unlink()
            target.symlink_to(self.source / self.PAGE)
            with self.assertRaises(ValueError):
                self.verify(data, root)

    def test_source_only_unsupported_cli_and_api_fail_before_io(self):
        args = ['--profile', 'closures', '--rule-index', '/absent/index',
                '--inventory', '/absent/inventory', '--source-root', '/absent/source',
                '--artifact', '/absent/artifact']
        extras = ([], ['--repo-root', '/absent/repo'], ['--evidence', '/absent/evidence'],
                  ['--repo-root', '/absent/repo', '--evidence', '/absent/evidence'])
        actions = ('generate-witness-links', 'build-witness-links',
                   'validate-witness-links', 'assess-witness-links')
        cases = [(action, extra) for action in actions for extra in extras]
        cases += [(action, extra) for action in ('generate', 'validate') for extra in extras[1:]]
        with contextlib.ExitStack() as stack:
            for name in ('read_bytes', 'read_text', 'open', 'is_symlink', 'is_file',
                         'write_bytes', 'write_text', 'mkdir'):
                stack.enter_context(mock.patch.object(Path, name, side_effect=AssertionError('IO')))
            stack.enter_context(mock.patch.object(subprocess, 'run', side_effect=AssertionError('execution')))
            for action, extra in cases:
                with self.subTest(action=action, extra=extra), self.assertRaises(ValueError):
                    main([action, *args, *extra])
            calls = (
                lambda: reconciliation.build_witness_links(
                    self.index, self.inventory, self.source, self.source, 'closures'),
                lambda: reconciliation.validate_witness_links(
                    {}, self.index, self.inventory, self.source, self.source, 'closures'),
                lambda: reconciliation.assess_witness_links(
                    {}, self.index, self.inventory, self.source, self.source, (), 'closures'),
            )
            for call in calls:
                with self.subTest(api=call), self.assertRaises(ValueError):
                    call()

    def test_cli_generate_validate_exclusive_output(self):
        data = self.artifact()
        with tempfile.TemporaryDirectory() as temp:
            artifact = Path(temp) / 'closures.json'
            args = ['--profile', 'closures', '--rule-index', str(self.index),
                    '--inventory', str(self.inventory), '--source-root', str(self.source),
                    '--artifact', str(artifact)]
            with contextlib.redirect_stdout(io.StringIO()):
                main(['generate', *args])
                main(['validate', *args])
            raw = artifact.read_bytes()
            self.assertEqual(raw, (json.dumps(data, indent=2) + '\n').encode())
            with self.assertRaises(FileExistsError):
                main(['generate', *args])
            self.assertEqual(artifact.read_bytes(), raw)
            data['rows'][0]['review_status'] = 'reviewed'
            artifact.write_text(json.dumps(data))
            with self.assertRaises(ValueError):
                main(['validate', *args])

    def test_prior_operators_bytes_unchanged_without_closures_module(self):
        with mock.patch.dict(sys.modules, {'closures_reconciliation_proposals': None}):
            data = build(self.index, self.inventory, self.source, 'operators')
        self.assertEqual(
            digest((json.dumps(data, indent=2) + '\n').encode()),
            '2d724517022229ad541ef7c2a66aafc414b13f56c68d3cb8ea633755d3996ba3')



class PinnedCallsProfileTests(unittest.TestCase):
    """Independent two-page source oracle, not execution or review credit."""

    PAGES = (
        ('src/expressions/call-expr.md',
         '051bf071deb7a9716282398e45aa299f6885a1e3e8091a99ea64344a3ae243da', (
             'expr.call', 'expr.call.syntax', 'expr.call.intro',
             'expr.call.convergence', 'expr.call.trait', 'expr.call.autoref-deref',
             'expr.call.desugar', 'expr.call.desugar.fully-qualified',
             'expr.call.desugar.ambiguity', 'expr.call.desugar.limits',
             'expr.call.desugar.explicit-path')),
        ('src/expressions/method-call-expr.md',
         '517cb05b767ce076f78f5b06ff961cc5c28382b7c2cd67d8af504e8492f504fe', (
             'expr.method', 'expr.method.syntax', 'expr.method.intro',
             'expr.method.target', 'expr.method.autoref-deref',
             'expr.method.candidate-receivers', 'expr.method.candidate-receivers-refs',
             'expr.method.candidate-search', 'expr.method.ambiguous-target',
             'expr.method.receiver-constraints', 'expr.method.ambiguous-search',
             'expr.method.edition2021')),
    )

    def setUp(self):
        source = os.environ.get('JUSTRUST_WITNESS_REFERENCE_ROOT')
        if not source:
            self.skipTest('set JUSTRUST_WITNESS_REFERENCE_ROOT for pinned-source structural checks')
        self.source = Path(source)
        repo = Path(os.environ.get('JUSTRUST_WITNESS_REPO_ROOT', BASE.parent))
        self.index = repo / 'data/compiler-language-reference-rules.json'
        self.inventory = repo / 'data/compiler-language-inventory.json'

    def artifact(self):
        return build(self.index, self.inventory, self.source, 'calls')

    def verify(self, data, source=None):
        return validate(data, self.index, self.inventory,
                        self.source if source is None else source, 'calls')

    def test_actual_source_hash_labels_blocks_ids_partition_and_zero_credit(self):
        import hashlib
        import re
        data = self.artifact()
        counts = self.verify(data)
        self.assertEqual([counts[key] for key in ('selected_rule_rows',
                         'unselected_rule_identities', 'global_rule_identities')], [23, 2942, 2965])
        for key in ('reviewed_rule_rows', 'rows_with_exact_executable_evidence'):
            self.assertEqual(counts[key], 0)
        self.assertEqual([(p['path'], p['sha256']) for p in data['selected_pages']],
                         [(page, sha) for page, sha, _ in self.PAGES])
        ordered_labels = tuple(label for _, _, labels in self.PAGES for label in labels)
        self.assertEqual(tuple(row['rule'] for row in data['rows']), ordered_labels)
        self.assertEqual(len({row['id'] for row in data['rows']}), 23)
        sha = lambda value: hashlib.sha256(value).hexdigest()
        offset = 0
        for page, page_sha, labels in self.PAGES:
            raw = (self.source / page).read_bytes()
            self.assertEqual(sha(raw), page_sha)
            lines = raw.splitlines(keepends=True)
            # Deliberately independent of the production Markdown parser.
            markers = [(number, match.group(1).decode())
                       for number, line in enumerate(lines, 1)
                       if (match := re.fullmatch(rb'r\[([^\]\r\n]+)\]\r?\n?', line))]
            self.assertEqual(tuple(label for _, label in markers), labels)
            rows = data['rows'][offset:offset + len(labels)]
            self.assertEqual(len(rows), len(labels))
            for position, (row, (start, label)) in enumerate(zip(rows, markers)):
                with self.subTest(page=page, label=label):
                    end = markers[position + 1][0] - 1 if position + 1 < len(markers) else len(lines)
                    self.assertEqual((row['page'], row['line'], row['end_line']), (page, start, end))
                    self.assertEqual(row['source_block_sha256'], sha(b''.join(lines[start - 1:end])))
                    prefix = 'reconcile-calls-'
                    self.assertEqual(row['id'], prefix + label.encode().hex())
                    self.assertEqual(bytes.fromhex(row['id'][len(prefix):]).decode(), label)
                    self.assertEqual(row['review_status'], 'unreviewed')
                    self.assertTrue(row['missing_test_needs'])
                    self.assertTrue(all(need.startswith(('positive: Missing ', 'invalid: Missing ',
                                                        'interaction: Missing ', 'end-to-end: Missing '))
                                        for need in row['missing_test_needs']))
            offset += len(labels)
        index = json.loads(self.index.read_bytes())
        universe = {(p['path'], r['label']) for p in index['pages'] + index['unlinked_markdown_files']
                    for r in p['rules']}
        selected = {(row['page'], row['rule']) for row in data['rows']}
        other = {(row['page'], row['rule']) for row in data['unselected_rules']}
        self.assertEqual((len(selected), len(other), len(universe)), (23, 2942, 2965))
        self.assertEqual(len(data['unselected_rules']), 2942)
        self.assertFalse(selected & other)
        self.assertEqual(selected | other, universe)
        self.assertEqual(data, self.artifact())

    def test_per_page_missing_duplicate_foreign_authored_labels_rejected(self):
        import calls_reconciliation_proposals as proposals
        valid = self.artifact()
        rows = proposals.ROWS.strip().splitlines()
        for page, _, labels in self.PAGES:
            positions = [i for i, row in enumerate(rows) if row.split('|')[0] in labels]
            self.assertEqual(len(positions), len(labels))
            foreign = rows[positions[0]].split('|')
            foreign[0] = 'expr.loop'
            missing = [row for i, row in enumerate(rows) if i != positions[-1]]
            absent_page = [row for i, row in enumerate(rows) if i not in positions]
            duplicate = rows + [rows[positions[0]]]
            substituted = rows.copy()
            substituted[positions[0]] = '|'.join(foreign)
            for mutation, changed in (('missing', missing), ('absent-page', absent_page),
                                      ('duplicate', duplicate), ('foreign', substituted)):
                with self.subTest(page=page, mutation=mutation), \
                        mock.patch.object(proposals, 'ROWS', '\n'.join(changed)):
                    with self.assertRaises(ValueError):
                        self.artifact()
                    with self.assertRaises(ValueError):
                        self.verify(valid)

    def test_forged_per_page_rows_partition_bindings_and_credits_rejected(self):
        original = self.artifact()
        for page, _, _ in self.PAGES:
            first = next(i for i, row in enumerate(original['rows']) if row['page'] == page)
            mutations = [
                lambda d: d['rows'].pop(first),
                lambda d: d['rows'].append(copy.deepcopy(d['rows'][first])),
                lambda d: d['rows'][first].update(page='src/tokens.md'),
                lambda d: d['rows'][first].update(rule='expr.loop'),
                lambda d: d['rows'][first].update(line=True),
                lambda d: d['rows'][first].update(end_line=99999),
                lambda d: d['rows'][first].update(source_block_sha256='0' * 64),
                lambda d: d['rows'][first].update(classifications=['normative-semantic']),
                lambda d: d['rows'][first].update(rationale='invented interpretation'),
                lambda d: d['rows'][first].update(candidate_obligation_ids=['foreign']),
                lambda d: d['rows'][first].update(missing_test_needs=[]),
                lambda d: d['rows'][first]['missing_test_needs'].__setitem__(0, 'positive: implemented'),
                lambda d: d['rows'][first].update(id=d['rows'][first + 1]['id']),
                lambda d: d['rows'][first].update(id='reconcile-calls-expr-call'),
                lambda d: d['rows'][first].update(review_status='reviewed'),
            ]
            for number, mutate in enumerate(mutations):
                changed = copy.deepcopy(original)
                mutate(changed)
                with self.subTest(page=page, mutation=number), self.assertRaises(ValueError):
                    self.verify(changed)
        mutations = [
            lambda d: d['counts'].update(reviewed_rule_rows=1),
            lambda d: d['counts'].update(rows_with_exact_executable_evidence=1),
            lambda d: d['selected_pages'][0].update(sha256='0' * 64),
            lambda d: d['selected_pages'][1].update(sha256='0' * 64),
            lambda d: d['selected_pages'].pop(0),
            lambda d: d['selected_pages'].pop(1),
            lambda d: d['selected_pages'].reverse(),
            lambda d: d['bindings'].update(reference_commit='0' * 40),
            lambda d: d['unselected_rules'].pop(),
            lambda d: d['unselected_rules'].append(copy.deepcopy(d['unselected_rules'][0])),
            lambda d: d['unselected_rules'][0].update(page=d['rows'][0]['page'],
                                                     rule=d['rows'][0]['rule']),
        ]
        for number, mutate in enumerate(mutations):
            changed = copy.deepcopy(original)
            mutate(changed)
            with self.subTest(partition_mutation=number), self.assertRaises(ValueError):
                self.verify(changed)

    def test_each_page_stale_missing_and_symlink_rejected(self):
        data = self.artifact()
        for page, _, _ in self.PAGES:
            with self.subTest(page=page), tempfile.TemporaryDirectory() as temp:
                root = Path(temp)
                for path, _, _ in self.PAGES:
                    target = root / path
                    target.parent.mkdir(parents=True, exist_ok=True)
                    target.write_bytes((self.source / path).read_bytes())
                target = root / page
                target.write_bytes(target.read_bytes() + b'\n')
                with self.assertRaises(ValueError):
                    self.verify(data, root)
                target.unlink()
                with self.assertRaises(FileNotFoundError):
                    self.verify(data, root)
                target.mkdir()
                with self.assertRaises(IsADirectoryError):
                    self.verify(data, root)
                target.rmdir()
                target.symlink_to(self.source / page)
                with self.assertRaises(ValueError):
                    self.verify(data, root)

    def test_source_only_unsupported_cli_and_api_fail_before_io(self):
        args = ['--profile', 'calls', '--rule-index', '/absent/index',
                '--inventory', '/absent/inventory', '--source-root', '/absent/source',
                '--artifact', '/absent/artifact']
        extras = ([], ['--repo-root', '/absent/repo'], ['--evidence', '/absent/evidence'],
                  ['--repo-root', '/absent/repo', '--evidence', '/absent/evidence'])
        actions = ('generate-witness-links', 'build-witness-links',
                   'validate-witness-links', 'assess-witness-links')
        cases = [(action, extra) for action in actions for extra in extras]
        cases += [(action, extra) for action in ('generate', 'validate') for extra in extras[1:]]
        with contextlib.ExitStack() as stack:
            for name in ('read_bytes', 'read_text', 'open', 'is_symlink', 'is_file',
                         'write_bytes', 'write_text', 'mkdir'):
                stack.enter_context(mock.patch.object(Path, name, side_effect=AssertionError('IO')))
            stack.enter_context(mock.patch.object(subprocess, 'run', side_effect=AssertionError('execution')))
            for action, extra in cases:
                with self.subTest(action=action, extra=extra), self.assertRaises(ValueError):
                    main([action, *args, *extra])
            calls = (
                lambda: reconciliation.build_witness_links(
                    self.index, self.inventory, self.source, self.source, 'calls'),
                lambda: reconciliation.validate_witness_links(
                    {}, self.index, self.inventory, self.source, self.source, 'calls'),
                lambda: reconciliation.assess_witness_links(
                    {}, self.index, self.inventory, self.source, self.source, (), 'calls'),
            )
            for call in calls:
                with self.subTest(api=call), self.assertRaises(ValueError):
                    call()

    def test_cli_generate_validate_exclusive_output(self):
        data = self.artifact()
        with tempfile.TemporaryDirectory() as temp:
            artifact = Path(temp) / 'calls.json'
            args = ['--profile', 'calls', '--rule-index', str(self.index),
                    '--inventory', str(self.inventory), '--source-root', str(self.source),
                    '--artifact', str(artifact)]
            with contextlib.redirect_stdout(io.StringIO()):
                main(['generate', *args])
                main(['validate', *args])
            raw = artifact.read_bytes()
            self.assertEqual(raw, (json.dumps(data, indent=2) + '\n').encode())
            with self.assertRaises(FileExistsError):
                main(['generate', *args])
            self.assertEqual(artifact.read_bytes(), raw)
            data['rows'][0]['review_status'] = 'reviewed'
            artifact.write_text(json.dumps(data))
            with self.assertRaises(ValueError):
                main(['validate', *args])

    def test_prior_closures_bytes_unchanged_without_calls_module(self):
        with mock.patch.dict(sys.modules, {'calls_reconciliation_proposals': None}):
            data = build(self.index, self.inventory, self.source, 'closures')
        self.assertEqual(
            digest((json.dumps(data, indent=2) + '\n').encode()),
            'b75b0495a40e114b0e698ac0e64a84a195ed0a1745f14a2b09f6b5ee043b4b64')



class PinnedAggregatesProfileTests(unittest.TestCase):
    """Independent pinned two-page oracle, not execution or review evidence."""

    PAGES = (
        ('src/expressions/array-expr.md',
         '2b5e385fc7243d93e9c9387931b214f069ba91b2528480758aaf186e43372a2b', (
             'expr.array', 'expr.array.syntax', 'expr.array.constructor',
             'expr.array.array', 'expr.array.array-syntax', 'expr.array.array-behavior',
             'expr.array.repeat', 'expr.array.repeat-operand', 'expr.array.length-operand',
             'expr.array.length-restriction', 'expr.array.repeat-behavior',
             'expr.array.repeat-copy', 'expr.array.repeat-const-item',
             'expr.array.repeat-evaluation-zero', 'expr.array.repeat-non-const',
             'expr.array.index', 'expr.array.index.syntax', 'expr.array.index.array',
             'expr.array.index.trait', 'expr.array.index.zero-index',
             'expr.array.index.const', 'expr.array.index.trait-impl')),
        ('src/expressions/tuple-expr.md',
         '909e6f0ad921fd84ddf8c0a482a100d33d2b2e0fc89c68bbf2773109986e02a8', (
             'expr.tuple', 'expr.tuple.syntax', 'expr.tuple.result', 'expr.tuple.intro',
             'expr.tuple.unary-tuple-restriction', 'expr.tuple.value', 'expr.tuple.type',
             'expr.tuple.unit', 'expr.tuple.fields', 'expr.tuple-index',
             'expr.tuple-index.syntax', 'expr.tuple-index.intro',
             'expr.tuple-index.index-syntax', 'expr.tuple-index.required-type',
             'expr.tuple-index.index-name-operand', 'expr.tuple-index.result')),
    )

    def setUp(self):
        source = os.environ.get('JUSTRUST_WITNESS_REFERENCE_ROOT')
        if not source:
            self.skipTest('set JUSTRUST_WITNESS_REFERENCE_ROOT for pinned-source structural checks')
        self.source = Path(source)
        repo = Path(os.environ.get('JUSTRUST_WITNESS_REPO_ROOT', BASE.parent))
        self.index = repo / 'data/compiler-language-reference-rules.json'
        self.inventory = repo / 'data/compiler-language-inventory.json'

    def artifact(self, source=None):
        return build(self.index, self.inventory,
                     self.source if source is None else source, 'aggregates')

    def verify(self, data, source=None):
        return validate(data, self.index, self.inventory,
                        self.source if source is None else source, 'aggregates')

    def test_actual_source_hash_labels_blocks_ids_partition_and_zero_credit(self):
        import hashlib
        import re
        data = self.artifact()
        counts = self.verify(data)
        self.assertEqual([counts[key] for key in ('selected_rule_rows',
                         'unselected_rule_identities', 'global_rule_identities')], [38, 2927, 2965])
        for key in ('reviewed_rule_rows', 'rows_with_exact_executable_evidence'):
            self.assertEqual(counts[key], 0)
        self.assertEqual([(p['path'], p['sha256']) for p in data['selected_pages']],
                         [(page, sha) for page, sha, _ in self.PAGES])
        self.assertEqual(tuple(row['rule'] for row in data['rows']),
                         tuple(label for _, _, labels in self.PAGES for label in labels))
        self.assertEqual(len({row['id'] for row in data['rows']}), 38)
        sha = lambda raw: hashlib.sha256(raw).hexdigest()
        offset = 0
        for page, page_sha, labels in self.PAGES:
            raw = (self.source / page).read_bytes()
            self.assertEqual(sha(raw), page_sha)
            lines = raw.splitlines(keepends=True)
            # Independent of parse_markdown and all authored proposal tables.
            markers = [(number, match.group(1).decode())
                       for number, line in enumerate(lines, 1)
                       if (match := re.fullmatch(rb'r\[([^\]\r\n]+)\]\r?\n?', line))]
            self.assertEqual(tuple(label for _, label in markers), labels)
            rows = data['rows'][offset:offset + len(labels)]
            self.assertEqual(len(rows), len(labels))
            for position, (row, (start, label)) in enumerate(zip(rows, markers)):
                with self.subTest(page=page, label=label):
                    end = markers[position + 1][0] - 1 if position + 1 < len(markers) else len(lines)
                    self.assertEqual((row['page'], row['line'], row['end_line']), (page, start, end))
                    self.assertEqual(row['source_block_sha256'], sha(b''.join(lines[start - 1:end])))
                    prefix = 'reconcile-aggregates-'
                    self.assertEqual(row['id'], prefix + label.encode().hex())
                    self.assertEqual(bytes.fromhex(row['id'][len(prefix):]).decode(), label)
                    self.assertEqual(row['review_status'], 'unreviewed')
                    self.assertTrue(row['missing_test_needs'])
                    self.assertTrue(all(need.startswith(('positive: Missing ', 'invalid: Missing ',
                                                        'interaction: Missing ', 'end-to-end: Missing '))
                                        for need in row['missing_test_needs']))
            offset += len(labels)
        index = json.loads(self.index.read_bytes())
        universe = {(p['path'], r['label']) for p in index['pages'] + index['unlinked_markdown_files']
                    for r in p['rules']}
        selected = {(row['page'], row['rule']) for row in data['rows']}
        other = {(row['page'], row['rule']) for row in data['unselected_rules']}
        self.assertEqual((len(selected), len(other), len(universe)), (38, 2927, 2965))
        self.assertEqual(len(data['unselected_rules']), 2927)
        self.assertFalse(selected & other)
        self.assertEqual(selected | other, universe)
        self.assertEqual(data, self.artifact())

    def test_per_page_missing_duplicate_foreign_authored_labels_rejected(self):
        import aggregates_reconciliation_proposals as proposals
        valid = self.artifact()
        rows = proposals.ROWS.strip().splitlines()
        for page, _, labels in self.PAGES:
            positions = [i for i, row in enumerate(rows) if row.split('|')[0] in labels]
            self.assertEqual(len(positions), len(labels))
            foreign = rows[positions[0]].split('|')
            foreign[0] = 'expr.loop'
            substituted = rows.copy()
            substituted[positions[0]] = '|'.join(foreign)
            cases = (
                ('missing', [row for i, row in enumerate(rows) if i != positions[-1]]),
                ('absent-page', [row for i, row in enumerate(rows) if i not in positions]),
                ('duplicate', rows + [rows[positions[0]]]),
                ('foreign', substituted),
            )
            for mutation, changed in cases:
                with self.subTest(page=page, mutation=mutation), \
                        mock.patch.object(proposals, 'ROWS', '\n'.join(changed)):
                    with self.assertRaises(ValueError):
                        self.artifact()
                    with self.assertRaises(ValueError):
                        self.verify(valid)

    def test_forged_per_page_rows_partition_bindings_and_credits_rejected(self):
        original = self.artifact()
        for page, _, _ in self.PAGES:
            first = next(i for i, row in enumerate(original['rows']) if row['page'] == page)
            foreign = next(row for row in original['rows'] if row['page'] != page)
            mutations = [
                lambda d: d['rows'].pop(first),
                lambda d: d['rows'].append(copy.deepcopy(d['rows'][first])),
                lambda d: d['rows'][first].update(page='src/tokens.md'),
                lambda d: d['rows'][first].update(rule='expr.loop'),
                lambda d: d['rows'][first].update(line=True),
                lambda d: d['rows'][first].update(end_line=99999),
                lambda d: d['rows'][first].update(source_block_sha256='0' * 64),
                lambda d: d['rows'][first].update(source_block_sha256=foreign['source_block_sha256']),
                lambda d: d['rows'][first].update(page=foreign['page'], rule=foreign['rule'],
                                                 line=foreign['line'], end_line=foreign['end_line'],
                                                 source_block_sha256=foreign['source_block_sha256']),
                lambda d: d['rows'][first].update(id=foreign['id']),
                lambda d: d['rows'][first].update(classifications=['normative-semantic']),
                lambda d: d['rows'][first].update(rationale='invented interpretation'),
                lambda d: d['rows'][first].update(candidate_obligation_ids=['foreign']),
                lambda d: d['rows'][first].update(missing_test_needs=[]),
                lambda d: d['rows'][first]['missing_test_needs'].__setitem__(0, 'positive: implemented'),
                lambda d: d['rows'][first].update(id='reconcile-aggregates-expr-array'),
                lambda d: d['rows'][first].update(review_status='reviewed'),
            ]
            for number, mutate in enumerate(mutations):
                changed = copy.deepcopy(original)
                mutate(changed)
                with self.subTest(page=page, mutation=number), self.assertRaises(ValueError):
                    self.verify(changed)
        mutations = [
            lambda d: d['counts'].update(reviewed_rule_rows=1),
            lambda d: d['counts'].update(rows_with_exact_executable_evidence=1),
            lambda d: d['selected_pages'][0].update(sha256=d['selected_pages'][1]['sha256']),
            lambda d: d['selected_pages'][1].update(sha256=d['selected_pages'][0]['sha256']),
            lambda d: d['selected_pages'].pop(0),
            lambda d: d['selected_pages'].pop(1),
            lambda d: d['selected_pages'].reverse(),
            lambda d: d['bindings'].update(reference_commit='0' * 40),
            lambda d: d['unselected_rules'].pop(),
            lambda d: d['unselected_rules'].append(copy.deepcopy(d['unselected_rules'][0])),
            lambda d: d['unselected_rules'][0].update(page=d['rows'][0]['page'], rule=d['rows'][0]['rule']),
        ]
        for number, mutate in enumerate(mutations):
            changed = copy.deepcopy(original)
            mutate(changed)
            with self.subTest(partition_mutation=number), self.assertRaises(ValueError):
                self.verify(changed)

    def test_each_page_stale_missing_directory_and_symlink_exact_api_errors(self):
        data = self.artifact()
        for page, _, _ in self.PAGES:
            with self.subTest(page=page), tempfile.TemporaryDirectory() as temp:
                root = Path(temp)
                for path, _, _ in self.PAGES:
                    target = root / path
                    target.parent.mkdir(parents=True, exist_ok=True)
                    target.write_bytes((self.source / path).read_bytes())
                target = root / page
                for state, error in (('stale', ValueError), ('missing', FileNotFoundError),
                                     ('directory', IsADirectoryError), ('symlink', ValueError)):
                    if state == 'stale':
                        target.write_bytes(target.read_bytes() + b'\n')
                    elif state == 'missing':
                        target.unlink()
                    elif state == 'directory':
                        target.mkdir()
                    else:
                        target.rmdir()
                        target.symlink_to(self.source / page)
                    for action in (lambda: self.artifact(root), lambda: self.verify(data, root)):
                        with self.subTest(state=state, action=action), self.assertRaises(error):
                            action()

    def test_source_only_unsupported_cli_and_api_fail_before_io(self):
        args = ['--profile', 'aggregates', '--rule-index', '/absent/index',
                '--inventory', '/absent/inventory', '--source-root', '/absent/source',
                '--artifact', '/absent/artifact']
        extras = ([], ['--repo-root', '/absent/repo'], ['--evidence', '/absent/evidence'],
                  ['--repo-root', '/absent/repo', '--evidence', '/absent/evidence'])
        actions = ('generate-witness-links', 'build-witness-links',
                   'validate-witness-links', 'assess-witness-links')
        cases = [(action, extra) for action in actions for extra in extras]
        cases += [(action, extra) for action in ('generate', 'validate') for extra in extras[1:]]
        with contextlib.ExitStack() as stack:
            for name in ('read_bytes', 'read_text', 'open', 'is_symlink', 'is_file',
                         'write_bytes', 'write_text', 'mkdir'):
                stack.enter_context(mock.patch.object(Path, name, side_effect=AssertionError('IO')))
            stack.enter_context(mock.patch.object(subprocess, 'run', side_effect=AssertionError('execution')))
            for action, extra in cases:
                with self.subTest(action=action, extra=extra), self.assertRaises(ValueError):
                    main([action, *args, *extra])
            calls = (
                lambda: reconciliation.build_witness_links(
                    self.index, self.inventory, self.source, self.source, 'aggregates'),
                lambda: reconciliation.validate_witness_links(
                    {}, self.index, self.inventory, self.source, self.source, 'aggregates'),
                lambda: reconciliation.assess_witness_links(
                    {}, self.index, self.inventory, self.source, self.source, (), 'aggregates'),
            )
            for call in calls:
                with self.subTest(api=call), self.assertRaises(ValueError):
                    call()

    def test_cli_generate_validate_exclusive_output(self):
        data = self.artifact()
        with tempfile.TemporaryDirectory() as temp:
            artifact = Path(temp) / 'aggregates.json'
            args = ['--profile', 'aggregates', '--rule-index', str(self.index),
                    '--inventory', str(self.inventory), '--source-root', str(self.source),
                    '--artifact', str(artifact)]
            with contextlib.redirect_stdout(io.StringIO()):
                main(['generate', *args])
                main(['validate', *args])
            raw = artifact.read_bytes()
            self.assertEqual(raw, (json.dumps(data, indent=2) + '\n').encode())
            with self.assertRaises(FileExistsError):
                main(['generate', *args])
            self.assertEqual(artifact.read_bytes(), raw)
            data['rows'][0]['review_status'] = 'reviewed'
            artifact.write_text(json.dumps(data))
            with self.assertRaises(ValueError):
                main(['validate', *args])

    def test_prior_calls_bytes_unchanged_without_aggregates_module(self):
        with mock.patch.dict(sys.modules, {'aggregates_reconciliation_proposals': None}):
            data = build(self.index, self.inventory, self.source, 'calls')
        self.assertEqual(
            digest((json.dumps(data, indent=2) + '\n').encode()),
            '42efdae31602a2af4f2fc243d5a74404159f877b278820905a7b4086281d302e')

class PinnedFieldsProfileTests(unittest.TestCase):
    """Independent pinned field-page oracle, not review or execution evidence."""

    PAGE = 'src/expressions/field-expr.md'
    SHA256 = 'd682b9f926ce090030c7b9ae16b0b6c89d65a921556f2bc0344192b7aaee5a37'
    LABELS = ('expr.field', 'expr.field.syntax', 'expr.field.intro',
              'expr.field.mut', 'expr.field.form', 'expr.field.not-method-call',
              'expr.field.autoref-deref', 'expr.field.borrow')

    def setUp(self):
        source = os.environ.get('JUSTRUST_WITNESS_REFERENCE_ROOT')
        if not source:
            self.skipTest('set JUSTRUST_WITNESS_REFERENCE_ROOT for pinned-source structural checks')
        self.source = Path(source)
        repo = Path(os.environ.get('JUSTRUST_WITNESS_REPO_ROOT', BASE.parent))
        self.index = repo / 'data/compiler-language-reference-rules.json'
        self.inventory = repo / 'data/compiler-language-inventory.json'

    def artifact(self, source=None):
        return build(self.index, self.inventory,
                     self.source if source is None else source, 'fields')

    def verify(self, data, source=None):
        return validate(data, self.index, self.inventory,
                        self.source if source is None else source, 'fields')

    def test_pinned_order_hash_lineblocks_reversible_ids_and_partition(self):
        import hashlib
        import re
        data = self.artifact()
        counts = self.verify(data)
        self.assertEqual([counts[key] for key in ('selected_rule_rows',
                         'unselected_rule_identities', 'global_rule_identities')], [8, 2957, 2965])
        self.assertEqual([(p['path'], p['sha256']) for p in data['selected_pages']],
                         [(self.PAGE, self.SHA256)])
        self.assertEqual(tuple(row['rule'] for row in data['rows']), self.LABELS)
        raw = (self.source / self.PAGE).read_bytes()
        sha = lambda value: hashlib.sha256(value).hexdigest()
        self.assertEqual(sha(raw), self.SHA256)
        lines = raw.splitlines(keepends=True)
        markers = [(number, match.group(1).decode())
                   for number, line in enumerate(lines, 1)
                   if (match := re.fullmatch(rb'r\[([^\]\r\n]+)\]\r?\n?', line))]
        self.assertEqual(tuple(label for _, label in markers), self.LABELS)
        for position, (row, (start, label)) in enumerate(zip(data['rows'], markers)):
            with self.subTest(label=label):
                end = markers[position + 1][0] - 1 if position + 1 < len(markers) else len(lines)
                self.assertEqual((row['page'], row['line'], row['end_line']), (self.PAGE, start, end))
                self.assertEqual(row['source_block_sha256'], sha(b''.join(lines[start - 1:end])))
                prefix = 'reconcile-fields-'
                self.assertEqual(row['id'], prefix + label.encode().hex())
                self.assertEqual(bytes.fromhex(row['id'][len(prefix):]).decode(), label)
                self.assertEqual(row['review_status'], 'unreviewed')
                self.assertTrue(row['missing_test_needs'])
                self.assertTrue(all(need.startswith(('positive: Missing ', 'invalid: Missing ',
                                                    'interaction: Missing ', 'end-to-end: Missing '))
                                    for need in row['missing_test_needs']))
        self.assertEqual(len({row['id'] for row in data['rows']}), 8)
        for key in ('reviewed_rule_rows', 'rows_with_exact_executable_evidence'):
            self.assertEqual(counts[key], 0)
        index = json.loads(self.index.read_bytes())
        universe = {(p['path'], r['label']) for p in index['pages'] + index['unlinked_markdown_files']
                    for r in p['rules']}
        selected = {(row['page'], row['rule']) for row in data['rows']}
        other = {(row['page'], row['rule']) for row in data['unselected_rules']}
        self.assertEqual((len(selected), len(other), len(universe)), (8, 2957, 2965))
        self.assertEqual(len(data['unselected_rules']), 2957)
        self.assertFalse(selected & other)
        self.assertEqual(selected | other, universe)
        self.assertEqual(data, self.artifact())

    def test_missing_duplicate_foreign_authored_labels_rejected(self):
        import fields_reconciliation_proposals as proposals
        self.assertEqual(proposals.SELECTED, (self.PAGE,))
        valid = self.artifact()
        rows = proposals.ROWS.strip().splitlines()
        self.assertEqual(tuple(row.split('|')[0] for row in rows), self.LABELS)
        foreign = rows[0].split('|')
        foreign[0] = 'expr.loop'
        for name, changed in (('missing', rows[:-1]), ('absent-page', []),
                              ('duplicate', rows + [rows[0]]),
                              ('foreign', ['|'.join(foreign), *rows[1:]])):
            with self.subTest(mutation=name), mock.patch.object(proposals, 'ROWS', '\n'.join(changed)):
                with self.assertRaises(ValueError):
                    self.artifact()
                with self.assertRaises(ValueError):
                    self.verify(valid)

    def test_forged_artifacts_cannot_claim_credit_or_rebind_source(self):
        original = self.artifact()
        mutations = (
            lambda d: d['rows'].pop(),
            lambda d: d['rows'].append(copy.deepcopy(d['rows'][0])),
            lambda d: d['rows'].reverse(),
            lambda d: d['rows'][0].update(page='src/tokens.md'),
            lambda d: d['rows'][0].update(rule='expr.loop'),
            lambda d: d['rows'][0].update(line=True),
            lambda d: d['rows'][0].update(end_line=99999),
            lambda d: d['rows'][0].update(source_block_sha256='0' * 64),
            lambda d: d['rows'][0].update(id='reconcile-fields-expr-field'),
            lambda d: d['rows'][0].update(id=d['rows'][1]['id']),
            lambda d: d['rows'][0].update(review_status='reviewed'),
            lambda d: d['rows'][0].update(rationale='invented interpretation'),
            lambda d: d['rows'][0].update(candidate_obligation_ids=['foreign']),
            lambda d: d['rows'][0].update(missing_test_needs=[]),
            lambda d: d['rows'][0]['missing_test_needs'].__setitem__(0, 'positive: implemented'),
            lambda d: d['counts'].update(reviewed_rule_rows=1),
            lambda d: d['counts'].update(rows_with_exact_executable_evidence=1),
            lambda d: d['selected_pages'][0].update(sha256='0' * 64),
            lambda d: d['selected_pages'].clear(),
            lambda d: d['bindings'].update(reference_commit='0' * 40),
            lambda d: d['unselected_rules'].pop(),
            lambda d: d['unselected_rules'].append(copy.deepcopy(d['unselected_rules'][0])),
            lambda d: d['unselected_rules'][0].update(page=d['rows'][0]['page'], rule=d['rows'][0]['rule']),
        )
        for number, mutate in enumerate(mutations):
            changed = copy.deepcopy(original)
            mutate(changed)
            with self.subTest(mutation=number), self.assertRaises(ValueError):
                self.verify(changed)

    def test_stale_missing_directory_symlink_preserve_exact_api_errors(self):
        data = self.artifact()
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            target = root / self.PAGE
            target.parent.mkdir(parents=True)
            target.write_bytes((self.source / self.PAGE).read_bytes())
            for state, error in (('stale', ValueError), ('missing', FileNotFoundError),
                                 ('directory', IsADirectoryError), ('symlink', ValueError)):
                if state == 'stale':
                    target.write_bytes(target.read_bytes() + b'\n')
                elif state == 'missing':
                    target.unlink()
                elif state == 'directory':
                    target.mkdir()
                else:
                    target.rmdir()
                    target.symlink_to(self.source / self.PAGE)
                for action in (lambda: self.artifact(root), lambda: self.verify(data, root)):
                    with self.subTest(state=state, action=action), self.assertRaises(error):
                        action()

    def test_source_only_witness_restrictions_fail_before_io(self):
        args = ['--profile', 'fields', '--rule-index', '/absent/index',
                '--inventory', '/absent/inventory', '--source-root', '/absent/source',
                '--artifact', '/absent/artifact']
        extras = ([], ['--repo-root', '/absent/repo'], ['--evidence', '/absent/evidence'],
                  ['--repo-root', '/absent/repo', '--evidence', '/absent/evidence'])
        actions = ('generate-witness-links', 'build-witness-links',
                   'validate-witness-links', 'assess-witness-links')
        cases = [(action, extra) for action in actions for extra in extras]
        cases += [(action, extra) for action in ('generate', 'validate') for extra in extras[1:]]
        with contextlib.ExitStack() as stack:
            for name in ('read_bytes', 'read_text', 'open', 'is_symlink', 'is_file',
                         'write_bytes', 'write_text', 'mkdir'):
                stack.enter_context(mock.patch.object(Path, name, side_effect=AssertionError('IO')))
            stack.enter_context(mock.patch.object(subprocess, 'run', side_effect=AssertionError('execution')))
            for action, extra in cases:
                with self.subTest(action=action, extra=extra), self.assertRaises(ValueError):
                    main([action, *args, *extra])
            calls = (
                lambda: reconciliation.build_witness_links(
                    self.index, self.inventory, self.source, self.source, 'fields'),
                lambda: reconciliation.validate_witness_links(
                    {}, self.index, self.inventory, self.source, self.source, 'fields'),
                lambda: reconciliation.assess_witness_links(
                    {}, self.index, self.inventory, self.source, self.source, (), 'fields'),
            )
            for call in calls:
                with self.subTest(api=call), self.assertRaises(ValueError):
                    call()

    def test_legacy_aggregate_bytes_unchanged_without_fields_module(self):
        with mock.patch.dict(sys.modules, {'fields_reconciliation_proposals': None}):
            data = build(self.index, self.inventory, self.source, 'aggregates')
        self.assertEqual(
            digest((json.dumps(data, indent=2) + '\n').encode()),
            '8c83d418848de65f5251d3b23248dd09121d7ac82b1099739648de6a034a55b3')


class PinnedLiteralsProfileTests(unittest.TestCase):
    """Independent pinned literal-value-page oracle, not review or execution evidence."""

    PAGE = 'src/expressions/literal-expr.md'
    SHA256 = 'e64c52af55902d0bdca89eed07fe6ac9bf1239e96c901dabfddfb0dc9a7d43d8'
    LABELS = (
        'expr.literal',
        'expr.literal.syntax',
        'expr.literal.intro',
        'expr.literal.const-expr',
        'expr.literal.literal-token',
        'expr.literal.string-representation',
        'expr.literal.escape',
        'expr.literal.escape.intro',
        'expr.literal.escape.sequence',
        'expr.literal.escape.simple',
        'expr.literal.escape.hex-octet',
        'expr.literal.escape.hex-ascii',
        'expr.literal.escape.unicode',
        'expr.literal.continuation',
        'expr.literal.char',
        'expr.literal.char.intro',
        'expr.literal.char.type',
        'expr.literal.char.no-suffix',
        'expr.literal.char.literal-content',
        'expr.literal.char.represented',
        'expr.literal.char.escape',
        'expr.literal.char.single',
        'expr.literal.char.result',
        'expr.literal.string',
        'expr.literal.string.intro',
        'expr.literal.string.type',
        'expr.literal.string.no-suffix',
        'expr.literal.string.literal-content',
        'expr.literal.string.represented',
        'expr.literal.string.escape',
        'expr.literal.string.raw',
        'expr.literal.string.result',
        'expr.literal.byte-char',
        'expr.literal.byte-char.intro',
        'expr.literal.byte-char.literal',
        'expr.literal.byte-char.no-suffix',
        'expr.literal.byte-char.literal-content',
        'expr.literal.byte-char.represented',
        'expr.literal.byte-char.escape',
        'expr.literal.byte-char.single',
        'expr.literal.byte-char.result',
        'expr.literal.byte-string',
        'expr.literal.byte-string.intro',
        'expr.literal.byte-string.type',
        'expr.literal.byte-string.no-suffix',
        'expr.literal.byte-string.literal-content',
        'expr.literal.byte-string.represented',
        'expr.literal.byte-string.escape',
        'expr.literal.byte-string.raw',
        'expr.literal.byte-string.result',
        'expr.literal.c-string',
        'expr.literal.c-string.intro',
        'expr.literal.c-string.type',
        'expr.literal.c-string.no-suffix',
        'expr.literal.c-string.literal-content',
        'expr.literal.c-string.represented',
        'expr.literal.c-string.escape',
        'expr.literal.c-string.raw',
        'expr.literal.c-string.result',
        'expr.literal.int',
        'expr.literal.int.intro',
        'expr.literal.int.suffix',
        'expr.literal.int.infer',
        'expr.literal.int.inference-unique-type',
        'expr.literal.int.inference-default',
        'expr.literal.int.inference-error',
        'expr.literal.int.representation',
        'expr.literal.int.radix',
        'expr.literal.int.radix-prefix-stripped',
        'expr.literal.int.type-suffix-stripped',
        'expr.literal.int.separators-stripped',
        'expr.literal.int.u128-value',
        'expr.literal.int.cast',
        'expr.literal.float',
        'expr.literal.float.intro',
        'expr.literal.float.suffix',
        'expr.literal.float.infer',
        'expr.literal.float.inference-unique-type',
        'expr.literal.float.inference-default',
        'expr.literal.float.inference-error',
        'expr.literal.float.result',
        'expr.literal.float.type-suffix-stripped',
        'expr.literal.float.separators-stripped',
        'expr.literal.float.value',
        'expr.literal.bool',
        'expr.literal.bool.intro',
        'expr.literal.bool.result',
    )

    def setUp(self):
        source = os.environ.get('JUSTRUST_WITNESS_REFERENCE_ROOT')
        if not source:
            self.skipTest('set JUSTRUST_WITNESS_REFERENCE_ROOT for pinned-source structural checks')
        self.source = Path(source)
        repo = Path(os.environ.get('JUSTRUST_WITNESS_REPO_ROOT', BASE.parent))
        self.index = repo / 'data/compiler-language-reference-rules.json'
        self.inventory = repo / 'data/compiler-language-inventory.json'

    def artifact(self, source=None):
        return build(self.index, self.inventory,
                     self.source if source is None else source, 'literals')

    def verify(self, data, source=None):
        return validate(data, self.index, self.inventory,
                        self.source if source is None else source, 'literals')

    def test_pinned_order_hash_lineblocks_reversible_ids_and_partition(self):
        import hashlib
        import re
        data = self.artifact()
        counts = self.verify(data)
        self.assertEqual([counts[key] for key in ('selected_rule_rows',
                         'unselected_rule_identities', 'global_rule_identities')], [87, 2878, 2965])
        self.assertEqual([(p['path'], p['sha256']) for p in data['selected_pages']],
                         [(self.PAGE, self.SHA256)])
        self.assertEqual(tuple(row['rule'] for row in data['rows']), self.LABELS)
        raw = (self.source / self.PAGE).read_bytes()
        sha = lambda value: hashlib.sha256(value).hexdigest()
        self.assertEqual(sha(raw), self.SHA256)
        lines = raw.splitlines(keepends=True)
        self.assertEqual(len(lines), 521)
        markers = [(number, match.group(1).decode())
                   for number, line in enumerate(lines, 1)
                   if (match := re.fullmatch(rb'r\[([^\]\r\n]+)\]\r?\n?', line))]
        self.assertEqual(tuple(label for _, label in markers), self.LABELS)
        for position, (row, (start, label)) in enumerate(zip(data['rows'], markers)):
            with self.subTest(label=label):
                end = markers[position + 1][0] - 1 if position + 1 < len(markers) else len(lines)
                self.assertEqual((row['page'], row['line'], row['end_line']), (self.PAGE, start, end))
                self.assertEqual(row['source_block_sha256'], sha(b''.join(lines[start - 1:end])))
                prefix = 'reconcile-literals-'
                self.assertEqual(row['id'], prefix + label.encode().hex())
                self.assertEqual(bytes.fromhex(row['id'][len(prefix):]).decode(), label)
                self.assertEqual(row['review_status'], 'unreviewed')
                self.assertTrue(row['missing_test_needs'])
                self.assertTrue(all(need.startswith(('positive: Missing ', 'invalid: Missing ',
                                                    'interaction: Missing ', 'end-to-end: Missing '))
                                    for need in row['missing_test_needs']))
        self.assertEqual(len({row['id'] for row in data['rows']}), 87)
        for key in ('reviewed_rule_rows', 'rows_with_exact_executable_evidence'):
            self.assertEqual(counts[key], 0)
        index = json.loads(self.index.read_bytes())
        universe = {(p['path'], r['label']) for p in index['pages'] + index['unlinked_markdown_files']
                    for r in p['rules']}
        selected = {(row['page'], row['rule']) for row in data['rows']}
        other = {(row['page'], row['rule']) for row in data['unselected_rules']}
        self.assertEqual((len(selected), len(other), len(universe)), (87, 2878, 2965))
        self.assertEqual(len(data['unselected_rules']), 2878)
        self.assertFalse(selected & other)
        self.assertEqual(selected | other, universe)
        self.assertEqual(data, self.artifact())

    def test_missing_duplicate_foreign_authored_labels_rejected(self):
        import literals_reconciliation_proposals as proposals
        self.assertEqual(proposals.SELECTED, (self.PAGE,))
        valid = self.artifact()
        rows = proposals.ROWS.strip().splitlines()
        self.assertEqual(tuple(row.split('|')[0] for row in rows), self.LABELS)
        foreign = rows[0].split('|')
        foreign[0] = 'expr.loop'
        blank_needs = []
        for column in (4, 5, 6):
            changed = rows[0].split('|')
            changed[column] = '   '
            blank_needs.append(('blank-need-' + str(column), ['|'.join(changed), *rows[1:]]))
        for name, changed in (('missing', rows[:-1]), ('absent-page', []),
                              ('duplicate', rows + [rows[0]]),
                              ('foreign', ['|'.join(foreign), *rows[1:]]),
                              *blank_needs):
            with self.subTest(mutation=name), mock.patch.object(proposals, 'ROWS', '\n'.join(changed)):
                with self.assertRaises(ValueError):
                    self.artifact()
                with self.assertRaises(ValueError):
                    self.verify(valid)

    def test_forged_artifacts_cannot_claim_credit_or_rebind_source(self):
        original = self.artifact()
        mutations = (
            lambda d: d['rows'].pop(),
            lambda d: d['rows'].append(copy.deepcopy(d['rows'][0])),
            lambda d: d['rows'].reverse(),
            lambda d: d['rows'][0].update(page='src/tokens.md'),
            lambda d: d['rows'][0].update(rule='expr.loop'),
            lambda d: d['rows'][0].update(line=True),
            lambda d: d['rows'][0].update(end_line=99999),
            lambda d: d['rows'][0].update(source_block_sha256='0' * 64),
            lambda d: d['rows'][0].update(id='reconcile-literals-expr-field'),
            lambda d: d['rows'][0].update(id=d['rows'][1]['id']),
            lambda d: d['rows'][0].update(review_status='reviewed'),
            lambda d: d['rows'][0].update(rationale='invented interpretation'),
            lambda d: d['rows'][0].update(candidate_obligation_ids=['foreign']),
            lambda d: d['rows'][0].update(missing_test_needs=[]),
            lambda d: d['rows'][0]['missing_test_needs'].__setitem__(0, 'positive: implemented'),
            lambda d: d['counts'].update(reviewed_rule_rows=1),
            lambda d: d['counts'].update(rows_with_exact_executable_evidence=1),
            lambda d: d['selected_pages'][0].update(sha256='0' * 64),
            lambda d: d['selected_pages'].clear(),
            lambda d: d['bindings'].update(reference_commit='0' * 40),
            lambda d: d['unselected_rules'].pop(),
            lambda d: d['unselected_rules'].append(copy.deepcopy(d['unselected_rules'][0])),
            lambda d: d['unselected_rules'][0].update(page=d['rows'][0]['page'], rule=d['rows'][0]['rule']),
        )
        for number, mutate in enumerate(mutations):
            changed = copy.deepcopy(original)
            mutate(changed)
            with self.subTest(mutation=number), self.assertRaises(ValueError):
                self.verify(changed)

    def test_stale_missing_directory_symlink_preserve_exact_api_errors(self):
        data = self.artifact()
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            target = root / self.PAGE
            target.parent.mkdir(parents=True)
            target.write_bytes((self.source / self.PAGE).read_bytes())
            for state, error in (('stale', ValueError), ('missing', FileNotFoundError),
                                 ('directory', IsADirectoryError), ('symlink', ValueError)):
                if state == 'stale':
                    target.write_bytes(target.read_bytes() + b'\n')
                elif state == 'missing':
                    target.unlink()
                elif state == 'directory':
                    target.mkdir()
                else:
                    target.rmdir()
                    target.symlink_to(self.source / self.PAGE)
                for action in (lambda: self.artifact(root), lambda: self.verify(data, root)):
                    with self.subTest(state=state, action=action), self.assertRaises(error):
                        action()

    def test_source_only_witness_restrictions_fail_before_io(self):
        args = ['--profile', 'literals', '--rule-index', '/absent/index',
                '--inventory', '/absent/inventory', '--source-root', '/absent/source',
                '--artifact', '/absent/artifact']
        extras = ([], ['--repo-root', '/absent/repo'], ['--evidence', '/absent/evidence'],
                  ['--repo-root', '/absent/repo', '--evidence', '/absent/evidence'])
        actions = ('generate-witness-links', 'build-witness-links',
                   'validate-witness-links', 'assess-witness-links')
        cases = [(action, extra) for action in actions for extra in extras]
        cases += [(action, extra) for action in ('generate', 'validate') for extra in extras[1:]]
        with contextlib.ExitStack() as stack:
            for name in ('read_bytes', 'read_text', 'open', 'is_symlink', 'is_file',
                         'write_bytes', 'write_text', 'mkdir'):
                stack.enter_context(mock.patch.object(Path, name, side_effect=AssertionError('IO')))
            stack.enter_context(mock.patch.object(subprocess, 'run', side_effect=AssertionError('execution')))
            for action, extra in cases:
                with self.subTest(action=action, extra=extra), self.assertRaises(ValueError):
                    main([action, *args, *extra])
            calls = (
                lambda: reconciliation.build_witness_links(
                    self.index, self.inventory, self.source, self.source, 'literals'),
                lambda: reconciliation.validate_witness_links(
                    {}, self.index, self.inventory, self.source, self.source, 'literals'),
                lambda: reconciliation.assess_witness_links(
                    {}, self.index, self.inventory, self.source, self.source, (), 'literals'),
            )
            for call in calls:
                with self.subTest(api=call), self.assertRaises(ValueError):
                    call()

    def test_cli_exclusive_output_and_validate(self):
        with tempfile.TemporaryDirectory() as temp:
            artifact = Path(temp) / 'literals.json'
            args = ['--profile', 'literals', '--rule-index', str(self.index),
                    '--inventory', str(self.inventory), '--source-root', str(self.source),
                    '--artifact', str(artifact)]
            with contextlib.redirect_stdout(io.StringIO()) as output:
                main(['generate', *args])
            canonical = (json.dumps(self.artifact(), indent=2) + '\n').encode()
            self.assertEqual(artifact.read_bytes(), canonical)
            self.assertEqual(json.loads(output.getvalue()), self.verify(self.artifact()))
            with contextlib.redirect_stdout(io.StringIO()), self.assertRaises(FileExistsError):
                main(['generate', *args])
            self.assertEqual(artifact.read_bytes(), canonical)
            with contextlib.redirect_stdout(io.StringIO()) as output:
                main(['validate', *args])
            self.assertEqual(json.loads(output.getvalue()), self.verify(self.artifact()))
            self.assertEqual(artifact.read_bytes(), canonical)
            forged = json.loads(canonical)
            forged['counts']['reviewed_rule_rows'] = 1
            artifact.write_text(json.dumps(forged))
            before = artifact.read_bytes()
            with contextlib.redirect_stdout(io.StringIO()), self.assertRaises(ValueError):
                main(['validate', *args])
            self.assertEqual(artifact.read_bytes(), before)

    def test_legacy_field_canonical_bytes_unchanged_without_literals_module(self):
        canonical = lambda data: (json.dumps(data, indent=2) + '\n').encode()
        expected = canonical(build(self.index, self.inventory, self.source, 'fields'))
        with mock.patch.dict(sys.modules, {'literals_reconciliation_proposals': None}):
            data = build(self.index, self.inventory, self.source, 'fields')
            validate(data, self.index, self.inventory, self.source, 'fields')
        self.assertEqual(canonical(data), expected)
        self.assertEqual(digest(canonical(data)),
                         'e53fbee9b1248907fb91f9524440cfa75e5991fa0146800c6f0be3f2c67541b2')


class PinnedGroupedProfileTests(unittest.TestCase):
    """Independent complete grouped-page oracle, not review or execution evidence."""

    PAGE = 'src/expressions/grouped-expr.md'
    SHA256 = '5f068eea2424954be26b8a0c8e74bea57105a07a8c38820f06f3224cfefca96d'
    LABELS = ('expr.paren', 'expr.paren.syntax', 'expr.paren.intro',
              'expr.paren.evaluation', 'expr.paren.place-or-value',
              'expr.paren.override-precedence')

    def setUp(self):
        source = os.environ.get('JUSTRUST_WITNESS_REFERENCE_ROOT')
        if not source:
            self.skipTest('set JUSTRUST_WITNESS_REFERENCE_ROOT for pinned-source structural checks')
        self.source = Path(source)
        repo = Path(os.environ.get('JUSTRUST_WITNESS_REPO_ROOT', BASE.parent))
        self.index = repo / 'data/compiler-language-reference-rules.json'
        self.inventory = repo / 'data/compiler-language-inventory.json'

    def artifact(self, source=None):
        return build(self.index, self.inventory,
                     self.source if source is None else source, 'grouped')

    def verify(self, data, source=None):
        return validate(data, self.index, self.inventory,
                        self.source if source is None else source, 'grouped')

    def test_pinned_order_hash_lineblocks_reversible_ids_and_partition(self):
        import hashlib
        import re
        data = self.artifact()
        counts = self.verify(data)
        self.assertEqual([counts[key] for key in ('selected_rule_rows',
                         'unselected_rule_identities', 'global_rule_identities')], [6, 2959, 2965])
        self.assertEqual([(p['path'], p['sha256']) for p in data['selected_pages']],
                         [(self.PAGE, self.SHA256)])
        self.assertEqual(tuple(row['rule'] for row in data['rows']), self.LABELS)
        raw = (self.source / self.PAGE).read_bytes()
        sha = lambda value: hashlib.sha256(value).hexdigest()
        self.assertEqual(sha(raw), self.SHA256)
        lines = raw.splitlines(keepends=True)
        self.assertEqual(len(lines), 47)
        markers = [(number, match.group(1).decode())
                   for number, line in enumerate(lines, 1)
                   if (match := re.fullmatch(rb'r\[([^\]\r\n]+)\]\r?\n?', line))]
        self.assertEqual(markers, list(zip((1, 4, 9, 12, 15, 18), self.LABELS)))
        for position, (row, (start, label)) in enumerate(zip(data['rows'], markers)):
            with self.subTest(label=label):
                end = markers[position + 1][0] - 1 if position + 1 < len(markers) else len(lines)
                self.assertEqual((row['page'], row['line'], row['end_line']), (self.PAGE, start, end))
                self.assertEqual(row['source_block_sha256'], sha(b''.join(lines[start - 1:end])))
                prefix = 'reconcile-grouped-'
                self.assertEqual(row['id'], prefix + label.encode().hex())
                self.assertEqual(bytes.fromhex(row['id'][len(prefix):]).decode(), label)
                self.assertEqual(row['review_status'], 'unreviewed')
                self.assertTrue(row['missing_test_needs'])
                self.assertTrue(all(need.startswith(('positive: Missing ', 'invalid: Missing ',
                                                    'interaction: Missing ', 'end-to-end: Missing '))
                                    for need in row['missing_test_needs']))
        self.assertEqual(len({row['id'] for row in data['rows']}), 6)
        for key in ('reviewed_rule_rows', 'rows_with_exact_executable_evidence'):
            self.assertEqual(counts[key], 0)
        index = json.loads(self.index.read_bytes())
        universe = {(p['path'], r['label']) for p in index['pages'] + index['unlinked_markdown_files']
                    for r in p['rules']}
        selected = {(row['page'], row['rule']) for row in data['rows']}
        other = {(row['page'], row['rule']) for row in data['unselected_rules']}
        self.assertEqual((len(selected), len(other), len(universe)), (6, 2959, 2965))
        self.assertEqual(len(data['unselected_rules']), 2959)
        self.assertFalse(selected & other)
        self.assertEqual(selected | other, universe)
        self.assertEqual(data, self.artifact())

    def test_missing_duplicate_foreign_and_malformed_authored_rows_rejected(self):
        import grouped_reconciliation_proposals as proposals
        self.assertEqual(proposals.SELECTED, (self.PAGE,))
        valid = self.artifact()
        rows = proposals.ROWS.strip().splitlines()
        self.assertEqual(tuple(row.split('|')[0] for row in rows), self.LABELS)
        foreign = rows[0].split('|')
        foreign[0] = 'expr.loop'
        blank_needs = []
        for column in (4, 5, 6):
            changed = rows[0].split('|')
            changed[column] = '   '
            blank_needs.append(('blank-need-' + str(column), ['|'.join(changed), *rows[1:]]))
        for name, changed in (('missing', rows[:-1]), ('absent-page', []),
                              ('duplicate', rows + [rows[0]]),
                              ('foreign', ['|'.join(foreign), *rows[1:]]),
                              ('short-row', ['|'.join(rows[0].split('|')[:-1]), *rows[1:]]),
                              ('long-row', [rows[0] + '|extra', *rows[1:]]),
                              *blank_needs):
            with self.subTest(mutation=name), mock.patch.object(proposals, 'ROWS', '\n'.join(changed)):
                with self.assertRaises(ValueError):
                    self.artifact()
                with self.assertRaises(ValueError):
                    self.verify(valid)

    def test_forged_artifacts_cannot_claim_credit_or_rebind_source(self):
        original = self.artifact()
        mutations = (
            lambda d: d['rows'].pop(),
            lambda d: d['rows'].append(copy.deepcopy(d['rows'][0])),
            lambda d: d['rows'].reverse(),
            lambda d: d['rows'][0].update(page='src/tokens.md'),
            lambda d: d['rows'][0].update(rule='expr.loop'),
            lambda d: d['rows'][0].update(line=True),
            lambda d: d['rows'][0].update(end_line=99999),
            lambda d: d['rows'][0].update(source_block_sha256='0' * 64),
            lambda d: d['rows'][0].update(id='reconcile-grouped-expr-paren'),
            lambda d: d['rows'][0].update(id=d['rows'][1]['id']),
            lambda d: d['rows'][0].update(review_status='reviewed'),
            lambda d: d['rows'][0].update(rationale='invented interpretation'),
            lambda d: d['rows'][0].update(candidate_obligation_ids=['foreign']),
            lambda d: d['rows'][0].update(missing_test_needs=[]),
            lambda d: d['rows'][0]['missing_test_needs'].__setitem__(0, 'positive: implemented'),
            lambda d: d['counts'].update(reviewed_rule_rows=1),
            lambda d: d['counts'].update(rows_with_exact_executable_evidence=1),
            lambda d: d['selected_pages'][0].update(sha256='0' * 64),
            lambda d: d['selected_pages'].clear(),
            lambda d: d['bindings'].update(reference_commit='0' * 40),
            lambda d: d['unselected_rules'].pop(),
            lambda d: d['unselected_rules'].append(copy.deepcopy(d['unselected_rules'][0])),
            lambda d: d['unselected_rules'][0].update(page=d['rows'][0]['page'], rule=d['rows'][0]['rule']),
        )
        for number, mutate in enumerate(mutations):
            changed = copy.deepcopy(original)
            mutate(changed)
            with self.subTest(mutation=number), self.assertRaises(ValueError):
                self.verify(changed)

    def test_stale_missing_directory_symlink_preserve_exact_api_errors(self):
        data = self.artifact()
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            target = root / self.PAGE
            target.parent.mkdir(parents=True)
            target.write_bytes((self.source / self.PAGE).read_bytes())
            for state, error in (('stale', ValueError), ('missing', FileNotFoundError),
                                 ('directory', IsADirectoryError), ('symlink', ValueError)):
                if state == 'stale':
                    target.write_bytes(target.read_bytes() + b'\n')
                elif state == 'missing':
                    target.unlink()
                elif state == 'directory':
                    target.mkdir()
                else:
                    target.rmdir()
                    target.symlink_to(self.source / self.PAGE)
                for action in (lambda: self.artifact(root), lambda: self.verify(data, root)):
                    with self.subTest(state=state, action=action), self.assertRaises(error):
                        action()

    def test_source_only_witness_restrictions_fail_before_io(self):
        args = ['--profile', 'grouped', '--rule-index', '/absent/index',
                '--inventory', '/absent/inventory', '--source-root', '/absent/source',
                '--artifact', '/absent/artifact']
        extras = ([], ['--repo-root', '/absent/repo'], ['--evidence', '/absent/evidence'],
                  ['--repo-root', '/absent/repo', '--evidence', '/absent/evidence'])
        actions = ('generate-witness-links', 'build-witness-links',
                   'validate-witness-links', 'assess-witness-links')
        cases = [(action, extra) for action in actions for extra in extras]
        cases += [(action, extra) for action in ('generate', 'validate') for extra in extras[1:]]
        with contextlib.ExitStack() as stack:
            for name in ('read_bytes', 'read_text', 'open', 'is_symlink', 'is_file',
                         'write_bytes', 'write_text', 'mkdir'):
                stack.enter_context(mock.patch.object(Path, name, side_effect=AssertionError('IO')))
            stack.enter_context(mock.patch.object(subprocess, 'run', side_effect=AssertionError('execution')))
            for action, extra in cases:
                with self.subTest(action=action, extra=extra), self.assertRaises(ValueError):
                    main([action, *args, *extra])
            calls = (
                lambda: reconciliation.build_witness_links(
                    self.index, self.inventory, self.source, self.source, 'grouped'),
                lambda: reconciliation.validate_witness_links(
                    {}, self.index, self.inventory, self.source, self.source, 'grouped'),
                lambda: reconciliation.assess_witness_links(
                    {}, self.index, self.inventory, self.source, self.source, (), 'grouped'),
            )
            for call in calls:
                with self.subTest(api=call), self.assertRaises(ValueError):
                    call()

    def test_cli_exclusive_output_and_validate(self):
        with tempfile.TemporaryDirectory() as temp:
            artifact = Path(temp) / 'grouped.json'
            args = ['--profile', 'grouped', '--rule-index', str(self.index),
                    '--inventory', str(self.inventory), '--source-root', str(self.source),
                    '--artifact', str(artifact)]
            with contextlib.redirect_stdout(io.StringIO()) as output:
                main(['generate', *args])
            canonical = (json.dumps(self.artifact(), indent=2) + '\n').encode()
            self.assertEqual(artifact.read_bytes(), canonical)
            self.assertEqual(json.loads(output.getvalue()), self.verify(self.artifact()))
            with contextlib.redirect_stdout(io.StringIO()), self.assertRaises(FileExistsError):
                main(['generate', *args])
            self.assertEqual(artifact.read_bytes(), canonical)
            with contextlib.redirect_stdout(io.StringIO()) as output:
                main(['validate', *args])
            self.assertEqual(json.loads(output.getvalue()), self.verify(self.artifact()))
            self.assertEqual(artifact.read_bytes(), canonical)
            forged = json.loads(canonical)
            forged['counts']['reviewed_rule_rows'] = 1
            artifact.write_text(json.dumps(forged))
            before = artifact.read_bytes()
            with contextlib.redirect_stdout(io.StringIO()), self.assertRaises(ValueError):
                main(['validate', *args])
            self.assertEqual(artifact.read_bytes(), before)

    def test_legacy_literals_canonical_bytes_unchanged_without_grouped_module(self):
        canonical = lambda data: (json.dumps(data, indent=2) + '\n').encode()
        expected = canonical(build(self.index, self.inventory, self.source, 'literals'))
        with mock.patch.dict(sys.modules, {'grouped_reconciliation_proposals': None}):
            data = build(self.index, self.inventory, self.source, 'literals')
            validate(data, self.index, self.inventory, self.source, 'literals')
        self.assertEqual(canonical(data), expected)
        self.assertEqual(digest(canonical(data)),
                         '159e5a9f425ac09e2b1b4ae96a8c8710264f5ebc3170ec97dfc48e1d947808d8')


class PinnedTransfersProfileTests(unittest.TestCase):
    """Independent pinned two-page source oracle, not execution evidence."""

    PAGES = (
        ('src/expressions/return-expr.md',
         'b0a5df6b4b546d676e9bc9f0500fed243ef0a8c6d609b9e4434abec43e432158', 30,
         ('expr.return', 'expr.return.syntax', 'expr.return.intro',
          'expr.return.behavior', 'expr.return.diverging'), (1, 4, 9, 12, 15)),
        ('src/expressions/await-expr.md',
         '7e3448a341fa95608d09b2be2578d5faa020ff204516f0bd3e88a974bab0e0f9', 68,
         ('expr.await', 'expr.await.syntax', 'expr.await.intro', 'expr.await.construct',
          'expr.await.allowed-positions', 'expr.await.effects', 'expr.await.edition2018',
          'expr.await.task', 'expr.await.desugar'), (1, 4, 9, 12, 15, 18, 28, 32, 37)),
    )

    def setUp(self):
        source = os.environ.get('JUSTRUST_WITNESS_REFERENCE_ROOT')
        if not source:
            self.skipTest('set JUSTRUST_WITNESS_REFERENCE_ROOT for pinned-source structural checks')
        self.source = Path(source)
        repo = Path(os.environ.get('JUSTRUST_WITNESS_REPO_ROOT', BASE.parent))
        self.index = repo / 'data/compiler-language-reference-rules.json'
        self.inventory = repo / 'data/compiler-language-inventory.json'

    def artifact(self, source=None):
        return build(self.index, self.inventory,
                     self.source if source is None else source, 'transfers')

    def verify(self, data, source=None):
        return validate(data, self.index, self.inventory,
                        self.source if source is None else source, 'transfers')

    def test_pinned_order_hash_lineblocks_reversible_ids_and_partition(self):
        import hashlib
        import re
        data = self.artifact()
        counts = self.verify(data)
        self.assertEqual([counts[k] for k in ('selected_rule_rows',
                         'unselected_rule_identities', 'global_rule_identities')], [14, 2951, 2965])
        self.assertEqual([(p['path'], p['sha256']) for p in data['selected_pages']],
                         [(p, h) for p, h, _, _, _ in self.PAGES])
        expected_labels = tuple(label for _, _, _, labels, _ in sorted(self.PAGES) for label in labels)
        self.assertEqual(tuple(row['rule'] for row in data['rows']), expected_labels)
        offset = 0
        sha = lambda value: hashlib.sha256(value).hexdigest()
        for page, expected_hash, length, labels, starts in sorted(self.PAGES):
            raw = (self.source / page).read_bytes()
            self.assertEqual(sha(raw), expected_hash)
            lines = raw.splitlines(keepends=True)
            self.assertEqual(len(lines), length)
            markers = [(number, match.group(1).decode())
                       for number, line in enumerate(lines, 1)
                       if (match := re.fullmatch(rb'r\[([^\]\r\n]+)\]\r?\n?', line))]
            self.assertEqual(markers, list(zip(starts, labels)))
            for position, (start, label) in enumerate(markers):
                row = data['rows'][offset + position]
                end = starts[position + 1] - 1 if position + 1 < len(starts) else length
                with self.subTest(page=page, label=label):
                    self.assertEqual((row['page'], row['line'], row['end_line']), (page, start, end))
                    self.assertEqual(row['source_block_sha256'], sha(b''.join(lines[start - 1:end])))
                    prefix = 'reconcile-transfers-'
                    self.assertEqual(row['id'], prefix + label.encode().hex())
                    self.assertEqual(bytes.fromhex(row['id'][len(prefix):]).decode(), label)
                    self.assertEqual(row['review_status'], 'unreviewed')
                    self.assertTrue(row['missing_test_needs'])
                    self.assertTrue(all(need.startswith(('positive: Missing ', 'invalid: Missing ',
                                                        'interaction: Missing ', 'end-to-end: Missing '))
                                        for need in row['missing_test_needs']))
            offset += len(labels)
        self.assertEqual(len({r['id'] for r in data['rows']}), 14)
        for key in ('reviewed_rule_rows', 'rows_with_exact_executable_evidence'):
            self.assertEqual(counts[key], 0)
        index = json.loads(self.index.read_bytes())
        universe = {(p['path'], r['label']) for p in index['pages'] + index['unlinked_markdown_files']
                    for r in p['rules']}
        selected = {(r['page'], r['rule']) for r in data['rows']}
        other = {(r['page'], r['rule']) for r in data['unselected_rules']}
        self.assertEqual((len(selected), len(other), len(universe)), (14, 2951, 2965))
        self.assertEqual(len(data['unselected_rules']), 2951)
        self.assertFalse(selected & other)
        self.assertEqual(selected | other, universe)
        self.assertEqual(data, self.artifact())

    def test_missing_duplicate_foreign_and_malformed_authored_rows_rejected(self):
        import transfers_reconciliation_proposals as proposals
        self.assertEqual(proposals.SELECTED, tuple(p[0] for p in self.PAGES))
        valid = self.artifact()
        rows = proposals.ROWS.strip().splitlines()
        self.assertEqual(tuple(row.split('|')[0] for row in rows),
                         tuple(label for p in self.PAGES for label in p[3]))
        foreign = rows[0].split('|')
        foreign[0] = 'expr.loop'
        cases = [('missing-return', rows[5:]), ('missing-await', rows[:5]),
                 ('missing-row', rows[:-1]), ('empty', []), ('duplicate', rows + [rows[0]]),
                 ('foreign', ['|'.join(foreign), *rows[1:]]),
                 ('short-row', ['|'.join(rows[0].split('|')[:-1]), *rows[1:]]),
                 ('long-row', [rows[0] + '|extra', *rows[1:]])]
        for column in (4, 5, 6):
            changed = rows[0].split('|')
            changed[column] = '   '
            cases.append(('blank-need-' + str(column), ['|'.join(changed), *rows[1:]]))
        for name, changed in cases:
            with self.subTest(mutation=name), mock.patch.object(proposals, 'ROWS', '\n'.join(changed)):
                with self.assertRaises(ValueError):
                    self.artifact()
                with self.assertRaises(ValueError):
                    self.verify(valid)

    def test_forged_artifacts_cannot_claim_credit_or_rebind_source(self):
        original = self.artifact()
        mutations = (
            lambda d: d['rows'].pop(),
            lambda d: d['rows'].append(copy.deepcopy(d['rows'][0])),
            lambda d: d['rows'].reverse(),
            lambda d: d['rows'][0].update(page=self.PAGES[0][0]),
            lambda d: d['rows'][0].update(rule='expr.loop'),
            lambda d: d['rows'][0].update(line=True),
            lambda d: d['rows'][0].update(end_line=99999),
            lambda d: d['rows'][0].update(source_block_sha256='0' * 64),
            lambda d: d['rows'][9].update(source_block_sha256='0' * 64),
            lambda d: d['rows'][0].update(id='reconcile-transfers-expr-return'),
            lambda d: d['rows'][0].update(id=d['rows'][1]['id']),
            lambda d: d['rows'][0].update(review_status='reviewed'),
            lambda d: d['rows'][0].update(rationale='invented interpretation'),
            lambda d: d['rows'][0].update(candidate_obligation_ids=['foreign']),
            lambda d: d['rows'][0].update(missing_test_needs=[]),
            lambda d: d['rows'][0]['missing_test_needs'].__setitem__(0, 'positive: implemented'),
            lambda d: d['counts'].update(reviewed_rule_rows=1),
            lambda d: d['counts'].update(rows_with_exact_executable_evidence=1),
            lambda d: d['selected_pages'][0].update(sha256='0' * 64),
            lambda d: d['selected_pages'][1].update(sha256='0' * 64),
            lambda d: d['selected_pages'].reverse(),
            lambda d: d['selected_pages'].clear(),
            lambda d: d['bindings'].update(reference_commit='0' * 40),
            lambda d: d['unselected_rules'].pop(),
            lambda d: d['unselected_rules'].append(copy.deepcopy(d['unselected_rules'][0])),
            lambda d: d['unselected_rules'][0].update(page=d['rows'][0]['page'], rule=d['rows'][0]['rule']),
        )
        for number, mutate in enumerate(mutations):
            changed = copy.deepcopy(original)
            mutate(changed)
            with self.subTest(mutation=number), self.assertRaises(ValueError):
                self.verify(changed)

    def test_either_page_source_fault_preserves_exact_api_error_type(self):
        data = self.artifact()
        for page, _, _, _, _ in self.PAGES:
            for state, error in (('stale', ValueError), ('missing', FileNotFoundError),
                                 ('directory', IsADirectoryError), ('symlink', ValueError)):
                with self.subTest(page=page, state=state), tempfile.TemporaryDirectory() as temp:
                    root = Path(temp)
                    for selected, _, _, _, _ in self.PAGES:
                        target = root / selected
                        target.parent.mkdir(parents=True, exist_ok=True)
                        target.write_bytes((self.source / selected).read_bytes())
                    target = root / page
                    if state == 'stale':
                        target.write_bytes(target.read_bytes() + b'\n')
                    else:
                        target.unlink()
                        if state == 'directory':
                            target.mkdir()
                        elif state == 'symlink':
                            target.symlink_to(self.source / page)
                    for action in (lambda: self.artifact(root), lambda: self.verify(data, root)):
                        with self.assertRaises(error) as caught:
                            action()
                        self.assertIs(type(caught.exception), error)

    def test_source_only_witness_restrictions_fail_before_io(self):
        args = ['--profile', 'transfers', '--rule-index', '/absent/index',
                '--inventory', '/absent/inventory', '--source-root', '/absent/source',
                '--artifact', '/absent/artifact']
        extras = ([], ['--repo-root', '/absent/repo'], ['--evidence', '/absent/evidence'],
                  ['--repo-root', '/absent/repo', '--evidence', '/absent/evidence'])
        cases = [(action, extra) for action in ('generate-witness-links', 'build-witness-links',
                  'validate-witness-links', 'assess-witness-links') for extra in extras]
        cases += [(action, extra) for action in ('generate', 'validate') for extra in extras[1:]]
        with contextlib.ExitStack() as stack:
            for name in ('read_bytes', 'read_text', 'open', 'is_symlink', 'is_file',
                         'write_bytes', 'write_text', 'mkdir'):
                stack.enter_context(mock.patch.object(Path, name, side_effect=AssertionError('IO')))
            stack.enter_context(mock.patch.object(subprocess, 'run', side_effect=AssertionError('execution')))
            for action, extra in cases:
                with self.subTest(action=action, extra=extra), self.assertRaises(ValueError):
                    main([action, *args, *extra])
            calls = (
                lambda: reconciliation.build_witness_links(
                    self.index, self.inventory, self.source, self.source, 'transfers'),
                lambda: reconciliation.validate_witness_links(
                    {}, self.index, self.inventory, self.source, self.source, 'transfers'),
                lambda: reconciliation.assess_witness_links(
                    {}, self.index, self.inventory, self.source, self.source, (), 'transfers'),
            )
            for call in calls:
                with self.subTest(api=call), self.assertRaises(ValueError):
                    call()

    def test_cli_exclusive_output_and_validate(self):
        with tempfile.TemporaryDirectory() as temp:
            artifact = Path(temp) / 'transfers.json'
            args = ['--profile', 'transfers', '--rule-index', str(self.index),
                    '--inventory', str(self.inventory), '--source-root', str(self.source),
                    '--artifact', str(artifact)]
            with contextlib.redirect_stdout(io.StringIO()) as output:
                main(['generate', *args])
            canonical = (json.dumps(self.artifact(), indent=2) + '\n').encode()
            self.assertEqual(artifact.read_bytes(), canonical)
            self.assertEqual(json.loads(output.getvalue()), self.verify(self.artifact()))
            with contextlib.redirect_stdout(io.StringIO()), self.assertRaises(FileExistsError):
                main(['generate', *args])
            self.assertEqual(artifact.read_bytes(), canonical)
            with contextlib.redirect_stdout(io.StringIO()) as output:
                main(['validate', *args])
            self.assertEqual(json.loads(output.getvalue()), self.verify(self.artifact()))
            self.assertEqual(artifact.read_bytes(), canonical)
            forged = json.loads(canonical)
            forged['counts']['reviewed_rule_rows'] = 1
            artifact.write_text(json.dumps(forged))
            before = artifact.read_bytes()
            with contextlib.redirect_stdout(io.StringIO()), self.assertRaises(ValueError):
                main(['validate', *args])
            self.assertEqual(artifact.read_bytes(), before)

    def test_legacy_grouped_canonical_bytes_unchanged_without_transfers_module(self):
        canonical = lambda data: (json.dumps(data, indent=2) + '\n').encode()
        expected = canonical(build(self.index, self.inventory, self.source, 'grouped'))
        with mock.patch.dict(sys.modules, {'transfers_reconciliation_proposals': None}):
            data = build(self.index, self.inventory, self.source, 'grouped')
            validate(data, self.index, self.inventory, self.source, 'grouped')
        self.assertEqual(canonical(data), expected)
        self.assertEqual(digest(canonical(data)),
                         '5a9393145150885b8c9f7717e99000003751454bd0a322ed7152e24e4289e279')


class PinnedRangesProfileTests(unittest.TestCase):
    """Independent pinned two-page source oracle, not execution evidence."""

    PAGES = (
        ('src/expressions/range-expr.md',
         '81d5e0721c0fbbd11e29859902b9ab7c6a4d630531f46e07b9bf474d0e86b819', 67,
         ('expr.range', 'expr.range.syntax', 'expr.range.behavior',
          'expr.range.equivalence', 'expr.range.for'), (1, 4, 27, 50, 60)),
        ('src/expressions/underscore-expr.md',
         '2114d3e87cb1573b416adc7fb1525a6fa51ae074c9352a7e57e21b6eb03ddcd2', 39,
         ('expr.placeholder', 'expr.placeholder.syntax', 'expr.placeholder.intro',
          'expr.placeholder.lhs-assignment-only', 'expr.placeholder.pattern'),
         (1, 4, 9, 12, 15)),
    )

    BLOCK_HASHES = (
        '6c1ff1b1dca4631d6cda5ffd29352c31e6b7fc857754a50006ac5cf918526cc8',
        'f39090e248305929807a57c4a74c8d5d08eccca261b32a4f6113478d3d9eeb85',
        '821773091a6f3aa2ace2dccb9aa6def7505bb4e676708d7a91750c0a791da472',
        '8539af32ece03c2d2363f08daa2f16db6a4ea1390985f2c8b43ce8e5d555b15b',
        '63ac9915a89aac16243419491f45cd6f90a3d04ac157750bda519482c30007ba',
        '85dd56065927fdf9698041daa7fb7537644f830ba8b060b05af6d7fe1a9c5db2',
        'fa444560b5aef14c9e1b3a346d80cf65ec74a4d7b15828ff41ceca92c092c689',
        '2def3087b20db161caab308d64ccbb0036b5a20deca0a4f7527475b8cb198365',
        '1da93861c235473e55a382b8eb6a24304b4826b1e9da4582b973b718178eb57b',
        '0d4f0ef18cde303d99a433d286e319b9cec9a97f52576caba514c3d74db2e889',
    )

    def setUp(self):
        source = os.environ.get('JUSTRUST_WITNESS_REFERENCE_ROOT')
        if not source:
            self.skipTest('set JUSTRUST_WITNESS_REFERENCE_ROOT for pinned-source structural checks')
        self.source = Path(source)
        repo = Path(os.environ.get('JUSTRUST_WITNESS_REPO_ROOT', BASE.parent))
        self.index = repo / 'data/compiler-language-reference-rules.json'
        self.inventory = repo / 'data/compiler-language-inventory.json'

    def artifact(self, source=None):
        return build(self.index, self.inventory,
                     self.source if source is None else source, 'ranges')

    def verify(self, data, source=None):
        return validate(data, self.index, self.inventory,
                        self.source if source is None else source, 'ranges')

    def test_pinned_order_hash_lineblocks_reversible_ids_and_partition(self):
        import hashlib
        import re
        data = self.artifact()
        counts = self.verify(data)
        self.assertEqual([counts[k] for k in ('selected_rule_rows',
                         'unselected_rule_identities', 'global_rule_identities')], [10, 2955, 2965])
        self.assertEqual([(p['path'], p['sha256']) for p in data['selected_pages']],
                         [(p, h) for p, h, _, _, _ in self.PAGES])
        expected_labels = tuple(label for _, _, _, labels, _ in sorted(self.PAGES) for label in labels)
        self.assertEqual(tuple(row['rule'] for row in data['rows']), expected_labels)
        offset = 0
        sha = lambda value: hashlib.sha256(value).hexdigest()
        for page, expected_hash, length, labels, starts in sorted(self.PAGES):
            raw = (self.source / page).read_bytes()
            self.assertEqual(sha(raw), expected_hash)
            lines = raw.splitlines(keepends=True)
            self.assertEqual(len(lines), length)
            markers = [(number, match.group(1).decode())
                       for number, line in enumerate(lines, 1)
                       if (match := re.fullmatch(rb'r\[([^\]\r\n]+)\]\r?\n?', line))]
            self.assertEqual(markers, list(zip(starts, labels)))
            for position, (start, label) in enumerate(markers):
                row = data['rows'][offset + position]
                end = starts[position + 1] - 1 if position + 1 < len(starts) else length
                with self.subTest(page=page, label=label):
                    self.assertEqual((row['page'], row['line'], row['end_line']), (page, start, end))
                    self.assertEqual(sha(b''.join(lines[start - 1:end])),
                                     self.BLOCK_HASHES[offset + position])
                    self.assertEqual(row['source_block_sha256'], self.BLOCK_HASHES[offset + position])
                    prefix = 'reconcile-ranges-'
                    self.assertEqual(row['id'], prefix + label.encode().hex())
                    self.assertEqual(bytes.fromhex(row['id'][len(prefix):]).decode(), label)
                    self.assertEqual(row['review_status'], 'unreviewed')
                    self.assertTrue(row['missing_test_needs'])
                    self.assertTrue(all(need.startswith(('positive: Missing ', 'invalid: Missing ',
                                                        'interaction: Missing ', 'end-to-end: Missing '))
                                        for need in row['missing_test_needs']))
            offset += len(labels)
        self.assertEqual(len({r['id'] for r in data['rows']}), 10)
        for key in ('reviewed_rule_rows', 'rows_with_exact_executable_evidence'):
            self.assertEqual(counts[key], 0)
        index = json.loads(self.index.read_bytes())
        universe = {(p['path'], r['label']) for p in index['pages'] + index['unlinked_markdown_files']
                    for r in p['rules']}
        selected = {(r['page'], r['rule']) for r in data['rows']}
        other = {(r['page'], r['rule']) for r in data['unselected_rules']}
        self.assertEqual((len(selected), len(other), len(universe)), (10, 2955, 2965))
        self.assertEqual(len(data['unselected_rules']), 2955)
        self.assertFalse(selected & other)
        self.assertEqual(selected | other, universe)
        self.assertEqual(data, self.artifact())

    def test_rows_sort_by_page_line_independently_of_authored_order(self):
        import ranges_reconciliation_proposals as proposals
        original = self.artifact()
        rows = proposals.ROWS.strip().splitlines()
        expected_order = [(p, start) for p, _, _, _, starts in sorted(self.PAGES)
                          for start in starts]
        for reordered in (rows[::-1], rows[5:] + rows[:5], rows[::2] + rows[1::2]):
            with self.subTest(order=[row.split('|')[0] for row in reordered]):
                with mock.patch.object(proposals, 'ROWS', '\n'.join(reordered)):
                    data = self.artifact()
                    self.assertEqual([(r['page'], r['line']) for r in data['rows']], expected_order)
                    self.assertEqual([p['path'] for p in data['selected_pages']],
                                     list(proposals.SELECTED))
                    self.assertEqual(data, original)
                    self.assertEqual(self.verify(data), self.verify(original))

    def test_missing_duplicate_foreign_and_malformed_authored_rows_rejected(self):
        import ranges_reconciliation_proposals as proposals
        self.assertEqual(proposals.SELECTED, tuple(p[0] for p in self.PAGES))
        valid = self.artifact()
        rows = proposals.ROWS.strip().splitlines()
        self.assertEqual(tuple(row.split('|')[0] for row in rows),
                         tuple(label for p in self.PAGES for label in p[3]))
        foreign = rows[0].split('|')
        foreign[0] = 'expr.loop'
        cases = [('missing-range', rows[5:]), ('missing-underscore', rows[:5]),
                 ('missing-row', rows[:-1]), ('empty', []), ('duplicate', rows + [rows[0]]),
                 ('foreign', ['|'.join(foreign), *rows[1:]]),
                 ('short-row', ['|'.join(rows[0].split('|')[:-1]), *rows[1:]]),
                 ('long-row', [rows[0] + '|extra', *rows[1:]])]
        for column in (4, 5, 6):
            changed = rows[0].split('|')
            changed[column] = '   '
            cases.append(('blank-need-' + str(column), ['|'.join(changed), *rows[1:]]))
        for name, changed in cases:
            with self.subTest(mutation=name), mock.patch.object(proposals, 'ROWS', '\n'.join(changed)):
                with self.assertRaises(ValueError):
                    self.artifact()
                with self.assertRaises(ValueError):
                    self.verify(valid)

    def test_forged_artifacts_cannot_claim_credit_or_rebind_source(self):
        original = self.artifact()
        mutations = (
            lambda d: d['rows'].pop(),
            lambda d: d['rows'].append(copy.deepcopy(d['rows'][0])),
            lambda d: d['rows'].reverse(),
            lambda d: d['rows'][0].update(page=self.PAGES[1][0]),
            lambda d: d['rows'][5].update(page=self.PAGES[0][0]),
            lambda d: d['rows'][0].update(rule='expr.loop'),
            lambda d: d['rows'][0].update(line=True),
            lambda d: d['rows'][0].update(end_line=99999),
            lambda d: d['rows'][0].update(source_block_sha256='0' * 64),
            lambda d: d['rows'][9].update(source_block_sha256='0' * 64),
            lambda d: d['rows'][0].update(id='reconcile-ranges-expr-range'),
            lambda d: d['rows'][0].update(id=d['rows'][1]['id']),
            lambda d: d['rows'][0].update(review_status='reviewed'),
            lambda d: d['rows'][0].update(rationale='invented interpretation'),
            lambda d: d['rows'][0].update(candidate_obligation_ids=['foreign']),
            lambda d: d['rows'][0].update(missing_test_needs=[]),
            lambda d: d['rows'][0]['missing_test_needs'].__setitem__(0, 'positive: implemented'),
            lambda d: d['counts'].update(reviewed_rule_rows=1),
            lambda d: d['counts'].update(rows_with_exact_executable_evidence=1),
            lambda d: d['selected_pages'][0].update(sha256='0' * 64),
            lambda d: d['selected_pages'][1].update(sha256='0' * 64),
            lambda d: d['selected_pages'].reverse(),
            lambda d: d['selected_pages'].clear(),
            lambda d: d['bindings'].update(reference_commit='0' * 40),
            lambda d: d['unselected_rules'].pop(),
            lambda d: d['unselected_rules'].append(copy.deepcopy(d['unselected_rules'][0])),
            lambda d: d['unselected_rules'][0].update(page=d['rows'][0]['page'], rule=d['rows'][0]['rule']),
        )
        for number, mutate in enumerate(mutations):
            changed = copy.deepcopy(original)
            mutate(changed)
            self.assertNotEqual(json.dumps(changed, sort_keys=True),
                                json.dumps(original, sort_keys=True),
                                'mutation must change JSON, including bool versus int')
            with self.subTest(mutation=number), self.assertRaises(ValueError):
                self.verify(changed)

    def test_either_page_source_fault_preserves_exact_api_error_type(self):
        data = self.artifact()
        for page, _, _, _, _ in self.PAGES:
            for state, error in (('stale', ValueError), ('missing', FileNotFoundError),
                                 ('directory', IsADirectoryError), ('symlink', ValueError)):
                with self.subTest(page=page, state=state), tempfile.TemporaryDirectory() as temp:
                    root = Path(temp)
                    for selected, _, _, _, _ in self.PAGES:
                        target = root / selected
                        target.parent.mkdir(parents=True, exist_ok=True)
                        target.write_bytes((self.source / selected).read_bytes())
                    target = root / page
                    if state == 'stale':
                        target.write_bytes(target.read_bytes() + b'\n')
                    else:
                        target.unlink()
                        if state == 'directory':
                            target.mkdir()
                        elif state == 'symlink':
                            target.symlink_to(self.source / page)
                    for action in (lambda: self.artifact(root), lambda: self.verify(data, root)):
                        with self.assertRaises(error) as caught:
                            action()
                        self.assertIs(type(caught.exception), error)

    def test_source_only_witness_restrictions_fail_before_io(self):
        args = ['--profile', 'ranges', '--rule-index', '/absent/index',
                '--inventory', '/absent/inventory', '--source-root', '/absent/source',
                '--artifact', '/absent/artifact']
        extras = ([], ['--repo-root', '/absent/repo'], ['--evidence', '/absent/evidence'],
                  ['--repo-root', '/absent/repo', '--evidence', '/absent/evidence'])
        cases = [(action, extra) for action in ('generate-witness-links', 'build-witness-links',
                  'validate-witness-links', 'assess-witness-links') for extra in extras]
        cases += [(action, extra) for action in ('generate', 'validate') for extra in extras[1:]]
        with contextlib.ExitStack() as stack:
            for name in ('read_bytes', 'read_text', 'open', 'is_symlink', 'is_file',
                         'write_bytes', 'write_text', 'mkdir'):
                stack.enter_context(mock.patch.object(Path, name, side_effect=AssertionError('IO')))
            stack.enter_context(mock.patch.object(subprocess, 'run', side_effect=AssertionError('execution')))
            for action, extra in cases:
                with self.subTest(action=action, extra=extra), self.assertRaises(ValueError):
                    main([action, *args, *extra])
            calls = (
                lambda: reconciliation.build_witness_links(
                    self.index, self.inventory, self.source, self.source, 'ranges'),
                lambda: reconciliation.validate_witness_links(
                    {}, self.index, self.inventory, self.source, self.source, 'ranges'),
                lambda: reconciliation.assess_witness_links(
                    {}, self.index, self.inventory, self.source, self.source, (), 'ranges'),
            )
            for call in calls:
                with self.subTest(api=call), self.assertRaises(ValueError):
                    call()

    def test_cli_exclusive_output_and_validate(self):
        with tempfile.TemporaryDirectory() as temp:
            artifact = Path(temp) / 'ranges.json'
            args = ['--profile', 'ranges', '--rule-index', str(self.index),
                    '--inventory', str(self.inventory), '--source-root', str(self.source),
                    '--artifact', str(artifact)]
            with contextlib.redirect_stdout(io.StringIO()) as output:
                main(['generate', *args])
            canonical = (json.dumps(self.artifact(), indent=2) + '\n').encode()
            self.assertEqual(artifact.read_bytes(), canonical)
            self.assertEqual(json.loads(output.getvalue()), self.verify(self.artifact()))
            with contextlib.redirect_stdout(io.StringIO()), self.assertRaises(FileExistsError):
                main(['generate', *args])
            self.assertEqual(artifact.read_bytes(), canonical)
            with contextlib.redirect_stdout(io.StringIO()) as output:
                main(['validate', *args])
            self.assertEqual(json.loads(output.getvalue()), self.verify(self.artifact()))
            self.assertEqual(artifact.read_bytes(), canonical)
            forged = json.loads(canonical)
            forged['counts']['reviewed_rule_rows'] = 1
            artifact.write_text(json.dumps(forged))
            before = artifact.read_bytes()
            with contextlib.redirect_stdout(io.StringIO()), self.assertRaises(ValueError):
                main(['validate', *args])
            self.assertEqual(artifact.read_bytes(), before)

    def test_legacy_transfers_canonical_bytes_unchanged_without_ranges_module(self):
        canonical = lambda data: (json.dumps(data, indent=2) + '\n').encode()
        expected = canonical(build(self.index, self.inventory, self.source, 'transfers'))
        with mock.patch.dict(sys.modules, {'ranges_reconciliation_proposals': None}):
            data = build(self.index, self.inventory, self.source, 'transfers')
            validate(data, self.index, self.inventory, self.source, 'transfers')
        self.assertEqual(canonical(data), expected)
        self.assertEqual(digest(canonical(data)),
                         '5f29782852c84ccf19cd0fde0ef1c4d9f54a6813eda59d1d9f1686bd387b123d')


class PinnedStructsProfileTests(unittest.TestCase):
    """Independent pinned single-page source oracle, not execution evidence."""

    PAGES = (('src/expressions/struct-expr.md',
        '0f3c3571e52b1c9945d4c6fedb2cca866156d8de0a49399b3915c8b5d1667903', 151,
        ('expr.struct', 'expr.struct.syntax', 'expr.struct.intro', 'expr.struct.field', 'expr.struct.field.intro', 'expr.struct.field.union-constraint', 'expr.struct.update', 'expr.struct.update.intro', 'expr.struct.update.base-same-type', 'expr.struct.update.fields', 'expr.struct.update.visibility-constraint', 'expr.struct.brace-restricted-positions', 'expr.struct.tuple-field', 'expr.struct.field.named'),
        (1, 4, 22, 83, 86, 89, 92, 95, 98, 101, 104, 115, 118, 128)),)

    BLOCK_HASHES = (
        'ef5e4798245146a8387a2ef3d815d596932a15681fb13dae263f61d6f06ef27c',
        'fc30c8c808a04803900e3779672421449452d0ebbf8ede412f2fd4f2e7ea8723',
        '2cfaa5ea3edfba79b1697b4ba44f602e6e117cf126e6fdaca10ac8ee54acd090',
        '25e62199996b2f70c25c97b13d80ac1d3d11e5edfea1ff32280c224c81c652fd',
        '2674170b1e506d49d68fbfdda6bb02857d27f53c3d6fa440ff2f9a1f58236359',
        'ee17639ae2b51326f6f6d18b5270e674b8099fc07b433c1b2ee974f6cc9b5062',
        '67de3cfa1588de153fe0841d857b1d93840885019e6d9bf82160c8b2760a1370',
        '711a584db46146981bb4d2594669cd63798fd19c6935bc38af1a81b346010e3b',
        '3a647a38ba82cf078ce9e35f4560bfec29ac40d9fc63519d263ff44632a54378',
        '79625a0aeedb379eb640a807c0c3823f33911639bc083368cc3ab9cfd67fc4a3',
        '9cf715fec5d20a48a417b8c2b4a0593ee4931e408ef109c63d139d09905cb9a1',
        '34586d37571c226b65dab99d5f3fc96858109dac596ec5ff4f26adce339c3b20',
        '564a9e2dde615ac777a848c9769acfd3a8c0e008832ddb8d4f83a7359204c028',
        '638409e2101f1f79f6cf7617794c6847ad4d19f803cd53012112ddd7724414e4',
    )

    def setUp(self):
        source = os.environ.get('JUSTRUST_WITNESS_REFERENCE_ROOT')
        if not source:
            self.skipTest('set JUSTRUST_WITNESS_REFERENCE_ROOT for pinned-source structural checks')
        self.source = Path(source)
        repo = Path(os.environ.get('JUSTRUST_WITNESS_REPO_ROOT', BASE.parent))
        self.index = repo / 'data/compiler-language-reference-rules.json'
        self.inventory = repo / 'data/compiler-language-inventory.json'

    def artifact(self, source=None):
        return build(self.index, self.inventory,
                     self.source if source is None else source, 'structs')

    def test_module_pins_actual_aliases_and_no_prior_identity_overlap(self):
        import structs_reconciliation_proposals as proposals
        self.assertEqual(proposals.SOURCE_COMMIT,
                         '86635e30bf861a038dc197d7e16fd09e7e514e7a')
        self.assertEqual(proposals.PAGE_SHA256, {p: h for p, h, _, _, _ in self.PAGES})
        self.assertEqual(proposals.ALIASES, {
            'struct': 'reference-expressions-struct-expr-construction-update',
            'union': 'reference-items-unions-union-access',
            'privacy': 'reference-visibility-and-privacy-privacy-reachability',
        })
        obligations = {o['id'] for section in json.loads(self.inventory.read_bytes())['sections']
                       for o in section['obligations']}
        self.assertTrue(set(proposals.ALIASES.values()) <= obligations)
        index = json.loads(self.index.read_bytes())
        prior_pages = {path for name, _ in reconciliation.PROPOSAL_COUNTS if name not in ('trait-objects', 'type-parameters', 'nominal-types', 'inferred-types', 'structs', 'blocks', 'expression-core', 'path-expressions', 'never-type', 'coercions', 'subtyping', 'type-expressions', 'primitive-types', 'sequence-types', 'pointer-types', 'function-pointers', 'function-items')
                       for path in reconciliation.profile_inputs(name)[0]}
        prior = {(page['path'], rule['label'])
                 for page in index['pages'] + index['unlinked_markdown_files']
                 if page['path'] in prior_pages for rule in page['rules']}
        current = {(row['page'], row['rule']) for row in self.artifact()['rows']}
        self.assertFalse(prior & current)
        self.assertEqual((len(prior), len(current), len(prior | current)), (706, 14, 720))

    def verify(self, data, source=None):
        return validate(data, self.index, self.inventory,
                        self.source if source is None else source, 'structs')

    def test_pinned_order_hash_lineblocks_reversible_ids_and_partition(self):
        import hashlib
        import re
        data = self.artifact()
        counts = self.verify(data)
        self.assertEqual([counts[k] for k in ('selected_rule_rows',
                         'unselected_rule_identities', 'global_rule_identities')], [14, 2951, 2965])
        self.assertEqual([(p['path'], p['sha256']) for p in data['selected_pages']],
                         [(p, h) for p, h, _, _, _ in self.PAGES])
        expected_labels = tuple(label for _, _, _, labels, _ in sorted(self.PAGES) for label in labels)
        self.assertEqual(tuple(row['rule'] for row in data['rows']), expected_labels)
        offset = 0
        sha = lambda value: hashlib.sha256(value).hexdigest()
        for page, expected_hash, length, labels, starts in sorted(self.PAGES):
            raw = (self.source / page).read_bytes()
            self.assertEqual(sha(raw), expected_hash)
            lines = raw.splitlines(keepends=True)
            self.assertEqual(len(lines), length)
            markers = [(number, match.group(1).decode())
                       for number, line in enumerate(lines, 1)
                       if (match := re.fullmatch(rb'r\[([^\]\r\n]+)\]\r?\n?', line))]
            self.assertEqual(markers, list(zip(starts, labels)))
            for position, (start, label) in enumerate(markers):
                row = data['rows'][offset + position]
                end = starts[position + 1] - 1 if position + 1 < len(starts) else length
                with self.subTest(page=page, label=label):
                    self.assertEqual((row['page'], row['line'], row['end_line']), (page, start, end))
                    self.assertEqual(sha(b''.join(lines[start - 1:end])),
                                     self.BLOCK_HASHES[offset + position])
                    self.assertEqual(row['source_block_sha256'], self.BLOCK_HASHES[offset + position])
                    prefix = 'reconcile-structs-'
                    self.assertEqual(row['id'], prefix + label.encode().hex())
                    self.assertEqual(bytes.fromhex(row['id'][len(prefix):]).decode(), label)
                    self.assertEqual(row['review_status'], 'unreviewed')
                    self.assertTrue(row['missing_test_needs'])
                    self.assertTrue(all(need.startswith(('positive: Missing ', 'invalid: Missing ',
                                                        'interaction: Missing ', 'end-to-end: Missing '))
                                        for need in row['missing_test_needs']))
            offset += len(labels)
        self.assertEqual(len({r['id'] for r in data['rows']}), 14)
        for key in ('reviewed_rule_rows', 'rows_with_exact_executable_evidence'):
            self.assertEqual(counts[key], 0)
        index = json.loads(self.index.read_bytes())
        universe = {(p['path'], r['label']) for p in index['pages'] + index['unlinked_markdown_files']
                    for r in p['rules']}
        selected = {(r['page'], r['rule']) for r in data['rows']}
        other = {(r['page'], r['rule']) for r in data['unselected_rules']}
        self.assertEqual((len(selected), len(other), len(universe)), (14, 2951, 2965))
        self.assertEqual(len(data['unselected_rules']), 2951)
        self.assertFalse(selected & other)
        self.assertEqual(selected | other, universe)
        self.assertEqual(data, self.artifact())

    def test_rows_sort_by_page_line_independently_of_authored_order(self):
        import structs_reconciliation_proposals as proposals
        original = self.artifact()
        rows = proposals.ROWS.strip().splitlines()
        expected_order = [(p, start) for p, _, _, _, starts in sorted(self.PAGES)
                          for start in starts]
        for reordered in (rows[::-1], rows[5:] + rows[:5], rows[::2] + rows[1::2]):
            with self.subTest(order=[row.split('|')[0] for row in reordered]):
                with mock.patch.object(proposals, 'ROWS', '\n'.join(reordered)):
                    data = self.artifact()
                    self.assertEqual([(r['page'], r['line']) for r in data['rows']], expected_order)
                    self.assertEqual([p['path'] for p in data['selected_pages']],
                                     list(proposals.SELECTED))
                    self.assertEqual(data, original)
                    self.assertEqual(self.verify(data), self.verify(original))

    def test_missing_duplicate_foreign_and_malformed_authored_rows_rejected(self):
        import structs_reconciliation_proposals as proposals
        self.assertEqual(proposals.SELECTED, tuple(p[0] for p in self.PAGES))
        valid = self.artifact()
        rows = proposals.ROWS.strip().splitlines()
        self.assertEqual(tuple(row.split('|')[0] for row in rows),
                         tuple(label for p in self.PAGES for label in p[3]))
        foreign = rows[0].split('|')
        foreign[0] = 'expr.loop'
        cases = [('missing-prefix', rows[5:]), ('missing-suffix', rows[:5]),
                 ('missing-row', rows[:-1]), ('empty', []), ('duplicate', rows + [rows[0]]),
                 ('foreign', ['|'.join(foreign), *rows[1:]]),
                 ('short-row', ['|'.join(rows[0].split('|')[:-1]), *rows[1:]]),
                 ('long-row', [rows[0] + '|extra', *rows[1:]])]
        for column in (4, 5, 6):
            changed = rows[0].split('|')
            changed[column] = '   '
            cases.append(('blank-need-' + str(column), ['|'.join(changed), *rows[1:]]))
        for name, changed in cases:
            with self.subTest(mutation=name), mock.patch.object(proposals, 'ROWS', '\n'.join(changed)):
                with self.assertRaises(ValueError):
                    self.artifact()
                with self.assertRaises(ValueError):
                    self.verify(valid)

    def test_forged_artifacts_cannot_claim_credit_or_rebind_source(self):
        original = self.artifact()
        mutations = (
            lambda d: d['rows'].pop(),
            lambda d: d['rows'].append(copy.deepcopy(d['rows'][0])),
            lambda d: d['rows'].reverse(),
            lambda d: d['rows'][0].update(page='src/expressions/range-expr.md'),
            lambda d: d['rows'][5].update(page='src/expressions/underscore-expr.md'),
            lambda d: d['rows'][0].update(rule='expr.loop'),
            lambda d: d['rows'][0].update(line=True),
            lambda d: d['rows'][0].update(end_line=99999),
            lambda d: d['rows'][0].update(source_block_sha256='0' * 64),
            lambda d: d['rows'][9].update(source_block_sha256='0' * 64),
            lambda d: d['rows'][0].update(id='reconcile-structs-expr-range'),
            lambda d: d['rows'][0].update(id=d['rows'][1]['id']),
            lambda d: d['rows'][0].update(review_status='reviewed'),
            lambda d: d['rows'][0].update(rationale='invented interpretation'),
            lambda d: d['rows'][0].update(candidate_obligation_ids=['foreign']),
            lambda d: d['rows'][0].update(missing_test_needs=[]),
            lambda d: d['rows'][0]['missing_test_needs'].__setitem__(0, 'positive: implemented'),
            lambda d: d['counts'].update(reviewed_rule_rows=1),
            lambda d: d['counts'].update(rows_with_exact_executable_evidence=1),
            lambda d: d['selected_pages'][0].update(sha256='0' * 64),
            lambda d: d['selected_pages'].clear(),
            lambda d: d['bindings'].update(reference_commit='0' * 40),
            lambda d: d['unselected_rules'].pop(),
            lambda d: d['unselected_rules'].append(copy.deepcopy(d['unselected_rules'][0])),
            lambda d: d['unselected_rules'][0].update(page=d['rows'][0]['page'], rule=d['rows'][0]['rule']),
        )
        for number, mutate in enumerate(mutations):
            changed = copy.deepcopy(original)
            mutate(changed)
            self.assertNotEqual(json.dumps(changed, sort_keys=True),
                                json.dumps(original, sort_keys=True),
                                'mutation must change JSON, including bool versus int')
            with self.subTest(mutation=number), self.assertRaises(ValueError):
                self.verify(changed)

    def test_either_page_source_fault_preserves_exact_api_error_type(self):
        data = self.artifact()
        for page, _, _, _, _ in self.PAGES:
            for state, error in (('stale', ValueError), ('missing', FileNotFoundError),
                                 ('directory', IsADirectoryError), ('symlink', ValueError)):
                with self.subTest(page=page, state=state), tempfile.TemporaryDirectory() as temp:
                    root = Path(temp)
                    for selected, _, _, _, _ in self.PAGES:
                        target = root / selected
                        target.parent.mkdir(parents=True, exist_ok=True)
                        target.write_bytes((self.source / selected).read_bytes())
                    target = root / page
                    if state == 'stale':
                        target.write_bytes(target.read_bytes() + b'\n')
                    else:
                        target.unlink()
                        if state == 'directory':
                            target.mkdir()
                        elif state == 'symlink':
                            target.symlink_to(self.source / page)
                    for action in (lambda: self.artifact(root), lambda: self.verify(data, root)):
                        with self.assertRaises(error) as caught:
                            action()
                        self.assertIs(type(caught.exception), error)

    def test_source_only_witness_restrictions_fail_before_io(self):
        args = ['--profile', 'structs', '--rule-index', '/absent/index',
                '--inventory', '/absent/inventory', '--source-root', '/absent/source',
                '--artifact', '/absent/artifact']
        extras = ([], ['--repo-root', '/absent/repo'], ['--evidence', '/absent/evidence'],
                  ['--repo-root', '/absent/repo', '--evidence', '/absent/evidence'])
        cases = [(action, extra) for action in ('generate-witness-links', 'build-witness-links',
                  'validate-witness-links', 'assess-witness-links') for extra in extras]
        cases += [(action, extra) for action in ('generate', 'validate') for extra in extras[1:]]
        with contextlib.ExitStack() as stack:
            for name in ('read_bytes', 'read_text', 'open', 'is_symlink', 'is_file',
                         'write_bytes', 'write_text', 'mkdir'):
                stack.enter_context(mock.patch.object(Path, name, side_effect=AssertionError('IO')))
            stack.enter_context(mock.patch.object(subprocess, 'run', side_effect=AssertionError('execution')))
            for action, extra in cases:
                with self.subTest(action=action, extra=extra), self.assertRaises(ValueError):
                    main([action, *args, *extra])
            calls = (
                lambda: reconciliation.build_witness_links(
                    self.index, self.inventory, self.source, self.source, 'structs'),
                lambda: reconciliation.validate_witness_links(
                    {}, self.index, self.inventory, self.source, self.source, 'structs'),
                lambda: reconciliation.assess_witness_links(
                    {}, self.index, self.inventory, self.source, self.source, (), 'structs'),
            )
            for call in calls:
                with self.subTest(api=call), self.assertRaises(ValueError):
                    call()

    def test_cli_exclusive_output_and_validate(self):
        with tempfile.TemporaryDirectory() as temp:
            artifact = Path(temp) / 'structs.json'
            args = ['--profile', 'structs', '--rule-index', str(self.index),
                    '--inventory', str(self.inventory), '--source-root', str(self.source),
                    '--artifact', str(artifact)]
            with contextlib.redirect_stdout(io.StringIO()) as output:
                main(['generate', *args])
            canonical = (json.dumps(self.artifact(), indent=2) + '\n').encode()
            self.assertEqual(artifact.read_bytes(), canonical)
            self.assertEqual(json.loads(output.getvalue()), self.verify(self.artifact()))
            with contextlib.redirect_stdout(io.StringIO()), self.assertRaises(FileExistsError):
                main(['generate', *args])
            self.assertEqual(artifact.read_bytes(), canonical)
            with contextlib.redirect_stdout(io.StringIO()) as output:
                main(['validate', *args])
            self.assertEqual(json.loads(output.getvalue()), self.verify(self.artifact()))
            self.assertEqual(artifact.read_bytes(), canonical)
            forged = json.loads(canonical)
            forged['counts']['reviewed_rule_rows'] = 1
            artifact.write_text(json.dumps(forged))
            before = artifact.read_bytes()
            with contextlib.redirect_stdout(io.StringIO()), self.assertRaises(ValueError):
                main(['validate', *args])
            self.assertEqual(artifact.read_bytes(), before)

    def test_legacy_ranges_canonical_bytes_unchanged_without_structs_module(self):
        canonical = lambda data: (json.dumps(data, indent=2) + '\n').encode()
        expected = canonical(build(self.index, self.inventory, self.source, 'ranges'))
        with mock.patch.dict(sys.modules, {'structs_reconciliation_proposals': None}):
            data = build(self.index, self.inventory, self.source, 'ranges')
            validate(data, self.index, self.inventory, self.source, 'ranges')
        self.assertEqual(canonical(data), expected)
        self.assertEqual(digest(canonical(data)),
                         '3017078dfc42f85b5e51849d80e1a7efc41a247bbb573f09a7c40b6c5826fdda')


class PinnedBlocksProfileTests(unittest.TestCase):
    """Independent pinned single-page source oracle, not execution evidence."""

    PAGES = (('src/expressions/block-expr.md',
        'dfd73faf7ef7c7c45dc5e0524fee661c641bbb2ea31e531c6a740639e7a6a146', 383,
        ('expr.block', 'expr.block.syntax', 'expr.block.intro', 'expr.block.sequential-evaluation', 'expr.block.namespace', 'expr.block.inner-attributes', 'expr.block.statements', 'expr.block.null-statement', 'expr.block.evaluation', 'expr.block.result', 'expr.block.value-trailing-expr', 'expr.block.value-no-trailing-expr', 'expr.block.value-diverges-no-trailing-expr', 'expr.block.diverging', 'expr.block.value', 'expr.block.async', 'expr.block.async.syntax', 'expr.block.async.intro', 'expr.block.async.future-result', 'expr.block.async.anonymous-type', 'expr.block.async.future', 'expr.block.async.layout-unspecified', 'expr.block.async.edition2018', 'expr.block.async.capture', 'expr.block.async.context', 'expr.block.async.function', 'expr.block.async.function.intro', 'expr.block.async.function.return-try', 'expr.block.async.function.control-flow', 'expr.block.const', 'expr.block.const.syntax', 'expr.block.const.intro', 'expr.block.const.context', 'expr.block.const.generic-params', 'expr.block.const.evaluation', 'expr.block.const.not-executed', 'expr.block.unsafe', 'expr.block.unsafe.syntax', 'expr.block.unsafe.intro', 'expr.block.label', 'expr.block.attributes', 'expr.block.attributes.inner-attributes', 'expr.block.attributes.valid',),
        (1, 4, 23, 26, 29, 32, 35, 41, 44, 47, 50, 60, 70, 90, 148, 173, 176, 181, 184, 187, 190, 193, 199, 203, 208, 213, 216, 219, 222, 233, 236, 241, 244, 247, 270, 285, 295, 298, 303, 320, 325, 328, 338,)),)

    BLOCK_HASHES = (
        '438e57a75e587a64e01d2087ea567672f69326782d2c24e48496ca5505c7fd4d',
        'ed1e7357a51b4198af0dd2da948dde067b68bd5060fa791ee6e84d30dc2d1992',
        'c00180b7150b297a507f05bdfd114b50c5a2acff07a3f9563ebea5442ddd4a72',
        'bbe6f2e7b9b854be1ad189adee89dcb1c8c3e6a0d171d6d3ab2156f33c6c87c6',
        '3f45cb7ec0afa9cea5b0bba23d19fed37a1e50a197a1c38a22aee890ac49d72d',
        'd1b942734c8a4a1643672d7404736dc738565cf5c41a81e8658d586467643686',
        '679dee117abd246fe5c0cc3c08e35da926e22db9e587921390a6b01e31bf5f38',
        'aab2f236caaa8d3886f2de39f3bb93ac3f30a364d369c61478bd8ba7a0e33050',
        '1206a4ed1fec5e71ea015aebbc85b41a8d1640f08abc31ea49e7935b5bc728e9',
        '6d687603f35d6d9b531b172dceec39ccf4a41cd1128916cfde1934dfb51dfccb',
        '271c4ae5449d793ac7af94473245b5008ae5b670c34939862e86a471ea13d536',
        'b0c7133d96a920a86f99e23deec0d79a22ca9e48d2786c3b5aad02d871530cd8',
        'f77e96322f75832910bb44ddf031a193dadb4f6b37f583c10b5ef0e94c93779b',
        '22086fbb79cb6736e4e878d8022630d37333906f53b998458a70a576152f4aef',
        '3bb9a404f339a654c7baa5f0631c037f3ea89ec5d1cedaed472b3f13b6ff8a92',
        '194a6186cdccb0ecbf9d993cc3d09a62a306862a5d47ebb9958b3c8783d7897e',
        '9d817c042dfb16bd0aa7c93b86dad0439cd1a739eb2a6865f85125d10b1926eb',
        'ad5a4b5ca6b109b9094b47772124cc5e0a980760a6f112e2414da4b46935c9c0',
        'c3a6a0c09d3d0ca1a1b5055e6c37be671e676ae3a3773d7db635910c659f5750',
        '1c4ae04e4769f12402fae17cc423dcdec9701d4a17d7bb585069f401b07801ab',
        '4a73104408d7a1076f8c6e726148e08b8beee5561198be9fe5e2dba3b6c138d4',
        '41232da6524b99b4da1c4860263dd97d36b5e69cbc73b247273dcb5f674c98b2',
        '40aa535df5be8dbae7f8890788a70ebeaaceefdabada52691b44f3747ccf9181',
        '462bdcc83f30cc6795c807936c565fbc6ff20dd6dc9370686f5bbb7c215c9589',
        '3e8904e6f2fb4d2306604e46e4dbfd5c85e58cae2240178b6636b486643b2e1f',
        '52772f45ce47c510e4cbebdc315216040869ad3172d943262d3a346c0eb33f0d',
        '4152d4987c1921b680c074c83558b89508cab788bc86719d5ac9e077bf3477b7',
        'd53d20ac04ab5bd9cc9aad2155b651d2c3dd6977e6cce5da81601046860d2a6b',
        'c27c826f3af182bd17b55c0d257ff9ea269b21b6a7d7f6b02453c4b5d8ca295f',
        '1356d10b4961310d369fbcbb75d64177131ac65b3e94611d3bcbc6a9e1c03801',
        'ee4e592fad964a439b6edb093110539a4469fbf0f9ae7a0ab4d9fae6ec0606fb',
        '0438ee98ed43e85289fecab5ec6966771be185790cf26f167b7f1db770b6b3e2',
        '184b025ca8b10ff02344a76edb9ec7996bd153d5c8ff96ed5fb1960b9ab21d62',
        '7ac53cc11e3e7e48028716ded4ebe05d98e4015543bcbefbd269b6f671b3a5f5',
        '757dfce0f24789e6806d8fd9cfd70103420e0ed303a01fb0ece9fa11f9c09af6',
        'faed8fcedd3cabec7099ce7f2ea7bb0e9edf77ba80c82bb7acff4908a17a0791',
        'd3ac81798e2d23a8184cd6676de34859d9092ae9d96cb541b9d3946f62c1dc5c',
        '0088064e4bf4d94f942bda24c02468ad282e0f48ccfe3fbff4132115f7d469c8',
        '3fb8d38056f90d0b09f9c7b47ffbbbc0270ffd380fa58a9325cbc56c1144ab64',
        '6cce3a6f5a5de8f4450f33ed932c431d20ea4c1074fb219da2b81ade121b441b',
        'f6e83321d9a93ddacc0b3fad4b6901ff1c93473a149608524bffd9ecc92dff1d',
        'd858696d4e1f370c67ef7926a2f0943b143874ecc30b1f0694f0bcdf9304b938',
        '602b906373bb37431104df3d1de336c6a3cc933f2c637002e237be822e2341fd',
    )

    def setUp(self):
        source = os.environ.get('JUSTRUST_WITNESS_REFERENCE_ROOT')
        if not source:
            self.skipTest('set JUSTRUST_WITNESS_REFERENCE_ROOT for pinned-source structural checks')
        self.source = Path(source)
        repo = Path(os.environ.get('JUSTRUST_WITNESS_REPO_ROOT', BASE.parent))
        self.index = repo / 'data/compiler-language-reference-rules.json'
        self.inventory = repo / 'data/compiler-language-inventory.json'

    def artifact(self, source=None):
        return build(self.index, self.inventory,
                     self.source if source is None else source, 'blocks')

    def test_module_pins_actual_aliases_and_no_prior_identity_overlap(self):
        import blocks_reconciliation_proposals as proposals
        self.assertEqual(proposals.SOURCE_COMMIT,
                         '86635e30bf861a038dc197d7e16fd09e7e514e7a')
        self.assertEqual(proposals.PAGE_SHA256, {p: h for p, h, _, _, _ in self.PAGES})
        self.assertEqual(proposals.ALIASES, {
            'block': 'reference-expressions-block-expr-block-values',
            'async': 'reference-expressions-block-expr-async-blocks',
            'const': 'reference-const-eval-const-legality',
            'diverge': 'reference-divergence-diverging-paths',
            'scope': 'reference-names-scopes-lexical-scopes',
            'unsafe': 'reference-unsafe-keyword-unsafe-forms',
            'attr': 'reference-attributes-attribute-placement',
            'lint': 'reference-attributes-diagnostics-lint-levels',
        })
        obligations = {o['id'] for section in json.loads(self.inventory.read_bytes())['sections']
                       for o in section['obligations']}
        self.assertTrue(set(proposals.ALIASES.values()) <= obligations)
        index = json.loads(self.index.read_bytes())
        prior_pages = {path for name, _ in reconciliation.PROPOSAL_COUNTS if name not in ('trait-objects', 'type-parameters', 'nominal-types', 'inferred-types', 'blocks', 'expression-core', 'path-expressions', 'never-type', 'coercions', 'subtyping', 'type-expressions', 'primitive-types', 'sequence-types', 'pointer-types', 'function-pointers', 'function-items')
                       for path in reconciliation.profile_inputs(name)[0]}
        prior = {(page['path'], rule['label'])
                 for page in index['pages'] + index['unlinked_markdown_files']
                 if page['path'] in prior_pages for rule in page['rules']}
        current = {(row['page'], row['rule']) for row in self.artifact()['rows']}
        self.assertFalse(prior & current)
        self.assertEqual((len(prior), len(current), len(prior | current)), (720, 43, 763))

    def verify(self, data, source=None):
        return validate(data, self.index, self.inventory,
                        self.source if source is None else source, 'blocks')

    def test_pinned_order_hash_lineblocks_reversible_ids_and_partition(self):
        import hashlib
        import re
        data = self.artifact()
        counts = self.verify(data)
        self.assertEqual([counts[k] for k in ('selected_rule_rows',
                         'unselected_rule_identities', 'global_rule_identities')], [43, 2922, 2965])
        self.assertEqual([(p['path'], p['sha256']) for p in data['selected_pages']],
                         [(p, h) for p, h, _, _, _ in self.PAGES])
        expected_labels = tuple(label for _, _, _, labels, _ in sorted(self.PAGES) for label in labels)
        self.assertEqual(tuple(row['rule'] for row in data['rows']), expected_labels)
        offset = 0
        sha = lambda value: hashlib.sha256(value).hexdigest()
        for page, expected_hash, length, labels, starts in sorted(self.PAGES):
            raw = (self.source / page).read_bytes()
            self.assertEqual(sha(raw), expected_hash)
            lines = raw.splitlines(keepends=True)
            self.assertEqual(len(lines), length)
            markers = [(number, match.group(1).decode())
                       for number, line in enumerate(lines, 1)
                       if (match := re.fullmatch(rb'r\[([^\]\r\n]+)\]\r?\n?', line))]
            self.assertEqual(markers, list(zip(starts, labels)))
            for position, (start, label) in enumerate(markers):
                row = data['rows'][offset + position]
                end = starts[position + 1] - 1 if position + 1 < len(starts) else length
                with self.subTest(page=page, label=label):
                    self.assertEqual((row['page'], row['line'], row['end_line']), (page, start, end))
                    self.assertEqual(sha(b''.join(lines[start - 1:end])),
                                     self.BLOCK_HASHES[offset + position])
                    self.assertEqual(row['source_block_sha256'], self.BLOCK_HASHES[offset + position])
                    prefix = 'reconcile-blocks-'
                    self.assertEqual(row['id'], prefix + label.encode().hex())
                    self.assertEqual(bytes.fromhex(row['id'][len(prefix):]).decode(), label)
                    self.assertEqual(row['review_status'], 'unreviewed')
                    self.assertTrue(row['missing_test_needs'])
                    self.assertTrue(all(need.startswith(('positive: Missing ', 'invalid: Missing ',
                                                        'interaction: Missing ', 'end-to-end: Missing '))
                                        for need in row['missing_test_needs']))
            offset += len(labels)
        self.assertEqual(len({r['id'] for r in data['rows']}), 43)
        for key in ('reviewed_rule_rows', 'rows_with_exact_executable_evidence'):
            self.assertEqual(counts[key], 0)
        index = json.loads(self.index.read_bytes())
        universe = {(p['path'], r['label']) for p in index['pages'] + index['unlinked_markdown_files']
                    for r in p['rules']}
        selected = {(r['page'], r['rule']) for r in data['rows']}
        other = {(r['page'], r['rule']) for r in data['unselected_rules']}
        self.assertEqual((len(selected), len(other), len(universe)), (43, 2922, 2965))
        self.assertEqual(len(data['unselected_rules']), 2922)
        self.assertFalse(selected & other)
        self.assertEqual(selected | other, universe)
        self.assertEqual(data, self.artifact())

    def test_rows_sort_by_page_line_independently_of_authored_order(self):
        import blocks_reconciliation_proposals as proposals
        original = self.artifact()
        rows = proposals.ROWS.strip().splitlines()
        expected_order = [(p, start) for p, _, _, _, starts in sorted(self.PAGES)
                          for start in starts]
        for reordered in (rows[::-1], rows[5:] + rows[:5], rows[::2] + rows[1::2]):
            with self.subTest(order=[row.split('|')[0] for row in reordered]):
                with mock.patch.object(proposals, 'ROWS', '\n'.join(reordered)):
                    data = self.artifact()
                    self.assertEqual([(r['page'], r['line']) for r in data['rows']], expected_order)
                    self.assertEqual([p['path'] for p in data['selected_pages']],
                                     list(proposals.SELECTED))
                    self.assertEqual(data, original)
                    self.assertEqual(self.verify(data), self.verify(original))

    def test_missing_duplicate_foreign_and_malformed_authored_rows_rejected(self):
        import blocks_reconciliation_proposals as proposals
        self.assertEqual(proposals.SELECTED, tuple(p[0] for p in self.PAGES))
        valid = self.artifact()
        rows = proposals.ROWS.strip().splitlines()
        self.assertEqual(tuple(row.split('|')[0] for row in rows),
                         tuple(label for p in self.PAGES for label in p[3]))
        foreign = rows[0].split('|')
        foreign[0] = 'expr.loop'
        cases = [('missing-prefix', rows[5:]), ('missing-suffix', rows[:5]),
                 ('missing-row', rows[:-1]), ('empty', []), ('duplicate', rows + [rows[0]]),
                 ('foreign', ['|'.join(foreign), *rows[1:]]),
                 ('short-row', ['|'.join(rows[0].split('|')[:-1]), *rows[1:]]),
                 ('long-row', [rows[0] + '|extra', *rows[1:]])]
        for column in (4, 5, 6):
            changed = rows[0].split('|')
            changed[column] = '   '
            cases.append(('blank-need-' + str(column), ['|'.join(changed), *rows[1:]]))
        for name, changed in cases:
            with self.subTest(mutation=name), mock.patch.object(proposals, 'ROWS', '\n'.join(changed)):
                with self.assertRaises(ValueError):
                    self.artifact()
                with self.assertRaises(ValueError):
                    self.verify(valid)

    def test_forged_artifacts_cannot_claim_credit_or_rebind_source(self):
        original = self.artifact()
        mutations = (
            lambda d: d['rows'].pop(),
            lambda d: d['rows'].append(copy.deepcopy(d['rows'][0])),
            lambda d: d['rows'].reverse(),
            lambda d: d['rows'][0].update(page='src/expressions/range-expr.md'),
            lambda d: d['rows'][5].update(page='src/expressions/underscore-expr.md'),
            lambda d: d['rows'][0].update(rule='expr.loop'),
            lambda d: d['rows'][0].update(line=True),
            lambda d: d['rows'][0].update(line=1.0),
            lambda d: d['counts'].update(reviewed_rule_rows=False),
            lambda d: d['rows'][0].update(relationship='exact-executable-evidence'),
            lambda d: d['rows'][0].update(classifications=['grammar']),
            lambda d: d['rows'][0].update(end_line=99999),
            lambda d: d['rows'][0].update(source_block_sha256='0' * 64),
            lambda d: d['rows'][9].update(source_block_sha256='0' * 64),
            lambda d: d['rows'][0].update(id='reconcile-blocks-expr-range'),
            lambda d: d['rows'][0].update(id=d['rows'][1]['id']),
            lambda d: d['rows'][0].update(review_status='reviewed'),
            lambda d: d['rows'][0].update(rationale='invented interpretation'),
            lambda d: d['rows'][0].update(candidate_obligation_ids=['foreign']),
            lambda d: d['rows'][0].update(missing_test_needs=[]),
            lambda d: d['rows'][0]['missing_test_needs'].__setitem__(0, 'positive: implemented'),
            lambda d: d['counts'].update(reviewed_rule_rows=1),
            lambda d: d['counts'].update(rows_with_exact_executable_evidence=1),
            lambda d: d['selected_pages'][0].update(sha256='0' * 64),
            lambda d: d['selected_pages'].clear(),
            lambda d: d['bindings'].update(reference_commit='0' * 40),
            lambda d: d['unselected_rules'].pop(),
            lambda d: d['unselected_rules'].append(copy.deepcopy(d['unselected_rules'][0])),
            lambda d: d['unselected_rules'][0].update(page=d['rows'][0]['page'], rule=d['rows'][0]['rule']),
        )
        for number, mutate in enumerate(mutations):
            changed = copy.deepcopy(original)
            mutate(changed)
            self.assertNotEqual(json.dumps(changed, sort_keys=True),
                                json.dumps(original, sort_keys=True),
                                'mutation must change JSON, including bool versus int')
            with self.subTest(mutation=number), self.assertRaises(ValueError):
                self.verify(changed)

    def test_either_page_source_fault_preserves_exact_api_error_type(self):
        data = self.artifact()
        for page, _, _, _, _ in self.PAGES:
            for state, error in (('stale', ValueError), ('missing', FileNotFoundError),
                                 ('directory', IsADirectoryError), ('symlink', ValueError)):
                with self.subTest(page=page, state=state), tempfile.TemporaryDirectory() as temp:
                    root = Path(temp)
                    for selected, _, _, _, _ in self.PAGES:
                        target = root / selected
                        target.parent.mkdir(parents=True, exist_ok=True)
                        target.write_bytes((self.source / selected).read_bytes())
                    target = root / page
                    if state == 'stale':
                        target.write_bytes(target.read_bytes() + b'\n')
                    else:
                        target.unlink()
                        if state == 'directory':
                            target.mkdir()
                        elif state == 'symlink':
                            target.symlink_to(self.source / page)
                    for action in (lambda: self.artifact(root), lambda: self.verify(data, root)):
                        with self.assertRaises(error) as caught:
                            action()
                        self.assertIs(type(caught.exception), error)

    def test_source_only_witness_restrictions_fail_before_io(self):
        args = ['--profile', 'blocks', '--rule-index', '/absent/index',
                '--inventory', '/absent/inventory', '--source-root', '/absent/source',
                '--artifact', '/absent/artifact']
        extras = ([], ['--repo-root', '/absent/repo'], ['--evidence', '/absent/evidence'],
                  ['--repo-root', '/absent/repo', '--evidence', '/absent/evidence'])
        cases = [(action, extra) for action in ('generate-witness-links', 'build-witness-links',
                  'validate-witness-links', 'assess-witness-links') for extra in extras]
        cases += [(action, extra) for action in ('generate', 'validate') for extra in extras[1:]]
        with contextlib.ExitStack() as stack:
            for name in ('read_bytes', 'read_text', 'open', 'is_symlink', 'is_file',
                         'write_bytes', 'write_text', 'mkdir'):
                stack.enter_context(mock.patch.object(Path, name, side_effect=AssertionError('IO')))
            stack.enter_context(mock.patch.object(subprocess, 'run', side_effect=AssertionError('execution')))
            for action, extra in cases:
                with self.subTest(action=action, extra=extra), self.assertRaises(ValueError):
                    main([action, *args, *extra])
            calls = (
                lambda: reconciliation.build_witness_links(
                    self.index, self.inventory, self.source, self.source, 'blocks'),
                lambda: reconciliation.validate_witness_links(
                    {}, self.index, self.inventory, self.source, self.source, 'blocks'),
                lambda: reconciliation.assess_witness_links(
                    {}, self.index, self.inventory, self.source, self.source, (), 'blocks'),
            )
            for call in calls:
                with self.subTest(api=call), self.assertRaises(ValueError):
                    call()

    def test_cli_exclusive_output_and_validate(self):
        with tempfile.TemporaryDirectory() as temp:
            artifact = Path(temp) / 'structs.json'
            args = ['--profile', 'blocks', '--rule-index', str(self.index),
                    '--inventory', str(self.inventory), '--source-root', str(self.source),
                    '--artifact', str(artifact)]
            with contextlib.redirect_stdout(io.StringIO()) as output:
                main(['generate', *args])
            canonical = (json.dumps(self.artifact(), indent=2) + '\n').encode()
            self.assertEqual(artifact.read_bytes(), canonical)
            self.assertEqual(json.loads(output.getvalue()), self.verify(self.artifact()))
            with contextlib.redirect_stdout(io.StringIO()), self.assertRaises(FileExistsError):
                main(['generate', *args])
            self.assertEqual(artifact.read_bytes(), canonical)
            with contextlib.redirect_stdout(io.StringIO()) as output:
                main(['validate', *args])
            self.assertEqual(json.loads(output.getvalue()), self.verify(self.artifact()))
            self.assertEqual(artifact.read_bytes(), canonical)
            forged = json.loads(canonical)
            forged['counts']['reviewed_rule_rows'] = 1
            artifact.write_text(json.dumps(forged))
            before = artifact.read_bytes()
            with contextlib.redirect_stdout(io.StringIO()), self.assertRaises(ValueError):
                main(['validate', *args])
            self.assertEqual(artifact.read_bytes(), before)

    def test_legacy_structs_canonical_bytes_unchanged_without_blocks_module(self):
        canonical = lambda data: (json.dumps(data, indent=2) + '\n').encode()
        expected = canonical(build(self.index, self.inventory, self.source, 'structs'))
        with mock.patch.dict(sys.modules, {'blocks_reconciliation_proposals': None}):
            data = build(self.index, self.inventory, self.source, 'structs')
            validate(data, self.index, self.inventory, self.source, 'structs')
        self.assertEqual(canonical(data), expected)
        self.assertEqual(digest(canonical(data)),
                         '5fb87b17efaa5ac3d34840f9735b2499760c748ba7d9a5fa6a8909f4321f621e')


class ExpressionCoreSourceReconciliationTests(unittest.TestCase):
    """Independent pinned-source structural checks, not semantic execution credit."""
    PAGE = 'src/expressions.md'
    PAGE_HASH = 'e8dbb403137e2debda174d4add7a603c34467de1e36b473f816132a03d380480'
    LABELS = ('expr', 'expr.syntax', 'expr.intro', 'expr.evaluation', 'expr.operands',
              'expr.behavior', 'expr.structure', 'expr.precedence', 'expr.operand-order',
              'expr.operand-order.default', 'expr.operand-order.operands-before-primary',
              'expr.place-value', 'expr.place-value.intro', 'expr.place-value.place-memory-location',
              'expr.place-value.place-expr-kinds', 'expr.place-value.value-expr-kinds',
              'expr.place-value.value-result', 'expr.place-value.place-context',
              'expr.place-value.assignee', 'expr.place-value.parenthesis', 'expr.move',
              'expr.move.intro', 'expr.move.copy', 'expr.move.requires-sized',
              'expr.move.movable-place', 'expr.move.deinitialization', 'expr.move.place-invalid',
              'expr.mut', 'expr.mut.intro', 'expr.mut.valid-places', 'expr.temporary',
              'expr.super-macros', 'expr.super-macros.intro', 'expr.super-macros.format_args',
              'expr.super-macros.format_args.super-operands',
              'expr.super-macros.format_args.super-temporaries', 'expr.super-macros.pin',
              'expr.super-macros.pin.super-operands', 'expr.super-macros.pin.super-temporaries',
              'expr.implicit-borrow', 'expr.implicit-borrow-intro',
              'expr.implicit-borrow.application', 'expr.overload', 'expr.attr',
              'expr.attr.restriction', 'expr.attr.never-before')
    STARTS = (1, 4, 48, 51, 54, 57, 64, 70, 97, 100, 123, 145, 148, 151, 154,
              157, 160, 163, 179, 190, 193, 196, 199, 202, 205, 213, 216, 219,
              222, 225, 237, 242, 245, 251, 254, 266, 295, 298, 308, 320, 323,
              339, 351, 356, 359, 367)
    BLOCK_HASHES = (
        '55de7ede8cbbe7cdc03be155f301808555d13b728775ae65802532b0f834281e',
        '866a57170cb813210c570e5b9f94ec2dbb51e358680206dc419234ee266bd488',
        '004c9613541bc0600bc8d4747cc4f056ece1c7ab77d35fe6b62857d8f9f96d12',
        'a552f7876e883bf0aaffefcac25520b1f6e35088fa54a0b296662deccca40c32',
        'b1288ff695d6e092fe11cb8d1e0f1329f19736429642a4e05d70b424a369a0f3',
        'f5c32a051e8c27588b400a6cea6d19bbb9f2ed667b04cde30681325c243d4e82',
        '54ea180f4f6912140dea8cbfa1bab8331a71252d655269223f0fa85844ac24f6',
        '8094a34f49fb66664ee40a285ca54415e3915fab6d0814aee90818790b9eb11d',
        'f1a4e292bb83334cd519de5d858e05b2bc4adbdc22dea153d78bc8ea5863bd60',
        '668094cb87140392fe93d73678da19bbf6f4a4c0cd267053e4713413ab2899db',
        'e0abb94eb9bc228c194ebbcf79c1676b1117a49536f41f625b534cb252738e38',
        'add853ace61090f1b733384ae1929c359f5894c57e524205f650b0bb17ae1e9f',
        '8709431a41a1d09a429adcccbf025f1aad5f9d44ea84c3cc642c7b4b948c78b5',
        'efe6cde8151e401f88fc2adfc50e903e132ae62ce8dd2b139763d49ddf80adb6',
        '40d6a9281ffe1f8148fbf3a4ba6021a41c5ccc0bf4fb5b1d02ff61bf50b7e8e3',
        '00b2e7a8fac2f68554298851b91f671f3189932a226b90b37b7cbeba67e2c5d0',
        '2f70fa99ef69419b7d68a14130fcf41a822d24b1a5437ae65d890362690830b3',
        'aea318df0a450db9eb8d6240ba79d1cf36a9944ac6dab2be5d197321094de192',
        '4df962ae9d600e204b0e72afbcb7ff02fe3a6f8f7e4e1161b99c4011ee64827b',
        'da95cd8312b2b2b12a3b80a156e43ec0cbe4f18538aaaf6cf935d28d2499c1c6',
        'b0614761e83eaf91e5da4dba00bb7c36c35aea51a038f6cf1df29b330c9c0ab6',
        'f1d2ae1dfcc3b91e732faa72931e68d90ab6507ff2b4368051f3ba5a00dfaa1e',
        '70116ffc341b1a2dade571a759193c4486320aafd1d215491896ece2888be5bb',
        '7e7b47e46f3223574b18c7afb1be87a36c231cb94bd4028ee47f3c90a1d57331',
        '90b0d523c2c4c3c5400e89b5d13ffaeb2bcd38c7bbfb4134dfd530d958d6735c',
        'd01cb12aa1c1187ee6847f73be29d779f18160a29a26a8da140bd7627d9b44aa',
        '9ab8035dfa683c4986aaf6b8fe43b92c0887466d2a8a40f155d49533d196b3e2',
        '5aaa5b2333ed96010a9fdb207da90b56486f7cf710cf57d655ec6451aef98084',
        '60cc35bbebb12c8b0724970cbb9a1bc1331c2e7323dd11813d6ae883693b8282',
        'def7b003a10beafa3005c5303fcd1f9968d883d29cab3d2c88bf76b698c07ce8',
        '38826e491af12ee7cf233a7c687f8d5c68f0d731ed2ed897b1ae4277a78309bb',
        'afbed40b3539e668a817d973db53b1549cb7a7faab6ae7c70a0de4a63471ccc0',
        'c39951cb56141d7a7f74891d438443e8e4a201948d1d7cb6fd3351dfc9b72fd5',
        'ba8514e28e44b3688ec059ae39673554b1e9700839f43a122dc6d18ab3cc80ef',
        '740cb330b521781b146cfe6d62e57beee4bf053081a01a1e3d6aa39c318cdd01',
        '3eb3ba700724c2932fc53eada5c60217b9f1c1bad62331cf77f848be8a5f4711',
        '666c7514d693c9d5a7bbbd07e6dfdf7d79adc7ac9afaddef558a9f69b740e38a',
        '6409e5cc023e9e0232e57d48602733e5908532ed7e49d841c3feeccc0e85a204',
        'e139fb115724062746738c6f8e9520dcea8546dfed7f58a6d5ba887c24d8c9de',
        '15bc2b0bf31859537b0969d73ac6d903fb3c995b41a9a474f9eeea0dfecf5054',
        '29cfb44744a50e7e31054a5d38dc7eea41ec97c42c6c4b8a3c2f237687aba01a',
        'a89b1ad249050130fb9887e7abda14dbb54ac87ac37d6d7bbf6e8c9986be6994',
        'e3e772c3165d7670a8bbb817a96bbb0d685a40ea205b624f912570ef3507d518',
        '473198874a4c81dd82c85a001cfb1460fa72202b9395da784aef0d9cf37a8bd5',
        '262c3cce95fbd19f46af464a0d815720abb6e8adb8546955264bbe29dbcb6674',
        '8dd167cb472763188f8962969913c9e78d3368f3f81663e0ae8644bd10ef59aa',
    )

    def setUp(self):
        source = os.environ.get('JUSTRUST_WITNESS_REFERENCE_ROOT')
        if not source:
            self.skipTest('set JUSTRUST_WITNESS_REFERENCE_ROOT for pinned-source structural checks')
        self.source = Path(source)
        repo = Path(os.environ.get('JUSTRUST_WITNESS_REPO_ROOT', BASE.parent))
        self.index = repo / 'data/compiler-language-reference-rules.json'
        self.inventory = repo / 'data/compiler-language-inventory.json'

    def artifact(self, source=None):
        return build(self.index, self.inventory,
                     self.source if source is None else source, 'expression-core')

    def verify(self, data, source=None):
        return validate(data, self.index, self.inventory,
                        self.source if source is None else source, 'expression-core')

    def test_actual_aliases_pins_no_overlap_and_integrated_totals(self):
        import expression_core_reconciliation_proposals as proposals
        self.assertEqual(proposals.SELECTED, (self.PAGE,))
        self.assertEqual(proposals.PAGE_SHA256, {self.PAGE: self.PAGE_HASH})
        self.assertEqual(proposals.SOURCE_COMMIT, '86635e30bf861a038dc197d7e16fd09e7e514e7a')
        self.assertEqual(proposals.ALIASES, {
            'order': 'reference-expressions-operand-order',
            'precedence': 'reference-expressions-precedence',
            'borrow': 'reference-expressions-operator-expr-borrow-deref',
            'assign': 'reference-expressions-operator-expr-assignments',
            'traits': 'reference-special-types-and-traits-lang-contracts',
            'drop': 'reference-destructors-drop-order',
            'extend': 'reference-destructors-temporary-extension',
            'attr': 'reference-attributes-attribute-placement',
        })
        obligations = {o['id'] for s in json.loads(self.inventory.read_bytes())['sections']
                       for o in s['obligations']}
        self.assertTrue(set(proposals.ALIASES.values()) <= obligations)
        index = json.loads(self.index.read_bytes())
        prior_pages = {p for name, _ in reconciliation.PROPOSAL_COUNTS if name not in ('trait-objects', 'type-parameters', 'nominal-types', 'inferred-types', 'expression-core', 'path-expressions', 'never-type', 'coercions', 'subtyping', 'type-expressions', 'primitive-types', 'sequence-types', 'pointer-types', 'function-pointers', 'function-items')
                       for p in reconciliation.profile_inputs(name)[0]}
        prior = {(p['path'], r['label']) for p in index['pages'] + index['unlinked_markdown_files']
                 if p['path'] in prior_pages for r in p['rules']}
        current = {(r['page'], r['rule']) for r in self.artifact()['rows']}
        self.assertFalse(prior & current)
        self.assertEqual((len(prior), len(current), len(prior | current)), (763, 46, 809))
        summary = reconciliation.summarize_proposals(self.index, self.inventory, include_proposals=True)
        self.assertEqual((summary['proposed_rule_identities'],
                          summary['rule_identities_without_proposals']), (1030, 1935))
        self.assertEqual(sum(p['proposed_rule_identities'] for p in summary['profiles'].values()), 1035)
        self.assertEqual(summary['profiles']['expression-core']['completed_rule_rows'], 0)

    def test_hardcoded_hash_labels_lineblocks_partition_and_zero_credit(self):
        import re
        data = self.artifact()
        counts = self.verify(data)
        self.assertEqual([counts[k] for k in ('selected_rule_rows', 'unselected_rule_identities',
                                             'global_rule_identities')], [46, 2919, 2965])
        raw = (self.source / self.PAGE).read_bytes()
        self.assertEqual(digest(raw), self.PAGE_HASH)
        lines = raw.splitlines(keepends=True)
        self.assertEqual(len(lines), 430)
        markers = [(i, m.group(1).decode()) for i, line in enumerate(lines, 1)
                   if (m := re.fullmatch(rb'r\[([^\]\r\n]+)\]\r?\n?', line))]
        self.assertEqual(markers, list(zip(self.STARTS, self.LABELS)))
        self.assertEqual(tuple(r['rule'] for r in data['rows']), self.LABELS)
        self.assertEqual([(p['path'], p['sha256']) for p in data['selected_pages']],
                         [(self.PAGE, self.PAGE_HASH)])
        for i, row in enumerate(data['rows']):
            end = self.STARTS[i + 1] - 1 if i + 1 < 46 else 430
            with self.subTest(rule=row['rule']):
                self.assertEqual((row['page'], row['line'], row['end_line']),
                                 (self.PAGE, self.STARTS[i], end))
                self.assertEqual(digest(b''.join(lines[self.STARTS[i] - 1:end])), self.BLOCK_HASHES[i])
                self.assertEqual(row['source_block_sha256'], self.BLOCK_HASHES[i])
                prefix = 'reconcile-expression-core-'
                self.assertEqual(row['id'], prefix + self.LABELS[i].encode().hex())
                self.assertEqual(bytes.fromhex(row['id'][len(prefix):]).decode(), self.LABELS[i])
                self.assertEqual(row['review_status'], 'unreviewed')
                self.assertEqual(len(row['missing_test_needs']), 4)
                for need, kind in zip(row['missing_test_needs'], ('positive', 'invalid', 'interaction', 'end-to-end')):
                    self.assertTrue(need.startswith(kind + ': Missing '))
                self.assertIn('stable1.98.1 eligibility classification', row['missing_test_needs'][-1])
        self.assertEqual(len({r['id'] for r in data['rows']}), 46)
        for k in ('reviewed_rule_rows', 'rows_with_exact_executable_evidence'):
            self.assertEqual(counts[k], 0)
        index = json.loads(self.index.read_bytes())
        universe = {(p['path'], r['label']) for p in index['pages'] + index['unlinked_markdown_files']
                    for r in p['rules']}
        selected = {(r['page'], r['rule']) for r in data['rows']}
        other = {(r['page'], r['rule']) for r in data['unselected_rules']}
        self.assertEqual((len(selected), len(other), len(universe)), (46, 2919, 2965))
        self.assertEqual(len(data['unselected_rules']), 2919)
        self.assertFalse(selected & other)
        self.assertEqual(selected | other, universe)
        self.assertEqual(self.artifact(), data)

    def test_permuted_authored_rows_preserve_canonical_order(self):
        import expression_core_reconciliation_proposals as proposals
        baseline = self.artifact()
        rows = proposals.ROWS.strip().splitlines()
        for changed in (rows[::-1], rows[7:] + rows[:7], rows[::2] + rows[1::2]):
            with mock.patch.object(proposals, 'ROWS', '\n'.join(changed)):
                data = self.artifact()
                self.assertEqual(data, baseline)
                self.assertEqual(tuple(r['rule'] for r in data['rows']), self.LABELS)
                self.assertEqual(self.verify(data), self.verify(baseline))

    def test_missing_duplicate_foreign_malformed_authored_rows_rejected(self):
        import expression_core_reconciliation_proposals as proposals
        valid = self.artifact()
        rows = proposals.ROWS.strip().splitlines()
        self.assertEqual(tuple(r.split('|')[0] for r in rows), self.LABELS)
        cases = [rows[5:], rows[:5], rows[:-1], [], rows + [rows[0]],
                 ['expr.loop' + rows[0][4:], *rows[1:]],
                 ['|'.join(rows[0].split('|')[:-1]), *rows[1:]],
                 [rows[0] + '|extra', *rows[1:]]]
        for col in (4, 5, 6):
            fields = rows[0].split('|')
            fields[col] = '   '
            cases.append(['|'.join(fields), *rows[1:]])
        for i, changed in enumerate(cases):
            with self.subTest(case=i), mock.patch.object(proposals, 'ROWS', '\n'.join(changed)):
                with self.assertRaises(ValueError):
                    self.artifact()
                with self.assertRaises(ValueError):
                    self.verify(valid)

    def test_29_actual_changing_forgeries_rejected(self):
        baseline = self.artifact()
        mutations = (
            lambda d: d['rows'].pop(),
            lambda d: d['rows'].append(copy.deepcopy(d['rows'][0])),
            lambda d: d['rows'].reverse(),
            lambda d: d['rows'][0].update(page='src/expressions/block-expr.md'),
            lambda d: d['rows'][5].update(page='src/expressions/struct-expr.md'),
            lambda d: d['rows'][0].update(rule='expr.loop'),
            lambda d: d['rows'][0].update(line=True),
            lambda d: d['rows'][0].update(line=1.0),
            lambda d: d['counts'].update(reviewed_rule_rows=False),
            lambda d: d['rows'][0].update(relationship='exact-executable-evidence'),
            lambda d: d['rows'][0].update(classifications=['grammar']),
            lambda d: d['rows'][0].update(end_line=99999),
            lambda d: d['rows'][0].update(source_block_sha256='0' * 64),
            lambda d: d['rows'][9].update(source_block_sha256='0' * 64),
            lambda d: d['rows'][0].update(id='reconcile-expression-core-expr'),
            lambda d: d['rows'][0].update(id=d['rows'][1]['id']),
            lambda d: d['rows'][0].update(review_status='reviewed'),
            lambda d: d['rows'][0].update(rationale='invented interpretation'),
            lambda d: d['rows'][0].update(candidate_obligation_ids=['foreign']),
            lambda d: d['rows'][0].update(missing_test_needs=[]),
            lambda d: d['rows'][0]['missing_test_needs'].__setitem__(0, 'positive: implemented'),
            lambda d: d['counts'].update(reviewed_rule_rows=1),
            lambda d: d['counts'].update(rows_with_exact_executable_evidence=1),
            lambda d: d['selected_pages'][0].update(sha256='0' * 64),
            lambda d: d['selected_pages'].clear(),
            lambda d: d['bindings'].update(reference_commit='0' * 40),
            lambda d: d['unselected_rules'].pop(),
            lambda d: d['unselected_rules'].append(copy.deepcopy(d['unselected_rules'][0])),
            lambda d: d['unselected_rules'][0].update(page=d['rows'][0]['page'], rule=d['rows'][0]['rule']),
        )
        self.assertEqual(len(mutations), 29)
        for i, mutate in enumerate(mutations):
            changed = copy.deepcopy(baseline)
            mutate(changed)
            self.assertNotEqual(json.dumps(changed, sort_keys=True), json.dumps(baseline, sort_keys=True))
            with self.subTest(mutation=i), self.assertRaises(ValueError):
                self.verify(changed)

    def test_source_faults_preserve_exact_error_types(self):
        data = self.artifact()
        for state, error in (('stale', ValueError), ('missing', FileNotFoundError),
                             ('directory', IsADirectoryError), ('symlink', ValueError)):
            with self.subTest(state=state), tempfile.TemporaryDirectory() as temp:
                root = Path(temp)
                target = root / self.PAGE
                target.parent.mkdir(parents=True)
                if state == 'stale':
                    target.write_bytes((self.source / self.PAGE).read_bytes() + b'\n')
                elif state == 'directory':
                    target.mkdir()
                elif state == 'symlink':
                    target.symlink_to(self.source / self.PAGE)
                for action in (lambda: self.artifact(root), lambda: self.verify(data, root)):
                    with self.assertRaises(error) as caught:
                        action()
                    self.assertIs(type(caught.exception), error)

    def test_source_only_restrictions_precede_io_and_execution(self):
        args = ['--profile', 'expression-core', '--rule-index', '/absent/index',
                '--inventory', '/absent/inventory', '--source-root', '/absent/source',
                '--artifact', '/absent/artifact']
        extras = ([], ['--repo-root', '/absent/repo'], ['--evidence', '/absent/evidence'],
                  ['--repo-root', '/absent/repo', '--evidence', '/absent/evidence'])
        cases = [(action, extra) for action in ('generate-witness-links', 'build-witness-links',
                  'validate-witness-links', 'assess-witness-links') for extra in extras]
        cases += [(action, extra) for action in ('generate', 'validate') for extra in extras[1:]]
        with contextlib.ExitStack() as stack:
            for name in ('read_bytes', 'read_text', 'open', 'is_symlink', 'is_file',
                         'write_bytes', 'write_text', 'mkdir'):
                stack.enter_context(mock.patch.object(Path, name, side_effect=AssertionError('IO')))
            stack.enter_context(mock.patch.object(subprocess, 'run', side_effect=AssertionError('execution')))
            for action, extra in cases:
                with self.subTest(action=action, extra=extra), self.assertRaises(ValueError):
                    main([action, *args, *extra])
            for call in (
                lambda: reconciliation.build_witness_links(
                    self.index, self.inventory, self.source, self.source, 'expression-core'),
                lambda: reconciliation.validate_witness_links(
                    {}, self.index, self.inventory, self.source, self.source, 'expression-core'),
                lambda: reconciliation.assess_witness_links(
                    {}, self.index, self.inventory, self.source, self.source, (), 'expression-core'),
            ):
                with self.assertRaises(ValueError):
                    call()

    def test_cli_exclusive_generation_validation_and_no_repair(self):
        with tempfile.TemporaryDirectory() as temp:
            artifact = Path(temp) / 'expression-core.json'
            args = ['--profile', 'expression-core', '--rule-index', str(self.index),
                    '--inventory', str(self.inventory), '--source-root', str(self.source),
                    '--artifact', str(artifact)]
            expected = (json.dumps(self.artifact(), indent=2) + '\n').encode()
            with contextlib.redirect_stdout(io.StringIO()) as output:
                main(['generate', *args])
            self.assertEqual(artifact.read_bytes(), expected)
            self.assertEqual(json.loads(output.getvalue()), self.verify(self.artifact()))
            with contextlib.redirect_stdout(io.StringIO()), self.assertRaises(FileExistsError):
                main(['generate', *args])
            self.assertEqual(artifact.read_bytes(), expected)
            with contextlib.redirect_stdout(io.StringIO()) as output:
                main(['validate', *args])
            self.assertEqual(json.loads(output.getvalue()), self.verify(self.artifact()))
            self.assertEqual(artifact.read_bytes(), expected)
            forged = json.loads(expected)
            forged['counts']['reviewed_rule_rows'] = 1
            artifact.write_text(json.dumps(forged))
            before = artifact.read_bytes()
            with contextlib.redirect_stdout(io.StringIO()), self.assertRaises(ValueError):
                main(['validate', *args])
            self.assertEqual(artifact.read_bytes(), before)

    def test_last_prior_blocks_canonical_bytes_without_new_module(self):
        canonical = lambda data: (json.dumps(data, indent=2) + '\n').encode()
        expected = canonical(build(self.index, self.inventory, self.source, 'blocks'))
        with mock.patch.dict(sys.modules, {'expression_core_reconciliation_proposals': None}):
            data = build(self.index, self.inventory, self.source, 'blocks')
            validate(data, self.index, self.inventory, self.source, 'blocks')
        self.assertEqual(canonical(data), expected)
        self.assertEqual(digest(canonical(data)),
                         '44247bc80441734de02b723d046fd34d801801ee2fda6c7d50fcd0154c5e3a90')


class PathExpressionsSourceReconciliationTests(unittest.TestCase):
    """Independent pinned-source structural checks, not semantic execution credit."""
    PAGE = 'src/expressions/path-expr.md'
    PAGE_HASH = 'e5a1ee1c83c4ee6bc7b39d7442e2107ea7147941eba0d8871fb8960940a9caf3'
    LABELS = ('expr.path', 'expr.path.syntax', 'expr.path.intro', 'expr.path.place',
              'expr.path.safety', 'expr.path.const')
    STARTS = (1, 4, 11, 14, 17, 34)
    BLOCK_HASHES = (
        '6decae74184a6ca2d9fd7280ef1c2726ba70d46ee1c7c1cd9e6e3b63f1e8e1e8',
        'b63ce3c4cb9524c7f7e106419a3ab1bcce668f4f797825989e9b041a08940b1b',
        '58c84f0b886e49d80d5553c652882018d93c5825094c56921f24cca49d9ac820',
        '16b0bcdc89a7e8c4279441f67680e74778e7ce31b6a9a8fc84c142dc8d404857',
        'e485d98865a41a7ce8772f16654a4bc602e16fbe2810ee5dafd22713e0bd151e',
        '4af8d69e8bcbd4e75c30222c3dc1fa5c0383c25b52e48d24fb6b7b8015dc8c26',
    )

    def setUp(self):
        source = os.environ.get('JUSTRUST_WITNESS_REFERENCE_ROOT')
        if not source:
            self.skipTest('set JUSTRUST_WITNESS_REFERENCE_ROOT for pinned-source structural checks')
        self.source = Path(source)
        repo = Path(os.environ.get('JUSTRUST_WITNESS_REPO_ROOT', BASE.parent))
        self.index = repo / 'data/compiler-language-reference-rules.json'
        self.inventory = repo / 'data/compiler-language-inventory.json'

    def artifact(self, source=None):
        return build(self.index, self.inventory,
                     self.source if source is None else source, 'path-expressions')

    def verify(self, data, source=None):
        return validate(data, self.index, self.inventory,
                        self.source if source is None else source, 'path-expressions')

    def test_actual_aliases_pins_no_overlap_and_integrated_totals(self):
        import path_expressions_reconciliation_proposals as proposals
        self.assertEqual(proposals.SELECTED, (self.PAGE,))
        self.assertEqual(proposals.PAGE_SHA256, {self.PAGE: self.PAGE_HASH})
        self.assertEqual(proposals.SOURCE_COMMIT, '86635e30bf861a038dc197d7e16fd09e7e514e7a')
        self.assertEqual(proposals.ALIASES, {
            'path': 'reference-expressions-path-expr-value-paths',
            'forms': 'reference-paths-path-forms',
            'resolve': 'reference-names-name-resolution-resolution-context',
            'privacy': 'reference-visibility-and-privacy-privacy-reachability',
            'borrow': 'reference-expressions-operator-expr-borrow-deref',
            'static': 'reference-items-static-items-static-identity',
            'associated': 'reference-items-associated-items-associated-constants',
            'block': 'reference-expressions-block-expr-block-values',
        })
        obligations = {o['id'] for s in json.loads(self.inventory.read_bytes())['sections']
                       for o in s['obligations']}
        self.assertTrue(set(proposals.ALIASES.values()) <= obligations)
        index = json.loads(self.index.read_bytes())
        prior_pages = {p for name, _ in reconciliation.PROPOSAL_COUNTS if name not in ('trait-objects', 'type-parameters', 'nominal-types', 'inferred-types', 'path-expressions', 'never-type', 'coercions', 'subtyping', 'type-expressions', 'primitive-types', 'sequence-types', 'pointer-types', 'function-pointers', 'function-items')
                       for p in reconciliation.profile_inputs(name)[0]}
        prior = {(p['path'], r['label']) for p in index['pages'] + index['unlinked_markdown_files']
                 if p['path'] in prior_pages for r in p['rules']}
        current = {(r['page'], r['rule']) for r in self.artifact()['rows']}
        self.assertFalse(prior & current)
        self.assertEqual((len(prior), len(current), len(prior | current)), (809, 6, 815))
        summary = reconciliation.summarize_proposals(self.index, self.inventory, include_proposals=True)
        self.assertEqual((summary['proposed_rule_identities'],
                          summary['rule_identities_without_proposals']), (1030, 1935))
        self.assertEqual(sum(p['proposed_rule_identities'] for p in summary['profiles'].values()), 1035)
        self.assertEqual(summary['profiles']['path-expressions']['completed_rule_rows'], 0)

    def test_hardcoded_hash_labels_lineblocks_partition_and_zero_credit(self):
        import re
        data = self.artifact()
        counts = self.verify(data)
        self.assertEqual([counts[k] for k in ('selected_rule_rows', 'unselected_rule_identities',
                                             'global_rule_identities')], [6, 2959, 2965])
        raw = (self.source / self.PAGE).read_bytes()
        self.assertEqual(digest(raw), self.PAGE_HASH)
        lines = raw.splitlines(keepends=True)
        self.assertEqual(len(lines), 42)
        markers = [(i, m.group(1).decode()) for i, line in enumerate(lines, 1)
                   if (m := re.fullmatch(rb'r\[([^\]\r\n]+)\]\r?\n?', line))]
        self.assertEqual(markers, list(zip(self.STARTS, self.LABELS)))
        self.assertEqual(tuple(r['rule'] for r in data['rows']), self.LABELS)
        self.assertEqual([(p['path'], p['sha256']) for p in data['selected_pages']],
                         [(self.PAGE, self.PAGE_HASH)])
        for i, row in enumerate(data['rows']):
            end = self.STARTS[i + 1] - 1 if i + 1 < 6 else 42
            with self.subTest(rule=row['rule']):
                self.assertEqual((row['page'], row['line'], row['end_line']),
                                 (self.PAGE, self.STARTS[i], end))
                self.assertEqual(digest(b''.join(lines[self.STARTS[i] - 1:end])), self.BLOCK_HASHES[i])
                self.assertEqual(row['source_block_sha256'], self.BLOCK_HASHES[i])
                prefix = 'reconcile-path-expressions-'
                self.assertEqual(row['id'], prefix + self.LABELS[i].encode().hex())
                self.assertEqual(bytes.fromhex(row['id'][len(prefix):]).decode(), self.LABELS[i])
                self.assertEqual(row['review_status'], 'unreviewed')
                self.assertEqual(len(row['missing_test_needs']), 4)
                for need, kind in zip(row['missing_test_needs'], ('positive', 'invalid', 'interaction', 'end-to-end')):
                    self.assertTrue(need.startswith(kind + ': Missing '))
                self.assertIn('stable1.98.1 eligibility classification', row['missing_test_needs'][-1])
        self.assertEqual(len({r['id'] for r in data['rows']}), 6)
        for k in ('reviewed_rule_rows', 'rows_with_exact_executable_evidence'):
            self.assertEqual(counts[k], 0)
        index = json.loads(self.index.read_bytes())
        universe = {(p['path'], r['label']) for p in index['pages'] + index['unlinked_markdown_files']
                    for r in p['rules']}
        selected = {(r['page'], r['rule']) for r in data['rows']}
        other = {(r['page'], r['rule']) for r in data['unselected_rules']}
        self.assertEqual((len(selected), len(other), len(universe)), (6, 2959, 2965))
        self.assertEqual(len(data['unselected_rules']), 2959)
        self.assertFalse(selected & other)
        self.assertEqual(selected | other, universe)
        self.assertEqual(self.artifact(), data)

    def test_permuted_authored_rows_preserve_canonical_order(self):
        import path_expressions_reconciliation_proposals as proposals
        baseline = self.artifact()
        rows = proposals.ROWS.strip().splitlines()
        for changed in (rows[::-1], rows[2:] + rows[:2], rows[::2] + rows[1::2]):
            with mock.patch.object(proposals, 'ROWS', '\n'.join(changed)):
                data = self.artifact()
                self.assertEqual(data, baseline)
                self.assertEqual(tuple(r['rule'] for r in data['rows']), self.LABELS)
                self.assertEqual(self.verify(data), self.verify(baseline))

    def test_missing_duplicate_foreign_malformed_authored_rows_rejected(self):
        import path_expressions_reconciliation_proposals as proposals
        valid = self.artifact()
        rows = proposals.ROWS.strip().splitlines()
        self.assertEqual(tuple(r.split('|')[0] for r in rows), self.LABELS)
        cases = [rows[5:], rows[:5], rows[:-1], [], rows + [rows[0]],
                 ['expr.foreign' + rows[0][len('expr.path'):], *rows[1:]],
                 ['|'.join(rows[0].split('|')[:-1]), *rows[1:]],
                 [rows[0] + '|extra', *rows[1:]]]
        for col in (4, 5, 6):
            fields = rows[0].split('|')
            fields[col] = '   '
            cases.append(['|'.join(fields), *rows[1:]])
        for i, changed in enumerate(cases):
            with self.subTest(case=i), mock.patch.object(proposals, 'ROWS', '\n'.join(changed)):
                with self.assertRaises(ValueError):
                    self.artifact()
                with self.assertRaises(ValueError):
                    self.verify(valid)

    def test_29_actual_changing_forgeries_rejected(self):
        baseline = self.artifact()
        mutations = (
            lambda d: d['rows'].pop(),
            lambda d: d['rows'].append(copy.deepcopy(d['rows'][0])),
            lambda d: d['rows'].reverse(),
            lambda d: d['rows'][0].update(page='src/expressions/block-expr.md'),
            lambda d: d['rows'][5].update(page='src/expressions/struct-expr.md'),
            lambda d: d['rows'][0].update(rule='expr.loop'),
            lambda d: d['rows'][0].update(line=True),
            lambda d: d['rows'][0].update(line=1.0),
            lambda d: d['counts'].update(reviewed_rule_rows=False),
            lambda d: d['rows'][0].update(relationship='exact-executable-evidence'),
            lambda d: d['rows'][0].update(classifications=['grammar']),
            lambda d: d['rows'][0].update(end_line=99999),
            lambda d: d['rows'][0].update(source_block_sha256='0' * 64),
            lambda d: d['rows'][4].update(source_block_sha256='0' * 64),
            lambda d: d['rows'][0].update(id='reconcile-path-expressions-expr'),
            lambda d: d['rows'][0].update(id=d['rows'][1]['id']),
            lambda d: d['rows'][0].update(review_status='reviewed'),
            lambda d: d['rows'][0].update(rationale='invented interpretation'),
            lambda d: d['rows'][0].update(candidate_obligation_ids=['foreign']),
            lambda d: d['rows'][0].update(missing_test_needs=[]),
            lambda d: d['rows'][0]['missing_test_needs'].__setitem__(0, 'positive: implemented'),
            lambda d: d['counts'].update(reviewed_rule_rows=1),
            lambda d: d['counts'].update(rows_with_exact_executable_evidence=1),
            lambda d: d['selected_pages'][0].update(sha256='0' * 64),
            lambda d: d['selected_pages'].clear(),
            lambda d: d['bindings'].update(reference_commit='0' * 40),
            lambda d: d['unselected_rules'].pop(),
            lambda d: d['unselected_rules'].append(copy.deepcopy(d['unselected_rules'][0])),
            lambda d: d['unselected_rules'][0].update(page=d['rows'][0]['page'], rule=d['rows'][0]['rule']),
        )
        self.assertEqual(len(mutations), 29)
        for i, mutate in enumerate(mutations):
            changed = copy.deepcopy(baseline)
            mutate(changed)
            self.assertNotEqual(json.dumps(changed, sort_keys=True), json.dumps(baseline, sort_keys=True))
            with self.subTest(mutation=i), self.assertRaises(ValueError):
                self.verify(changed)

    def test_source_faults_preserve_exact_error_types(self):
        data = self.artifact()
        for state, error in (('stale', ValueError), ('missing', FileNotFoundError),
                             ('directory', IsADirectoryError), ('symlink', ValueError)):
            with self.subTest(state=state), tempfile.TemporaryDirectory() as temp:
                root = Path(temp)
                target = root / self.PAGE
                target.parent.mkdir(parents=True)
                if state == 'stale':
                    target.write_bytes((self.source / self.PAGE).read_bytes() + b'\n')
                elif state == 'directory':
                    target.mkdir()
                elif state == 'symlink':
                    target.symlink_to(self.source / self.PAGE)
                for action in (lambda: self.artifact(root), lambda: self.verify(data, root)):
                    with self.assertRaises(error) as caught:
                        action()
                    self.assertIs(type(caught.exception), error)

    def test_source_only_restrictions_precede_io_and_execution(self):
        args = ['--profile', 'path-expressions', '--rule-index', '/absent/index',
                '--inventory', '/absent/inventory', '--source-root', '/absent/source',
                '--artifact', '/absent/artifact']
        extras = ([], ['--repo-root', '/absent/repo'], ['--evidence', '/absent/evidence'],
                  ['--repo-root', '/absent/repo', '--evidence', '/absent/evidence'])
        cases = [(action, extra) for action in ('generate-witness-links', 'build-witness-links',
                  'validate-witness-links', 'assess-witness-links') for extra in extras]
        cases += [(action, extra) for action in ('generate', 'validate') for extra in extras[1:]]
        with contextlib.ExitStack() as stack:
            for name in ('read_bytes', 'read_text', 'open', 'is_symlink', 'is_file',
                         'write_bytes', 'write_text', 'mkdir'):
                stack.enter_context(mock.patch.object(Path, name, side_effect=AssertionError('IO')))
            stack.enter_context(mock.patch.object(subprocess, 'run', side_effect=AssertionError('execution')))
            for action, extra in cases:
                with self.subTest(action=action, extra=extra), self.assertRaises(ValueError):
                    main([action, *args, *extra])
            for call in (
                lambda: reconciliation.build_witness_links(
                    self.index, self.inventory, self.source, self.source, 'path-expressions'),
                lambda: reconciliation.validate_witness_links(
                    {}, self.index, self.inventory, self.source, self.source, 'path-expressions'),
                lambda: reconciliation.assess_witness_links(
                    {}, self.index, self.inventory, self.source, self.source, (), 'path-expressions'),
            ):
                with self.assertRaises(ValueError):
                    call()

    def test_cli_exclusive_generation_validation_and_no_repair(self):
        with tempfile.TemporaryDirectory() as temp:
            artifact = Path(temp) / 'path-expressions.json'
            args = ['--profile', 'path-expressions', '--rule-index', str(self.index),
                    '--inventory', str(self.inventory), '--source-root', str(self.source),
                    '--artifact', str(artifact)]
            expected = (json.dumps(self.artifact(), indent=2) + '\n').encode()
            with contextlib.redirect_stdout(io.StringIO()) as output:
                main(['generate', *args])
            self.assertEqual(artifact.read_bytes(), expected)
            self.assertEqual(json.loads(output.getvalue()), self.verify(self.artifact()))
            with contextlib.redirect_stdout(io.StringIO()), self.assertRaises(FileExistsError):
                main(['generate', *args])
            self.assertEqual(artifact.read_bytes(), expected)
            with contextlib.redirect_stdout(io.StringIO()) as output:
                main(['validate', *args])
            self.assertEqual(json.loads(output.getvalue()), self.verify(self.artifact()))
            self.assertEqual(artifact.read_bytes(), expected)
            forged = json.loads(expected)
            forged['counts']['reviewed_rule_rows'] = 1
            artifact.write_text(json.dumps(forged))
            before = artifact.read_bytes()
            with contextlib.redirect_stdout(io.StringIO()), self.assertRaises(ValueError):
                main(['validate', *args])
            self.assertEqual(artifact.read_bytes(), before)

    def test_last_prior_expression_core_canonical_bytes_without_new_module(self):
        canonical = lambda data: (json.dumps(data, indent=2) + '\n').encode()
        expected = canonical(build(self.index, self.inventory, self.source, 'expression-core'))
        with mock.patch.dict(sys.modules, {'path_expressions_reconciliation_proposals': None}):
            data = build(self.index, self.inventory, self.source, 'expression-core')
            validate(data, self.index, self.inventory, self.source, 'expression-core')
        self.assertEqual(canonical(data), expected)
        self.assertEqual(digest(canonical(data)),
                         'c97e514ad4cf1babc0dc6901c432dfac71f98522fcbfbeafde1b920056f5f18a')


class NeverTypeSourceReconciliationTests(unittest.TestCase):
    """Independent pinned-source structural checks, not semantic execution credit."""
    PAGE = 'src/types/never.md'
    PAGE_HASH = 'dcd1e1976fe36b8b503e7d2c900a68e48badd4103778954143de5f36e7a7ae2b'
    LABELS = ('type.never', 'type.never.syntax', 'type.never.intro',
              'type.never.coercion', 'type.never.constraint')
    STARTS = (1, 4, 9, 12, 15)
    BLOCK_HASHES = (
        'a0d5ad93d8875c1b40dbb2fff6ca1660bfe8a1d66b7a23655d4fe40054b4cbc7',
        'df1bd04d27f8861e054899464e756780c4c50315300e7aad0db72068b20b1492',
        '46423fad458d8a218f5f898a88759411ff8122292f3580f885cdc50fb7d3ab56',
        'c9bb7ed7947347d132dad164c880325ec6509e9259364b479f16f26bb9772058',
        'cd1953893f6efa05f4cbd24f1d841a918babc2a4f5d5dc4f1006769c69a2cb9b',
    )

    def setUp(self):
        source = os.environ.get('JUSTRUST_WITNESS_REFERENCE_ROOT')
        if not source:
            self.skipTest('set JUSTRUST_WITNESS_REFERENCE_ROOT for pinned-source structural checks')
        self.source = Path(source)
        repo = Path(os.environ.get('JUSTRUST_WITNESS_REPO_ROOT', BASE.parent))
        self.index = repo / 'data/compiler-language-reference-rules.json'
        self.inventory = repo / 'data/compiler-language-inventory.json'

    def artifact(self, source=None):
        return build(self.index, self.inventory,
                     self.source if source is None else source, 'never-type')

    def verify(self, data, source=None):
        return validate(data, self.index, self.inventory,
                        self.source if source is None else source, 'never-type')

    def test_actual_aliases_pins_no_overlap_and_integrated_totals(self):
        import never_type_reconciliation_proposals as proposals
        self.assertEqual(proposals.SELECTED, (self.PAGE,))
        self.assertEqual(proposals.PAGE_SHA256, {self.PAGE: self.PAGE_HASH})
        self.assertEqual(proposals.SOURCE_COMMIT, '86635e30bf861a038dc197d7e16fd09e7e514e7a')
        self.assertEqual(proposals.ALIASES, {'never': 'reference-types-never-never-stable'})
        obligations = {o['id'] for s in json.loads(self.inventory.read_bytes())['sections']
                       for o in s['obligations']}
        self.assertTrue(set(proposals.ALIASES.values()) <= obligations)
        index = json.loads(self.index.read_bytes())
        prior_pages = {p for name, _ in reconciliation.PROPOSAL_COUNTS if name not in ('trait-objects', 'type-parameters', 'nominal-types', 'inferred-types', 'never-type', 'coercions', 'subtyping', 'type-expressions', 'primitive-types', 'sequence-types', 'pointer-types', 'function-pointers', 'function-items')
                       for p in reconciliation.profile_inputs(name)[0]}
        prior = {(p['path'], r['label']) for p in index['pages'] + index['unlinked_markdown_files']
                 if p['path'] in prior_pages for r in p['rules']}
        current = {(r['page'], r['rule']) for r in self.artifact()['rows']}
        self.assertFalse(prior & current)
        self.assertEqual((len(prior), len(current), len(prior | current)), (815, 5, 820))
        summary = reconciliation.summarize_proposals(self.index, self.inventory, include_proposals=True)
        self.assertEqual((summary['proposed_rule_identities'],
                          summary['rule_identities_without_proposals']), (1030, 1935))
        self.assertEqual(sum(p['proposed_rule_identities'] for p in summary['profiles'].values()), 1035)
        self.assertEqual(summary['profiles']['never-type']['completed_rule_rows'], 0)

    def test_hardcoded_hash_labels_lineblocks_partition_and_zero_credit(self):
        import re
        data = self.artifact()
        counts = self.verify(data)
        self.assertEqual([counts[k] for k in ('selected_rule_rows', 'unselected_rule_identities',
                                             'global_rule_identities')], [5, 2960, 2965])
        raw = (self.source / self.PAGE).read_bytes()
        self.assertEqual(digest(raw), self.PAGE_HASH)
        lines = raw.splitlines(keepends=True)
        self.assertEqual(len(lines), 28)
        markers = [(i, m.group(1).decode()) for i, line in enumerate(lines, 1)
                   if (m := re.fullmatch(rb'r\[([^\]\r\n]+)\]\r?\n?', line))]
        self.assertEqual(markers, list(zip(self.STARTS, self.LABELS)))
        self.assertEqual(tuple(r['rule'] for r in data['rows']), self.LABELS)
        self.assertEqual([(p['path'], p['sha256']) for p in data['selected_pages']],
                         [(self.PAGE, self.PAGE_HASH)])
        for i, row in enumerate(data['rows']):
            end = self.STARTS[i + 1] - 1 if i + 1 < 5 else 28
            with self.subTest(rule=row['rule']):
                self.assertEqual((row['page'], row['line'], row['end_line']),
                                 (self.PAGE, self.STARTS[i], end))
                self.assertEqual(digest(b''.join(lines[self.STARTS[i] - 1:end])), self.BLOCK_HASHES[i])
                self.assertEqual(row['source_block_sha256'], self.BLOCK_HASHES[i])
                prefix = 'reconcile-never-type-'
                self.assertEqual(row['id'], prefix + self.LABELS[i].encode().hex())
                self.assertEqual(bytes.fromhex(row['id'][len(prefix):]).decode(), self.LABELS[i])
                self.assertEqual(row['review_status'], 'unreviewed')
                self.assertEqual(len(row['missing_test_needs']), 4)
                for need, kind in zip(row['missing_test_needs'], ('positive', 'invalid', 'interaction', 'end-to-end')):
                    self.assertTrue(need.startswith(kind + ': Missing '))
                self.assertIn('stable1.98.1 eligibility classification', row['missing_test_needs'][-1])
        self.assertEqual(len({r['id'] for r in data['rows']}), 5)
        for k in ('reviewed_rule_rows', 'rows_with_exact_executable_evidence'):
            self.assertEqual(counts[k], 0)
        index = json.loads(self.index.read_bytes())
        universe = {(p['path'], r['label']) for p in index['pages'] + index['unlinked_markdown_files']
                    for r in p['rules']}
        selected = {(r['page'], r['rule']) for r in data['rows']}
        other = {(r['page'], r['rule']) for r in data['unselected_rules']}
        self.assertEqual((len(selected), len(other), len(universe)), (5, 2960, 2965))
        self.assertEqual(len(data['unselected_rules']), 2960)
        self.assertFalse(selected & other)
        self.assertEqual(selected | other, universe)
        self.assertEqual(self.artifact(), data)

    def test_permuted_authored_rows_preserve_canonical_order(self):
        import never_type_reconciliation_proposals as proposals
        baseline = self.artifact()
        rows = proposals.ROWS.strip().splitlines()
        for changed in (rows[::-1], rows[2:] + rows[:2], rows[::2] + rows[1::2]):
            with mock.patch.object(proposals, 'ROWS', '\n'.join(changed)):
                data = self.artifact()
                self.assertEqual(data, baseline)
                self.assertEqual(tuple(r['rule'] for r in data['rows']), self.LABELS)
                self.assertEqual(self.verify(data), self.verify(baseline))

    def test_missing_duplicate_foreign_malformed_authored_rows_rejected(self):
        import never_type_reconciliation_proposals as proposals
        valid = self.artifact()
        rows = proposals.ROWS.strip().splitlines()
        self.assertEqual(tuple(r.split('|')[0] for r in rows), self.LABELS)
        cases = [rows[4:], rows[:4], rows[:-1], [], rows + [rows[0]],
                 ['expr.foreign' + rows[0][len('type.never'):], *rows[1:]],
                 ['|'.join(rows[0].split('|')[:-1]), *rows[1:]],
                 [rows[0] + '|extra', *rows[1:]]]
        for col in (4, 5, 6):
            fields = rows[0].split('|')
            fields[col] = '   '
            cases.append(['|'.join(fields), *rows[1:]])
        for i, changed in enumerate(cases):
            with self.subTest(case=i), mock.patch.object(proposals, 'ROWS', '\n'.join(changed)):
                with self.assertRaises(ValueError):
                    self.artifact()
                with self.assertRaises(ValueError):
                    self.verify(valid)

    def test_29_actual_changing_forgeries_rejected(self):
        baseline = self.artifact()
        mutations = (
            lambda d: d['rows'].pop(),
            lambda d: d['rows'].append(copy.deepcopy(d['rows'][0])),
            lambda d: d['rows'].reverse(),
            lambda d: d['rows'][0].update(page='src/expressions/block-expr.md'),
            lambda d: d['rows'][4].update(page='src/expressions/struct-expr.md'),
            lambda d: d['rows'][0].update(rule='expr.loop'),
            lambda d: d['rows'][0].update(line=True),
            lambda d: d['rows'][0].update(line=1.0),
            lambda d: d['counts'].update(reviewed_rule_rows=False),
            lambda d: d['rows'][0].update(relationship='exact-executable-evidence'),
            lambda d: d['rows'][0].update(classifications=['grammar']),
            lambda d: d['rows'][0].update(end_line=99999),
            lambda d: d['rows'][0].update(source_block_sha256='0' * 64),
            lambda d: d['rows'][4].update(source_block_sha256='0' * 64),
            lambda d: d['rows'][0].update(id='reconcile-never-type-expr'),
            lambda d: d['rows'][0].update(id=d['rows'][1]['id']),
            lambda d: d['rows'][0].update(review_status='reviewed'),
            lambda d: d['rows'][0].update(rationale='invented interpretation'),
            lambda d: d['rows'][0].update(candidate_obligation_ids=['foreign']),
            lambda d: d['rows'][0].update(missing_test_needs=[]),
            lambda d: d['rows'][0]['missing_test_needs'].__setitem__(0, 'positive: implemented'),
            lambda d: d['counts'].update(reviewed_rule_rows=1),
            lambda d: d['counts'].update(rows_with_exact_executable_evidence=1),
            lambda d: d['selected_pages'][0].update(sha256='0' * 64),
            lambda d: d['selected_pages'].clear(),
            lambda d: d['bindings'].update(reference_commit='0' * 40),
            lambda d: d['unselected_rules'].pop(),
            lambda d: d['unselected_rules'].append(copy.deepcopy(d['unselected_rules'][0])),
            lambda d: d['unselected_rules'][0].update(page=d['rows'][0]['page'], rule=d['rows'][0]['rule']),
        )
        self.assertEqual(len(mutations), 29)
        for i, mutate in enumerate(mutations):
            changed = copy.deepcopy(baseline)
            mutate(changed)
            self.assertNotEqual(json.dumps(changed, sort_keys=True), json.dumps(baseline, sort_keys=True))
            with self.subTest(mutation=i), self.assertRaises(ValueError):
                self.verify(changed)

    def test_source_faults_preserve_exact_error_types(self):
        data = self.artifact()
        for state, error in (('stale', ValueError), ('missing', FileNotFoundError),
                             ('directory', IsADirectoryError), ('symlink', ValueError)):
            with self.subTest(state=state), tempfile.TemporaryDirectory() as temp:
                root = Path(temp)
                target = root / self.PAGE
                target.parent.mkdir(parents=True)
                if state == 'stale':
                    target.write_bytes((self.source / self.PAGE).read_bytes() + b'\n')
                elif state == 'directory':
                    target.mkdir()
                elif state == 'symlink':
                    target.symlink_to(self.source / self.PAGE)
                for action in (lambda: self.artifact(root), lambda: self.verify(data, root)):
                    with self.assertRaises(error) as caught:
                        action()
                    self.assertIs(type(caught.exception), error)

    def test_source_only_restrictions_precede_io_and_execution(self):
        args = ['--profile', 'never-type', '--rule-index', '/absent/index',
                '--inventory', '/absent/inventory', '--source-root', '/absent/source',
                '--artifact', '/absent/artifact']
        extras = ([], ['--repo-root', '/absent/repo'], ['--evidence', '/absent/evidence'],
                  ['--repo-root', '/absent/repo', '--evidence', '/absent/evidence'])
        cases = [(action, extra) for action in ('generate-witness-links', 'build-witness-links',
                  'validate-witness-links', 'assess-witness-links') for extra in extras]
        cases += [(action, extra) for action in ('generate', 'validate') for extra in extras[1:]]
        with contextlib.ExitStack() as stack:
            for name in ('read_bytes', 'read_text', 'open', 'is_symlink', 'is_file',
                         'write_bytes', 'write_text', 'mkdir'):
                stack.enter_context(mock.patch.object(Path, name, side_effect=AssertionError('IO')))
            stack.enter_context(mock.patch.object(subprocess, 'run', side_effect=AssertionError('execution')))
            for action, extra in cases:
                with self.subTest(action=action, extra=extra), self.assertRaises(ValueError):
                    main([action, *args, *extra])
            for call in (
                lambda: reconciliation.build_witness_links(
                    self.index, self.inventory, self.source, self.source, 'never-type'),
                lambda: reconciliation.validate_witness_links(
                    {}, self.index, self.inventory, self.source, self.source, 'never-type'),
                lambda: reconciliation.assess_witness_links(
                    {}, self.index, self.inventory, self.source, self.source, (), 'never-type'),
            ):
                with self.assertRaises(ValueError):
                    call()

    def test_cli_exclusive_generation_validation_and_no_repair(self):
        with tempfile.TemporaryDirectory() as temp:
            artifact = Path(temp) / 'never-type.json'
            args = ['--profile', 'never-type', '--rule-index', str(self.index),
                    '--inventory', str(self.inventory), '--source-root', str(self.source),
                    '--artifact', str(artifact)]
            expected = (json.dumps(self.artifact(), indent=2) + '\n').encode()
            with contextlib.redirect_stdout(io.StringIO()) as output:
                main(['generate', *args])
            self.assertEqual(artifact.read_bytes(), expected)
            self.assertEqual(json.loads(output.getvalue()), self.verify(self.artifact()))
            with contextlib.redirect_stdout(io.StringIO()), self.assertRaises(FileExistsError):
                main(['generate', *args])
            self.assertEqual(artifact.read_bytes(), expected)
            with contextlib.redirect_stdout(io.StringIO()) as output:
                main(['validate', *args])
            self.assertEqual(json.loads(output.getvalue()), self.verify(self.artifact()))
            self.assertEqual(artifact.read_bytes(), expected)
            forged = json.loads(expected)
            forged['counts']['reviewed_rule_rows'] = 1
            artifact.write_text(json.dumps(forged))
            before = artifact.read_bytes()
            with contextlib.redirect_stdout(io.StringIO()), self.assertRaises(ValueError):
                main(['validate', *args])
            self.assertEqual(artifact.read_bytes(), before)


    def test_all_21_prior_canonical_bytes_without_new_module(self):
        # Captured by generation against exact main 8d32b52df568273aa4e48e41c0798371d93c409e.
        expected = {
            'tokens': 'ea7fa27afdf901b91464ead14c54883debd091819c25bf02870e7a434ae61785',
            'const-eval': '0ce718c6e2834b6c518ff43f2fed2b5de72fd75eccf01d4611c3217cbdf053c2',
            'unsafe': 'bc752efc1f0172cd85dad9399a1309c06489b3b0e4d4f74884c8bc1ef58ebb58',
            'abi': 'c4e561a9a78002281c34cbc35fe7fcaee3f29e82c659b718844504514ac1d317',
            'visibility': '49883324b912c7f107d3573d6b95a3b4cc52b3d97253653eb39dfee6af9cbd58',
            'branching': 'b08afed11ff9bd56c0b15f31bbaa3109105181bde417f0565ff1082cf340e74a',
            'destructors': 'af8e7a87b177e4c72efd7f0dddfe26c2a0390d710377a507f9765f3d1b9d188e',
            'loops': '4a77bd5239c74bde49e8ba7933248484acd773c4f08239ce6f1fce23efb2261a',
            'operators': '2d724517022229ad541ef7c2a66aafc414b13f56c68d3cb8ea633755d3996ba3',
            'closures': 'b75b0495a40e114b0e698ac0e64a84a195ed0a1745f14a2b09f6b5ee043b4b64',
            'calls': '42efdae31602a2af4f2fc243d5a74404159f877b278820905a7b4086281d302e',
            'aggregates': '8c83d418848de65f5251d3b23248dd09121d7ac82b1099739648de6a034a55b3',
            'fields': 'e53fbee9b1248907fb91f9524440cfa75e5991fa0146800c6f0be3f2c67541b2',
            'literals': '159e5a9f425ac09e2b1b4ae96a8c8710264f5ebc3170ec97dfc48e1d947808d8',
            'grouped': '5a9393145150885b8c9f7717e99000003751454bd0a322ed7152e24e4289e279',
            'transfers': '5f29782852c84ccf19cd0fde0ef1c4d9f54a6813eda59d1d9f1686bd387b123d',
            'ranges': '3017078dfc42f85b5e51849d80e1a7efc41a247bbb573f09a7c40b6c5826fdda',
            'structs': '5fb87b17efaa5ac3d34840f9735b2499760c748ba7d9a5fa6a8909f4321f621e',
            'blocks': '44247bc80441734de02b723d046fd34d801801ee2fda6c7d50fcd0154c5e3a90',
            'expression-core': 'c97e514ad4cf1babc0dc6901c432dfac71f98522fcbfbeafde1b920056f5f18a',
            'path-expressions': '5e4dbc27da9288eab71e26816f052348b13dbedcc703244cafa4fa64fb6f993f',
        }
        self.assertEqual(len(expected), 21)
        self.assertEqual(set(expected), {name for name, _ in reconciliation.PROPOSAL_COUNTS
                                         if name not in ('trait-objects', 'type-parameters', 'nominal-types', 'inferred-types', 'never-type', 'coercions', 'subtyping', 'type-expressions', 'primitive-types', 'sequence-types', 'pointer-types', 'function-pointers', 'function-items')})
        with mock.patch.dict(sys.modules, {'never_type_reconciliation_proposals': None}):
            for profile, sha in expected.items():
                with self.subTest(profile=profile):
                    data = build(self.index, self.inventory, self.source, profile)
                    validate(data, self.index, self.inventory, self.source, profile)
                    self.assertEqual(digest((json.dumps(data, indent=2) + '\n').encode()), sha)
            default = build(self.index, self.inventory, self.source)
            self.assertEqual(digest((json.dumps(default, indent=2) + '\n').encode()), expected['tokens'])


class CoercionsSourceReconciliationTests(unittest.TestCase):
    """Independent pinned-source structural checks, not semantic execution credit."""
    PAGE = 'src/type-coercions.md'
    PAGE_HASH = '75d07f1492ec3af3011b25aee419aac4583ce17eb91b58d42cefe5e1e01b96f1'
    LABELS = ('coerce',
     'coerce.intro',
     'coerce.as',
     'coerce.site',
     'coerce.site.intro',
     'coerce.site.let',
     'coerce.site.value',
     'coerce.site.argument',
     'coerce.site.constructor',
     'coerce.site.return',
     'coerce.site.assignment',
     'coerce.site.subexpr',
     'coerce.site.array',
     'coerce.site.repeat',
     'coerce.site.tuple',
     'coerce.site.parenthesis',
     'coerce.site.block',
     'coerce.types',
     'coerce.types.intro',
     'coerce.types.reflexive',
     'coerce.types.transitive',
     'coerce.types.mut-reborrow',
     'coerce.types.mut-pointer',
     'coerce.types.ref-to-pointer',
     'coerce.types.mut-to-pointer',
     'coerce.types.deref',
     'coerce.types.deref-mut',
     'coerce.types.unsize',
     'coerce.types.fn',
     'coerce.types.closure',
     'coerce.types.never',
     'coerce.unsize',
     'coerce.unsize.intro',
     'coerce.unsize.trait',
     'coerce.unsize.slice',
     'coerce.unsize.trait-object',
     'coerce.unsize.trait-upcast',
     'coerce.unsized.composite',
     'coerce.unsized.pointer',
     'coerce.least-upper-bound',
     'coerce.least-upper-bound.intro',
     'coerce.least-upper-bound.target',
     'coerce.least-upper-bound.computation',
     'coerce.least-upper-bound.computation-identity',
     'coerce.least-upper-bound.computation-replace',
     'coerce.least-upper-bound.computation-unify')
    STARTS = (1,
     4,
     7,
     12,
     15,
     18,
     27,
     30,
     47,
     60,
     72,
     82,
     85,
     88,
     91,
     94,
     97,
     100,
     103,
     106,
     109,
     114,
     117,
     120,
     123,
     126,
     152,
     155,
     169,
     172,
     175,
     178,
     181,
     184,
     187,
     190,
     193,
     198,
     206,
     212,
     215,
     226,
     229,
     232,
     235,
     238)
    BLOCK_HASHES = ('66d2c076b81c1bc0eaaf7bdaaedde780cf5a96a3b56033dfd345bbd5f912a389',
     '72dd2b9403dde6ca92c8dd37c4d5aab902fa2c57f615efefafd5ca3e7bd37a5e',
     '12af5d8f870fc6a295150883743c4830d0eb9f57d531f2db540df0eb518411fb',
     '4425a932d48e328d7287d3e15bb04b468aadaac6eca3aad456fb6c2639747d7c',
     'aab6e9dbcb053a899dd8fa26752a9da54e464f17538a6ea66a755c076f5e522b',
     '50b502bc78433a762ab8d8006f6bca6b934a4f73bfa7560e1c60a56237c9f30f',
     'b9e0ae5001fa97e7fcdac7eb542c50a88b1e04856311b76cba71765b5f1fe426',
     'ca9d6c2a0c5c32e2fc21de2dc9d69b33a03397f09fdaf9f718e3e6bdaab0b8d6',
     '0cd1951bf6192893ae9283b538a36c6bb1c36a9b06d6c4d1cba5de25a7c35cfd',
     '31f98d8ccef1890eefcaaa88ea7da2b5897b2b9f0b2a88ba56379bed8f93810c',
     '6f7e03916f6aeecbabb6d6e1f2a4619ed0c5782083390befd02d85e2cb7dd71f',
     'f9d494e1729285cb11625927394f109b65dfaf3dbf0cbcb8b769192ddac2961c',
     'c6e03a2abd361cf79faa9ef7ed8ccd0a75b7b589dab203a9ca8c132593616bb0',
     '3226baf4f1158048b4a66f5c3db8f6786cd4fafb6859beafe904c4005af17028',
     'ab2be7baa70a0b82fc98645f2e146640194fefc47dbf64422883e772b0efa2b5',
     '346fbc9b6fed1def8285bf6266307a0f1638869109ca85d8c9dcc824064b8c38',
     '725d9b8c8e8936a46eadbce831ef8a0cf7bc33372f7e76963b54791b9633ab5c',
     '3b6c642cf2a170843a0233e0be5e29e42f4e9951a1ce1a1e6a3356abd49fe23b',
     '88f8522999e7098eb1a11b3b3b4f7d507c71eca629bc95524b954833a94b2b77',
     '8d6d245792c04ae7b52fc734cda01903a60e6619bce29e3f2e30c135d413e130',
     'f6de4266193be60fa5528fdc180d9241f1b3fd8faae561b5ea2cf0e31f6a8529',
     '04c34f91a5e24b61f9d9675384827ef13d5110bf033b8a9a57b6188f20b68352',
     'b906b985fcdc4c6bf4ac16526909310b29833abdb644ef499c6c7259ccfd502e',
     '1c9761468722e0a873505324d34869d80c39fef7e8764c0826aba3920807881c',
     '15375ad95c9a39c87e5c428f17e31242aac30dcc6afb36f30c081e6f54482c91',
     '11a740e0409f65e3e0276d884a34bc73beb8cd405bf2ab584e1219b256d233db',
     '31209232af89cf66443d4663904715c19f9c30cf9fa58eabef5cc0277925dc0d',
     '81ee0fc44f3caf9561afa5a2c034b1a5b987c55446216c7454ffd7f8100bb7b6',
     '77967cf36fae25e09dfbef1bd5c3c8dd5e2120d53a99cb95e9df65510262874a',
     '882c45ac5662b6b96a2cd39689bfacf749c37035ed81a85288abbfcb4a125df8',
     '5ecb8303f8e08a727f7a5121086f629140f644b10fa96fc14070f9b9af57cc6d',
     '7bbdb95dffbcc4c3556506245cbb7f0c6069d5637f91b2297686fc8e189cdfbc',
     'ce7a27af111db6379eb8d64557dd9432f0390bd52d98693fd6e32ee062f54d5d',
     'e8a1ece9c516948f71b671badf3d5cd548cba018c2431b86fb48020e51e3038f',
     '6ee1ada61e72e0d89bda83c6a221403654cda516c088f2b72da6002d22b947f3',
     '9aff5db917be819b48af4d3865f06cb7e9d942495fc9e952d9821e466db1ff32',
     '3ea36a3a616f9c85052948956788da9a9176414d9b094a0204549539ccf10270',
     '16dafc8f30c0307b179fe24b613824289f909d83e5db717f64f8a70478463c31',
     '080e2a11f37c217209eac93e6f83f5d32a5a7f3bfaa97bde22585787fb48677f',
     'b0dc0fa8b9d6985b884c5c29f3d97043cf4c8213f1e6f0bacd9e5ab81149cb78',
     'b5e4f6161c55d3463b6a7ab3b29f491e9227ef3b77129728007324929aa8d8d4',
     'deece581756b08dfe08b953e9c67058e0bd8ddd9177cb2f883296673e744ff3e',
     'de95a60b7ec753bc1726305863f5c5ca11029ede4e12a118968531677f844df9',
     '459d4d4d8fec86468de131504dd04db2e181f4e328c61ebf22f734db7624a115',
     'a148a43d1104f03ac28e97c2be0d90713200d46919d572f709ba04651023123e',
     '44e6b9e61ed75b713787c45397149ebad98d1d09af0059d83513067315765d93')

    def setUp(self):
        source = os.environ.get('JUSTRUST_WITNESS_REFERENCE_ROOT')
        if not source:
            self.skipTest('set JUSTRUST_WITNESS_REFERENCE_ROOT for pinned-source structural checks')
        self.source = Path(source)
        repo = Path(os.environ.get('JUSTRUST_WITNESS_REPO_ROOT', BASE.parent))
        self.index = repo / 'data/compiler-language-reference-rules.json'
        self.inventory = repo / 'data/compiler-language-inventory.json'

    def artifact(self, source=None):
        return build(self.index, self.inventory,
                     self.source if source is None else source, 'coercions')

    def verify(self, data, source=None):
        return validate(data, self.index, self.inventory,
                        self.source if source is None else source, 'coercions')

    def test_actual_aliases_pins_no_overlap_and_integrated_totals(self):
        import coercions_reconciliation_proposals as proposals
        self.assertEqual(proposals.SELECTED, (self.PAGE,))
        self.assertEqual(proposals.PAGE_SHA256, {self.PAGE: self.PAGE_HASH})
        self.assertEqual(proposals.SOURCE_COMMIT, '86635e30bf861a038dc197d7e16fd09e7e514e7a')
        self.assertEqual(proposals.ALIASES, {'sites': 'reference-type-coercions-coercion-sites'})
        obligations = {o['id'] for s in json.loads(self.inventory.read_bytes())['sections']
                       for o in s['obligations']}
        self.assertTrue(set(proposals.ALIASES.values()) <= obligations)
        index = json.loads(self.index.read_bytes())
        prior_pages = {p for name, _ in reconciliation.PROPOSAL_COUNTS if name not in ('trait-objects', 'type-parameters', 'nominal-types', 'inferred-types', 'coercions', 'subtyping', 'type-expressions', 'primitive-types', 'sequence-types', 'pointer-types', 'function-pointers', 'function-items')
                       for p in reconciliation.profile_inputs(name)[0]}
        prior = {(p['path'], r['label']) for p in index['pages'] + index['unlinked_markdown_files']
                 if p['path'] in prior_pages for r in p['rules']}
        current = {(r['page'], r['rule']) for r in self.artifact()['rows']}
        self.assertFalse(prior & current)
        self.assertEqual((len(prior), len(current), len(prior | current)), (820, 46, 866))
        summary = reconciliation.summarize_proposals(self.index, self.inventory, include_proposals=True)
        self.assertEqual((summary['proposed_rule_identities'],
                          summary['rule_identities_without_proposals']), (1030, 1935))
        self.assertEqual(sum(p['proposed_rule_identities'] for p in summary['profiles'].values()), 1035)
        self.assertEqual(summary['profiles']['coercions']['completed_rule_rows'], 0)

    def test_hardcoded_hash_labels_lineblocks_partition_and_zero_credit(self):
        import re
        data = self.artifact()
        counts = self.verify(data)
        self.assertEqual([counts[k] for k in ('selected_rule_rows', 'unselected_rule_identities',
                                             'global_rule_identities')], [46, 2919, 2965])
        raw = (self.source / self.PAGE).read_bytes()
        self.assertEqual(digest(raw), self.PAGE_HASH)
        lines = raw.splitlines(keepends=True)
        self.assertEqual(len(lines), 303)
        markers = [(i, m.group(1).decode()) for i, line in enumerate(lines, 1)
                   if (m := re.fullmatch(rb'r\[([^\]\r\n]+)\]\r?\n?', line))]
        self.assertEqual(markers, list(zip(self.STARTS, self.LABELS)))
        self.assertEqual(tuple(r['rule'] for r in data['rows']), self.LABELS)
        self.assertEqual([(p['path'], p['sha256']) for p in data['selected_pages']],
                         [(self.PAGE, self.PAGE_HASH)])
        for i, row in enumerate(data['rows']):
            end = self.STARTS[i + 1] - 1 if i + 1 < 46 else 303
            with self.subTest(rule=row['rule']):
                self.assertEqual((row['page'], row['line'], row['end_line']),
                                 (self.PAGE, self.STARTS[i], end))
                self.assertEqual(digest(b''.join(lines[self.STARTS[i] - 1:end])), self.BLOCK_HASHES[i])
                self.assertEqual(row['source_block_sha256'], self.BLOCK_HASHES[i])
                prefix = 'reconcile-coercions-'
                self.assertEqual(row['id'], prefix + self.LABELS[i].encode().hex())
                self.assertEqual(bytes.fromhex(row['id'][len(prefix):]).decode(), self.LABELS[i])
                self.assertEqual(row['review_status'], 'unreviewed')
                self.assertEqual(len(row['missing_test_needs']), 4)
                for need, kind in zip(row['missing_test_needs'], ('positive', 'invalid', 'interaction', 'end-to-end')):
                    self.assertTrue(need.startswith(kind + ': Missing '))
                self.assertIn('stable1.98.1 eligibility classification', row['missing_test_needs'][-1])
        self.assertEqual(len({r['id'] for r in data['rows']}), 46)
        for k in ('reviewed_rule_rows', 'rows_with_exact_executable_evidence'):
            self.assertEqual(counts[k], 0)
        index = json.loads(self.index.read_bytes())
        universe = {(p['path'], r['label']) for p in index['pages'] + index['unlinked_markdown_files']
                    for r in p['rules']}
        selected = {(r['page'], r['rule']) for r in data['rows']}
        other = {(r['page'], r['rule']) for r in data['unselected_rules']}
        self.assertEqual((len(selected), len(other), len(universe)), (46, 2919, 2965))
        self.assertEqual(len(data['unselected_rules']), 2919)
        self.assertFalse(selected & other)
        self.assertEqual(selected | other, universe)
        self.assertEqual(self.artifact(), data)

    def test_permuted_authored_rows_preserve_canonical_order(self):
        import coercions_reconciliation_proposals as proposals
        baseline = self.artifact()
        rows = proposals.ROWS.strip().splitlines()
        for changed in (rows[::-1], rows[2:] + rows[:2], rows[::2] + rows[1::2]):
            with mock.patch.object(proposals, 'ROWS', '\n'.join(changed)):
                data = self.artifact()
                self.assertEqual(data, baseline)
                self.assertEqual(tuple(r['rule'] for r in data['rows']), self.LABELS)
                self.assertEqual(self.verify(data), self.verify(baseline))

    def test_missing_duplicate_foreign_malformed_authored_rows_rejected(self):
        import coercions_reconciliation_proposals as proposals
        valid = self.artifact()
        rows = proposals.ROWS.strip().splitlines()
        self.assertEqual(tuple(r.split('|')[0] for r in rows), self.LABELS)
        cases = [rows[4:], rows[:4], rows[:-1], [], rows + [rows[0]],
                 ['expr.foreign' + rows[0][len('coerce'):], *rows[1:]],
                 ['|'.join(rows[0].split('|')[:-1]), *rows[1:]],
                 [rows[0] + '|extra', *rows[1:]]]
        cases.extend(rows[:i] + rows[i + 1:] for i in range(len(rows)))
        for col in (4, 5, 6):
            fields = rows[0].split('|')
            fields[col] = '   '
            cases.append(['|'.join(fields), *rows[1:]])
        for i, changed in enumerate(cases):
            with self.subTest(case=i), mock.patch.object(proposals, 'ROWS', '\n'.join(changed)):
                with self.assertRaises(ValueError):
                    self.artifact()
                with self.assertRaises(ValueError):
                    self.verify(valid)

    def test_29_actual_changing_forgeries_rejected(self):
        baseline = self.artifact()
        mutations = (
            lambda d: d['rows'].pop(),
            lambda d: d['rows'].append(copy.deepcopy(d['rows'][0])),
            lambda d: d['rows'].reverse(),
            lambda d: d['rows'][0].update(page='src/expressions/block-expr.md'),
            lambda d: d['rows'][4].update(page='src/expressions/struct-expr.md'),
            lambda d: d['rows'][0].update(rule='expr.loop'),
            lambda d: d['rows'][0].update(line=True),
            lambda d: d['rows'][0].update(line=1.0),
            lambda d: d['counts'].update(reviewed_rule_rows=False),
            lambda d: d['rows'][0].update(relationship='exact-executable-evidence'),
            lambda d: d['rows'][0].update(classifications=['grammar']),
            lambda d: d['rows'][0].update(end_line=99999),
            lambda d: d['rows'][0].update(source_block_sha256='0' * 64),
            lambda d: d['rows'][4].update(source_block_sha256='0' * 64),
            lambda d: d['rows'][0].update(id='reconcile-coercions-expr'),
            lambda d: d['rows'][0].update(id=d['rows'][1]['id']),
            lambda d: d['rows'][0].update(review_status='reviewed'),
            lambda d: d['rows'][0].update(rationale='invented interpretation'),
            lambda d: d['rows'][0].update(candidate_obligation_ids=['foreign']),
            lambda d: d['rows'][0].update(missing_test_needs=[]),
            lambda d: d['rows'][0]['missing_test_needs'].__setitem__(0, 'positive: implemented'),
            lambda d: d['counts'].update(reviewed_rule_rows=1),
            lambda d: d['counts'].update(rows_with_exact_executable_evidence=1),
            lambda d: d['selected_pages'][0].update(sha256='0' * 64),
            lambda d: d['selected_pages'].clear(),
            lambda d: d['bindings'].update(reference_commit='0' * 40),
            lambda d: d['unselected_rules'].pop(),
            lambda d: d['unselected_rules'].append(copy.deepcopy(d['unselected_rules'][0])),
            lambda d: d['unselected_rules'][0].update(page=d['rows'][0]['page'], rule=d['rows'][0]['rule']),
        )
        self.assertEqual(len(mutations), 29)
        for i, mutate in enumerate(mutations):
            changed = copy.deepcopy(baseline)
            mutate(changed)
            self.assertNotEqual(json.dumps(changed, sort_keys=True), json.dumps(baseline, sort_keys=True))
            with self.subTest(mutation=i), self.assertRaises(ValueError):
                self.verify(changed)

    def test_source_faults_preserve_exact_error_types(self):
        data = self.artifact()
        for state, error in (('stale', ValueError), ('missing', FileNotFoundError),
                             ('directory', IsADirectoryError), ('symlink', ValueError)):
            with self.subTest(state=state), tempfile.TemporaryDirectory() as temp:
                root = Path(temp)
                target = root / self.PAGE
                target.parent.mkdir(parents=True)
                if state == 'stale':
                    target.write_bytes((self.source / self.PAGE).read_bytes() + b'\n')
                elif state == 'directory':
                    target.mkdir()
                elif state == 'symlink':
                    target.symlink_to(self.source / self.PAGE)
                for action in (lambda: self.artifact(root), lambda: self.verify(data, root)):
                    with self.assertRaises(error) as caught:
                        action()
                    self.assertIs(type(caught.exception), error)

    def test_source_only_restrictions_precede_io_and_execution(self):
        args = ['--profile', 'coercions', '--rule-index', '/absent/index',
                '--inventory', '/absent/inventory', '--source-root', '/absent/source',
                '--artifact', '/absent/artifact']
        extras = ([], ['--repo-root', '/absent/repo'], ['--evidence', '/absent/evidence'],
                  ['--repo-root', '/absent/repo', '--evidence', '/absent/evidence'])
        cases = [(action, extra) for action in ('generate-witness-links', 'build-witness-links',
                  'validate-witness-links', 'assess-witness-links') for extra in extras]
        cases += [(action, extra) for action in ('generate', 'validate') for extra in extras[1:]]
        with contextlib.ExitStack() as stack:
            for name in ('read_bytes', 'read_text', 'open', 'is_symlink', 'is_file',
                         'write_bytes', 'write_text', 'mkdir'):
                stack.enter_context(mock.patch.object(Path, name, side_effect=AssertionError('IO')))
            stack.enter_context(mock.patch.object(subprocess, 'run', side_effect=AssertionError('execution')))
            for action, extra in cases:
                with self.subTest(action=action, extra=extra), self.assertRaises(ValueError):
                    main([action, *args, *extra])
            for call in (
                lambda: reconciliation.build_witness_links(
                    self.index, self.inventory, self.source, self.source, 'coercions'),
                lambda: reconciliation.validate_witness_links(
                    {}, self.index, self.inventory, self.source, self.source, 'coercions'),
                lambda: reconciliation.assess_witness_links(
                    {}, self.index, self.inventory, self.source, self.source, (), 'coercions'),
            ):
                with self.assertRaises(ValueError):
                    call()

    def test_cli_exclusive_generation_validation_and_no_repair(self):
        with tempfile.TemporaryDirectory() as temp:
            artifact = Path(temp) / 'coercions.json'
            args = ['--profile', 'coercions', '--rule-index', str(self.index),
                    '--inventory', str(self.inventory), '--source-root', str(self.source),
                    '--artifact', str(artifact)]
            expected = (json.dumps(self.artifact(), indent=2) + '\n').encode()
            with contextlib.redirect_stdout(io.StringIO()) as output:
                main(['generate', *args])
            self.assertEqual(artifact.read_bytes(), expected)
            self.assertEqual(json.loads(output.getvalue()), self.verify(self.artifact()))
            with contextlib.redirect_stdout(io.StringIO()), self.assertRaises(FileExistsError):
                main(['generate', *args])
            self.assertEqual(artifact.read_bytes(), expected)
            with contextlib.redirect_stdout(io.StringIO()) as output:
                main(['validate', *args])
            self.assertEqual(json.loads(output.getvalue()), self.verify(self.artifact()))
            self.assertEqual(artifact.read_bytes(), expected)
            forged = json.loads(expected)
            forged['counts']['reviewed_rule_rows'] = 1
            artifact.write_text(json.dumps(forged))
            before = artifact.read_bytes()
            with contextlib.redirect_stdout(io.StringIO()), self.assertRaises(ValueError):
                main(['validate', *args])
            self.assertEqual(artifact.read_bytes(), before)


    def test_all_22_prior_canonical_bytes_without_new_module(self):
        # Captured from exact ab88f83a32609c3dfb2b30ec7336fdbed8634cd0.
        expected = {'tokens': 'ea7fa27afdf901b91464ead14c54883debd091819c25bf02870e7a434ae61785',
         'const-eval': '0ce718c6e2834b6c518ff43f2fed2b5de72fd75eccf01d4611c3217cbdf053c2',
         'unsafe': 'bc752efc1f0172cd85dad9399a1309c06489b3b0e4d4f74884c8bc1ef58ebb58',
         'abi': 'c4e561a9a78002281c34cbc35fe7fcaee3f29e82c659b718844504514ac1d317',
         'visibility': '49883324b912c7f107d3573d6b95a3b4cc52b3d97253653eb39dfee6af9cbd58',
         'branching': 'b08afed11ff9bd56c0b15f31bbaa3109105181bde417f0565ff1082cf340e74a',
         'destructors': 'af8e7a87b177e4c72efd7f0dddfe26c2a0390d710377a507f9765f3d1b9d188e',
         'loops': '4a77bd5239c74bde49e8ba7933248484acd773c4f08239ce6f1fce23efb2261a',
         'operators': '2d724517022229ad541ef7c2a66aafc414b13f56c68d3cb8ea633755d3996ba3',
         'closures': 'b75b0495a40e114b0e698ac0e64a84a195ed0a1745f14a2b09f6b5ee043b4b64',
         'calls': '42efdae31602a2af4f2fc243d5a74404159f877b278820905a7b4086281d302e',
         'aggregates': '8c83d418848de65f5251d3b23248dd09121d7ac82b1099739648de6a034a55b3',
         'fields': 'e53fbee9b1248907fb91f9524440cfa75e5991fa0146800c6f0be3f2c67541b2',
         'literals': '159e5a9f425ac09e2b1b4ae96a8c8710264f5ebc3170ec97dfc48e1d947808d8',
         'grouped': '5a9393145150885b8c9f7717e99000003751454bd0a322ed7152e24e4289e279',
         'transfers': '5f29782852c84ccf19cd0fde0ef1c4d9f54a6813eda59d1d9f1686bd387b123d',
         'ranges': '3017078dfc42f85b5e51849d80e1a7efc41a247bbb573f09a7c40b6c5826fdda',
         'structs': '5fb87b17efaa5ac3d34840f9735b2499760c748ba7d9a5fa6a8909f4321f621e',
         'blocks': '44247bc80441734de02b723d046fd34d801801ee2fda6c7d50fcd0154c5e3a90',
         'expression-core': 'c97e514ad4cf1babc0dc6901c432dfac71f98522fcbfbeafde1b920056f5f18a',
         'path-expressions': '5e4dbc27da9288eab71e26816f052348b13dbedcc703244cafa4fa64fb6f993f',
         'never-type': '5c2f254ae61a3838279518c4279654bfd2e5218a3a0492b5976153dab23a9ac5'}
        self.assertEqual(len(expected), 22)
        self.assertEqual(set(expected), {name for name, _ in reconciliation.PROPOSAL_COUNTS
                                         if name not in ('trait-objects', 'type-parameters', 'nominal-types', 'inferred-types', 'coercions', 'subtyping', 'type-expressions', 'primitive-types', 'sequence-types', 'pointer-types', 'function-pointers', 'function-items')})
        with mock.patch.dict(sys.modules, {'coercions_reconciliation_proposals': None}):
            for profile, sha in expected.items():
                with self.subTest(profile=profile):
                    data = build(self.index, self.inventory, self.source, profile)
                    validate(data, self.index, self.inventory, self.source, profile)
                    self.assertEqual(digest((json.dumps(data, indent=2) + '\n').encode()), sha)
            default = build(self.index, self.inventory, self.source)
            self.assertEqual(digest((json.dumps(default, indent=2) + '\n').encode()), expected['tokens'])

    def test_authored_caveats_are_explicit_without_credit(self):
        import coercions_reconciliation_proposals as proposals
        rows = {line.split('|')[0]: line.split('|')
                for line in proposals.ROWS.strip().splitlines()}
        self.assertEqual(len(rows), 46)
        self.assertEqual(rows['coerce.as'][1], 'N X')
        self.assertEqual(next(row for row in self.artifact()['rows']
                              if row['rule'] == 'coerce.as')['classifications'],
                         ['normative-semantic', 'cross-reference'])
        self.assertTrue(all(len(fields) == 7 for fields in rows.values()))
        for label, phrases in {
            'coerce.site.intro': ('without type inference', 'designated sites'),
            'coerce.site.argument': ('method receivers', 'formal parameter'),
            'coerce.site.subexpr': ('recursively', 'specified relevant'),
            'coerce.types.transitive': ('not fully supported', 'universal transitivity'),
            'coerce.types.deref-mut': ('DerefMut', 'Deref-only'),
            'coerce.types.unsize': ('Box', 'unlisted recursive tuple'),
            'coerce.unsize.trait-object': ('Sized', 'dyn compatible'),
            'coerce.unsize.trait-upcast': ('supertrait', 'unguaranteed auto-trait addition'),
            'coerce.unsized.composite': ('struct Foo', 'last field', 'no T in any other field',
                                         'nested Unsize'),
            'coerce.unsized.pointer': ('not yet stable', 'eligibility unknown from source pin'),
            'coerce.least-upper-bound.intro': ('labeled-block', 'loop breaks', 'function multiple'),
            'coerce.least-upper-bound.computation': ('initial target T0', 'pair permutations'),
            'coerce.least-upper-bound.computation-replace': ('all source expressions', 'implicit coercions'),
            'coerce.least-upper-bound.computation-unify': ('neither ordered', 'mutual supertype', 'informal'),
        }.items():
            for phrase in phrases:
                with self.subTest(label=label, phrase=phrase):
                    self.assertIn(phrase, '|'.join(rows[label]))
        for phrase in ('UNREVIEWED', 'unknown from this source pin', 'undefined behavior',
                       'pair-order independence', 'not semantic equivalence'):
            self.assertIn(phrase, ' '.join(proposals.LIMITATIONS))

    def test_actual_process_cli_generate_validate(self):
        with tempfile.TemporaryDirectory() as temp:
            artifact = Path(temp) / 'coercions.json'
            args = [sys.executable, str(BASE / 'reconcile_token_rules.py'),
                    '--profile', 'coercions', '--rule-index', str(self.index),
                    '--inventory', str(self.inventory), '--source-root', str(self.source),
                    '--artifact', str(artifact)]
            expected = (json.dumps(self.artifact(), indent=2) + '\n').encode()
            for action in ('generate', 'validate'):
                result = subprocess.run([*args[:2], action, *args[2:]],
                                        capture_output=True, text=True, check=False)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertEqual(json.loads(result.stdout), self.verify(self.artifact()))
                self.assertEqual(artifact.read_bytes(), expected)



class SubtypingProposalTests(unittest.TestCase):
    PAGE = 'src/subtyping.md'
    PAGE_HASH = '42a19dd521df4060998d0e5e8516d7259a592ebb587e0716198c8e96957ab944'
    LABELS = ('subtype',
     'subtype.intro',
     'subtype.kinds',
     'subtype.higher-ranked',
     'subtyping.variance',
     'subtyping.variance.intro',
     'subtyping.variance.covariant',
     'subtyping.variance.contravariant',
     'subtyping.variance.invariant',
     'subtyping.variance.builtin-types',
     'subtyping.variance.user-composite-types',
     'subtyping.variance.builtin-composite-types')
    STARTS = (1, 4, 7, 21, 38, 41, 44, 47, 50, 53, 69, 86)
    BLOCK_HASHES = ('d56e8a8be9b48f8af63893fddb692f51e45c72865b824b09db1009612c9a47a9',
     'e83e477b50950e3dbfd3b53affad4d1b1a79156e51460db072d64542cedcd61c',
     'b945152cf53feaee36ccb6041007b6d0bc3f447f6c2cc11b4f54de8fd97721ab',
     '8ff75ab08a39695963ea4e7c132bc5ea82c2d3b1d7ea680a801c3276942f9ee8',
     'a039a677c35c4e0433dbe76802fe57015487907b7d37bb0515948f3ab98b07d6',
     '4fa9383a5708cc41ef246db4e5c66cde78b3943629319b40ea935a3073148972',
     '63ebe756b5507a8ba3c1965368809d8ca06106fa6793f8a92365f159dc4e330d',
     'ea54052e11710635570e4d17707ea93ed4eec23d9da5506c43d85c4909f0b2c5',
     '3c7672ea3ea0e4395e56714bd559fb69bb74963d10fd8ca8b0f33535ae85769c',
     '8306c324d9cc21a0b8e251f7d7d1265a66e5d5a156f1b9eef85e6fb511bfc73a',
     'de6e6798c9bf207b2d2291c4e3bfedd96930fd0cbd8f8441ca78ecef43dbfabd',
     '630792d7a267b8155292f21c851d358c1e385ebbe30644f97c06870b1a71ad07')

    def setUp(self):
        source = os.environ.get('JUSTRUST_WITNESS_REFERENCE_ROOT')
        if not source:
            self.skipTest('set JUSTRUST_WITNESS_REFERENCE_ROOT for pinned-source structural checks')
        self.source = Path(source)
        repo = Path(os.environ.get('JUSTRUST_WITNESS_REPO_ROOT', BASE.parent))
        self.index = repo / 'data/compiler-language-reference-rules.json'
        self.inventory = repo / 'data/compiler-language-inventory.json'

    def artifact(self, source=None):
        return build(self.index, self.inventory,
                     self.source if source is None else source, 'subtyping')

    def verify(self, data, source=None):
        return validate(data, self.index, self.inventory,
                        self.source if source is None else source, 'subtyping')

    def test_actual_aliases_pins_no_overlap_and_integrated_totals(self):
        import subtyping_reconciliation_proposals as proposals
        self.assertEqual(proposals.SELECTED, (self.PAGE,))
        self.assertEqual(proposals.PAGE_SHA256, {self.PAGE: self.PAGE_HASH})
        self.assertEqual(proposals.SOURCE_COMMIT, '86635e30bf861a038dc197d7e16fd09e7e514e7a')
        self.assertEqual(proposals.ALIASES, {'variance': 'reference-subtyping-variance'})
        obligations = {o['id'] for s in json.loads(self.inventory.read_bytes())['sections']
                       for o in s['obligations']}
        self.assertTrue(set(proposals.ALIASES.values()) <= obligations)
        index = json.loads(self.index.read_bytes())
        prior_pages = {p for name, _ in reconciliation.PROPOSAL_COUNTS if name not in ('trait-objects', 'type-parameters', 'nominal-types', 'inferred-types', 'subtyping', 'type-expressions', 'primitive-types', 'sequence-types', 'pointer-types', 'function-pointers', 'function-items')
                       for p in reconciliation.profile_inputs(name)[0]}
        prior = {(p['path'], r['label']) for p in index['pages'] + index['unlinked_markdown_files']
                 if p['path'] in prior_pages for r in p['rules']}
        current = {(r['page'], r['rule']) for r in self.artifact()['rows']}
        self.assertFalse(prior & current)
        self.assertEqual((len(prior), len(current), len(prior | current)), (866, 12, 878))
        summary = reconciliation.summarize_proposals(self.index, self.inventory, include_proposals=True)
        self.assertEqual((summary['proposed_rule_identities'],
                          summary['rule_identities_without_proposals']), (1030, 1935))
        self.assertEqual(sum(p['proposed_rule_identities'] for p in summary['profiles'].values()), 1035)
        self.assertEqual(summary['profiles']['subtyping']['completed_rule_rows'], 0)

    def test_hardcoded_hash_labels_lineblocks_partition_and_zero_credit(self):
        import re
        data = self.artifact()
        counts = self.verify(data)
        self.assertEqual([counts[k] for k in ('selected_rule_rows', 'unselected_rule_identities',
                                             'global_rule_identities')], [12, 2953, 2965])
        raw = (self.source / self.PAGE).read_bytes()
        self.assertEqual(data['bindings']['reference_commit'],
                         '86635e30bf861a038dc197d7e16fd09e7e514e7a')
        self.assertEqual(digest(raw), self.PAGE_HASH)
        lines = raw.splitlines(keepends=True)
        self.assertEqual(len(lines), 113)
        markers = [(i, m.group(1).decode()) for i, line in enumerate(lines, 1)
                   if (m := re.fullmatch(rb'r\[([^\]\r\n]+)\]\r?\n?', line))]
        self.assertEqual(markers, list(zip(self.STARTS, self.LABELS)))
        self.assertEqual(tuple(r['rule'] for r in data['rows']), self.LABELS)
        self.assertEqual([(p['path'], p['sha256']) for p in data['selected_pages']],
                         [(self.PAGE, self.PAGE_HASH)])
        for i, row in enumerate(data['rows']):
            end = self.STARTS[i + 1] - 1 if i + 1 < 12 else 113
            with self.subTest(rule=row['rule']):
                self.assertEqual((row['page'], row['line'], row['end_line']),
                                 (self.PAGE, self.STARTS[i], end))
                self.assertEqual(digest(b''.join(lines[self.STARTS[i] - 1:end])), self.BLOCK_HASHES[i])
                self.assertEqual(row['source_block_sha256'], self.BLOCK_HASHES[i])
                prefix = 'reconcile-subtyping-'
                self.assertEqual(row['id'], prefix + self.LABELS[i].encode().hex())
                self.assertEqual(bytes.fromhex(row['id'][len(prefix):]).decode(), self.LABELS[i])
                self.assertEqual(row['review_status'], 'unreviewed')
                self.assertEqual(len(row['missing_test_needs']), 4)
                for need, kind in zip(row['missing_test_needs'], ('positive', 'invalid', 'interaction', 'end-to-end')):
                    self.assertTrue(need.startswith(kind + ': Missing '))
                self.assertIn('stable1.98.1 eligibility classification', row['missing_test_needs'][-1])
        self.assertEqual(len({r['id'] for r in data['rows']}), 12)
        for k in ('reviewed_rule_rows', 'rows_with_exact_executable_evidence'):
            self.assertEqual(counts[k], 0)
        index = json.loads(self.index.read_bytes())
        universe = {(p['path'], r['label']) for p in index['pages'] + index['unlinked_markdown_files']
                    for r in p['rules']}
        selected = {(r['page'], r['rule']) for r in data['rows']}
        other = {(r['page'], r['rule']) for r in data['unselected_rules']}
        self.assertEqual((len(selected), len(other), len(universe)), (12, 2953, 2965))
        self.assertEqual(len(data['unselected_rules']), 2953)
        self.assertFalse(selected & other)
        self.assertEqual(selected | other, universe)
        self.assertEqual(self.artifact(), data)

    def test_permuted_authored_rows_preserve_canonical_order(self):
        import subtyping_reconciliation_proposals as proposals
        baseline = self.artifact()
        rows = proposals.ROWS.strip().splitlines()
        for changed in (rows[::-1], rows[2:] + rows[:2], rows[::2] + rows[1::2]):
            with mock.patch.object(proposals, 'ROWS', '\n'.join(changed)):
                data = self.artifact()
                self.assertEqual(data, baseline)
                self.assertEqual(tuple(r['rule'] for r in data['rows']), self.LABELS)
                self.assertEqual(self.verify(data), self.verify(baseline))

    def test_missing_duplicate_foreign_malformed_authored_rows_rejected(self):
        import subtyping_reconciliation_proposals as proposals
        valid = self.artifact()
        rows = proposals.ROWS.strip().splitlines()
        self.assertEqual(tuple(r.split('|')[0] for r in rows), self.LABELS)
        cases = [rows[4:], rows[:4], rows[:-1], [], rows + [rows[0]],
                 ['expr.foreign' + rows[0][len('subtype'):], *rows[1:]],
                 ['|'.join(rows[0].split('|')[:-1]), *rows[1:]],
                 [rows[0] + '|extra', *rows[1:]]]
        cases.extend(rows[:i] + rows[i + 1:] for i in range(len(rows)))
        for col in (4, 5, 6):
            fields = rows[0].split('|')
            fields[col] = '   '
            cases.append(['|'.join(fields), *rows[1:]])
        for i, changed in enumerate(cases):
            with self.subTest(case=i), mock.patch.object(proposals, 'ROWS', '\n'.join(changed)):
                with self.assertRaises(ValueError):
                    self.artifact()
                with self.assertRaises(ValueError):
                    self.verify(valid)

    def test_29_actual_changing_forgeries_rejected(self):
        baseline = self.artifact()
        mutations = (
            lambda d: d['rows'].pop(),
            lambda d: d['rows'].append(copy.deepcopy(d['rows'][0])),
            lambda d: d['rows'].reverse(),
            lambda d: d['rows'][0].update(page='src/expressions/block-expr.md'),
            lambda d: d['rows'][4].update(page='src/expressions/struct-expr.md'),
            lambda d: d['rows'][0].update(rule='expr.loop'),
            lambda d: d['rows'][0].update(line=True),
            lambda d: d['rows'][0].update(line=1.0),
            lambda d: d['counts'].update(reviewed_rule_rows=False),
            lambda d: d['rows'][0].update(relationship='exact-executable-evidence'),
            lambda d: d['rows'][0].update(classifications=['grammar']),
            lambda d: d['rows'][0].update(end_line=99999),
            lambda d: d['rows'][0].update(source_block_sha256='0' * 64),
            lambda d: d['rows'][4].update(source_block_sha256='0' * 64),
            lambda d: d['rows'][0].update(id='reconcile-subtyping-expr'),
            lambda d: d['rows'][0].update(id=d['rows'][1]['id']),
            lambda d: d['rows'][0].update(review_status='reviewed'),
            lambda d: d['rows'][0].update(rationale='invented interpretation'),
            lambda d: d['rows'][0].update(candidate_obligation_ids=['foreign']),
            lambda d: d['rows'][0].update(missing_test_needs=[]),
            lambda d: d['rows'][0]['missing_test_needs'].__setitem__(0, 'positive: implemented'),
            lambda d: d['counts'].update(reviewed_rule_rows=1),
            lambda d: d['counts'].update(rows_with_exact_executable_evidence=1),
            lambda d: d['selected_pages'][0].update(sha256='0' * 64),
            lambda d: d['selected_pages'].clear(),
            lambda d: d['bindings'].update(reference_commit='0' * 40),
            lambda d: d['unselected_rules'].pop(),
            lambda d: d['unselected_rules'].append(copy.deepcopy(d['unselected_rules'][0])),
            lambda d: d['unselected_rules'][0].update(page=d['rows'][0]['page'], rule=d['rows'][0]['rule']),
        )
        self.assertEqual(len(mutations), 29)
        for i, mutate in enumerate(mutations):
            changed = copy.deepcopy(baseline)
            mutate(changed)
            self.assertNotEqual(json.dumps(changed, sort_keys=True), json.dumps(baseline, sort_keys=True))
            with self.subTest(mutation=i), self.assertRaises(ValueError):
                self.verify(changed)

    def test_source_faults_preserve_exact_error_types(self):
        data = self.artifact()
        for state, error in (('stale', ValueError), ('missing', FileNotFoundError),
                             ('directory', IsADirectoryError), ('symlink', ValueError)):
            with self.subTest(state=state), tempfile.TemporaryDirectory() as temp:
                root = Path(temp)
                target = root / self.PAGE
                target.parent.mkdir(parents=True)
                if state == 'stale':
                    target.write_bytes((self.source / self.PAGE).read_bytes() + b'\n')
                elif state == 'directory':
                    target.mkdir()
                elif state == 'symlink':
                    target.symlink_to(self.source / self.PAGE)
                for action in (lambda: self.artifact(root), lambda: self.verify(data, root)):
                    with self.assertRaises(error) as caught:
                        action()
                    self.assertIs(type(caught.exception), error)

    def test_source_only_restrictions_precede_io_and_execution(self):
        args = ['--profile', 'subtyping', '--rule-index', '/absent/index',
                '--inventory', '/absent/inventory', '--source-root', '/absent/source',
                '--artifact', '/absent/artifact']
        extras = ([], ['--repo-root', '/absent/repo'], ['--evidence', '/absent/evidence'],
                  ['--repo-root', '/absent/repo', '--evidence', '/absent/evidence'])
        cases = [(action, extra) for action in ('generate-witness-links', 'build-witness-links',
                  'validate-witness-links', 'assess-witness-links') for extra in extras]
        cases += [(action, extra) for action in ('generate', 'validate') for extra in extras[1:]]
        with contextlib.ExitStack() as stack:
            for name in ('read_bytes', 'read_text', 'open', 'is_symlink', 'is_file',
                         'write_bytes', 'write_text', 'mkdir'):
                stack.enter_context(mock.patch.object(Path, name, side_effect=AssertionError('IO')))
            stack.enter_context(mock.patch.object(subprocess, 'run', side_effect=AssertionError('execution')))
            for action, extra in cases:
                with self.subTest(action=action, extra=extra), self.assertRaises(ValueError):
                    main([action, *args, *extra])
            for call in (
                lambda: reconciliation.build_witness_links(
                    self.index, self.inventory, self.source, self.source, 'subtyping'),
                lambda: reconciliation.validate_witness_links(
                    {}, self.index, self.inventory, self.source, self.source, 'subtyping'),
                lambda: reconciliation.assess_witness_links(
                    {}, self.index, self.inventory, self.source, self.source, (), 'subtyping'),
            ):
                with self.assertRaises(ValueError):
                    call()

    def test_cli_exclusive_generation_validation_and_no_repair(self):
        with tempfile.TemporaryDirectory() as temp:
            artifact = Path(temp) / 'subtyping.json'
            args = ['--profile', 'subtyping', '--rule-index', str(self.index),
                    '--inventory', str(self.inventory), '--source-root', str(self.source),
                    '--artifact', str(artifact)]
            expected = (json.dumps(self.artifact(), indent=2) + '\n').encode()
            with contextlib.redirect_stdout(io.StringIO()) as output:
                main(['generate', *args])
            self.assertEqual(artifact.read_bytes(), expected)
            self.assertEqual(json.loads(output.getvalue()), self.verify(self.artifact()))
            with contextlib.redirect_stdout(io.StringIO()), self.assertRaises(FileExistsError):
                main(['generate', *args])
            self.assertEqual(artifact.read_bytes(), expected)
            with contextlib.redirect_stdout(io.StringIO()) as output:
                main(['validate', *args])
            self.assertEqual(json.loads(output.getvalue()), self.verify(self.artifact()))
            self.assertEqual(artifact.read_bytes(), expected)
            forged = json.loads(expected)
            forged['counts']['reviewed_rule_rows'] = 1
            artifact.write_text(json.dumps(forged))
            before = artifact.read_bytes()
            with contextlib.redirect_stdout(io.StringIO()), self.assertRaises(ValueError):
                main(['validate', *args])
            self.assertEqual(artifact.read_bytes(), before)


    def test_all_23_prior_canonical_bytes_without_new_module(self):
        # Captured independently from exact base 062bb861089e8a74da44380cffacc7970f6007fd.
        expected = {'tokens': 'ea7fa27afdf901b91464ead14c54883debd091819c25bf02870e7a434ae61785',
         'const-eval': '0ce718c6e2834b6c518ff43f2fed2b5de72fd75eccf01d4611c3217cbdf053c2',
         'unsafe': 'bc752efc1f0172cd85dad9399a1309c06489b3b0e4d4f74884c8bc1ef58ebb58',
         'abi': 'c4e561a9a78002281c34cbc35fe7fcaee3f29e82c659b718844504514ac1d317',
         'visibility': '49883324b912c7f107d3573d6b95a3b4cc52b3d97253653eb39dfee6af9cbd58',
         'branching': 'b08afed11ff9bd56c0b15f31bbaa3109105181bde417f0565ff1082cf340e74a',
         'destructors': 'af8e7a87b177e4c72efd7f0dddfe26c2a0390d710377a507f9765f3d1b9d188e',
         'loops': '4a77bd5239c74bde49e8ba7933248484acd773c4f08239ce6f1fce23efb2261a',
         'operators': '2d724517022229ad541ef7c2a66aafc414b13f56c68d3cb8ea633755d3996ba3',
         'closures': 'b75b0495a40e114b0e698ac0e64a84a195ed0a1745f14a2b09f6b5ee043b4b64',
         'calls': '42efdae31602a2af4f2fc243d5a74404159f877b278820905a7b4086281d302e',
         'aggregates': '8c83d418848de65f5251d3b23248dd09121d7ac82b1099739648de6a034a55b3',
         'fields': 'e53fbee9b1248907fb91f9524440cfa75e5991fa0146800c6f0be3f2c67541b2',
         'literals': '159e5a9f425ac09e2b1b4ae96a8c8710264f5ebc3170ec97dfc48e1d947808d8',
         'grouped': '5a9393145150885b8c9f7717e99000003751454bd0a322ed7152e24e4289e279',
         'transfers': '5f29782852c84ccf19cd0fde0ef1c4d9f54a6813eda59d1d9f1686bd387b123d',
         'ranges': '3017078dfc42f85b5e51849d80e1a7efc41a247bbb573f09a7c40b6c5826fdda',
         'structs': '5fb87b17efaa5ac3d34840f9735b2499760c748ba7d9a5fa6a8909f4321f621e',
         'blocks': '44247bc80441734de02b723d046fd34d801801ee2fda6c7d50fcd0154c5e3a90',
         'expression-core': 'c97e514ad4cf1babc0dc6901c432dfac71f98522fcbfbeafde1b920056f5f18a',
         'path-expressions': '5e4dbc27da9288eab71e26816f052348b13dbedcc703244cafa4fa64fb6f993f',
         'never-type': '5c2f254ae61a3838279518c4279654bfd2e5218a3a0492b5976153dab23a9ac5',
         'coercions': '7bc24b2e1a5b422ef29ce96bbcfb767c52aee9b9be6e4d75ea40c890f4d90526'}
        self.assertEqual(len(expected), 23)
        self.assertEqual(set(expected), {name for name, _ in reconciliation.PROPOSAL_COUNTS
                                         if name not in ('trait-objects', 'type-parameters', 'nominal-types', 'inferred-types', 'subtyping', 'type-expressions', 'primitive-types', 'sequence-types', 'pointer-types', 'function-pointers', 'function-items')})
        with mock.patch.dict(sys.modules, {'subtyping_reconciliation_proposals': None}):
            for profile, sha in expected.items():
                with self.subTest(profile=profile):
                    data = build(self.index, self.inventory, self.source, profile)
                    validate(data, self.index, self.inventory, self.source, profile)
                    self.assertEqual(digest((json.dumps(data, indent=2) + '\n').encode()), sha)
            default = build(self.index, self.inventory, self.source)
            self.assertEqual(digest((json.dumps(default, indent=2) + '\n').encode()), expected['tokens'])

    def test_authored_caveats_are_explicit_without_credit(self):
        import subtyping_reconciliation_proposals as proposals
        rows = {line.split('|')[0]: line.split('|')
                for line in proposals.ROWS.strip().splitlines()}
        self.assertEqual(tuple(rows), self.LABELS)
        self.assertEqual(rows['subtype.higher-ranked'][1], 'N X')
        self.assertTrue(all(len(fields) == 7 for fields in rows.values()))
        for label, phrases in {
            'subtype.intro': ('any stage', 'inference'),
            'subtype.kinds': ('restricted', 'erasing lifetimes', 'type equality', 'Static outlives'),
            'subtype.higher-ranked': ('function pointers and trait objects', 'two independent binders', 'escaping binder leaks', 'without converse'),
            'subtyping.variance.covariant': ('F<T> subtype F<U>', 'preserving direction'),
            'subtyping.variance.contravariant': ('F<U> subtype F<T>', 'reversing direction', 'function argument'),
            'subtyping.variance.invariant': ('no subtyping relation', 'equal types'),
            'subtyping.variance.builtin-types': ('Shared references', 'mutable references covariant in lifetime but invariant in T', 'const pointers covariant', 'mut pointers invariant', 'slices and arrays covariant', 'function returns covariant and arguments contravariant', 'UnsafeCell invariant', 'PhantomData covariant', 'dyn Trait<T> covariant in object lifetime but invariant in T'),
            'subtyping.variance.user-composite-types': ('Struct, enum and union', 'field type positions', 'different variances', 'covariant in a and T but invariant in b, c and U', 'U outlives a', 'not bounds-only'),
            'subtyping.variance.builtin-composite-types': ('each parameter occurrence', 'UnsafeCell occurrence stays long', 'contravariant argument extends to static', 'long:short or middle:short'),
        }.items():
            for phrase in phrases:
                with self.subTest(label=label, phrase=phrase):
                    self.assertIn(phrase, '|'.join(rows[label]))
        for phrase in ('UNREVIEWED', 'unknown from this source pin', 'not semantic equivalence',
                       'Function arguments are contravariant', 'positions are checked separately',
                       'not imported unpinned contracts', 'never authorizes undefined behavior'):
            self.assertIn(phrase, ' '.join(proposals.LIMITATIONS))

    def test_actual_process_cli_generate_validate(self):
        with tempfile.TemporaryDirectory() as temp:
            artifact = Path(temp) / 'subtyping.json'
            args = [sys.executable, str(BASE / 'reconcile_token_rules.py'),
                    '--profile', 'subtyping', '--rule-index', str(self.index),
                    '--inventory', str(self.inventory), '--source-root', str(self.source),
                    '--artifact', str(artifact)]
            expected = (json.dumps(self.artifact(), indent=2) + '\n').encode()
            for action in ('generate', 'validate'):
                result = subprocess.run([*args[:2], action, *args[2:]],
                                        capture_output=True, text=True, check=False)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertEqual(json.loads(result.stdout), self.verify(self.artifact()))
                self.assertEqual(artifact.read_bytes(), expected)


class TypeExpressionsProposalTests(unittest.TestCase):
    PAGE = 'src/types.md'
    PAGE_HASH = '8d34ff2d91d438c6520316751de9259072d47eb31e8a7c931c46f9d409979837'
    LABELS = ('type', 'type.intro', 'type.builtin', 'type.user-defined', 'type.kinds',
              'type.name', 'type.name.syntax', 'type.name.intro', 'type.name.sequence',
              'type.name.path', 'type.name.pointer', 'type.name.inference', 'type.name.grouped',
              'type.name.trait', 'type.name.never', 'type.name.macro-expansion',
              'type.name.parenthesized', 'type.name.parenthesized.syntax',
              'type.name.parenthesized.intro', 'type.recursive', 'type.recursive.intro',
              'type.recursive.constraint')
    STARTS = (1, 4, 7, 10, 13, 41, 44, 68, 71, 74, 81, 84, 87, 90, 93, 96, 99, 102, 107, 115, 118, 121)
    BLOCK_HASHES = (
        'd10a5252418bcac4d919d25ce2c39b30ec6e0d5b1160217f9336b249780006ed',
        '7744026996eaf7808671cf4804bca6bba0ee76f23757db2a0c8f6a7c0811aeed',
        'a4070d2f162c22af4b91167982e18636a606200c936ac745b35e66d4f18faccd',
        '49273d743d3be5b5ca70e77637d8dfd8af57b00347b94ab3e0f8f38ac53fc4ff',
        'aa542687bac053459ba6d63a51f3edb0b3ccbeaee581a7a9296403b78ce46253',
        '30be7a04981b7796f88d6e5c3fd5f2793d2485438dffd8f59eca906b369606da',
        '6b3d2a2c64c76006bf61ef8299833cfd4d127f71b38a392d515714cf44836b7d',
        '597036da52418b1fb49a45dcd0b1bf1b771af920676c768d6532cac777103ddc',
        '55c35256fd962bacf648eccf8896871da8978c7fa8ddfb729347429b911f698e',
        'c3a1316690c13014062afbf014c39c21d57916b735007152cdd2ccf99a9825e7',
        'e68f7d099fbad2feedefc088d1c7f50df002da9847ffc433afdce982e3e8f040',
        '15274f8aac7d37020a4f39c076b6e428599f1f2aec0131933496ed364d25ea6b',
        'd828524c42ad140726e6baa8000cb1768cff5a28ce25f9fa3d4ea1e81dd8615d',
        'adb6fae0bb50c609215b8f90431a2d1d5f018617fef8eedbeb17bd89f2d4ddc9',
        'dd9429c2d9ce736930fda5ef8bf435b5ef05e6dc771d03ec2ce6eca07500c300',
        '700ec578477f2ec356dbd023fd4f19dc23e27fa96ba30abfc3b53c1fc586def1',
        '4109914a5c42d9bc3cda6f0200463399a77b2d6e2403dfdd24a92948e3201b0c',
        '7fc5191bfd69662143345827c1281922a826dc789696406f33bd27c021dd6d33',
        '2b646861df3d293b0458aa963d37760a340e8e55f00b9bd4cc8e8cea14ab27f6',
        'd50e835884f92880e6375eec5830f6c464809f9070856dc84dccf063cc29252a',
        '99093d1da5f552ab908b2c81682da2b1903046e9ef269241adb3f1bacdeff11c',
        '74e289539b1f483504c33fb98438f9152dd0c3474ab665b884856137d45f5d0c')

    def test_exact_grammar_alternatives_and_authored_caveats_without_credit(self):
        import re
        import type_expressions_reconciliation_proposals as proposals
        raw = (self.source / self.PAGE).read_text()
        grammar = re.search(r'Type ->\n(.*?)\n\nTypeNoBounds ->\n(.*?)\n```', raw, re.S)
        self.assertIsNotNone(grammar)
        alternatives = lambda text: tuple(line.strip().removeprefix('|').strip()
                                          for line in text.splitlines())
        self.assertEqual(alternatives(grammar[1]),
                         ('TypeNoBounds', 'ImplTraitType', 'TraitObjectType'))
        self.assertEqual(alternatives(grammar[2]), (
            'ParenthesizedType', 'ImplTraitTypeOneBound', 'TraitObjectTypeOneBound',
            'TypePath', 'TupleType', 'NeverType', 'RawPointerType', 'ReferenceType',
            'ArrayType', 'SliceType', 'InferredType', 'QualifiedPathInType',
            'BareFunctionType', 'MacroInvocation'))
        self.assertEqual(tuple(map(len, map(alternatives, (grammar[1], grammar[2])))), (3, 14))
        self.assertIn('ParenthesizedType -> `(` Type `)`', raw)
        rows = {line.split('|')[0]: line.split('|')
                for line in proposals.ROWS.strip().splitlines()}
        self.assertEqual(tuple(rows), self.LABELS)
        self.assertTrue(all(len(fields) == 7 for fields in rows.values()))
        self.assertEqual(rows['type.intro'][1], 'N X')
        self.assertEqual(rows['type.name.syntax'][1], 'G')
        self.assertEqual(rows['type.name.parenthesized.syntax'][1], 'G')
        self.assertEqual(rows['type.name.parenthesized.intro'][1], 'N G X')
        self.assertEqual(rows['type.recursive.constraint'][1], 'N X')
        artifact = {row['rule']: row for row in self.artifact()['rows']}
        for label in ('type', 'type.builtin', 'type.user-defined', 'type.kinds'):
            self.assertEqual(artifact[label]['relationship'], 'context-only')
            self.assertFalse({'normative-semantic', 'grammar'} &
                             set(artifact[label]['classifications']))
        for label, phrases in {
            'type.intro': ('Every variable, item and value', 'memory', 'not a standalone diagnostic'),
            'type.builtin': ('not emulable', 'not a standalone rejection rule'),
            'type.user-defined': ('limited capabilities', 'does not enumerate limitations'),
            'type.name.syntax': ('(3)', '(14)', 'QualifiedPathInType', 'InferredType', 'MacroInvocation'),
            'type.name.path': ('Self as implementing type', 'generic type parameters', 'type alias'),
            'type.name.never': ('placement eligibility', 'without assuming stable1.98.1 eligibility'),
            'type.name.macro-expansion': ('expand to a type expression', 'expansions producing expressions'),
            'type.name.parenthesized.intro': ('TypeNoBounds instead of Type', 'ParenthesizedType enclosing Type',
                                             'dyn Any + Send', 'singleton tuple syntax', 'expression grouping'),
            'type.recursive.intro': ('structs, enumerations and unions', 'enum variant or struct or union field',
                                     'enclosing enum or struct type itself', 'linked details'),
            'type.recursive.constraint': ('BOTH', 'AND finite size', 'not mere type aliases',
                                          'structural arrays or tuples', "type Rec = &'static [Rec]",
                                          'Nil and Cons(T, Box<List<T>>)', 'even unused',
                                          'pointer indirection not rescuing alias-only cycles'),
        }.items():
            for phrase in phrases:
                with self.subTest(label=label, phrase=phrase):
                    self.assertIn(phrase, '|'.join(rows[label]))
        for phrase in ('UNREVIEWED', 'unknown from this source pin', 'not semantic equivalence',
                       'or a semantic witness', 'not imported unpinned contracts',
                       'not standalone diagnostics', '14, independently counted',
                       'BOTH nominal involvement and finite size', 'linked details are not absent'):
            self.assertIn(phrase, ' '.join(proposals.LIMITATIONS))

    def setUp(self):
        source = os.environ.get('JUSTRUST_WITNESS_REFERENCE_ROOT')
        if not source:
            self.skipTest('set JUSTRUST_WITNESS_REFERENCE_ROOT for pinned-source structural checks')
        self.source = Path(source)
        repo = Path(os.environ.get('JUSTRUST_WITNESS_REPO_ROOT', BASE.parent))
        self.index = repo / 'data/compiler-language-reference-rules.json'
        self.inventory = repo / 'data/compiler-language-inventory.json'

    def artifact(self, source=None):
        return build(self.index, self.inventory,
                     self.source if source is None else source, 'type-expressions')

    def verify(self, data, source=None):
        return validate(data, self.index, self.inventory,
                        self.source if source is None else source, 'type-expressions')

    def test_actual_aliases_pins_no_overlap_and_integrated_totals(self):
        import type_expressions_reconciliation_proposals as proposals
        self.assertEqual(proposals.SELECTED, (self.PAGE,))
        self.assertEqual(proposals.PAGE_SHA256, {self.PAGE: self.PAGE_HASH})
        self.assertEqual(proposals.SOURCE_COMMIT, '86635e30bf861a038dc197d7e16fd09e7e514e7a')
        self.assertEqual(proposals.ALIASES, {'types': 'reference-types-source-index-review'})
        obligations = {o['id'] for s in json.loads(self.inventory.read_bytes())['sections']
                       for o in s['obligations']}
        self.assertTrue(set(proposals.ALIASES.values()) <= obligations)
        index = json.loads(self.index.read_bytes())
        prior_pages = {p for name, _ in reconciliation.PROPOSAL_COUNTS if name not in ('trait-objects', 'type-parameters', 'nominal-types', 'inferred-types', 'type-expressions', 'primitive-types', 'sequence-types', 'pointer-types', 'function-pointers', 'function-items')
                       for p in reconciliation.profile_inputs(name)[0]}
        prior = {(p['path'], r['label']) for p in index['pages'] + index['unlinked_markdown_files']
                 if p['path'] in prior_pages for r in p['rules']}
        current = {(r['page'], r['rule']) for r in self.artifact()['rows']}
        self.assertFalse(prior & current)
        self.assertEqual((len(prior), len(current), len(prior | current)), (878, 22, 900))
        summary = reconciliation.summarize_proposals(self.index, self.inventory, include_proposals=True)
        self.assertEqual((summary['proposed_rule_identities'],
                          summary['rule_identities_without_proposals']), (1030, 1935))
        self.assertEqual(sum(p['proposed_rule_identities'] for p in summary['profiles'].values()), 1035)
        self.assertEqual(summary['profiles']['type-expressions']['completed_rule_rows'], 0)

    def test_hardcoded_hash_labels_lineblocks_partition_and_zero_credit(self):
        import re
        data = self.artifact()
        counts = self.verify(data)
        self.assertEqual([counts[k] for k in ('selected_rule_rows', 'unselected_rule_identities',
                                             'global_rule_identities')], [22, 2943, 2965])
        raw = (self.source / self.PAGE).read_bytes()
        self.assertEqual(data['bindings']['reference_commit'],
                         '86635e30bf861a038dc197d7e16fd09e7e514e7a')
        self.assertEqual(digest(raw), self.PAGE_HASH)
        lines = raw.splitlines(keepends=True)
        self.assertEqual(len(lines), 176)
        markers = [(i, m.group(1).decode()) for i, line in enumerate(lines, 1)
                   if (m := re.fullmatch(rb'r\[([^\]\r\n]+)\]\r?\n?', line))]
        self.assertEqual(markers, list(zip(self.STARTS, self.LABELS)))
        self.assertEqual(tuple(r['rule'] for r in data['rows']), self.LABELS)
        self.assertEqual([(p['path'], p['sha256']) for p in data['selected_pages']],
                         [(self.PAGE, self.PAGE_HASH)])
        for i, row in enumerate(data['rows']):
            end = self.STARTS[i + 1] - 1 if i + 1 < 22 else 176
            with self.subTest(rule=row['rule']):
                self.assertEqual((row['page'], row['line'], row['end_line']),
                                 (self.PAGE, self.STARTS[i], end))
                self.assertEqual(digest(b''.join(lines[self.STARTS[i] - 1:end])), self.BLOCK_HASHES[i])
                self.assertEqual(row['source_block_sha256'], self.BLOCK_HASHES[i])
                prefix = 'reconcile-type-expressions-'
                self.assertEqual(row['id'], prefix + self.LABELS[i].encode().hex())
                self.assertEqual(bytes.fromhex(row['id'][len(prefix):]).decode(), self.LABELS[i])
                self.assertEqual(row['review_status'], 'unreviewed')
                self.assertEqual(len(row['missing_test_needs']), 4)
                for need, kind in zip(row['missing_test_needs'], ('positive', 'invalid', 'interaction', 'end-to-end')):
                    self.assertTrue(need.startswith(kind + ': Missing '))
                self.assertIn('stable1.98.1 eligibility classification', row['missing_test_needs'][-1])
        self.assertEqual(len({r['id'] for r in data['rows']}), 22)
        for k in ('reviewed_rule_rows', 'rows_with_exact_executable_evidence'):
            self.assertEqual(counts[k], 0)
        index = json.loads(self.index.read_bytes())
        universe = {(p['path'], r['label']) for p in index['pages'] + index['unlinked_markdown_files']
                    for r in p['rules']}
        selected = {(r['page'], r['rule']) for r in data['rows']}
        other = {(r['page'], r['rule']) for r in data['unselected_rules']}
        self.assertEqual((len(selected), len(other), len(universe)), (22, 2943, 2965))
        self.assertEqual(len(data['unselected_rules']), 2943)
        self.assertFalse(selected & other)
        self.assertEqual(selected | other, universe)
        self.assertEqual(self.artifact(), data)

    def test_permuted_authored_rows_preserve_canonical_order(self):
        import type_expressions_reconciliation_proposals as proposals
        baseline = self.artifact()
        rows = proposals.ROWS.strip().splitlines()
        for changed in (rows[::-1], rows[2:] + rows[:2], rows[::2] + rows[1::2]):
            with mock.patch.object(proposals, 'ROWS', '\n'.join(changed)):
                data = self.artifact()
                self.assertEqual(data, baseline)
                self.assertEqual(tuple(r['rule'] for r in data['rows']), self.LABELS)
                self.assertEqual(self.verify(data), self.verify(baseline))

    def test_missing_duplicate_foreign_malformed_authored_rows_rejected(self):
        import type_expressions_reconciliation_proposals as proposals
        valid = self.artifact()
        rows = proposals.ROWS.strip().splitlines()
        self.assertEqual(tuple(r.split('|')[0] for r in rows), self.LABELS)
        cases = [rows[4:], rows[:4], rows[:-1], [], rows + [rows[0]],
                 ['expr.foreign' + rows[0][len('type'):], *rows[1:]],
                 ['|'.join(rows[0].split('|')[:-1]), *rows[1:]],
                 [rows[0] + '|extra', *rows[1:]]]
        cases.extend(rows[:i] + rows[i + 1:] for i in range(len(rows)))
        for col in (4, 5, 6):
            fields = rows[0].split('|')
            fields[col] = '   '
            cases.append(['|'.join(fields), *rows[1:]])
        for i, changed in enumerate(cases):
            with self.subTest(case=i), mock.patch.object(proposals, 'ROWS', '\n'.join(changed)):
                with self.assertRaises(ValueError):
                    self.artifact()
                with self.assertRaises(ValueError):
                    self.verify(valid)

    def test_29_actual_changing_forgeries_rejected(self):
        baseline = self.artifact()
        mutations = (
            lambda d: d['rows'].pop(),
            lambda d: d['rows'].append(copy.deepcopy(d['rows'][0])),
            lambda d: d['rows'].reverse(),
            lambda d: d['rows'][0].update(page='src/expressions/block-expr.md'),
            lambda d: d['rows'][4].update(page='src/expressions/struct-expr.md'),
            lambda d: d['rows'][0].update(rule='expr.loop'),
            lambda d: d['rows'][0].update(line=True),
            lambda d: d['rows'][0].update(line=1.0),
            lambda d: d['counts'].update(reviewed_rule_rows=False),
            lambda d: d['rows'][0].update(relationship='exact-executable-evidence'),
            lambda d: d['rows'][0].update(classifications=['grammar']),
            lambda d: d['rows'][0].update(end_line=99999),
            lambda d: d['rows'][0].update(source_block_sha256='0' * 64),
            lambda d: d['rows'][4].update(source_block_sha256='0' * 64),
            lambda d: d['rows'][0].update(id='reconcile-type-expressions-expr'),
            lambda d: d['rows'][0].update(id=d['rows'][1]['id']),
            lambda d: d['rows'][0].update(review_status='reviewed'),
            lambda d: d['rows'][0].update(rationale='invented interpretation'),
            lambda d: d['rows'][0].update(candidate_obligation_ids=['foreign']),
            lambda d: d['rows'][0].update(missing_test_needs=[]),
            lambda d: d['rows'][0]['missing_test_needs'].__setitem__(0, 'positive: implemented'),
            lambda d: d['counts'].update(reviewed_rule_rows=1),
            lambda d: d['counts'].update(rows_with_exact_executable_evidence=1),
            lambda d: d['selected_pages'][0].update(sha256='0' * 64),
            lambda d: d['selected_pages'].clear(),
            lambda d: d['bindings'].update(reference_commit='0' * 40),
            lambda d: d['unselected_rules'].pop(),
            lambda d: d['unselected_rules'].append(copy.deepcopy(d['unselected_rules'][0])),
            lambda d: d['unselected_rules'][0].update(page=d['rows'][0]['page'], rule=d['rows'][0]['rule']),
        )
        self.assertEqual(len(mutations), 29)
        for i, mutate in enumerate(mutations):
            changed = copy.deepcopy(baseline)
            mutate(changed)
            self.assertNotEqual(json.dumps(changed, sort_keys=True), json.dumps(baseline, sort_keys=True))
            with self.subTest(mutation=i), self.assertRaises(ValueError):
                self.verify(changed)

    def test_source_faults_preserve_exact_error_types(self):
        data = self.artifact()
        for state, error in (('stale', ValueError), ('missing', FileNotFoundError),
                             ('directory', IsADirectoryError), ('symlink', ValueError)):
            with self.subTest(state=state), tempfile.TemporaryDirectory() as temp:
                root = Path(temp)
                target = root / self.PAGE
                target.parent.mkdir(parents=True)
                if state == 'stale':
                    target.write_bytes((self.source / self.PAGE).read_bytes() + b'\n')
                elif state == 'directory':
                    target.mkdir()
                elif state == 'symlink':
                    target.symlink_to(self.source / self.PAGE)
                for action in (lambda: self.artifact(root), lambda: self.verify(data, root)):
                    with self.assertRaises(error) as caught:
                        action()
                    self.assertIs(type(caught.exception), error)

    def test_source_only_restrictions_precede_io_and_execution(self):
        args = ['--profile', 'type-expressions', '--rule-index', '/absent/index',
                '--inventory', '/absent/inventory', '--source-root', '/absent/source',
                '--artifact', '/absent/artifact']
        extras = ([], ['--repo-root', '/absent/repo'], ['--evidence', '/absent/evidence'],
                  ['--repo-root', '/absent/repo', '--evidence', '/absent/evidence'])
        cases = [(action, extra) for action in ('generate-witness-links', 'build-witness-links',
                  'validate-witness-links', 'assess-witness-links') for extra in extras]
        cases += [(action, extra) for action in ('generate', 'validate') for extra in extras[1:]]
        with contextlib.ExitStack() as stack:
            for name in ('read_bytes', 'read_text', 'open', 'is_symlink', 'is_file',
                         'write_bytes', 'write_text', 'mkdir'):
                stack.enter_context(mock.patch.object(Path, name, side_effect=AssertionError('IO')))
            stack.enter_context(mock.patch.object(subprocess, 'run', side_effect=AssertionError('execution')))
            for action, extra in cases:
                with self.subTest(action=action, extra=extra), self.assertRaises(ValueError):
                    main([action, *args, *extra])
            for call in (
                lambda: reconciliation.build_witness_links(
                    self.index, self.inventory, self.source, self.source, 'type-expressions'),
                lambda: reconciliation.validate_witness_links(
                    {}, self.index, self.inventory, self.source, self.source, 'type-expressions'),
                lambda: reconciliation.assess_witness_links(
                    {}, self.index, self.inventory, self.source, self.source, (), 'type-expressions'),
            ):
                with self.assertRaises(ValueError):
                    call()

    def test_cli_exclusive_generation_validation_and_no_repair(self):
        with tempfile.TemporaryDirectory() as temp:
            artifact = Path(temp) / 'type-expressions.json'
            args = ['--profile', 'type-expressions', '--rule-index', str(self.index),
                    '--inventory', str(self.inventory), '--source-root', str(self.source),
                    '--artifact', str(artifact)]
            expected = (json.dumps(self.artifact(), indent=2) + '\n').encode()
            with contextlib.redirect_stdout(io.StringIO()) as output:
                main(['generate', *args])
            self.assertEqual(artifact.read_bytes(), expected)
            self.assertEqual(json.loads(output.getvalue()), self.verify(self.artifact()))
            with contextlib.redirect_stdout(io.StringIO()), self.assertRaises(FileExistsError):
                main(['generate', *args])
            self.assertEqual(artifact.read_bytes(), expected)
            with contextlib.redirect_stdout(io.StringIO()) as output:
                main(['validate', *args])
            self.assertEqual(json.loads(output.getvalue()), self.verify(self.artifact()))
            self.assertEqual(artifact.read_bytes(), expected)
            forged = json.loads(expected)
            forged['counts']['reviewed_rule_rows'] = 1
            artifact.write_text(json.dumps(forged))
            before = artifact.read_bytes()
            with contextlib.redirect_stdout(io.StringIO()), self.assertRaises(ValueError):
                main(['validate', *args])
            self.assertEqual(artifact.read_bytes(), before)


    def test_all_24_prior_canonical_bytes_without_new_module(self):
        # Captured independently from exact base fe7452a41232a390ef950bb4e294e4bfe3c691d2.
        expected = {'tokens': 'ea7fa27afdf901b91464ead14c54883debd091819c25bf02870e7a434ae61785',
         'const-eval': '0ce718c6e2834b6c518ff43f2fed2b5de72fd75eccf01d4611c3217cbdf053c2',
         'unsafe': 'bc752efc1f0172cd85dad9399a1309c06489b3b0e4d4f74884c8bc1ef58ebb58',
         'abi': 'c4e561a9a78002281c34cbc35fe7fcaee3f29e82c659b718844504514ac1d317',
         'visibility': '49883324b912c7f107d3573d6b95a3b4cc52b3d97253653eb39dfee6af9cbd58',
         'branching': 'b08afed11ff9bd56c0b15f31bbaa3109105181bde417f0565ff1082cf340e74a',
         'destructors': 'af8e7a87b177e4c72efd7f0dddfe26c2a0390d710377a507f9765f3d1b9d188e',
         'loops': '4a77bd5239c74bde49e8ba7933248484acd773c4f08239ce6f1fce23efb2261a',
         'operators': '2d724517022229ad541ef7c2a66aafc414b13f56c68d3cb8ea633755d3996ba3',
         'closures': 'b75b0495a40e114b0e698ac0e64a84a195ed0a1745f14a2b09f6b5ee043b4b64',
         'calls': '42efdae31602a2af4f2fc243d5a74404159f877b278820905a7b4086281d302e',
         'aggregates': '8c83d418848de65f5251d3b23248dd09121d7ac82b1099739648de6a034a55b3',
         'fields': 'e53fbee9b1248907fb91f9524440cfa75e5991fa0146800c6f0be3f2c67541b2',
         'literals': '159e5a9f425ac09e2b1b4ae96a8c8710264f5ebc3170ec97dfc48e1d947808d8',
         'grouped': '5a9393145150885b8c9f7717e99000003751454bd0a322ed7152e24e4289e279',
         'transfers': '5f29782852c84ccf19cd0fde0ef1c4d9f54a6813eda59d1d9f1686bd387b123d',
         'ranges': '3017078dfc42f85b5e51849d80e1a7efc41a247bbb573f09a7c40b6c5826fdda',
         'structs': '5fb87b17efaa5ac3d34840f9735b2499760c748ba7d9a5fa6a8909f4321f621e',
         'blocks': '44247bc80441734de02b723d046fd34d801801ee2fda6c7d50fcd0154c5e3a90',
         'expression-core': 'c97e514ad4cf1babc0dc6901c432dfac71f98522fcbfbeafde1b920056f5f18a',
         'path-expressions': '5e4dbc27da9288eab71e26816f052348b13dbedcc703244cafa4fa64fb6f993f',
         'never-type': '5c2f254ae61a3838279518c4279654bfd2e5218a3a0492b5976153dab23a9ac5',
         'coercions': '7bc24b2e1a5b422ef29ce96bbcfb767c52aee9b9be6e4d75ea40c890f4d90526',
         'subtyping': 'a7d6097d9ee152ae8490c09c692570cb4b65c78b61c68a33c2b89caee694d6cc'}
        self.assertEqual(len(expected), 24)
        self.assertEqual(set(expected), {name for name, _ in reconciliation.PROPOSAL_COUNTS
                                         if name not in ('trait-objects', 'type-parameters', 'nominal-types', 'inferred-types', 'type-expressions', 'primitive-types', 'sequence-types', 'pointer-types', 'function-pointers', 'function-items')})
        with mock.patch.dict(sys.modules, {'type_expressions_reconciliation_proposals': None}):
            for profile, sha in expected.items():
                with self.subTest(profile=profile):
                    data = build(self.index, self.inventory, self.source, profile)
                    validate(data, self.index, self.inventory, self.source, profile)
                    self.assertEqual(digest((json.dumps(data, indent=2) + '\n').encode()), sha)
            default = build(self.index, self.inventory, self.source)
            self.assertEqual(digest((json.dumps(default, indent=2) + '\n').encode()), expected['tokens'])

    def test_actual_process_cli_generate_validate(self):
        with tempfile.TemporaryDirectory() as temp:
            artifact = Path(temp) / 'type-expressions.json'
            args = [sys.executable, str(BASE / 'reconcile_token_rules.py'),
                    '--profile', 'type-expressions', '--rule-index', str(self.index),
                    '--inventory', str(self.inventory), '--source-root', str(self.source),
                    '--artifact', str(artifact)]
            expected = (json.dumps(self.artifact(), indent=2) + '\n').encode()
            for action in ('generate', 'validate'):
                result = subprocess.run([*args[:2], action, *args[2:]],
                                        capture_output=True, text=True, check=False)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertEqual(json.loads(result.stdout), self.verify(self.artifact()))
                self.assertEqual(artifact.read_bytes(), expected)



class PrimitiveTypesProposalTests(unittest.TestCase):
    """Source-only structural checks, never primitive implementation evidence."""
    PAGES = ('src/types/boolean.md', 'src/types/numeric.md', 'src/types/char.md', 'src/types/str.md')
    PAGE_HASHES = {'src/types/boolean.md': 'ad5e69fb6fa11a152b0b3803245ac8b1480c394cb523d600900e2c9e9e198b0f',
     'src/types/char.md': '06f4952d5e4142b65e1fa92a6d08b8081d2ee742efb245385d1802bd3c32bb46',
     'src/types/numeric.md': 'fd6e3e0ef3ac168b8f09d01037f9a44c32f5c4c3d3c3fc798a1c6062f09f2ed8',
     'src/types/str.md': 'd8d381cffdb8159dc85de2d6f241916a45035681817cc941057fad1879fbfeff'}
    PINS = (('src/types/boolean.md', 'type.bool', 1, 7, '7bbdc8969ee0d8b9be9173a6583efbdcd5615d40bb0151caf59c512770baea41', 'I'),
     ('src/types/boolean.md', 'type.bool.intro', 8, 10, '01cfe5a01d00acd410383143bf41a033cb5b705034e24211fb163a22698f842a', 'N'),
     ('src/types/boolean.md', 'type.bool.literal', 11, 13, '8e5fc79aa61e72f298d944d701fb03b12f2c4d4c15f675810efd8a29652e7f65', 'N X'),
     ('src/types/boolean.md',
      'type.bool.namespace',
      14,
      16,
      '6e8f674429e453e6d06782525c40ff2d62f2ce01f5227730455dfb523d2b10ee',
      'N X'),
     ('src/types/boolean.md', 'type.bool.layout', 17, 19, 'bac1b23bde0220fa9f441150ea02026c97ae8e97d0d763eb04bdfe425d00fa70', 'N X'),
     ('src/types/boolean.md', 'type.bool.repr', 20, 22, 'd6644fbb2457aeb68df6f9e31877c8c88aabfff8386bb52ef5862a9aeec843cf', 'N X'),
     ('src/types/boolean.md', 'type.bool.use', 23, 25, '243893f7747ba65bf7d7992507ec045cf922a6df357eef6817aaa27a256cdee7', 'I X'),
     ('src/types/boolean.md',
      'type.bool.use-in-condition',
      26,
      28,
      'caec9551064a82a39e8d1054dd0a891a0c91fb912534d477a17f14e9f41091e4',
      'N X'),
     ('src/types/boolean.md',
      'type.bool.use-in-lazy-operator',
      29,
      34,
      '0b096f2ffb8945f0e5881d2364ed47cd7699ee6405c819f2440bee28f3a9c4a3',
      'N X'),
     ('src/types/boolean.md', 'type.bool.traits', 35, 40, '6249c629dfb0f794d87b5b628af568d23d796b208577a7c5e9d23fe7dc067ea9', 'N X'),
     ('src/types/boolean.md', 'type.bool.expr', 41, 45, 'e15f9e31b2f5b9c20061429df17469aa662a63846f68333760cd6195b16ffd3b', 'N X'),
     ('src/types/boolean.md',
      'type.bool.expr.not',
      46,
      53,
      'e70d9148151c3aa724631eb05df7cf33004df7e1597dcfb157f5dae222bcbd8b',
      'N X'),
     ('src/types/boolean.md', 'type.bool.expr.or', 54, 63, 'b297589430041c697ece61458866ccfbafc368640f51ba44041d99779a4dd939', 'N X'),
     ('src/types/boolean.md',
      'type.bool.expr.and',
      64,
      73,
      'fc6e04199fdb45055fa48b3f8ba2eb4a5a627ecb3abaca6e7d0c9f2544b4f4a4',
      'N X'),
     ('src/types/boolean.md',
      'type.bool.expr.xor',
      74,
      83,
      'b2c6552d51a0366597a08bad9b6f83ff550e4efaa9fb8525b2a4be394104f648',
      'N X'),
     ('src/types/boolean.md', 'type.bool.expr.cmp', 84, 86, '86b1614bfc9bde0cc246258df366f195e520e47d40d936d271ee438cee46132d', 'I'),
     ('src/types/boolean.md',
      'type.bool.expr.cmp.eq',
      87,
      94,
      'ac7fbeb98e8b0c8ab25ed84edaaf277c4db523940b64f91536b1a2e0ea62c2d9',
      'N X'),
     ('src/types/boolean.md',
      'type.bool.expr.cmp.greater',
      95,
      102,
      'd82373e63d00eb316320e9454bc2b682ae116d085069b2ba06b0ea118a507015',
      'N X'),
     ('src/types/boolean.md',
      'type.bool.expr.cmp.not-eq',
      103,
      105,
      'ff10cb8c04cb35a8d9f7c323d1447e5fa971fb43ca27c3fc64dc4eb50092148e',
      'N'),
     ('src/types/boolean.md',
      'type.bool.expr.cmp.greater-eq',
      106,
      108,
      '165dedfa1c7db03fa0386f87cbfe49bbe5f771d5ba62494aca0ecb0d94707027',
      'N'),
     ('src/types/boolean.md',
      'type.bool.expr.cmp.less',
      109,
      111,
      '5198d91bff7509ee4feef250bddf2950aa2651fcd0d8e5612f181a7d72d36aae',
      'N'),
     ('src/types/boolean.md',
      'type.bool.expr.cmp.less-eq',
      112,
      114,
      '20301898054c3deee8d509417d3909f8f396ee0c63be6498d39b052a3b1dd952',
      'N'),
     ('src/types/boolean.md',
      'type.bool.validity',
      115,
      142,
      'c052c64394e03c9a279d8086e56d84e574ccc8a7187d327df81efe731ef8ec3e',
      'N'),
     ('src/types/numeric.md', 'type.numeric', 1, 3, '23ee86db9a0fb0be869a60bc0456dcee03a8c35c73c6da38f7c09f3b701d7224', 'I'),
     ('src/types/numeric.md', 'type.numeric.int', 4, 6, 'eae305bf4dc84316b6484132ed4f269652f5c812539e3a1d24eafff810befa96', 'I'),
     ('src/types/numeric.md',
      'type.numeric.int.unsigned',
      7,
      17,
      '8538a81794dd67acf0f7031473b380ecff7f453192c67609cde4db733023cd77',
      'N'),
     ('src/types/numeric.md',
      'type.numeric.int.signed',
      18,
      28,
      'c6510229ce7b993951298d3fe1dfe4dee83e044fcac6367693b873615e15a5be',
      'N'),
     ('src/types/numeric.md', 'type.numeric.float', 29, 33, 'fa0c0d7dcb136cbcdcfbd3a5fda3f4a0e630966a34e555ba0b5594678d5d5d41', 'N'),
     ('src/types/numeric.md',
      'type.numeric.int.size',
      34,
      36,
      'cfb36b109f3bbc1fe0f1c3d57b3badc261b127bfadaef3d9bbf337e2d2d7ebb7',
      'I'),
     ('src/types/numeric.md',
      'type.numeric.int.size.usize',
      37,
      42,
      'a10c600b4dfdad8dca601f805967f7fb9b520666aecdaa69f78aa648f9e900a1',
      'N X'),
     ('src/types/numeric.md',
      'type.numeric.int.size.isize',
      43,
      45,
      '73d83ded64d0c2b20a6efa9507967309f783ca81debbe399086a9a8806756cb7',
      'N'),
     ('src/types/numeric.md',
      'type.numeric.int.size.minimum',
      46,
      51,
      'ee3e7a32305acbb505a09a2154f0275751c6f4305270feed6d55ddbe10956f14',
      'N'),
     ('src/types/numeric.md',
      'type.numeric.validity',
      52,
      57,
      'e781f5e63c10774b6dc72fb654492c55e60f1e92b186f7dc171a31130c8ff3dc',
      'N'),
     ('src/types/char.md', 'type.char', 1, 3, '5605961353265634ccaec230ae51744a778024c8cc2040830d643f26cad1b10f', 'I'),
     ('src/types/char.md', 'type.char.intro', 4, 16, 'e4985d63ce7cd91a21885b7aaf3a48a3af9218875573762f8cece86cdd646290', 'N X'),
     ('src/types/char.md', 'type.char.value', 17, 19, '28b4fe2573817985b9ab0f63c670d94564c491f0ea13cbf5814a5cb7c58a6515', 'N X'),
     ('src/types/char.md', 'type.char.layout', 20, 22, '449310c0453d9995a59b8ec1b1ac129d8e4f8e2b7c36332501337438f47f9850', 'N'),
     ('src/types/char.md', 'type.char.validity', 23, 27, 'a69df9ce40ce467f661a500d929cb63233632de5833833d431b60a0e66fab3e1', 'N'),
     ('src/types/str.md', 'type.str', 1, 3, 'dd1b0f1d46db04a479f224ae316ce6b4a34b119f2a9c54155bb4837c8c915854', 'I'),
     ('src/types/str.md', 'type.str.intro', 4, 14, '2ba8f67a1d4809c27b3a7962d37974155e72293371af4d1ccd2de1bb34c9db32', 'N X'),
     ('src/types/str.md', 'type.str.value', 15, 20, '27c489917bc56b30495de930512676a0bb758d6dab90969efc8faac7a0d496bc', 'N X'),
     ('src/types/str.md', 'type.str.unsized', 21, 25, '0f48578d808eb992cdb106968454c6160f273e352db93d13c843968278f52fab', 'N X'))
    LABELS = tuple(row[1] for row in PINS)
    PRIOR_HASHES = {'abi': 'c4e561a9a78002281c34cbc35fe7fcaee3f29e82c659b718844504514ac1d317',
     'aggregates': '8c83d418848de65f5251d3b23248dd09121d7ac82b1099739648de6a034a55b3',
     'blocks': '44247bc80441734de02b723d046fd34d801801ee2fda6c7d50fcd0154c5e3a90',
     'branching': 'b08afed11ff9bd56c0b15f31bbaa3109105181bde417f0565ff1082cf340e74a',
     'calls': '42efdae31602a2af4f2fc243d5a74404159f877b278820905a7b4086281d302e',
     'closures': 'b75b0495a40e114b0e698ac0e64a84a195ed0a1745f14a2b09f6b5ee043b4b64',
     'coercions': '7bc24b2e1a5b422ef29ce96bbcfb767c52aee9b9be6e4d75ea40c890f4d90526',
     'const-eval': '0ce718c6e2834b6c518ff43f2fed2b5de72fd75eccf01d4611c3217cbdf053c2',
     'destructors': 'af8e7a87b177e4c72efd7f0dddfe26c2a0390d710377a507f9765f3d1b9d188e',
     'expression-core': 'c97e514ad4cf1babc0dc6901c432dfac71f98522fcbfbeafde1b920056f5f18a',
     'fields': 'e53fbee9b1248907fb91f9524440cfa75e5991fa0146800c6f0be3f2c67541b2',
     'grouped': '5a9393145150885b8c9f7717e99000003751454bd0a322ed7152e24e4289e279',
     'literals': '159e5a9f425ac09e2b1b4ae96a8c8710264f5ebc3170ec97dfc48e1d947808d8',
     'loops': '4a77bd5239c74bde49e8ba7933248484acd773c4f08239ce6f1fce23efb2261a',
     'never-type': '5c2f254ae61a3838279518c4279654bfd2e5218a3a0492b5976153dab23a9ac5',
     'operators': '2d724517022229ad541ef7c2a66aafc414b13f56c68d3cb8ea633755d3996ba3',
     'path-expressions': '5e4dbc27da9288eab71e26816f052348b13dbedcc703244cafa4fa64fb6f993f',
     'ranges': '3017078dfc42f85b5e51849d80e1a7efc41a247bbb573f09a7c40b6c5826fdda',
     'structs': '5fb87b17efaa5ac3d34840f9735b2499760c748ba7d9a5fa6a8909f4321f621e',
     'subtyping': 'a7d6097d9ee152ae8490c09c692570cb4b65c78b61c68a33c2b89caee694d6cc',
     'tokens': 'ea7fa27afdf901b91464ead14c54883debd091819c25bf02870e7a434ae61785',
     'transfers': '5f29782852c84ccf19cd0fde0ef1c4d9f54a6813eda59d1d9f1686bd387b123d',
     'type-expressions': 'a15d70d0dbba1ff597de605fd8dbb31c7053934431e2b324a78fc49995d94611',
     'unsafe': 'bc752efc1f0172cd85dad9399a1309c06489b3b0e4d4f74884c8bc1ef58ebb58',
     'visibility': '49883324b912c7f107d3573d6b95a3b4cc52b3d97253653eb39dfee6af9cbd58'}

    def setUp(self):
        source = os.environ.get('JUSTRUST_WITNESS_REFERENCE_ROOT')
        if not source:
            self.skipTest('set JUSTRUST_WITNESS_REFERENCE_ROOT for pinned-source structural checks')
        self.source = Path(source)
        repo = Path(os.environ.get('JUSTRUST_WITNESS_REPO_ROOT', BASE.parent))
        self.index = repo / 'data/compiler-language-reference-rules.json'
        self.inventory = repo / 'data/compiler-language-inventory.json'

    def artifact(self, source=None):
        return build(self.index, self.inventory,
                     self.source if source is None else source, 'primitive-types')

    def verify(self, data, source=None):
        return validate(data, self.index, self.inventory,
                        self.source if source is None else source, 'primitive-types')

    def test_independent_complete_page_label_block_classification_pins(self):
        import re
        import primitive_types_reconciliation_proposals as proposals
        self.assertEqual(proposals.SELECTED, self.PAGES)
        self.assertEqual(proposals.PAGE_SHA256, self.PAGE_HASHES)
        self.assertEqual(proposals.SOURCE_COMMIT, '86635e30bf861a038dc197d7e16fd09e7e514e7a')
        self.assertEqual(proposals.ALIASES, {
            'bool': 'reference-types-boolean-bool-values',
            'numeric': 'reference-types-numeric-numeric-domains',
            'char': 'reference-types-char-char-domain',
            'str': 'reference-types-str-str-invariants'})
        authored = [row.split('|') for row in proposals.ROWS.strip().splitlines()]
        self.assertEqual(tuple(row[0] for row in authored), self.LABELS)
        self.assertTrue(all(len(row) == 7 for row in authored))
        self.assertEqual(tuple(row[1] for row in authored), tuple(row[5] for row in self.PINS))
        data = self.artifact()
        counts = self.verify(data)
        self.assertEqual([counts[k] for k in ('selected_rule_rows', 'unselected_rule_identities',
                                             'global_rule_identities')], [42, 2923, 2965])
        actual = {(r['page'], r['rule']): r for r in data['rows']}
        self.assertEqual(len(actual), 42)
        self.assertEqual(len({r['id'] for r in actual.values()}), 42)
        self.assertEqual([(p['path'], p['sha256']) for p in data['selected_pages']],
                         [(page, self.PAGE_HASHES[page]) for page in self.PAGES])
        for page, line_count, label_count in zip(self.PAGES, (142, 57, 27, 25), (23, 10, 5, 4)):
            raw = (self.source / page).read_bytes()
            self.assertEqual(digest(raw), self.PAGE_HASHES[page])
            lines = raw.splitlines(keepends=True)
            self.assertEqual(len(lines), line_count)
            markers = [(i, m.group(1).decode()) for i, line in enumerate(lines, 1)
                       if (m := re.fullmatch(rb'r\[([^\]\r\n]+)\]\r?\n?', line))]
            expected = [(row[2], row[1]) for row in self.PINS if row[0] == page]
            self.assertEqual(markers, expected)
            self.assertEqual(len(markers), label_count)
            for pinned_page, label, start, end, block_hash, classes in self.PINS:
                if pinned_page != page:
                    continue
                row = actual[page, label]
                with self.subTest(rule=label):
                    self.assertEqual((row['line'], row['end_line']), (start, end))
                    self.assertEqual(digest(b''.join(lines[start - 1:end])), block_hash)
                    self.assertEqual(row['source_block_sha256'], block_hash)
                    self.assertEqual(row['classifications'], [proposals.CLASS_NAMES[c] for c in classes.split()])
                    self.assertEqual(row['id'], 'reconcile-primitive-types-' + label.encode().hex())
                    self.assertEqual(row['review_status'], 'unreviewed')
                    self.assertEqual(len(row['missing_test_needs']), 4)
                    for need, kind in zip(row['missing_test_needs'], ('positive', 'invalid', 'interaction', 'end-to-end')):
                        self.assertTrue(need.startswith(kind + ': Missing '))
                    self.assertIn('Never execute undefined behavior', row['missing_test_needs'][-1])
        self.assertEqual(counts['reviewed_rule_rows'], 0)
        self.assertEqual(counts['rows_with_exact_executable_evidence'], 0)
        universe = {(p['path'], r['label']) for p in json.loads(self.index.read_bytes())['pages'] +
                    json.loads(self.index.read_bytes())['unlinked_markdown_files'] for r in p['rules']}
        selected = set(actual)
        other = {(r['page'], r['rule']) for r in data['unselected_rules']}
        self.assertEqual((len(selected), len(other), len(universe)), (42, 2923, 2965))
        self.assertFalse(selected & other)
        self.assertEqual(selected | other, universe)
        self.assertEqual(self.artifact(), data)

    def test_each_artifact_row_omission_and_classification_forgery_rejected(self):
        baseline = self.artifact()
        for i, row in enumerate(baseline['rows']):
            with self.subTest(rule=row['rule'], mutation='omission'):
                changed = copy.deepcopy(baseline)
                changed['rows'].pop(i)
                self.assertNotEqual(changed, baseline)
                with self.assertRaises(ValueError):
                    self.verify(changed)
            with self.subTest(rule=row['rule'], mutation='classification'):
                changed = copy.deepcopy(baseline)
                changed['rows'][i]['classifications'] = ['grammar']
                self.assertNotEqual(changed, baseline)
                with self.assertRaises(ValueError):
                    self.verify(changed)

    def test_global_unique_additive_union_not_authored_sum(self):
        index = json.loads(self.index.read_bytes())
        pages = index['pages'] + index['unlinked_markdown_files']
        prior_pages = {p for name, _ in reconciliation.PROPOSAL_COUNTS if name not in ('trait-objects', 'type-parameters', 'nominal-types', 'inferred-types', 'primitive-types', 'sequence-types', 'pointer-types', 'function-pointers', 'function-items')
                       for p in reconciliation.profile_inputs(name)[0]}
        prior = {(p['path'], r['label']) for p in pages if p['path'] in prior_pages for r in p['rules']}
        current = {(r['page'], r['rule']) for r in self.artifact()['rows']}
        self.assertEqual((len(prior), len(current), len(prior & current), len(prior | current)),
                         (900, 42, 0, 942))
        summary = reconciliation.summarize_proposals(self.index, self.inventory, include_proposals=True)
        self.assertEqual((summary['proposed_rule_identities'], summary['rule_identities_without_proposals']),
                         (1030, 1935))
        self.assertEqual(sum(p['proposed_rule_identities'] for p in summary['profiles'].values()), 1035)
        for field in ('reviewed_rule_rows', 'rows_with_exact_executable_evidence', 'completed_rule_rows'):
            self.assertEqual(summary[field], 0)
            self.assertEqual(summary['profiles']['primitive-types'][field], 0)
        self.assertNotIn('src/types/textual.md', self.PAGES)

    def test_all_25_prior_canonical_bytes_without_new_module(self):
        self.assertEqual(len(self.PRIOR_HASHES), 25)
        self.assertEqual(set(self.PRIOR_HASHES), {name for name, _ in reconciliation.PROPOSAL_COUNTS
                                                if name not in ('trait-objects', 'type-parameters', 'nominal-types', 'inferred-types', 'primitive-types', 'sequence-types', 'pointer-types', 'function-pointers', 'function-items')})
        with mock.patch.dict(sys.modules, {'primitive_types_reconciliation_proposals': None}):
            for profile, sha in self.PRIOR_HASHES.items():
                with self.subTest(profile=profile):
                    data = build(self.index, self.inventory, self.source, profile)
                    validate(data, self.index, self.inventory, self.source, profile)
                    self.assertEqual(digest((json.dumps(data, indent=2) + '\n').encode()), sha)
            self.assertEqual(digest((json.dumps(build(self.index, self.inventory, self.source), indent=2) + '\n').encode()),
                             self.PRIOR_HASHES['tokens'])

    def test_each_source_page_fault_preserves_exact_error_types(self):
        data = self.artifact()
        for page in self.PAGES:
            for state, error in (('stale', ValueError), ('missing', FileNotFoundError),
                                 ('directory', IsADirectoryError), ('symlink', ValueError)):
                with self.subTest(page=page, state=state), tempfile.TemporaryDirectory() as temp:
                    root = Path(temp)
                    for name in self.PAGES:
                        target = root / name
                        target.parent.mkdir(parents=True, exist_ok=True)
                        if name != page:
                            target.write_bytes((self.source / name).read_bytes())
                        elif state == 'stale':
                            target.write_bytes((self.source / name).read_bytes() + b'\n')
                        elif state == 'directory':
                            target.mkdir()
                        elif state == 'symlink':
                            target.symlink_to(self.source / name)
                    for action in (lambda: self.artifact(root), lambda: self.verify(data, root)):
                        with self.assertRaises(error) as caught:
                            action()
                        self.assertIs(type(caught.exception), error)

    def test_rule_specific_semantic_caveats_remain_open(self):
        import primitive_types_reconciliation_proposals as proposals
        rows = {r.split('|')[0]: r.split('|') for r in proposals.ROWS.strip().splitlines()}
        phrases = {
            'type.bool.intro': ('primitive', 'exactly true and false'),
            'type.bool.layout': ('size 1 and alignment 1',),
            'type.bool.repr': ('0x00', '0x01', 'undefined behavior', '0x02 through 0xFF'),
            'type.bool.use-in-lazy-operator': ('not an enum', 'bool::true', 'eager bitwise', 'without importing linked short-circuit'),
            'type.bool.traits': ('Clone, Copy, Sized, Send and Sync', 'five listed traits', 'not all imported'),
            'type.bool.expr': ('Certain operator expressions', 'boolean logic', 'specific child attribution', 'not a standalone broad rejection'),
            'type.bool.expr.not': ('true to false and false to true',),
            'type.bool.expr.or': ('true,true,true,false', 'TT,TF,FT,FF', 'eager'),
            'type.bool.expr.and': ('true,false,false,false', 'TT,TF,FT,FF', 'eager'),
            'type.bool.expr.xor': ('false,true,true,false', 'TT,TF,FT,FF', 'eager'),
            'type.bool.expr.cmp.eq': ('true,false,false,true', 'TT,TF,FT,FF'),
            'type.bool.expr.cmp.greater': ('false,true,false,false', 'TT,TF,FT,FF'),
            'type.bool.expr.cmp.not-eq': ('!(a == b)',),
            'type.bool.expr.cmp.greater-eq': ('a == b bitwise-or a > b',),
            'type.bool.expr.cmp.less': ('!(a >= b)',),
            'type.bool.expr.cmp.less-eq': ('a == b bitwise-or a < b',),
            'type.bool.validity': ('initialized', 'inverse is not always sound', 'one-way'),
            'type.numeric.int.unsigned': ('u8,u16,u32,u64,u128', '2^8-1,2^16-1,2^32-1,2^64-1,2^128-1', 'minimum 0'),
            'type.numeric.int.signed': ("two's complement", 'i8,i16,i32,i64,i128', '-2^7,-2^15,-2^31,-2^63,-2^127', '2^7-1,2^15-1,2^31-1,2^63-1,2^127-1'),
            'type.numeric.float': ('IEEE 754-2008 binary32 and binary64', 'without inventing NaN or rounding'),
            'type.numeric.int.size.usize': ('not necessarily reversible', 'address versus pointer/provenance', 'universal 64-bit'),
            'type.numeric.int.size.isize': ('theoretical upper bound', 'one past the end', 'versus allocation feasibility'),
            'type.numeric.int.size.minimum': ('at least 16 bits', 'without assuming 16-bit ecosystem support'),
            'type.numeric.validity': ('all byte patterns', 'only with initialized bytes', 'uninitialized u8 is invalid'),
            'type.char.intro': ('Unicode scalar value', 'not a surrogate', 'versus grapheme or byte'),
            'type.char.value': ('0x0000..=0xD7FF or 0xE000..=0x10FFFF', 'immediate undefined behavior', '0xD800..=0xDFFF'),
            'type.char.layout': ('same size and alignment as u32', 'not assumed alignment 4'),
            'type.char.validity': ('Every char byte is initialized', 'inverse transmute is not always sound', 'one-way'),
            'type.str.value': ('same as [u8]', 'assume and ensure valid UTF-8', 'now or in the future', 'without executing methods'),
            'type.str.unsized': ('dynamically sized', 'only through a pointer', '&str has the same layout as &[u8]'),
        }
        for label, needs in phrases.items():
            for phrase in needs:
                with self.subTest(label=label, phrase=phrase):
                    self.assertIn(phrase, '|'.join(rows[label]))
        for phrase in ('42 labels', 'UNREVIEWED', 'not complete primitive support', 'not contract identity',
                       'context only', 'Legacy src/types/textual.md is not selected', 'not semantic equivalence',
                       'not execution-order proof imported from these tables'):
            self.assertIn(phrase, ' '.join(proposals.LIMITATIONS))

    def test_permuted_authored_rows_preserve_canonical_order(self):
        import primitive_types_reconciliation_proposals as proposals
        baseline = self.artifact()
        rows = proposals.ROWS.strip().splitlines()
        for changed in (rows[::-1], rows[2:] + rows[:2], rows[::2] + rows[1::2]):
            with mock.patch.object(proposals, 'ROWS', '\n'.join(changed)):
                data = self.artifact()
                self.assertEqual(data, baseline)
                self.assertEqual(tuple(r['rule'] for r in data['rows']),
                                 tuple(row[1] for row in sorted(self.PINS, key=lambda row: (row[0], row[2]))))
                self.assertEqual(self.verify(data), self.verify(baseline))


    def test_missing_duplicate_foreign_malformed_authored_rows_rejected(self):
        import primitive_types_reconciliation_proposals as proposals
        valid = self.artifact()
        rows = proposals.ROWS.strip().splitlines()
        self.assertEqual(tuple(r.split('|')[0] for r in rows), self.LABELS)
        cases = [rows[4:], rows[:4], rows[:-1], [], rows + [rows[0]],
                 ['expr.foreign' + rows[0][len('type'):], *rows[1:]],
                 ['|'.join(rows[0].split('|')[:-1]), *rows[1:]],
                 [rows[0] + '|extra', *rows[1:]]]
        cases.extend(rows[:i] + rows[i + 1:] for i in range(len(rows)))
        for col in (4, 5, 6):
            fields = rows[0].split('|')
            fields[col] = '   '
            cases.append(['|'.join(fields), *rows[1:]])
        for i, changed in enumerate(cases):
            with self.subTest(case=i), mock.patch.object(proposals, 'ROWS', '\n'.join(changed)):
                with self.assertRaises(ValueError):
                    self.artifact()
                with self.assertRaises(ValueError):
                    self.verify(valid)


    def test_29_actual_changing_forgeries_rejected(self):
        baseline = self.artifact()
        mutations = (
            lambda d: d['rows'].pop(),
            lambda d: d['rows'].append(copy.deepcopy(d['rows'][0])),
            lambda d: d['rows'].reverse(),
            lambda d: d['rows'][0].update(page='src/expressions/block-expr.md'),
            lambda d: d['rows'][4].update(page='src/expressions/struct-expr.md'),
            lambda d: d['rows'][0].update(rule='expr.loop'),
            lambda d: d['rows'][0].update(line=True),
            lambda d: d['rows'][0].update(line=1.0),
            lambda d: d['counts'].update(reviewed_rule_rows=False),
            lambda d: d['rows'][0].update(relationship='exact-executable-evidence'),
            lambda d: d['rows'][0].update(classifications=['grammar']),
            lambda d: d['rows'][0].update(end_line=99999),
            lambda d: d['rows'][0].update(source_block_sha256='0' * 64),
            lambda d: d['rows'][4].update(source_block_sha256='0' * 64),
            lambda d: d['rows'][0].update(id='reconcile-primitive-types-expr'),
            lambda d: d['rows'][0].update(id=d['rows'][1]['id']),
            lambda d: d['rows'][0].update(review_status='reviewed'),
            lambda d: d['rows'][0].update(rationale='invented interpretation'),
            lambda d: d['rows'][0].update(candidate_obligation_ids=['foreign']),
            lambda d: d['rows'][0].update(missing_test_needs=[]),
            lambda d: d['rows'][0]['missing_test_needs'].__setitem__(0, 'positive: implemented'),
            lambda d: d['counts'].update(reviewed_rule_rows=1),
            lambda d: d['counts'].update(rows_with_exact_executable_evidence=1),
            lambda d: d['selected_pages'][0].update(sha256='0' * 64),
            lambda d: d['selected_pages'].clear(),
            lambda d: d['bindings'].update(reference_commit='0' * 40),
            lambda d: d['unselected_rules'].pop(),
            lambda d: d['unselected_rules'].append(copy.deepcopy(d['unselected_rules'][0])),
            lambda d: d['unselected_rules'][0].update(page=d['rows'][0]['page'], rule=d['rows'][0]['rule']),
        )
        self.assertEqual(len(mutations), 29)
        for i, mutate in enumerate(mutations):
            changed = copy.deepcopy(baseline)
            mutate(changed)
            self.assertNotEqual(json.dumps(changed, sort_keys=True), json.dumps(baseline, sort_keys=True))
            with self.subTest(mutation=i), self.assertRaises(ValueError):
                self.verify(changed)


    def test_source_only_restrictions_precede_io_and_execution(self):
        args = ['--profile', 'primitive-types', '--rule-index', '/absent/index',
                '--inventory', '/absent/inventory', '--source-root', '/absent/source',
                '--artifact', '/absent/artifact']
        extras = ([], ['--repo-root', '/absent/repo'], ['--evidence', '/absent/evidence'],
                  ['--repo-root', '/absent/repo', '--evidence', '/absent/evidence'])
        cases = [(action, extra) for action in ('generate-witness-links', 'build-witness-links',
                  'validate-witness-links', 'assess-witness-links') for extra in extras]
        cases += [(action, extra) for action in ('generate', 'validate') for extra in extras[1:]]
        with contextlib.ExitStack() as stack:
            for name in ('read_bytes', 'read_text', 'open', 'is_symlink', 'is_file',
                         'write_bytes', 'write_text', 'mkdir'):
                stack.enter_context(mock.patch.object(Path, name, side_effect=AssertionError('IO')))
            stack.enter_context(mock.patch.object(subprocess, 'run', side_effect=AssertionError('execution')))
            for action, extra in cases:
                with self.subTest(action=action, extra=extra), self.assertRaises(ValueError):
                    main([action, *args, *extra])
            for call in (
                lambda: reconciliation.build_witness_links(
                    self.index, self.inventory, self.source, self.source, 'primitive-types'),
                lambda: reconciliation.validate_witness_links(
                    {}, self.index, self.inventory, self.source, self.source, 'primitive-types'),
                lambda: reconciliation.assess_witness_links(
                    {}, self.index, self.inventory, self.source, self.source, (), 'primitive-types'),
            ):
                with self.assertRaises(ValueError):
                    call()


    def test_cli_exclusive_generation_validation_and_no_repair(self):
        with tempfile.TemporaryDirectory() as temp:
            artifact = Path(temp) / 'primitive-types.json'
            args = ['--profile', 'primitive-types', '--rule-index', str(self.index),
                    '--inventory', str(self.inventory), '--source-root', str(self.source),
                    '--artifact', str(artifact)]
            expected = (json.dumps(self.artifact(), indent=2) + '\n').encode()
            with contextlib.redirect_stdout(io.StringIO()) as output:
                main(['generate', *args])
            self.assertEqual(artifact.read_bytes(), expected)
            self.assertEqual(json.loads(output.getvalue()), self.verify(self.artifact()))
            with contextlib.redirect_stdout(io.StringIO()), self.assertRaises(FileExistsError):
                main(['generate', *args])
            self.assertEqual(artifact.read_bytes(), expected)
            with contextlib.redirect_stdout(io.StringIO()) as output:
                main(['validate', *args])
            self.assertEqual(json.loads(output.getvalue()), self.verify(self.artifact()))
            self.assertEqual(artifact.read_bytes(), expected)
            forged = json.loads(expected)
            forged['counts']['reviewed_rule_rows'] = 1
            artifact.write_text(json.dumps(forged))
            before = artifact.read_bytes()
            with contextlib.redirect_stdout(io.StringIO()), self.assertRaises(ValueError):
                main(['validate', *args])
            self.assertEqual(artifact.read_bytes(), before)


    def test_actual_process_cli_generate_validate(self):
        with tempfile.TemporaryDirectory() as temp:
            artifact = Path(temp) / 'primitive-types.json'
            args = [sys.executable, str(BASE / 'reconcile_token_rules.py'),
                    '--profile', 'primitive-types', '--rule-index', str(self.index),
                    '--inventory', str(self.inventory), '--source-root', str(self.source),
                    '--artifact', str(artifact)]
            expected = (json.dumps(self.artifact(), indent=2) + '\n').encode()
            for action in ('generate', 'validate'):
                result = subprocess.run([*args[:2], action, *args[2:]],
                                        capture_output=True, text=True, check=False)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertEqual(json.loads(result.stdout), self.verify(self.artifact()))
                self.assertEqual(artifact.read_bytes(), expected)



class SequenceTypesProposalTests(unittest.TestCase):
    """Pinned source-only proposals, never evidence of sequence implementation."""
    PAGES = ('src/types/array.md', 'src/types/slice.md', 'src/types/tuple.md')
    PAGE_HASHES = {
        'src/types/array.md': '82482ed63dda5923c9bb408a57af7f22795d5b027f2b0118e35bfaa74a10b5d3',
        'src/types/slice.md': 'ad59ee3e089e43b287eabf1789829f55627c3b3eca577968ad4add918d7e313a',
        'src/types/tuple.md': '5ba695522d4226449499ccaf2d46920fc2a1dab231ff3a88690379fbfc0fd61a',
    }
    # Literal page/block/line/classification pins independent of the proposal module.
    PINS = (
        ('src/types/array.md', 'type.array', 1, 3, '6b5a9d767288e6f5038dddaa417cf6229cc4e3312da62b2a5abd555dffdfe568', 'I'),
        ('src/types/array.md', 'type.array.syntax', 4, 8, '11d27ad53c57510dd269134a10fbcf1a8bc2efeeaea586b87f2bff8a03a2bf71', 'G X'),
        ('src/types/array.md', 'type.array.intro', 9, 11, '9e01f6320faa9f63c7811e531f8d7da93d2456da77d89c88aaabb507dea1f6c4', 'N'),
        ('src/types/array.md', 'type.array.constraint', 12, 24, '17da9229a7f2ed7b5b342df2ce11c2bb85ac96afb903049cbc7f89fab7759e74', 'N X'),
        ('src/types/array.md', 'type.array.index', 25, 32, 'f15474710f1caa01239373c19fbadf74e648c870d959f7f2cdc9f0df857b7326', 'N X'),
        ('src/types/slice.md', 'type.slice', 1, 3, '9d3732cffd632ececc68bc30e927c54730e5b7c199c23f43b7799278a18406bc', 'I'),
        ('src/types/slice.md', 'type.slice.syntax', 4, 8, '5e45ba7a33b317a90c497d59a8108f535ed91c3f065129777230a68e776ea1ec', 'G X'),
        ('src/types/slice.md', 'type.slice.intro', 9, 11, 'b33ad1af520575aec754f995e2dc440b2fe95e72cf220b2536f52d0af8eada34', 'N X'),
        ('src/types/slice.md', 'type.slice.unsized', 12, 28, 'a6e6757e05d7fbed3c97ddb35810cf51d322bd6d9b11d81fde7a315b957a6025', 'N X'),
        ('src/types/slice.md', 'type.slice.safe', 29, 32, 'cac151b7c6fe3f9419534599a7be0d307e63c7edb206a9841a207b6106c9ca7d', 'N'),
        ('src/types/tuple.md', 'type.tuple', 1, 3, '6e163f4e6818283b5ac39933069832664e9b19fe23da263a26908e81c5b0d0ee', 'I'),
        ('src/types/tuple.md', 'type.tuple.syntax', 4, 10, '0bfa53f0f92a6abad61d7c875ab2ad2d50b2977c560f8b86fd28790f1902d3ae', 'G X'),
        ('src/types/tuple.md', 'type.tuple.intro', 11, 15, 'c60461a5b493dbf3a06a358aa55d72333b9378fbae4f3957492a43cfafc2545a', 'N'),
        ('src/types/tuple.md', 'type.tuple.restriction', 16, 18, '87ec5e0ccadb48769dbfcdde92d5a07db691e3b7cc86a0dd4829db9529ce072c', 'N X'),
        ('src/types/tuple.md', 'type.tuple.field-number', 19, 21, 'b87988b1833380638e4d896ad97670bbea960ed55bfc59429ca07bd5e09646ea', 'N'),
        ('src/types/tuple.md', 'type.tuple.field-name', 22, 24, '9033357d229a70e1f1517d795f000713a5323049aa1d358b2899e528135da988', 'N'),
        ('src/types/tuple.md', 'type.tuple.unit', 25, 36, 'c802ebecb12a826c7d6f3f16f7cb3ad6fad7f64c902dea6306be084284607e75', 'N'),
        ('src/types/tuple.md', 'type.tuple.constructor', 37, 39, 'f215a792d3fc6e95ee7f764e6bef5e1ae6b0653870d87217117655b98688d7ac', 'N X'),
        ('src/types/tuple.md', 'type.tuple.access', 40, 49, '3ac9dd0c3e95ebf355f5e7d63eb07c4f41fb522fc184e3d9276cc40f2fe39156', 'N X'),
    )
    PRIOR_HASHES = dict(PrimitiveTypesProposalTests.PRIOR_HASHES, **{
        'primitive-types': '2e867b91d4e8ab2021e8894bf7f6237d3a92436011e00ab6a048bae61f208bf2'})

    setUp = PrimitiveTypesProposalTests.setUp

    def artifact(self, source=None):
        return build(self.index, self.inventory,
                     self.source if source is None else source, 'sequence-types')

    def verify(self, data, source=None):
        return validate(data, self.index, self.inventory,
                        self.source if source is None else source, 'sequence-types')

    def test_independent_complete_page_label_block_classification_pins(self):
        import re
        import sequence_types_reconciliation_proposals as proposals
        self.assertEqual(proposals.SELECTED, self.PAGES)
        self.assertEqual(proposals.PAGE_SHA256, self.PAGE_HASHES)
        self.assertEqual(proposals.SOURCE_COMMIT, '86635e30bf861a038dc197d7e16fd09e7e514e7a')
        self.assertEqual(proposals.ALIASES, {
            'array': 'reference-types-array-array-identity',
            'slice': 'reference-types-slice-slice-metadata',
            'tuple': 'reference-types-tuple-tuple-identity'})
        class_names = {'N': 'normative-semantic', 'G': 'grammar', 'I': 'index',
                       'X': 'cross-reference', 'E': 'edition-context'}
        self.assertEqual(proposals.CLASS_NAMES, class_names)
        authored = [row.split('|') for row in proposals.ROWS.strip().splitlines()]
        self.assertTrue(all(len(row) == 7 and all(f.strip() for f in row) for row in authored))
        self.assertEqual(tuple((r[0], r[1]) for r in authored), tuple((r[1], r[5]) for r in self.PINS))
        self.assertEqual(reconciliation.SOURCE_ONLY_PAGE_COUNTS['sequence-types'], (5, 5, 9))
        data = self.artifact()
        counts = self.verify(data)
        self.assertEqual([counts[k] for k in ('selected_rule_rows', 'unselected_rule_identities',
                                             'global_rule_identities')], [19, 2946, 2965])
        self.assertEqual(counts['proposed_classification_counts'], {
            'normative-semantic': 13, 'grammar': 3, 'index': 3,
            'cross-reference': 10, 'edition-context': 0})
        actual = {(r['page'], r['rule']): r for r in data['rows']}
        self.assertEqual(len(actual), 19)
        self.assertEqual(len({r['id'] for r in actual.values()}), 19)
        self.assertEqual(tuple(r['rule'] for r in data['rows']), tuple(r[1] for r in self.PINS))
        self.assertEqual([(p['path'], p['sha256']) for p in data['selected_pages']],
                         [(page, self.PAGE_HASHES[page]) for page in self.PAGES])
        for page, line_count, label_count in zip(self.PAGES, (32, 32, 49), (5, 5, 9)):
            raw = (self.source / page).read_bytes()
            self.assertEqual(digest(raw), self.PAGE_HASHES[page])
            lines = raw.splitlines(keepends=True)
            self.assertEqual(len(lines), line_count)
            markers = [(i, m.group(1).decode()) for i, line in enumerate(lines, 1)
                       if (m := re.fullmatch(rb'r\[([^\]\r\n]+)\]\r?\n?', line))]
            self.assertEqual(markers, [(r[2], r[1]) for r in self.PINS if r[0] == page])
            self.assertEqual(len(markers), label_count)
            for pinned_page, label, start, end, block_hash, classes in self.PINS:
                if pinned_page != page:
                    continue
                row = actual[page, label]
                with self.subTest(rule=label):
                    self.assertEqual((row['line'], row['end_line']), (start, end))
                    self.assertEqual(digest(b''.join(lines[start - 1:end])), block_hash)
                    self.assertEqual(row['source_block_sha256'], block_hash)
                    self.assertEqual(row['classifications'], [class_names[c] for c in classes.split()])
                    self.assertEqual(row['id'], 'reconcile-sequence-types-' + label.encode().hex())
                    self.assertEqual(row['candidate_obligation_ids'], [proposals.ALIASES[label.split('.')[1]]])
                    self.assertEqual(row['review_status'], 'unreviewed')
                    self.assertEqual(row['relationship'], 'context-only' if classes == 'I'
                                     else 'proposed-partial-decomposition')
                    self.assertEqual(len(row['missing_test_needs']), 4)
                    for need, kind in zip(row['missing_test_needs'], ('positive', 'invalid', 'interaction', 'end-to-end')):
                        self.assertTrue(need.startswith(kind + ': Missing '))
                    for phrase in ('stable1.98.1 eligibility', 'workers1/8', 'edition2024',
                                   'Linux x86-64', 'Never execute undefined behavior',
                                   'CHECK must not demand codegen', 'not necessarily fail compilation'):
                        self.assertIn(phrase, row['missing_test_needs'][-1])
        for kind in ('reviewed_rule_rows', 'rows_with_exact_executable_evidence'):
            self.assertEqual(counts[kind], 0)
        index = json.loads(self.index.read_bytes())
        universe = {(p['path'], r['label']) for p in index['pages'] + index['unlinked_markdown_files']
                    for r in p['rules']}
        selected = set(actual)
        other = {(r['page'], r['rule']) for r in data['unselected_rules']}
        self.assertEqual((len(selected), len(other), len(universe)), (19, 2946, 2965))
        self.assertFalse(selected & other)
        self.assertEqual(selected | other, universe)
        self.assertEqual(self.artifact(), data)

    def test_each_artifact_row_omission_and_actual_classification_forgery_rejected(self):
        baseline = self.artifact()
        for i, row in enumerate(baseline['rows']):
            for kind in ('omission', 'classification'):
                changed = copy.deepcopy(baseline)
                if kind == 'omission':
                    changed['rows'].pop(i)
                else:
                    changed['rows'][i]['classifications'] = (
                        ['index'] if row['classifications'] != ['index'] else ['grammar'])
                self.assertNotEqual(changed, baseline)
                with self.subTest(rule=row['rule'], mutation=kind), self.assertRaises(ValueError):
                    self.verify(changed)

    def test_global_unique_additive_union_not_authored_sum(self):
        index = json.loads(self.index.read_bytes())
        pages = index['pages'] + index['unlinked_markdown_files']
        prior_pages = {p for name, _ in reconciliation.PROPOSAL_COUNTS if name not in ('trait-objects', 'type-parameters', 'nominal-types', 'inferred-types', 'sequence-types', 'pointer-types', 'function-pointers', 'function-items')
                       for p in reconciliation.profile_inputs(name)[0]}
        prior = {(p['path'], r['label']) for p in pages if p['path'] in prior_pages for r in p['rules']}
        current = {(r['page'], r['rule']) for r in self.artifact()['rows']}
        self.assertEqual((len(prior), len(current), len(prior & current), len(prior | current)),
                         (942, 19, 0, 961))
        summary = reconciliation.summarize_proposals(self.index, self.inventory, include_proposals=True)
        self.assertEqual((summary['proposed_rule_identities'], summary['rule_identities_without_proposals']),
                         (1030, 1935))
        self.assertEqual(sum(p['proposed_rule_identities'] for p in summary['profiles'].values()), 1035)
        sequence = summary['profiles']['sequence-types']
        self.assertEqual(len(sequence['authored_proposals']), 19)
        for field in ('reviewed_rule_rows', 'rows_with_exact_executable_evidence', 'completed_rule_rows'):
            self.assertEqual(summary[field], 0)
            self.assertEqual(sequence[field], 0)
        self.assertIsNone(sequence['full_rust_completion_percent'])
        for page, count in zip(self.PAGES, (5, 5, 9)):
            result = reconciliation.query_rule_backlog(self.index, self.inventory, page=page,
                                                       include_proposals=True)
            self.assertEqual((result['selected_rule_identities'], result['selected_authored_proposal_count']),
                             (count, count))
            self.assertEqual(result['selected_rule_identities_without_proposals'], 0)
            self.assertTrue(all(row['proposal_profiles'] == ['sequence-types'] for row in result['rows']))
            missing = reconciliation.query_rule_backlog(self.index, self.inventory, page=page, missing_only=True)
            self.assertEqual(missing['rows'], [])

    def test_all_26_prior_canonical_bytes_without_new_module(self):
        self.assertEqual(len(self.PRIOR_HASHES), 26)
        self.assertEqual(set(self.PRIOR_HASHES), {name for name, _ in reconciliation.PROPOSAL_COUNTS
                                                if name not in ('trait-objects', 'type-parameters', 'nominal-types', 'inferred-types', 'sequence-types', 'pointer-types', 'function-pointers', 'function-items')})
        with mock.patch.dict(sys.modules, {'sequence_types_reconciliation_proposals': None}):
            for profile, sha in self.PRIOR_HASHES.items():
                with self.subTest(profile=profile):
                    data = build(self.index, self.inventory, self.source, profile)
                    validate(data, self.index, self.inventory, self.source, profile)
                    self.assertEqual(digest((json.dumps(data, indent=2) + '\n').encode()), sha)
            self.assertEqual(digest((json.dumps(build(self.index, self.inventory, self.source), indent=2) + '\n').encode()),
                             self.PRIOR_HASHES['tokens'])

    test_each_source_page_fault_preserves_exact_error_types = (
        PrimitiveTypesProposalTests.test_each_source_page_fault_preserves_exact_error_types)
    test_29_actual_changing_forgeries_rejected = (
        PrimitiveTypesProposalTests.test_29_actual_changing_forgeries_rejected)

    def test_permuted_authored_rows_preserve_canonical_order(self):
        import sequence_types_reconciliation_proposals as proposals
        baseline = self.artifact()
        rows = proposals.ROWS.strip().splitlines()
        for changed in (rows[::-1], rows[2:] + rows[:2], rows[::2] + rows[1::2]):
            with mock.patch.object(proposals, 'ROWS', '\n'.join(changed)):
                data = self.artifact()
                self.assertEqual(data, baseline)
                self.assertEqual(tuple(r['rule'] for r in data['rows']), tuple(r[1] for r in self.PINS))
                self.assertEqual(self.verify(data), self.verify(baseline))

    def test_missing_duplicate_foreign_malformed_authored_rows_rejected(self):
        import sequence_types_reconciliation_proposals as proposals
        valid = self.artifact()
        rows = proposals.ROWS.strip().splitlines()
        cases = [[], rows + [rows[0]], ['foreign.rule' + rows[0][len('type.array'):], *rows[1:]],
                 ['|'.join(rows[0].split('|')[:-1]), *rows[1:]], [rows[0] + '|extra', *rows[1:]]]
        cases.extend(rows[:i] + rows[i + 1:] for i in range(len(rows)))
        for col in (4, 5, 6):
            fields = rows[0].split('|')
            fields[col] = '   '
            cases.append(['|'.join(fields), *rows[1:]])
        for i, changed in enumerate(cases):
            with self.subTest(case=i), mock.patch.object(proposals, 'ROWS', '\n'.join(changed)):
                with self.assertRaises(ValueError):
                    self.artifact()
                with self.assertRaises(ValueError):
                    self.verify(valid)

    def test_rule_specific_semantic_caveats_remain_open(self):
        import sequence_types_reconciliation_proposals as proposals
        rows = {r.split('|')[0]: r.split('|') for r in proposals.ROWS.strip().splitlines()}
        phrases = {
            'type.array': ('not executable behavior',),
            'type.array.syntax': ('Type semicolon Expression', 'does not grant nonconstant lengths', 'constant-usize'),
            'type.array.intro': ('fixed-size', 'N elements', 'not byte count', 'homogeneous'),
            'type.array.constraint': ('constant expression evaluating to usize', 'nonconstant lengths', 'Box', 'without importing'),
            'type.array.index': ('initialized', 'safe methods and operators', 'possible runtime panic', 'not all compile-invalid', 'without executing undefined behavior', 'without mandatory unoptimized runtime-check nodes', 'Vec context'),
            'type.slice': ('not metadata', 'partial metadata-named alias'),
            'type.slice.syntax': ('bracket Type bracket', '[T; N]'),
            'type.slice.intro': ('dynamically sized', 'view', 'homogeneous', 'without importing linked DST'),
            'type.slice.unsized': ('generally used through pointers', '&[T] borrows without owning', '&mut [T] mutably borrows', 'Box<[T]>', 'generally versus only', 'no specified fat-pointer layout or metadata'),
            'type.slice.safe': ('initialized', 'possible runtime panic', 'not universal compile errors', 'without executing undefined behavior', 'without mandatory runtime-check nodes'),
            'type.tuple': ('not executable behavior',),
            'type.tuple.syntax': ('exactly two alternatives', '()', '(Type comma)+ followed by optional Type', '(i32,)', '(i32, bool,)', '(i32) is a one-element tuple'),
            'type.tuple.intro': ('structural', 'heterogeneous', 'equivalent ordered internal types', 'Eq trait or layout equivalence', 'nominal tuple-struct'),
            'type.tuple.restriction': ('one-element tuple requires a comma', 'parenthesized type', '(i32,)'),
            'type.tuple.field-number': ('length of its type list', 'arity', 'versus storage size'),
            'type.tuple.field-name': ('starting at 0', 'same position', 'positional field types', 'not physical offsets'),
            'type.tuple.unit': ('arity zero', 'one value', 'without invented unit size or layout', '(String, i32)', '(i32, String)'),
            'type.tuple.constructor': ('tuple expressions', 'unit', 'context only', 'not imported'),
            'type.tuple.access': ('tuple index expressions or pattern matching', 'structural-type footnote', 'context only, not imported contracts', 'Eq trait or layout equivalence'),
        }
        self.assertEqual(set(phrases), set(rows))
        for label, needs in phrases.items():
            for phrase in needs:
                with self.subTest(label=label, phrase=phrase):
                    self.assertIn(phrase, '|'.join(rows[label]))
        for phrase in ('19 labels', 'UNREVIEWED', 'not complete Rust sequences', 'partial routing',
                       'not contract identity', 'context only, not imported contracts',
                       'no fat-pointer layout or metadata contract', 'do not establish full standard-library support'):
            self.assertIn(phrase, ' '.join(proposals.LIMITATIONS))

    def test_source_only_restrictions_precede_io_and_execution(self):
        args = ['--profile', 'sequence-types', '--rule-index', '/absent/index',
                '--inventory', '/absent/inventory', '--source-root', '/absent/source',
                '--artifact', '/absent/artifact']
        extras = ([], ['--repo-root', '/absent/repo'], ['--evidence', '/absent/evidence'],
                  ['--repo-root', '/absent/repo', '--evidence', '/absent/evidence'])
        cases = [(action, extra) for action in ('generate-witness-links', 'build-witness-links',
                  'validate-witness-links', 'assess-witness-links') for extra in extras]
        cases += [(action, extra) for action in ('generate', 'validate') for extra in extras[1:]]
        with contextlib.ExitStack() as stack:
            for name in ('read_bytes', 'read_text', 'open', 'is_symlink', 'is_file',
                         'write_bytes', 'write_text', 'mkdir'):
                stack.enter_context(mock.patch.object(Path, name, side_effect=AssertionError('IO')))
            stack.enter_context(mock.patch.object(subprocess, 'run', side_effect=AssertionError('execution')))
            for action, extra in cases:
                with self.subTest(action=action, extra=extra), self.assertRaises(ValueError):
                    main([action, *args, *extra])
            for call in (
                lambda: reconciliation.build_witness_links(
                    self.index, self.inventory, self.source, self.source, 'sequence-types'),
                lambda: reconciliation.validate_witness_links(
                    {}, self.index, self.inventory, self.source, self.source, 'sequence-types'),
                lambda: reconciliation.assess_witness_links(
                    {}, self.index, self.inventory, self.source, self.source, (), 'sequence-types'),
            ):
                with self.assertRaises(ValueError):
                    call()

    def test_cli_exclusive_generation_validation_and_no_repair(self):
        with tempfile.TemporaryDirectory() as temp:
            artifact = Path(temp) / 'sequence-types.json'
            args = ['--profile', 'sequence-types', '--rule-index', str(self.index),
                    '--inventory', str(self.inventory), '--source-root', str(self.source),
                    '--artifact', str(artifact)]
            expected = (json.dumps(self.artifact(), indent=2) + '\n').encode()
            with contextlib.redirect_stdout(io.StringIO()) as output:
                main(['generate', *args])
            self.assertEqual(artifact.read_bytes(), expected)
            self.assertEqual(json.loads(output.getvalue()), self.verify(self.artifact()))
            with contextlib.redirect_stdout(io.StringIO()), self.assertRaises(FileExistsError):
                main(['generate', *args])
            self.assertEqual(artifact.read_bytes(), expected)
            with contextlib.redirect_stdout(io.StringIO()) as output:
                main(['validate', *args])
            self.assertEqual(json.loads(output.getvalue()), self.verify(self.artifact()))
            self.assertEqual(artifact.read_bytes(), expected)
            forged = json.loads(expected)
            forged['counts']['reviewed_rule_rows'] = 1
            artifact.write_text(json.dumps(forged))
            before = artifact.read_bytes()
            with contextlib.redirect_stdout(io.StringIO()), self.assertRaises(ValueError):
                main(['validate', *args])
            self.assertEqual(artifact.read_bytes(), before)

    def test_actual_process_cli_generate_validate(self):
        with tempfile.TemporaryDirectory() as temp:
            artifact = Path(temp) / 'sequence-types.json'
            args = [sys.executable, str(BASE / 'reconcile_token_rules.py'),
                    '--profile', 'sequence-types', '--rule-index', str(self.index),
                    '--inventory', str(self.inventory), '--source-root', str(self.source),
                    '--artifact', str(artifact)]
            expected = (json.dumps(self.artifact(), indent=2) + '\n').encode()
            for action in ('generate', 'validate'):
                result = subprocess.run([*args[:2], action, *args[2:]],
                                        capture_output=True, text=True, check=False)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertEqual(json.loads(result.stdout), self.verify(self.artifact()))
                self.assertEqual(artifact.read_bytes(), expected)


class PointerTypesProposalTests(unittest.TestCase):
    """Complete pinned source-only pointer proposals, never compiler evidence."""
    PAGES = ('src/types/pointer.md',)
    PAGE_HASHES = {'src/types/pointer.md': '8c29e8796f8faabe789e98ad7ab2f7a139b572993bbeeb5dfe6c006c64e946ac'}
    # Independent literal source/block/line and authored classification pins.
    PINS = (
        ('type.pointer', 1, 3, '7fd86d36b4ec4fc20f2dd72f8b1f9b11888aa11dfe326a6f3071a20df1740527', 'I'),
        ('type.pointer.intro', 4, 6, 'f06a519b621e476a822351c7ff0347af34e3598414e9a8f6858c9129a153fced', 'N'),
        ('type.pointer.reference', 7, 9, '43a18cb05de0e5f27b644f9a3bb7ff018d805eeae4400b961bf293cac5cadbd7', 'I'),
        ('type.pointer.reference.syntax', 10, 14, '552d1f73bce63cbae67535ce49f96a6e4e3f422215a4c97aa68f41b8749e7307', 'G X'),
        ('type.pointer.reference.shared', 15, 17, 'ff86ce29ae01f7989bd7c6be1aaf833e0f559fd0217ab6ddd069fd002ad26d55', 'I'),
        ('type.pointer.reference.shared.intro', 18, 20, 'b1c3ff52d4ff1296265ab6b6a5070889658dbe22c7ff322a8fa01ff8e4d6d05f', 'N'),
        ('type.pointer.reference.shared.constraint-mutation', 21, 23, '76ed9f660ffead7e7684d08b2fd25aacec2181c13bc542763a45019870726077', 'N X'),
        ('type.pointer.reference.shared.copy', 24, 26, 'bfe7351c4d9f3792dc846aabaccced92bb5debb9d528a912ad3893ede8553ce8', 'N X'),
        ('type.pointer.reference.mut', 27, 29, 'ee8c8401055d38737c46a3c2fdee389fc25f6dcd8698d8a654b2addfd7a2d981', 'I'),
        ('type.pointer.reference.mut.intro', 30, 32, '0d0834115dd62d7ec0e063c7f4de65541e3e298e28d041dd22a5e71090308547', 'N'),
        ('type.pointer.reference.mut.copy', 33, 35, 'd8162e9ce864cfbbc89bac6768e657bf75a7b9d6a2a1304c13dddd58e417a63c', 'N'),
        ('type.pointer.raw', 36, 38, '69a513e57b5361270395327725175cb3b231d8b484a0347391e5c9fe505a82d8', 'I'),
        ('type.pointer.raw.syntax', 39, 43, 'b52d58377e197f0186fd77596af6fcda4ecd0c4da86f7ad099c1f1145f8d52e1', 'G X'),
        ('type.pointer.raw.intro', 44, 46, '4136ef510f3fbab925baee113c6f800eb6909eeccc62ce0f34551e3854005cf4', 'N'),
        ('type.pointer.raw.copy', 47, 49, 'f76eda38b8a4fa2be2ee370e6f1b59dca2f2bce560af74006d06a1145610cb39', 'N'),
        ('type.pointer.raw.safety', 50, 54, 'af4e88a7c4fe217760b5139df30c653de5a4cd98031a2319f1c98a51832145b8', 'N X'),
        ('type.pointer.raw.cmp', 55, 57, '8530ba4fabf4e041b4f9d7d27c13e741f356068d841d573adbcf10c629adda48', 'N X'),
        ('type.pointer.raw.constructor', 58, 60, 'c5b06dfeb1d84626e647704fb4c73bada3586c62497f4a6092e5010cc8399f03', 'N'),
        ('type.pointer.smart', 61, 65, '5a26e6efe6ff680db2772d5b52f12b503db546d618277026d44c43d5bd41fdf6', 'N I'),
        ('type.pointer.validity', 66, 68, '000b58d906e431bb4370afcf1a5b5c66b9cf0c5ca2ac4885ae3b96f28fab1cba', 'I'),
        ('type.pointer.validity.pointer-fragment', 69, 71, '9ff4c9ca69ed8bd26dc6b47f712b0b5a6fc52e9c42fe779ecf8342e0d53668d6', 'N'),
        ('type.pointer.validity.raw', 72, 79, '42fa1c0180c71aa827df6a9a49514ce89b2488bd63a3a9dc14ac35d1a897f5cf', 'N X'),
    )
    PRIOR_HASHES = dict(SequenceTypesProposalTests.PRIOR_HASHES, **{
        'sequence-types': '7798b51b6a210003ed87d05744d2678601a247d02d9ec23030a37285c1365ecd'})
    setUp = PrimitiveTypesProposalTests.setUp

    def artifact(self, source=None):
        return build(self.index, self.inventory,
                     self.source if source is None else source, 'pointer-types')

    def verify(self, data, source=None):
        return validate(data, self.index, self.inventory,
                        self.source if source is None else source, 'pointer-types')

    def test_independent_complete_page_label_block_classification_pins(self):
        import re
        import pointer_types_reconciliation_proposals as proposals
        self.assertEqual(proposals.SELECTED, self.PAGES)
        self.assertEqual(proposals.PAGE_SHA256, self.PAGE_HASHES)
        self.assertEqual(proposals.SOURCE_COMMIT, '86635e30bf861a038dc197d7e16fd09e7e514e7a')
        self.assertEqual(proposals.ALIASES, {
            'pointer': 'reference-types-pointer-reference-types',
            'reborrow': 'reference-types-pointer-reborrow-lifetimes'})
        classes = {'N': 'normative-semantic', 'G': 'grammar', 'I': 'index',
                   'X': 'cross-reference', 'E': 'edition-context'}
        self.assertEqual(proposals.CLASS_NAMES, classes)
        authored = [r.split('|') for r in proposals.ROWS.strip().splitlines()]
        self.assertTrue(all(len(r) == 7 and all(f.strip() for f in r) for r in authored))
        self.assertEqual(tuple((r[0], r[1]) for r in authored), tuple((r[0], r[4]) for r in self.PINS))
        self.assertEqual(reconciliation.SOURCE_ONLY_PAGE_COUNTS['pointer-types'], (22,))
        data = self.artifact()
        counts = self.verify(data)
        self.assertEqual([counts[k] for k in ('selected_rule_rows', 'unselected_rule_identities',
                                             'global_rule_identities')], [22, 2943, 2965])
        self.assertEqual(counts['proposed_classification_counts'], {
            'normative-semantic': 14, 'grammar': 2, 'index': 7,
            'cross-reference': 7, 'edition-context': 0})
        page = self.PAGES[0]
        raw = (self.source / page).read_bytes()
        self.assertEqual(digest(raw), self.PAGE_HASHES[page])
        lines = raw.splitlines(keepends=True)
        self.assertEqual(len(lines), 79)
        markers = [(i, m.group(1).decode()) for i, line in enumerate(lines, 1)
                   if (m := re.fullmatch(rb'r\[([^\]\r\n]+)\]\r?\n?', line))]
        self.assertEqual(markers, [(r[1], r[0]) for r in self.PINS])
        self.assertEqual(tuple(r['rule'] for r in data['rows']), tuple(r[0] for r in self.PINS))
        self.assertEqual(len({r['id'] for r in data['rows']}), 22)
        self.assertEqual([(p['path'], p['sha256']) for p in data['selected_pages']],
                         [(page, self.PAGE_HASHES[page])])
        for row, authored_row, (label, start, end, block_hash, codes) in zip(data['rows'], authored, self.PINS):
            with self.subTest(rule=label):
                self.assertEqual((row['page'], row['line'], row['end_line']), (page, start, end))
                self.assertEqual(digest(b''.join(lines[start - 1:end])), block_hash)
                self.assertEqual(row['source_block_sha256'], block_hash)
                self.assertEqual(row['classifications'], [classes[c] for c in codes.split()])
                self.assertEqual(row['candidate_obligation_ids'], [proposals.ALIASES[a] for a in authored_row[3].split()])
                self.assertEqual(row['id'], 'reconcile-pointer-types-' + label.encode().hex())
                self.assertEqual(row['review_status'], 'unreviewed')
                self.assertEqual(row['relationship'], 'context-only' if codes == 'I'
                                 else 'proposed-partial-decomposition')
                self.assertEqual(len(row['missing_test_needs']), 4)
                for need, kind in zip(row['missing_test_needs'], ('positive', 'invalid', 'interaction', 'end-to-end')):
                    self.assertTrue(need.startswith(kind + ': Missing '))
                for phrase in ('stable1.98.1 eligibility', 'workers1/8', 'edition2024', 'Linux x86-64',
                               'Never execute undefined behavior', 'CHECK must not demand codegen'):
                    self.assertIn(phrase, row['missing_test_needs'][-1])
        for field in ('reviewed_rule_rows', 'rows_with_exact_executable_evidence'):
            self.assertEqual(counts[field], 0)
        index = json.loads(self.index.read_bytes())
        universe = {(p['path'], r['label']) for p in index['pages'] + index['unlinked_markdown_files']
                    for r in p['rules']}
        selected = {(r['page'], r['rule']) for r in data['rows']}
        other = {(r['page'], r['rule']) for r in data['unselected_rules']}
        self.assertEqual((len(selected), len(other), len(universe)), (22, 2943, 2965))
        self.assertFalse(selected & other)
        self.assertEqual(selected | other, universe)
        self.assertEqual(self.artifact(), data)

    def test_each_row_omission_and_actual_changing_classification_forgery(self):
        baseline = self.artifact()
        for i, row in enumerate(baseline['rows']):
            for mutation in ('omission', 'classification'):
                changed = copy.deepcopy(baseline)
                if mutation == 'omission':
                    changed['rows'].pop(i)
                else:
                    changed['rows'][i]['classifications'] = (
                        ['index'] if row['classifications'] != ['index'] else ['grammar'])
                self.assertNotEqual(changed, baseline)
                with self.subTest(rule=row['rule'], mutation=mutation), self.assertRaises(ValueError):
                    self.verify(changed)

    def test_authored_omissions_duplicates_foreign_and_malformed_rows(self):
        import pointer_types_reconciliation_proposals as proposals
        baseline = self.artifact()
        rows = proposals.ROWS.strip().splitlines()
        cases = [[], rows + [rows[0]], ['type.foreign' + rows[0][len('type.pointer'):], *rows[1:]],
                 ['|'.join(rows[0].split('|')[:-1]), *rows[1:]], [rows[0] + '|extra', *rows[1:]]]
        cases.extend(rows[:i] + rows[i + 1:] for i in range(22))
        for col in range(7):
            fields = rows[0].split('|')
            fields[col] = '   '
            cases.append(['|'.join(fields), *rows[1:]])
        for i, changed in enumerate(cases):
            self.assertNotEqual(changed, rows)
            with self.subTest(case=i), mock.patch.object(proposals, 'ROWS', '\n'.join(changed)):
                with self.assertRaises(ValueError):
                    self.artifact()
                with self.assertRaises(ValueError):
                    self.verify(baseline)

    def test_permuted_authored_rows_preserve_canonical_order(self):
        import pointer_types_reconciliation_proposals as proposals
        baseline = self.artifact()
        rows = proposals.ROWS.strip().splitlines()
        for changed in (rows[::-1], rows[2:] + rows[:2], rows[::2] + rows[1::2]):
            with mock.patch.object(proposals, 'ROWS', '\n'.join(changed)):
                self.assertEqual(self.artifact(), baseline)
                self.assertEqual(self.verify(self.artifact()), self.verify(baseline))

    def test_all_27_prior_canonical_bytes_without_new_module(self):
        self.assertEqual(len(self.PRIOR_HASHES), 27)
        self.assertEqual(set(self.PRIOR_HASHES), {p for p, _ in reconciliation.PROPOSAL_COUNTS if p not in ('trait-objects', 'type-parameters', 'nominal-types', 'inferred-types', 'pointer-types', 'function-pointers', 'function-items')})
        with mock.patch.dict(sys.modules, {'pointer_types_reconciliation_proposals': None}):
            for profile, sha in self.PRIOR_HASHES.items():
                with self.subTest(profile=profile):
                    data = build(self.index, self.inventory, self.source, profile)
                    validate(data, self.index, self.inventory, self.source, profile)
                    self.assertEqual(digest((json.dumps(data, indent=2) + '\n').encode()), sha)
            self.assertEqual(digest((json.dumps(build(self.index, self.inventory, self.source), indent=2) + '\n').encode()),
                             self.PRIOR_HASHES['tokens'])

    def test_global_unique_union_and_zero_credit(self):
        index = json.loads(self.index.read_bytes())
        pages = index['pages'] + index['unlinked_markdown_files']
        prior_pages = {p for profile, _ in reconciliation.PROPOSAL_COUNTS if profile not in ('trait-objects', 'type-parameters', 'nominal-types', 'inferred-types', 'pointer-types', 'function-pointers', 'function-items')
                       for p in reconciliation.profile_inputs(profile)[0]}
        prior = {(p['path'], r['label']) for p in pages if p['path'] in prior_pages for r in p['rules']}
        current = {(r['page'], r['rule']) for r in self.artifact()['rows']}
        self.assertEqual((len(prior), len(current), len(prior & current), len(prior | current)), (961, 22, 0, 983))
        summary = reconciliation.summarize_proposals(self.index, self.inventory, include_proposals=True)
        self.assertEqual((summary['proposed_rule_identities'], summary['rule_identities_without_proposals']), (1030, 1935))
        self.assertEqual(sum(p['proposed_rule_identities'] for p in summary['profiles'].values()), 1035)
        pointer = summary['profiles']['pointer-types']
        self.assertEqual(len(pointer['authored_proposals']), 22)
        for field in ('reviewed_rule_rows', 'rows_with_exact_executable_evidence', 'completed_rule_rows'):
            self.assertEqual(summary[field], 0)
            self.assertEqual(pointer[field], 0)
        self.assertIsNone(pointer['full_rust_completion_percent'])
        result = reconciliation.query_rule_backlog(self.index, self.inventory, page=self.PAGES[0], include_proposals=True)
        self.assertEqual((result['selected_rule_identities'], result['selected_authored_proposal_count']), (22, 22))
        self.assertEqual(result['selected_rule_identities_without_proposals'], 0)
        self.assertTrue(all(r['proposal_profiles'] == ['pointer-types'] for r in result['rows']))
        self.assertEqual(reconciliation.query_rule_backlog(self.index, self.inventory, page=self.PAGES[0], missing_only=True)['rows'], [])

    test_each_source_page_fault_preserves_exact_error_types = PrimitiveTypesProposalTests.test_each_source_page_fault_preserves_exact_error_types
    test_29_actual_changing_forgeries_rejected = PrimitiveTypesProposalTests.test_29_actual_changing_forgeries_rejected

    def test_source_only_restrictions_precede_io_and_execution(self):
        args = ['--profile', 'pointer-types', '--rule-index', '/absent/index', '--inventory', '/absent/inventory',
                '--source-root', '/absent/source', '--artifact', '/absent/artifact']
        extras = ([], ['--repo-root', '/absent/repo'], ['--evidence', '/absent/evidence'],
                  ['--repo-root', '/absent/repo', '--evidence', '/absent/evidence'])
        cases = [(a, e) for a in ('generate-witness-links', 'build-witness-links', 'validate-witness-links',
                                  'assess-witness-links') for e in extras]
        cases += [(a, e) for a in ('generate', 'validate') for e in extras[1:]]
        with contextlib.ExitStack() as stack:
            for name in ('read_bytes', 'read_text', 'open', 'is_symlink', 'is_file', 'write_bytes', 'write_text', 'mkdir'):
                stack.enter_context(mock.patch.object(Path, name, side_effect=AssertionError('IO')))
            stack.enter_context(mock.patch.object(subprocess, 'run', side_effect=AssertionError('execution')))
            for action, extra in cases:
                with self.subTest(action=action, extra=extra), self.assertRaises(ValueError):
                    main([action, *args, *extra])
            for call in (
                lambda: reconciliation.build_witness_links(self.index, self.inventory, self.source, self.source, 'pointer-types'),
                lambda: reconciliation.validate_witness_links({}, self.index, self.inventory, self.source, self.source, 'pointer-types'),
                lambda: reconciliation.assess_witness_links({}, self.index, self.inventory, self.source, self.source, (), 'pointer-types'),
            ):
                with self.assertRaises(ValueError):
                    call()

    def test_cli_exclusive_generation_validation_and_no_repair(self):
        with tempfile.TemporaryDirectory() as temp:
            artifact = Path(temp) / 'pointer-types.json'
            args = ['--profile', 'pointer-types', '--rule-index', str(self.index), '--inventory', str(self.inventory),
                    '--source-root', str(self.source), '--artifact', str(artifact)]
            expected = (json.dumps(self.artifact(), indent=2) + '\n').encode()
            with contextlib.redirect_stdout(io.StringIO()) as output:
                main(['generate', *args])
            self.assertEqual(artifact.read_bytes(), expected)
            self.assertEqual(json.loads(output.getvalue()), self.verify(self.artifact()))
            with contextlib.redirect_stdout(io.StringIO()), self.assertRaises(FileExistsError):
                main(['generate', *args])
            self.assertEqual(artifact.read_bytes(), expected)
            with contextlib.redirect_stdout(io.StringIO()) as output:
                main(['validate', *args])
            self.assertEqual(json.loads(output.getvalue()), self.verify(self.artifact()))
            self.assertEqual(artifact.read_bytes(), expected)
            forged = json.loads(expected)
            forged['counts']['reviewed_rule_rows'] = 1
            artifact.write_text(json.dumps(forged))
            before = artifact.read_bytes()
            with contextlib.redirect_stdout(io.StringIO()), self.assertRaises(ValueError):
                main(['validate', *args])
            self.assertEqual(artifact.read_bytes(), before)

    def test_actual_process_cli_generate_validate(self):
        with tempfile.TemporaryDirectory() as temp:
            artifact = Path(temp) / 'pointer-types.json'
            args = [sys.executable, str(BASE / 'reconcile_token_rules.py'), '--profile', 'pointer-types',
                    '--rule-index', str(self.index), '--inventory', str(self.inventory),
                    '--source-root', str(self.source), '--artifact', str(artifact)]
            expected = (json.dumps(self.artifact(), indent=2) + '\n').encode()
            for action in ('generate', 'validate'):
                result = subprocess.run([*args[:2], action, *args[2:]], capture_output=True, text=True, check=False)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertEqual(json.loads(result.stdout), self.verify(self.artifact()))
                self.assertEqual(artifact.read_bytes(), expected)

    def test_rule_specific_safety_and_direction_caveats_remain_open(self):
        import pointer_types_reconciliation_proposals as proposals
        rows = {r.split('|')[0]: r.split('|') for r in proposals.ROWS.strip().splitlines()}
        phrases = {
            'type.pointer.intro': ('explicit first-class', 'stored in data structs', 'returned from functions', 'prohibit universalizing Copy to &mut'),
            'type.pointer.reference.syntax': ('optional Lifetime', 'optional mut', 'TypeNoBounds'),
            'type.pointer.reference.shared.intro': ('owned by some other value',),
            'type.pointer.reference.shared.constraint-mutation': ('interior-mutability exceptions', 'any number', "&'a type"),
            'type.pointer.reference.shared.copy': ('shallow', 'copying only the pointer', 'no effect on its referent', 'temporary', 'mutable reference Copy'),
            'type.pointer.reference.mut.intro': ('owned by some other value', "&'a mut type"),
            'type.pointer.reference.mut.copy': ("hasn't been borrowed", 'not Copy', 'not a ban on reborrowing'),
            'type.pointer.raw.syntax': ('star, then mut or const', 'TypeNoBounds'),
            'type.pointer.raw.intro': ('no safety or liveness guarantees', '32-bit integer', 'without executing invalid dereferences'),
            'type.pointer.raw.copy': ("no effect on any other value's lifecycle", 'pointee lifecycle'),
            'type.pointer.raw.safety': ('unsafe', '&* and &mut *', 'not automatically', 'reference safety constraints'),
            'type.pointer.raw.cmp': ('addresses, not pointee values', 'additional data', 'memory layout or provenance'),
            'type.pointer.raw.constructor': ('&raw const', '&raw mut', 'versus reference construction'),
            'type.pointer.smart': ('standard library contains additional', 'normative library prose, not a pure heading', 'without importing complete standard-library'),
            'type.pointer.validity.pointer-fragment': ('currently undecided', '[u8; size_of::<P>()]', 'may not be valid', 'safe or reversible byte encoding', 'without executing undecided'),
            'type.pointer.validity.raw': ('thin raw', 'T: Sized', 'integer or array of integers', 'always valid', 'may not be dereferenced', 'not even for zero-sized T', 'without executing UB'),
        }
        for label, needs in phrases.items():
            for phrase in needs:
                with self.subTest(label=label, phrase=phrase):
                    self.assertIn(phrase, '|'.join(rows[label]))
        for phrase in ('22 labels', '79 lines', 'UNREVIEWED', 'partial routing only', 'not semantic equivalence',
                       'eligibility is unknown', 'not contract identity', 'context only, not imported complete proofs'):
            self.assertIn(phrase, ' '.join(proposals.LIMITATIONS))


class FunctionPointersProposalTests(unittest.TestCase):
    """Complete pinned source-only pointer proposals, never compiler evidence."""
    PAGES = ('src/types/function-pointer.md',)
    PAGE_HASHES = {'src/types/function-pointer.md': '80219bfb130e611e963a7cfabc3a65472c18b2d445b3da930f279d9676f7a942'}
    # Independent literal source/block/line and authored classification pins.
    PINS = (
        ('type.fn-pointer', 1, 3, 'f77c3126d6fe3f859ec7a80504a28387980c66b109e70886eb3e056362af8462', 'I'),
        ('type.fn-pointer.syntax', 4, 26, '09a450603f55c9f6ca4ad655d6a6eab6d662c6fe5d243b8efc3d7d7269228802', 'G X'),
        ('type.fn-pointer.intro', 27, 43, '8d0acdd41dde31f81b7876322e82c2dfd18f7a7fd38cacbf384da8343da7dab8', 'N'),
        ('type.fn-pointer.coercion', 44, 46, 'df3a39dc23eb8cce0b1e90a7a6c6e5e680b983675f1ad89ff2bbca5861db3b2d', 'N X'),
        ('type.fn-pointer.qualifiers', 47, 49, 'cae354382a91f7209b14bc0a68f06944da8a65b3bc40709866888bfaf2347978', 'N X'),
        ('type.fn-pointer.constraint-variadic', 50, 52, 'f92eb54b974ce293f58549c3a7462729adf2bd2a4cef3726e06fc9ebd889b8b2', 'N X'),
        ('type.fn-pointer.attributes', 53, 63, 'd9ea9d52e65a658ac34372be38c3e86ec396aabe9e7cc1d099901d6eb0c0c33a', 'N I X'),
    )
    PRIOR_HASHES = dict(PointerTypesProposalTests.PRIOR_HASHES, **{
        'pointer-types': '77bf88fdd8c7c74e934cb77232dd8d758e695bc0431ad31d51d8da0df2a7c62b'})
    setUp = PrimitiveTypesProposalTests.setUp

    def artifact(self, source=None):
        return build(self.index, self.inventory,
                     self.source if source is None else source, 'function-pointers')

    def verify(self, data, source=None):
        return validate(data, self.index, self.inventory,
                        self.source if source is None else source, 'function-pointers')

    def test_independent_complete_page_label_block_classification_pins(self):
        import re
        import function_pointers_reconciliation_proposals as proposals
        self.assertEqual(proposals.SELECTED, self.PAGES)
        self.assertEqual(proposals.PAGE_SHA256, self.PAGE_HASHES)
        self.assertEqual(proposals.SOURCE_COMMIT, '86635e30bf861a038dc197d7e16fd09e7e514e7a')
        self.assertEqual(proposals.ALIASES, {
            'function-pointer': 'reference-types-function-pointer-fn-pointer-signatures'})
        classes = {'N': 'normative-semantic', 'G': 'grammar', 'I': 'index',
                   'X': 'cross-reference', 'E': 'edition-context'}
        self.assertEqual(proposals.CLASS_NAMES, classes)
        authored = [r.split('|') for r in proposals.ROWS.strip().splitlines()]
        self.assertTrue(all(len(r) == 7 and all(f.strip() for f in r) for r in authored))
        self.assertEqual(tuple((r[0], r[1]) for r in authored), tuple((r[0], r[4]) for r in self.PINS))
        self.assertEqual(reconciliation.SOURCE_ONLY_PAGE_COUNTS['function-pointers'], (7,))
        data = self.artifact()
        counts = self.verify(data)
        self.assertEqual([counts[k] for k in ('selected_rule_rows', 'unselected_rule_identities',
                                             'global_rule_identities')], [7, 2958, 2965])
        self.assertEqual(counts['proposed_classification_counts'], {
            'normative-semantic': 5, 'grammar': 1, 'index': 2,
            'cross-reference': 5, 'edition-context': 0})
        page = self.PAGES[0]
        raw = (self.source / page).read_bytes()
        self.assertEqual(digest(raw), self.PAGE_HASHES[page])
        lines = raw.splitlines(keepends=True)
        self.assertEqual(len(lines), 63)
        markers = [(i, m.group(1).decode()) for i, line in enumerate(lines, 1)
                   if (m := re.fullmatch(rb'r\[([^\]\r\n]+)\]\r?\n?', line))]
        self.assertEqual(markers, [(r[1], r[0]) for r in self.PINS])
        self.assertEqual(tuple(r['rule'] for r in data['rows']), tuple(r[0] for r in self.PINS))
        self.assertEqual(len({r['id'] for r in data['rows']}), 7)
        self.assertEqual([(p['path'], p['sha256']) for p in data['selected_pages']],
                         [(page, self.PAGE_HASHES[page])])
        for row, authored_row, (label, start, end, block_hash, codes) in zip(data['rows'], authored, self.PINS):
            with self.subTest(rule=label):
                self.assertEqual((row['page'], row['line'], row['end_line']), (page, start, end))
                self.assertEqual(digest(b''.join(lines[start - 1:end])), block_hash)
                self.assertEqual(row['source_block_sha256'], block_hash)
                self.assertEqual(row['classifications'], [classes[c] for c in codes.split()])
                self.assertEqual(row['candidate_obligation_ids'], [proposals.ALIASES[a] for a in authored_row[3].split()])
                self.assertEqual(row['id'], 'reconcile-function-pointers-' + label.encode().hex())
                self.assertEqual(row['review_status'], 'unreviewed')
                self.assertEqual(row['relationship'], 'context-only' if codes == 'I'
                                 else 'proposed-partial-decomposition')
                self.assertEqual(len(row['missing_test_needs']), 4)
                for need, kind in zip(row['missing_test_needs'], ('positive', 'invalid', 'interaction', 'end-to-end')):
                    self.assertTrue(need.startswith(kind + ': Missing '))
                for phrase in ('stable1.98.1 eligibility', 'workers1/8', 'edition2024', 'Linux x86-64',
                               'Never execute undefined behavior', 'CHECK must not demand codegen'):
                    self.assertIn(phrase, row['missing_test_needs'][-1])
        for field in ('reviewed_rule_rows', 'rows_with_exact_executable_evidence'):
            self.assertEqual(counts[field], 0)
        index = json.loads(self.index.read_bytes())
        universe = {(p['path'], r['label']) for p in index['pages'] + index['unlinked_markdown_files']
                    for r in p['rules']}
        selected = {(r['page'], r['rule']) for r in data['rows']}
        other = {(r['page'], r['rule']) for r in data['unselected_rules']}
        self.assertEqual((len(selected), len(other), len(universe)), (7, 2958, 2965))
        self.assertFalse(selected & other)
        self.assertEqual(selected | other, universe)
        self.assertEqual(self.artifact(), data)

    def test_each_row_omission_and_actual_changing_classification_forgery(self):
        baseline = self.artifact()
        for i, row in enumerate(baseline['rows']):
            for mutation in ('omission', 'classification'):
                changed = copy.deepcopy(baseline)
                if mutation == 'omission':
                    changed['rows'].pop(i)
                else:
                    changed['rows'][i]['classifications'] = (
                        ['index'] if row['classifications'] != ['index'] else ['grammar'])
                self.assertNotEqual(changed, baseline)
                with self.subTest(rule=row['rule'], mutation=mutation), self.assertRaises(ValueError):
                    self.verify(changed)

    def test_authored_omissions_duplicates_foreign_and_malformed_rows(self):
        import function_pointers_reconciliation_proposals as proposals
        baseline = self.artifact()
        rows = proposals.ROWS.strip().splitlines()
        cases = [[], rows + [rows[0]], ['type.foreign' + rows[0][len('type.fn-pointer'):], *rows[1:]],
                 ['|'.join(rows[0].split('|')[:-1]), *rows[1:]], [rows[0] + '|extra', *rows[1:]]]
        cases.extend(rows[:i] + rows[i + 1:] for i in range(7))
        for col in range(7):
            fields = rows[0].split('|')
            fields[col] = '   '
            cases.append(['|'.join(fields), *rows[1:]])
        for i, changed in enumerate(cases):
            self.assertNotEqual(changed, rows)
            with self.subTest(case=i), mock.patch.object(proposals, 'ROWS', '\n'.join(changed)):
                with self.assertRaises(ValueError):
                    self.artifact()
                with self.assertRaises(ValueError):
                    self.verify(baseline)

    def test_permuted_authored_rows_preserve_canonical_order(self):
        import function_pointers_reconciliation_proposals as proposals
        baseline = self.artifact()
        rows = proposals.ROWS.strip().splitlines()
        for changed in (rows[::-1], rows[2:] + rows[:2], rows[::2] + rows[1::2]):
            with mock.patch.object(proposals, 'ROWS', '\n'.join(changed)):
                self.assertEqual(self.artifact(), baseline)
                self.assertEqual(self.verify(self.artifact()), self.verify(baseline))

    def test_all_28_prior_canonical_bytes_without_new_module(self):
        self.assertEqual(len(self.PRIOR_HASHES), 28)
        self.assertEqual(set(self.PRIOR_HASHES), {p for p, _ in reconciliation.PROPOSAL_COUNTS if p not in ('trait-objects', 'type-parameters', 'nominal-types', 'inferred-types', 'function-pointers', 'function-items')})
        with mock.patch.dict(sys.modules, {'function_pointers_reconciliation_proposals': None}):
            for profile, sha in self.PRIOR_HASHES.items():
                with self.subTest(profile=profile):
                    data = build(self.index, self.inventory, self.source, profile)
                    validate(data, self.index, self.inventory, self.source, profile)
                    self.assertEqual(digest((json.dumps(data, indent=2) + '\n').encode()), sha)
            self.assertEqual(digest((json.dumps(build(self.index, self.inventory, self.source), indent=2) + '\n').encode()),
                             self.PRIOR_HASHES['tokens'])

    def test_global_unique_union_and_zero_credit(self):
        index = json.loads(self.index.read_bytes())
        pages = index['pages'] + index['unlinked_markdown_files']
        prior_pages = {p for profile, _ in reconciliation.PROPOSAL_COUNTS if profile not in ('trait-objects', 'type-parameters', 'nominal-types', 'inferred-types', 'function-pointers', 'function-items')
                       for p in reconciliation.profile_inputs(profile)[0]}
        prior = {(p['path'], r['label']) for p in pages if p['path'] in prior_pages for r in p['rules']}
        current = {(r['page'], r['rule']) for r in self.artifact()['rows']}
        self.assertEqual((len(prior), len(current), len(prior & current), len(prior | current)), (983, 7, 0, 990))
        summary = reconciliation.summarize_proposals(self.index, self.inventory, include_proposals=True)
        self.assertEqual((summary['proposed_rule_identities'], summary['rule_identities_without_proposals']), (1030, 1935))
        self.assertEqual(sum(p['proposed_rule_identities'] for p in summary['profiles'].values()), 1035)
        pointer = summary['profiles']['function-pointers']
        self.assertEqual(len(pointer['authored_proposals']), 7)
        for field in ('reviewed_rule_rows', 'rows_with_exact_executable_evidence', 'completed_rule_rows'):
            self.assertEqual(summary[field], 0)
            self.assertEqual(pointer[field], 0)
        self.assertIsNone(pointer['full_rust_completion_percent'])
        result = reconciliation.query_rule_backlog(self.index, self.inventory, page=self.PAGES[0], include_proposals=True)
        self.assertEqual((result['selected_rule_identities'], result['selected_authored_proposal_count']), (7, 7))
        self.assertEqual(result['selected_rule_identities_without_proposals'], 0)
        self.assertTrue(all(r['proposal_profiles'] == ['function-pointers'] for r in result['rows']))
        self.assertEqual(reconciliation.query_rule_backlog(self.index, self.inventory, page=self.PAGES[0], missing_only=True)['rows'], [])

    test_each_source_page_fault_preserves_exact_error_types = PrimitiveTypesProposalTests.test_each_source_page_fault_preserves_exact_error_types
    test_29_actual_changing_forgeries_rejected = PrimitiveTypesProposalTests.test_29_actual_changing_forgeries_rejected

    def test_source_only_restrictions_precede_io_and_execution(self):
        args = ['--profile', 'function-pointers', '--rule-index', '/absent/index', '--inventory', '/absent/inventory',
                '--source-root', '/absent/source', '--artifact', '/absent/artifact']
        extras = ([], ['--repo-root', '/absent/repo'], ['--evidence', '/absent/evidence'],
                  ['--repo-root', '/absent/repo', '--evidence', '/absent/evidence'])
        cases = [(a, e) for a in ('generate-witness-links', 'build-witness-links', 'validate-witness-links',
                                  'assess-witness-links') for e in extras]
        cases += [(a, e) for a in ('generate', 'validate') for e in extras[1:]]
        with contextlib.ExitStack() as stack:
            for name in ('read_bytes', 'read_text', 'open', 'is_symlink', 'is_file', 'write_bytes', 'write_text', 'mkdir'):
                stack.enter_context(mock.patch.object(Path, name, side_effect=AssertionError('IO')))
            stack.enter_context(mock.patch.object(subprocess, 'run', side_effect=AssertionError('execution')))
            for action, extra in cases:
                with self.subTest(action=action, extra=extra), self.assertRaises(ValueError):
                    main([action, *args, *extra])
            for call in (
                lambda: reconciliation.build_witness_links(self.index, self.inventory, self.source, self.source, 'function-pointers'),
                lambda: reconciliation.validate_witness_links({}, self.index, self.inventory, self.source, self.source, 'function-pointers'),
                lambda: reconciliation.assess_witness_links({}, self.index, self.inventory, self.source, self.source, (), 'function-pointers'),
            ):
                with self.assertRaises(ValueError):
                    call()

    def test_cli_exclusive_generation_validation_and_no_repair(self):
        with tempfile.TemporaryDirectory() as temp:
            artifact = Path(temp) / 'function-pointers.json'
            args = ['--profile', 'function-pointers', '--rule-index', str(self.index), '--inventory', str(self.inventory),
                    '--source-root', str(self.source), '--artifact', str(artifact)]
            expected = (json.dumps(self.artifact(), indent=2) + '\n').encode()
            with contextlib.redirect_stdout(io.StringIO()) as output:
                main(['generate', *args])
            self.assertEqual(artifact.read_bytes(), expected)
            self.assertEqual(json.loads(output.getvalue()), self.verify(self.artifact()))
            with contextlib.redirect_stdout(io.StringIO()), self.assertRaises(FileExistsError):
                main(['generate', *args])
            self.assertEqual(artifact.read_bytes(), expected)
            with contextlib.redirect_stdout(io.StringIO()) as output:
                main(['validate', *args])
            self.assertEqual(json.loads(output.getvalue()), self.verify(self.artifact()))
            self.assertEqual(artifact.read_bytes(), expected)
            forged = json.loads(expected)
            forged['counts']['reviewed_rule_rows'] = 1
            artifact.write_text(json.dumps(forged))
            before = artifact.read_bytes()
            with contextlib.redirect_stdout(io.StringIO()), self.assertRaises(ValueError):
                main(['validate', *args])
            self.assertEqual(artifact.read_bytes(), before)

    def test_actual_process_cli_generate_validate(self):
        with tempfile.TemporaryDirectory() as temp:
            artifact = Path(temp) / 'function-pointers.json'
            args = [sys.executable, str(BASE / 'reconcile_token_rules.py'), '--profile', 'function-pointers',
                    '--rule-index', str(self.index), '--inventory', str(self.inventory),
                    '--source-root', str(self.source), '--artifact', str(artifact)]
            expected = (json.dumps(self.artifact(), indent=2) + '\n').encode()
            for action in ('generate', 'validate'):
                result = subprocess.run([*args[:2], action, *args[2:]], capture_output=True, text=True, check=False)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertEqual(json.loads(result.stdout), self.verify(self.artifact()))
                self.assertEqual(artifact.read_bytes(), expected)


    def test_complete_grammar_and_semantic_caveats_remain_open(self):
        import function_pointers_reconciliation_proposals as proposals
        rows = {r.split('|')[0]: r.split('|') for r in proposals.ROWS.strip().splitlines()}
        phrases = {
            'type.fn-pointer': ('heading', 'not independent executable behavior'),
            'type.fn-pointer.syntax': ('optional ForLifetimes', 'optional parameters and return',
                'optional unsafe', 'optional extern with optional ABI', 'TypeNoBounds',
                'parameter types use Type', 'identifier or underscore', 'outer attributes',
                'comma and attribute positions', 'empty and named parameters', 'trailing commas',
                'grammar versus semantic admission', 'variadic ABI constraints'),
            'type.fn-pointer.intro': ('not necessarily known at compile time', 'Binop',
                'without inventing an unknown-identity requirement', 'mandatory dynamic dispatch',
                'optimization, representation or pointer-address claims'),
            'type.fn-pointer.coercion': ('function items', 'non-capturing non-async closures',
                'capturing or async closure', 'without importing complete linked'),
            'type.fn-pointer.qualifiers': ('unsafe function value', 'extern function value',
                'never executing invalid calls', 'type qualifier identity versus call safety',
                'not arbitrary qualifier interchangeability'),
            'type.fn-pointer.constraint-variadic': ('items.extern.variadic.conventions',
                'independently verified permitted extern ABI', 'without treating every extern ABI as permitted',
                'without importing a complete convention list'),
            'type.fn-pointer.attributes': ('heading also requires', 'rules and restrictions',
                'normative restriction-by-reference', 'pure index heading or blanket attribute allowlist',
                'context until separately reviewed'),
        }
        for label, needs in phrases.items():
            for phrase in needs:
                with self.subTest(label=label, phrase=phrase):
                    self.assertIn(phrase, '|'.join(rows[label]))
        for phrase in ('seven labels', '63 lines', 'UNREVIEWED', 'partial routing only',
                       'not semantic equivalence', 'does not establish stable1.98.1 eligibility',
                       'context only', 'not a pure heading', 'not supply a blanket attribute allowlist'):
            self.assertIn(phrase, ' '.join(proposals.LIMITATIONS))
        for phrase in ('invalid Rust', 'unsupported valid source', 'resource limits',
                       'infrastructure failures', 'Never execute undefined behavior',
                       'No semantic review, execution or completion credit'):
            self.assertIn(phrase, proposals.END_TO_END)
        source = (self.source / self.PAGES[0]).read_text()
        for grammar in (
                'ForLifetimes? FunctionTypeQualifiers `fn`',
                '`(` FunctionParametersMaybeNamedVariadic? `)` BareFunctionReturnType?',
                'FunctionTypeQualifiers -> `unsafe`? (`extern` Abi?)?',
                'BareFunctionReturnType -> `->` TypeNoBounds',
                'MaybeNamedFunctionParameters ->\n    MaybeNamedParam ( `,` MaybeNamedParam )* `,`?',
                'OuterAttribute* ( ( IDENTIFIER | `_` ) `:` )? Type',
                '( MaybeNamedParam `,` )* MaybeNamedParam `,` OuterAttribute* `...`'):
            with self.subTest(grammar=grammar):
                self.assertIn(grammar, source)


class FunctionItemsProposalTests(unittest.TestCase):
    """Complete pinned source-only pointer proposals, never compiler evidence."""
    PAGES = ('src/types/function-item.md',)
    PAGE_HASHES = {'src/types/function-item.md': 'bdcabb9164f4eb341ab0277cd6cf6a6fa95af3ce0e148f3d3991611c39975754'}
    # Independent literal source/block/line and authored classification pins.
    PINS = (
        ('type.fn-item', 1, 3, 'e89e71705b98d793a8e667228ca8e83f58ad774af05dbbde53f3677c90bd4921', 'I'),
        ('type.fn-item.intro', 4, 6, 'e5f27908c0403fdc99e6efc251c72d47727ecaa2aefe9347105dd46b63817a0e', 'N X'),
        ('type.fn-item.unique', 7, 9, '9e36fe983495098b9ec077be32d87c62d5ade4ee4a8aaca6fa60d7b56d7c2b4b', 'N'),
        ('type.fn-item.name', 10, 20, '7e65a1c9ad50a0913a3072f7482aaa04146f0b95baa757a4fce22b2a2cfc9cc2', 'N'),
        ('type.fn-item.coercion', 21, 38, 'a89f78614b8f7592a0d44025c640ce36070ef0eb51467d854bce763ac48a6935', 'N X'),
        ('type.fn-item.traits', 39, 54, '888b0963716592d0ecdb4484e79d7f362aa4d09c075c8fb1fc7511d07f02f3ae', 'N X'),
    )
    PRIOR_HASHES = dict(FunctionPointersProposalTests.PRIOR_HASHES, **{
        'function-pointers': '9e2c451b9dc25ae50d1a84f7985bd93c7f89c32dec764e76edc08d6c2c39445d'})
    setUp = PrimitiveTypesProposalTests.setUp

    def artifact(self, source=None):
        return build(self.index, self.inventory,
                     self.source if source is None else source, 'function-items')

    def verify(self, data, source=None):
        return validate(data, self.index, self.inventory,
                        self.source if source is None else source, 'function-items')

    def test_independent_complete_page_label_block_classification_pins(self):
        import re
        import function_items_reconciliation_proposals as proposals
        self.assertEqual(proposals.SELECTED, self.PAGES)
        self.assertEqual(proposals.PAGE_SHA256, self.PAGE_HASHES)
        self.assertEqual(proposals.SOURCE_COMMIT, '86635e30bf861a038dc197d7e16fd09e7e514e7a')
        self.assertEqual(proposals.ALIASES, {
            'function-item': 'reference-types-function-item-fn-item-identity'})
        classes = {'N': 'normative-semantic', 'G': 'grammar', 'I': 'index',
                   'X': 'cross-reference', 'E': 'edition-context'}
        self.assertEqual(proposals.CLASS_NAMES, classes)
        authored = [r.split('|') for r in proposals.ROWS.strip().splitlines()]
        self.assertTrue(all(len(r) == 7 and all(f.strip() for f in r) for r in authored))
        self.assertEqual(tuple((r[0], r[1]) for r in authored), tuple((r[0], r[4]) for r in self.PINS))
        self.assertEqual(reconciliation.SOURCE_ONLY_PAGE_COUNTS['function-items'], (6,))
        data = self.artifact()
        counts = self.verify(data)
        self.assertEqual([counts[k] for k in ('selected_rule_rows', 'unselected_rule_identities',
                                             'global_rule_identities')], [6, 2959, 2965])
        self.assertEqual(counts['proposed_classification_counts'], {
            'normative-semantic': 5, 'grammar': 0, 'index': 1,
            'cross-reference': 3, 'edition-context': 0})
        page = self.PAGES[0]
        raw = (self.source / page).read_bytes()
        self.assertEqual(digest(raw), self.PAGE_HASHES[page])
        lines = raw.splitlines(keepends=True)
        self.assertEqual(len(lines), 54)
        markers = [(i, m.group(1).decode()) for i, line in enumerate(lines, 1)
                   if (m := re.fullmatch(rb'r\[([^\]\r\n]+)\]\r?\n?', line))]
        self.assertEqual(markers, [(r[1], r[0]) for r in self.PINS])
        self.assertEqual(tuple(r['rule'] for r in data['rows']), tuple(r[0] for r in self.PINS))
        self.assertEqual(len({r['id'] for r in data['rows']}), 6)
        self.assertEqual([(p['path'], p['sha256']) for p in data['selected_pages']],
                         [(page, self.PAGE_HASHES[page])])
        for row, authored_row, (label, start, end, block_hash, codes) in zip(data['rows'], authored, self.PINS):
            with self.subTest(rule=label):
                self.assertEqual((row['page'], row['line'], row['end_line']), (page, start, end))
                self.assertEqual(digest(b''.join(lines[start - 1:end])), block_hash)
                self.assertEqual(row['source_block_sha256'], block_hash)
                self.assertEqual(row['classifications'], [classes[c] for c in codes.split()])
                self.assertEqual(row['candidate_obligation_ids'], [proposals.ALIASES[a] for a in authored_row[3].split()])
                self.assertEqual(row['id'], 'reconcile-function-items-' + label.encode().hex())
                self.assertEqual(row['review_status'], 'unreviewed')
                self.assertEqual(row['relationship'], 'context-only' if codes == 'I'
                                 else 'proposed-partial-decomposition')
                self.assertEqual(len(row['missing_test_needs']), 4)
                for need, kind in zip(row['missing_test_needs'], ('positive', 'invalid', 'interaction', 'end-to-end')):
                    self.assertTrue(need.startswith(kind + ': Missing '))
                for phrase in ('stable1.98.1 eligibility', 'workers1/8', 'edition2024', 'Linux x86-64',
                               'Never execute undefined behavior', 'CHECK must not demand codegen'):
                    self.assertIn(phrase, row['missing_test_needs'][-1])
        for field in ('reviewed_rule_rows', 'rows_with_exact_executable_evidence'):
            self.assertEqual(counts[field], 0)
        index = json.loads(self.index.read_bytes())
        universe = {(p['path'], r['label']) for p in index['pages'] + index['unlinked_markdown_files']
                    for r in p['rules']}
        selected = {(r['page'], r['rule']) for r in data['rows']}
        other = {(r['page'], r['rule']) for r in data['unselected_rules']}
        self.assertEqual((len(selected), len(other), len(universe)), (6, 2959, 2965))
        self.assertFalse(selected & other)
        self.assertEqual(selected | other, universe)
        self.assertEqual(self.artifact(), data)

    def test_each_row_omission_and_actual_changing_classification_forgery(self):
        baseline = self.artifact()
        for i, row in enumerate(baseline['rows']):
            for mutation in ('omission', 'classification'):
                changed = copy.deepcopy(baseline)
                if mutation == 'omission':
                    changed['rows'].pop(i)
                else:
                    changed['rows'][i]['classifications'] = (
                        ['index'] if row['classifications'] != ['index'] else ['grammar'])
                self.assertNotEqual(changed, baseline)
                with self.subTest(rule=row['rule'], mutation=mutation), self.assertRaises(ValueError):
                    self.verify(changed)

    def test_authored_omissions_duplicates_foreign_and_malformed_rows(self):
        import function_items_reconciliation_proposals as proposals
        baseline = self.artifact()
        rows = proposals.ROWS.strip().splitlines()
        cases = [[], rows + [rows[0]], ['type.foreign' + rows[0][len('type.fn-item'):], *rows[1:]],
                 ['|'.join(rows[0].split('|')[:-1]), *rows[1:]], [rows[0] + '|extra', *rows[1:]]]
        cases.extend(rows[:i] + rows[i + 1:] for i in range(6))
        for col in range(7):
            fields = rows[0].split('|')
            fields[col] = '   '
            cases.append(['|'.join(fields), *rows[1:]])
        for i, changed in enumerate(cases):
            self.assertNotEqual(changed, rows)
            with self.subTest(case=i), mock.patch.object(proposals, 'ROWS', '\n'.join(changed)):
                with self.assertRaises(ValueError):
                    self.artifact()
                with self.assertRaises(ValueError):
                    self.verify(baseline)

    def test_permuted_authored_rows_preserve_canonical_order(self):
        import function_items_reconciliation_proposals as proposals
        baseline = self.artifact()
        rows = proposals.ROWS.strip().splitlines()
        for changed in (rows[::-1], rows[2:] + rows[:2], rows[::2] + rows[1::2]):
            with mock.patch.object(proposals, 'ROWS', '\n'.join(changed)):
                self.assertEqual(self.artifact(), baseline)
                self.assertEqual(self.verify(self.artifact()), self.verify(baseline))

    def test_all_29_prior_canonical_bytes_without_new_module(self):
        self.assertEqual(len(self.PRIOR_HASHES), 29)
        self.assertEqual(set(self.PRIOR_HASHES), {p for p, _ in reconciliation.PROPOSAL_COUNTS if p not in ('trait-objects', 'type-parameters', 'nominal-types', 'inferred-types', 'function-items',)})
        with mock.patch.dict(sys.modules, {'function_items_reconciliation_proposals': None}):
            for profile, sha in self.PRIOR_HASHES.items():
                with self.subTest(profile=profile):
                    data = build(self.index, self.inventory, self.source, profile)
                    validate(data, self.index, self.inventory, self.source, profile)
                    self.assertEqual(digest((json.dumps(data, indent=2) + '\n').encode()), sha)
            self.assertEqual(digest((json.dumps(build(self.index, self.inventory, self.source), indent=2) + '\n').encode()),
                             self.PRIOR_HASHES['tokens'])

    def test_global_unique_union_and_zero_credit(self):
        index = json.loads(self.index.read_bytes())
        pages = index['pages'] + index['unlinked_markdown_files']
        prior_pages = {p for profile, _ in reconciliation.PROPOSAL_COUNTS if profile not in ('trait-objects', 'type-parameters', 'nominal-types', 'inferred-types', 'function-items',)
                       for p in reconciliation.profile_inputs(profile)[0]}
        prior = {(p['path'], r['label']) for p in pages if p['path'] in prior_pages for r in p['rules']}
        current = {(r['page'], r['rule']) for r in self.artifact()['rows']}
        self.assertEqual((len(prior), len(current), len(prior & current), len(prior | current)), (990, 6, 0, 996))
        summary = reconciliation.summarize_proposals(self.index, self.inventory, include_proposals=True)
        self.assertEqual((summary['proposed_rule_identities'], summary['rule_identities_without_proposals']), (1030, 1935))
        self.assertEqual(sum(p['proposed_rule_identities'] for p in summary['profiles'].values()), 1035)
        pointer = summary['profiles']['function-items']
        self.assertEqual(len(pointer['authored_proposals']), 6)
        for field in ('reviewed_rule_rows', 'rows_with_exact_executable_evidence', 'completed_rule_rows'):
            self.assertEqual(summary[field], 0)
            self.assertEqual(pointer[field], 0)
        self.assertIsNone(pointer['full_rust_completion_percent'])
        result = reconciliation.query_rule_backlog(self.index, self.inventory, page=self.PAGES[0], include_proposals=True)
        self.assertEqual((result['selected_rule_identities'], result['selected_authored_proposal_count']), (6, 6))
        self.assertEqual(result['selected_rule_identities_without_proposals'], 0)
        self.assertTrue(all(r['proposal_profiles'] == ['function-items'] for r in result['rows']))
        self.assertEqual(reconciliation.query_rule_backlog(self.index, self.inventory, page=self.PAGES[0], missing_only=True)['rows'], [])

    test_each_source_page_fault_preserves_exact_error_types = PrimitiveTypesProposalTests.test_each_source_page_fault_preserves_exact_error_types
    test_29_actual_changing_forgeries_rejected = PrimitiveTypesProposalTests.test_29_actual_changing_forgeries_rejected

    def test_source_only_restrictions_precede_io_and_execution(self):
        args = ['--profile', 'function-items', '--rule-index', '/absent/index', '--inventory', '/absent/inventory',
                '--source-root', '/absent/source', '--artifact', '/absent/artifact']
        extras = ([], ['--repo-root', '/absent/repo'], ['--evidence', '/absent/evidence'],
                  ['--repo-root', '/absent/repo', '--evidence', '/absent/evidence'])
        cases = [(a, e) for a in ('generate-witness-links', 'build-witness-links', 'validate-witness-links',
                                  'assess-witness-links') for e in extras]
        cases += [(a, e) for a in ('generate', 'validate') for e in extras[1:]]
        with contextlib.ExitStack() as stack:
            for name in ('read_bytes', 'read_text', 'open', 'is_symlink', 'is_file', 'write_bytes', 'write_text', 'mkdir'):
                stack.enter_context(mock.patch.object(Path, name, side_effect=AssertionError('IO')))
            stack.enter_context(mock.patch.object(subprocess, 'run', side_effect=AssertionError('execution')))
            for action, extra in cases:
                with self.subTest(action=action, extra=extra), self.assertRaises(ValueError):
                    main([action, *args, *extra])
            for call in (
                lambda: reconciliation.build_witness_links(self.index, self.inventory, self.source, self.source, 'function-items'),
                lambda: reconciliation.validate_witness_links({}, self.index, self.inventory, self.source, self.source, 'function-items'),
                lambda: reconciliation.assess_witness_links({}, self.index, self.inventory, self.source, self.source, (), 'function-items'),
            ):
                with self.assertRaises(ValueError):
                    call()

    def test_cli_exclusive_generation_validation_and_no_repair(self):
        with tempfile.TemporaryDirectory() as temp:
            artifact = Path(temp) / 'function-items.json'
            args = ['--profile', 'function-items', '--rule-index', str(self.index), '--inventory', str(self.inventory),
                    '--source-root', str(self.source), '--artifact', str(artifact)]
            expected = (json.dumps(self.artifact(), indent=2) + '\n').encode()
            with contextlib.redirect_stdout(io.StringIO()) as output:
                main(['generate', *args])
            self.assertEqual(artifact.read_bytes(), expected)
            self.assertEqual(json.loads(output.getvalue()), self.verify(self.artifact()))
            with contextlib.redirect_stdout(io.StringIO()), self.assertRaises(FileExistsError):
                main(['generate', *args])
            self.assertEqual(artifact.read_bytes(), expected)
            with contextlib.redirect_stdout(io.StringIO()) as output:
                main(['validate', *args])
            self.assertEqual(json.loads(output.getvalue()), self.verify(self.artifact()))
            self.assertEqual(artifact.read_bytes(), expected)
            forged = json.loads(expected)
            forged['counts']['reviewed_rule_rows'] = 1
            artifact.write_text(json.dumps(forged))
            before = artifact.read_bytes()
            with contextlib.redirect_stdout(io.StringIO()), self.assertRaises(ValueError):
                main(['validate', *args])
            self.assertEqual(artifact.read_bytes(), before)

    def test_actual_process_cli_generate_validate(self):
        with tempfile.TemporaryDirectory() as temp:
            artifact = Path(temp) / 'function-items.json'
            args = [sys.executable, str(BASE / 'reconcile_token_rules.py'), '--profile', 'function-items',
                    '--rule-index', str(self.index), '--inventory', str(self.inventory),
                    '--source-root', str(self.source), '--artifact', str(artifact)]
            expected = (json.dumps(self.artifact(), indent=2) + '\n').encode()
            for action in ('generate', 'validate'):
                result = subprocess.run([*args[:2], action, *args[2:]], capture_output=True, text=True, check=False)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertEqual(json.loads(result.stdout), self.verify(self.artifact()))
                self.assertEqual(artifact.read_bytes(), expected)



    def test_complete_identity_coercion_and_trait_caveats_remain_open(self):
        import function_items_reconciliation_proposals as proposals
        rows = {r.split('|')[0]: r.split('|') for r in proposals.ROWS.strip().splitlines()}
        phrases = {
            'type.fn-item': ('heading', 'not standalone runtime behavior'),
            'type.fn-item.intro': ('tuple-like struct or enum variant', 'zero-sized', 'without imported layout or alignment'),
            'type.fn-item.unique': ('early-bound lifetime arguments', 'not late-bound lifetime arguments',
                'assigned only when called', 'resolved declarations', 'ordered type and early lifetime arguments',
                'no-indirection-needed explanation versus a mandatory emitted instruction shape'),
            'type.fn-item.name': ('No syntax directly refers', 'diagnostics display illustrative',
                'fn(signature) {fn_name}', 'E0308', 'not an exact diagnostic-byte promise',
                'possible function-pointer coercion'),
            'type.fn-item.coercion': ('same-signature function pointers', 'directly expected',
                'same if or match', 'not imported complete coercion proofs'),
            'type.fn-item.traits': ('All function items implement Copy, Clone, Send and Sync',
                'Fn, FnMut and FnOnce', 'unsafe', 'target_feature', 'ABI other than Rust',
                'each separate exclusion', 'without unsafe or unsupported CPU-feature execution',
                'unconditional versus conditional', 'ABI identity rather than extern spelling'),
        }
        for label, needs in phrases.items():
            for phrase in needs:
                with self.subTest(label=label, phrase=phrase):
                    self.assertIn(phrase, '|'.join(rows[label]))
        for phrase in ('six labels', '54 lines', 'UNREVIEWED', 'early-bound lifetime',
                       'not late-bound lifetime', 'diagnostic', 'not a parseable type syntax',
                       'not unconditional equality', 'all three explicit exclusions'):
            self.assertIn(phrase, ' '.join(proposals.LIMITATIONS))
        for phrase in ('invalid Rust', 'unsupported valid source', 'resource limits',
                       'infrastructure failures', 'Never execute undefined behavior',
                       'No semantic review, execution or completion credit'):
            self.assertIn(phrase, proposals.END_TO_END)


class InferredTypesProposalTests(unittest.TestCase):
    """Complete pinned source-only pointer proposals, never compiler evidence."""
    PAGES = ('src/types/inferred.md',)
    PAGE_HASHES = {'src/types/inferred.md': 'cb764e3df2fd66b1d1968a6fd700259763a1b9b4ea4fa1e047b57ee692cadb58'}
    PINS = (
        ('type.inferred', 1, 3, '32fda05c43cf12a8b91dea5bd897d7d9d98da893ed8b2d53a1713e69d9f6cbef', 'I'),
        ('type.inferred.syntax', 4, 8, '570b9b1faaa6a2f685bfa3a4cc4b39e1683d16d2a09820955a3cfaf0a77df8fe', 'G'),
        ('type.inferred.intro', 9, 18, 'df29e5195085233093823ee0479655666e46669d7ed61354dd5de7b4f22ea156', 'N'),
        ('type.inferred.constraint', 19, 26, '33880d9fea0b66e2ca85cd2022054ee562f9848925d278c17e248aababe72022', 'N'),
    )
    PRIOR_HASHES = dict(FunctionItemsProposalTests.PRIOR_HASHES, **{
        'function-items': '5f0ea0b6463f0f8cd2d080a20b534baa5fb7402641955c1e07fef67f329bbd66'})
    setUp = PrimitiveTypesProposalTests.setUp

    def artifact(self, source=None):
        return build(self.index, self.inventory,
                     self.source if source is None else source, 'inferred-types')

    def verify(self, data, source=None):
        return validate(data, self.index, self.inventory,
                        self.source if source is None else source, 'inferred-types')

    def test_independent_complete_page_label_block_classification_pins(self):
        import re
        import inferred_types_reconciliation_proposals as proposals
        self.assertEqual(proposals.SELECTED, self.PAGES)
        self.assertEqual(proposals.PAGE_SHA256, self.PAGE_HASHES)
        self.assertEqual(proposals.SOURCE_COMMIT, '86635e30bf861a038dc197d7e16fd09e7e514e7a')
        self.assertEqual(proposals.ALIASES, {
            'inferred': 'reference-types-inferred-placeholder-types'})
        classes = {'N': 'normative-semantic', 'G': 'grammar', 'I': 'index',
                   'X': 'cross-reference', 'E': 'edition-context'}
        self.assertEqual(proposals.CLASS_NAMES, classes)
        authored = [r.split('|') for r in proposals.ROWS.strip().splitlines()]
        self.assertTrue(all(len(r) == 7 and all(f.strip() for f in r) for r in authored))
        self.assertEqual(tuple((r[0], r[1]) for r in authored), tuple((r[0], r[4]) for r in self.PINS))
        self.assertEqual(reconciliation.SOURCE_ONLY_PAGE_COUNTS['inferred-types'], (4,))
        data = self.artifact()
        counts = self.verify(data)
        self.assertEqual([counts[k] for k in ('selected_rule_rows', 'unselected_rule_identities',
                                             'global_rule_identities')], [4, 2961, 2965])
        self.assertEqual(counts['proposed_classification_counts'], {
            'normative-semantic': 2, 'grammar': 1, 'index': 1,
            'cross-reference': 0, 'edition-context': 0})
        page = self.PAGES[0]
        raw = (self.source / page).read_bytes()
        self.assertEqual(digest(raw), self.PAGE_HASHES[page])
        lines = raw.splitlines(keepends=True)
        self.assertEqual(len(lines), 26)
        markers = [(i, m.group(1).decode()) for i, line in enumerate(lines, 1)
                   if (m := re.fullmatch(rb'r\[([^\]\r\n]+)\]\r?\n?', line))]
        self.assertEqual(markers, [(r[1], r[0]) for r in self.PINS])
        self.assertEqual(tuple(r['rule'] for r in data['rows']), tuple(r[0] for r in self.PINS))
        self.assertEqual(len({r['id'] for r in data['rows']}), 4)
        self.assertEqual([(p['path'], p['sha256']) for p in data['selected_pages']],
                         [(page, self.PAGE_HASHES[page])])
        for row, authored_row, (label, start, end, block_hash, codes) in zip(data['rows'], authored, self.PINS):
            with self.subTest(rule=label):
                self.assertEqual((row['page'], row['line'], row['end_line']), (page, start, end))
                self.assertEqual(digest(b''.join(lines[start - 1:end])), block_hash)
                self.assertEqual(row['source_block_sha256'], block_hash)
                self.assertEqual(row['classifications'], [classes[c] for c in codes.split()])
                self.assertEqual(row['candidate_obligation_ids'], [proposals.ALIASES[a] for a in authored_row[3].split()])
                self.assertEqual(row['id'], 'reconcile-inferred-types-' + label.encode().hex())
                self.assertEqual(row['review_status'], 'unreviewed')
                self.assertEqual(row['relationship'], 'context-only' if codes == 'I'
                                 else 'proposed-partial-decomposition')
                self.assertEqual(len(row['missing_test_needs']), 4)
                for need, kind in zip(row['missing_test_needs'], ('positive', 'invalid', 'interaction', 'end-to-end')):
                    self.assertTrue(need.startswith(kind + ': Missing '))
                for phrase in ('stable1.98.1 eligibility', 'workers1/8', 'edition2024', 'Linux x86-64',
                               'CHECK must not demand codegen'):
                    self.assertIn(phrase, row['missing_test_needs'][-1])
        for field in ('reviewed_rule_rows', 'rows_with_exact_executable_evidence'):
            self.assertEqual(counts[field], 0)
        index = json.loads(self.index.read_bytes())
        universe = {(p['path'], r['label']) for p in index['pages'] + index['unlinked_markdown_files']
                    for r in p['rules']}
        selected = {(r['page'], r['rule']) for r in data['rows']}
        other = {(r['page'], r['rule']) for r in data['unselected_rules']}
        self.assertEqual((len(selected), len(other), len(universe)), (4, 2961, 2965))
        self.assertFalse(selected & other)
        self.assertEqual(selected | other, universe)
        self.assertEqual(self.artifact(), data)

    def test_each_row_omission_and_actual_changing_classification_forgery(self):
        baseline = self.artifact()
        for i, row in enumerate(baseline['rows']):
            for mutation in ('omission', 'classification'):
                changed = copy.deepcopy(baseline)
                if mutation == 'omission':
                    changed['rows'].pop(i)
                else:
                    changed['rows'][i]['classifications'] = (
                        ['index'] if row['classifications'] != ['index'] else ['grammar'])
                self.assertNotEqual(changed, baseline)
                with self.subTest(rule=row['rule'], mutation=mutation), self.assertRaises(ValueError):
                    self.verify(changed)

    def test_authored_omissions_duplicates_foreign_and_malformed_rows(self):
        import inferred_types_reconciliation_proposals as proposals
        baseline = self.artifact()
        rows = proposals.ROWS.strip().splitlines()
        cases = [[], rows + [rows[0]], ['type.foreign' + rows[0][len('type.inferred'):], *rows[1:]],
                 ['|'.join(rows[0].split('|')[:-1]), *rows[1:]], [rows[0] + '|extra', *rows[1:]]]
        cases.extend(rows[:i] + rows[i + 1:] for i in range(4))
        for col in range(7):
            fields = rows[0].split('|')
            fields[col] = '   '
            cases.append(['|'.join(fields), *rows[1:]])
        for i, changed in enumerate(cases):
            self.assertNotEqual(changed, rows)
            with self.subTest(case=i), mock.patch.object(proposals, 'ROWS', '\n'.join(changed)):
                with self.assertRaises(ValueError):
                    self.artifact()
                with self.assertRaises(ValueError):
                    self.verify(baseline)

    def test_permuted_authored_rows_preserve_canonical_order(self):
        import inferred_types_reconciliation_proposals as proposals
        baseline = self.artifact()
        rows = proposals.ROWS.strip().splitlines()
        for changed in (rows[::-1], rows[2:] + rows[:2], rows[::2] + rows[1::2]):
            with mock.patch.object(proposals, 'ROWS', '\n'.join(changed)):
                self.assertEqual(self.artifact(), baseline)
                self.assertEqual(self.verify(self.artifact()), self.verify(baseline))

    def test_all_30_prior_canonical_bytes_without_new_module(self):
        self.assertEqual(len(self.PRIOR_HASHES), 30)
        self.assertEqual(set(self.PRIOR_HASHES), {p for p, _ in reconciliation.PROPOSAL_COUNTS if p not in ('trait-objects', 'type-parameters', 'nominal-types', 'inferred-types',)})
        with mock.patch.dict(sys.modules, {'inferred_types_reconciliation_proposals': None}):
            for profile, sha in self.PRIOR_HASHES.items():
                with self.subTest(profile=profile):
                    data = build(self.index, self.inventory, self.source, profile)
                    validate(data, self.index, self.inventory, self.source, profile)
                    self.assertEqual(digest((json.dumps(data, indent=2) + '\n').encode()), sha)
            self.assertEqual(digest((json.dumps(build(self.index, self.inventory, self.source), indent=2) + '\n').encode()),
                             self.PRIOR_HASHES['tokens'])

    def test_global_unique_union_and_zero_credit(self):
        index = json.loads(self.index.read_bytes())
        pages = index['pages'] + index['unlinked_markdown_files']
        prior_pages = {p for profile, _ in reconciliation.PROPOSAL_COUNTS if profile not in ('trait-objects', 'type-parameters', 'nominal-types', 'inferred-types',)
                       for p in reconciliation.profile_inputs(profile)[0]}
        prior = {(p['path'], r['label']) for p in pages if p['path'] in prior_pages for r in p['rules']}
        current = {(r['page'], r['rule']) for r in self.artifact()['rows']}
        self.assertEqual((len(prior), len(current), len(prior & current), len(prior | current)), (996, 4, 0, 1000))
        summary = reconciliation.summarize_proposals(self.index, self.inventory, include_proposals=True)
        self.assertEqual((summary['proposed_rule_identities'], summary['rule_identities_without_proposals']), (1030, 1935))
        self.assertEqual(sum(p['proposed_rule_identities'] for p in summary['profiles'].values()), 1035)
        pointer = summary['profiles']['inferred-types']
        self.assertEqual(len(pointer['authored_proposals']), 4)
        for field in ('reviewed_rule_rows', 'rows_with_exact_executable_evidence', 'completed_rule_rows'):
            self.assertEqual(summary[field], 0)
            self.assertEqual(pointer[field], 0)
        self.assertIsNone(pointer['full_rust_completion_percent'])
        result = reconciliation.query_rule_backlog(self.index, self.inventory, page=self.PAGES[0], include_proposals=True)
        self.assertEqual((result['selected_rule_identities'], result['selected_authored_proposal_count']), (4, 4))
        self.assertEqual(result['selected_rule_identities_without_proposals'], 0)
        self.assertTrue(all(r['proposal_profiles'] == ['inferred-types'] for r in result['rows']))
        self.assertEqual(reconciliation.query_rule_backlog(self.index, self.inventory, page=self.PAGES[0], missing_only=True)['rows'], [])

    test_each_source_page_fault_preserves_exact_error_types = PrimitiveTypesProposalTests.test_each_source_page_fault_preserves_exact_error_types
    def test_29_actual_changing_forgeries_rejected(self):
        baseline = self.artifact()
        mutations = (
            lambda d: d['rows'].pop(),
            lambda d: d['rows'].append(copy.deepcopy(d['rows'][0])),
            lambda d: d['rows'].reverse(),
            lambda d: d['rows'][0].update(page='src/expressions/block-expr.md'),
            lambda d: d['rows'][-1].update(page='src/expressions/struct-expr.md'),
            lambda d: d['rows'][0].update(rule='expr.loop'),
            lambda d: d['rows'][0].update(line=True),
            lambda d: d['rows'][0].update(line=1.0),
            lambda d: d['counts'].update(reviewed_rule_rows=False),
            lambda d: d['rows'][0].update(relationship='exact-executable-evidence'),
            lambda d: d['rows'][0].update(classifications=['grammar']),
            lambda d: d['rows'][0].update(end_line=99999),
            lambda d: d['rows'][0].update(source_block_sha256='0' * 64),
            lambda d: d['rows'][-1].update(source_block_sha256='0' * 64),
            lambda d: d['rows'][0].update(id='reconcile-foreign-expr'),
            lambda d: d['rows'][0].update(id=d['rows'][1]['id']),
            lambda d: d['rows'][0].update(review_status='reviewed'),
            lambda d: d['rows'][0].update(rationale='invented interpretation'),
            lambda d: d['rows'][0].update(candidate_obligation_ids=['foreign']),
            lambda d: d['rows'][0].update(missing_test_needs=[]),
            lambda d: d['rows'][0]['missing_test_needs'].__setitem__(0, 'positive: implemented'),
            lambda d: d['counts'].update(reviewed_rule_rows=1),
            lambda d: d['counts'].update(rows_with_exact_executable_evidence=1),
            lambda d: d['selected_pages'][0].update(sha256='0' * 64),
            lambda d: d['selected_pages'].clear(),
            lambda d: d['bindings'].update(reference_commit='0' * 40),
            lambda d: d['unselected_rules'].pop(),
            lambda d: d['unselected_rules'].append(copy.deepcopy(d['unselected_rules'][0])),
            lambda d: d['unselected_rules'][0].update(page=d['rows'][0]['page'], rule=d['rows'][0]['rule']),
        )
        self.assertEqual(len(mutations), 29)
        for i, mutate in enumerate(mutations):
            changed = copy.deepcopy(baseline)
            mutate(changed)
            self.assertNotEqual(json.dumps(changed, sort_keys=True), json.dumps(baseline, sort_keys=True))
            with self.subTest(mutation=i), self.assertRaises(ValueError):
                self.verify(changed)

    def test_source_only_restrictions_precede_io_and_execution(self):
        args = ['--profile', 'inferred-types', '--rule-index', '/absent/index', '--inventory', '/absent/inventory',
                '--source-root', '/absent/source', '--artifact', '/absent/artifact']
        extras = ([], ['--repo-root', '/absent/repo'], ['--evidence', '/absent/evidence'],
                  ['--repo-root', '/absent/repo', '--evidence', '/absent/evidence'])
        cases = [(a, e) for a in ('generate-witness-links', 'build-witness-links', 'validate-witness-links',
                                  'assess-witness-links') for e in extras]
        cases += [(a, e) for a in ('generate', 'validate') for e in extras[1:]]
        with contextlib.ExitStack() as stack:
            for name in ('read_bytes', 'read_text', 'open', 'is_symlink', 'is_file', 'write_bytes', 'write_text', 'mkdir'):
                stack.enter_context(mock.patch.object(Path, name, side_effect=AssertionError('IO')))
            stack.enter_context(mock.patch.object(subprocess, 'run', side_effect=AssertionError('execution')))
            for action, extra in cases:
                with self.subTest(action=action, extra=extra), self.assertRaises(ValueError):
                    main([action, *args, *extra])
            for call in (
                lambda: reconciliation.build_witness_links(self.index, self.inventory, self.source, self.source, 'inferred-types'),
                lambda: reconciliation.validate_witness_links({}, self.index, self.inventory, self.source, self.source, 'inferred-types'),
                lambda: reconciliation.assess_witness_links({}, self.index, self.inventory, self.source, self.source, (), 'inferred-types'),
            ):
                with self.assertRaises(ValueError):
                    call()

    def test_cli_exclusive_generation_validation_and_no_repair(self):
        with tempfile.TemporaryDirectory() as temp:
            artifact = Path(temp) / 'inferred-types.json'
            args = ['--profile', 'inferred-types', '--rule-index', str(self.index), '--inventory', str(self.inventory),
                    '--source-root', str(self.source), '--artifact', str(artifact)]
            expected = (json.dumps(self.artifact(), indent=2) + '\n').encode()
            with contextlib.redirect_stdout(io.StringIO()) as output:
                main(['generate', *args])
            self.assertEqual(artifact.read_bytes(), expected)
            self.assertEqual(json.loads(output.getvalue()), self.verify(self.artifact()))
            with contextlib.redirect_stdout(io.StringIO()), self.assertRaises(FileExistsError):
                main(['generate', *args])
            self.assertEqual(artifact.read_bytes(), expected)
            with contextlib.redirect_stdout(io.StringIO()) as output:
                main(['validate', *args])
            self.assertEqual(json.loads(output.getvalue()), self.verify(self.artifact()))
            self.assertEqual(artifact.read_bytes(), expected)
            forged = json.loads(expected)
            forged['counts']['reviewed_rule_rows'] = 1
            artifact.write_text(json.dumps(forged))
            before = artifact.read_bytes()
            with contextlib.redirect_stdout(io.StringIO()), self.assertRaises(ValueError):
                main(['validate', *args])
            self.assertEqual(artifact.read_bytes(), before)

    def test_actual_process_cli_generate_validate(self):
        with tempfile.TemporaryDirectory() as temp:
            artifact = Path(temp) / 'inferred-types.json'
            args = [sys.executable, str(BASE / 'reconcile_token_rules.py'), '--profile', 'inferred-types',
                    '--rule-index', str(self.index), '--inventory', str(self.inventory),
                    '--source-root', str(self.source), '--artifact', str(artifact)]
            expected = (json.dumps(self.artifact(), indent=2) + '\n').encode()
            for action in ('generate', 'validate'):
                result = subprocess.run([*args[:2], action, *args[2:]], capture_output=True, text=True, check=False)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertEqual(json.loads(result.stdout), self.verify(self.artifact()))
                self.assertEqual(artifact.read_bytes(), expected)



    def test_scope_and_partial_alias_are_not_semantic_credit(self):
        import inferred_types_reconciliation_proposals as proposals
        text = ' '.join(proposals.LIMITATIONS)
        for phrase in ('26 lines', 'UNREVIEWED', 'not a complete inference algorithm',
                       'unification', 'numeric defaulting', 'trait-solving', 'partial routing only',
                       'type context', 'wildcard pattern', 'inferred const argument',
                       'expression placeholder', 'if possible', 'ambiguity',
                       'version and position', 'stock-valid fixture into invalid'):
            self.assertIn(phrase, text)
        inventory = json.loads(self.inventory.read_bytes())
        self.assertIn('reference-types-inferred-placeholder-types', json.dumps(inventory))
        for phrase in ('invalid Rust', 'unsupported valid source', 'resource limits',
                       'infrastructure failures', 'No semantic review, execution or completion credit'):
            self.assertIn(phrase, proposals.END_TO_END)





class NominalTypesProposalTests(unittest.TestCase):
    """Independent source pins and source-only checks, not semantic evidence."""
    PAGES = ('src/types/enum.md', 'src/types/struct.md', 'src/types/union.md')
    PAGE_HASHES = {
        'src/types/enum.md': 'f64201be5124c2b4b2e5e753af02c7334cb601e574ad9b50420471fb30e4b393',
        'src/types/struct.md': '5e79e4a28f9690d8999c9263e754240443ca313fb88645068e821f1b8b5d0a9d',
        'src/types/union.md': '41c02a3c65cd4f6340f77d61d4027508cf7eaedd8d3a6e80ef7e912640926459',
    }
    PINS = (
        ('src/types/enum.md', 'type.enum', 1, 3, 'ffbaff9be7b2d74574bb71209206c3a5165597c6cb0017cf436bbc7fbb6a8a29', 'I'),
        ('src/types/enum.md', 'type.enum.intro', 4, 6, '945ffceb782dfcfa828aef7b53f6cc54cc89227e1f914f3a08d0bf8d61c7ceaa', 'N X'),
        ('src/types/enum.md', 'type.enum.declaration', 7, 9, '49656e80899882cd30ca9e0041c2be452539782b02425a3d8e8ff9f7a327aac8', 'N X'),
        ('src/types/enum.md', 'type.enum.constructor', 10, 12, '476897ba2ba0f84851f53d30a9c5431e08f0d27547804acb173b634a784019fc', 'N X'),
        ('src/types/enum.md', 'type.enum.value', 13, 15, '337be5b6d21a889054d60c394834619172990365e1ed62d7f9b4b0c7479d21b9', 'N'),
        ('src/types/enum.md', 'type.enum.name', 16, 22, '040af0f74b330d7fb89aa5227a875975d320a45d35411d227c6c65495e6ceb99', 'N X'),
        ('src/types/struct.md', 'type.struct', 1, 3, '6651116793c9b22489dd1636c37a98da032d716106b3984cb973de953ba586e6', 'I'),
        ('src/types/struct.md', 'type.struct.intro', 4, 6, 'c8d54c30aa8fa1627e2afed3f50370e3092d876aacdb1630c9c2fe0f71d0cd5d', 'N'),
        ('src/types/struct.md', 'type.struct.constructor', 7, 9, 'e67bd2e82efe64c7c2e69e3b18ec3b1e8af9211ef03796c4da08ae0a16a150ba', 'N X'),
        ('src/types/struct.md', 'type.struct.layout', 10, 12, 'b377c4d35a975ac803a9c2c3de32aedf8252ff3b29fd87b2b8c705d375cb775e', 'N X'),
        ('src/types/struct.md', 'type.struct.field-visibility', 13, 15, 'f53565e84529737a0f44579db51222ccf711726776c74ca07259200c1e314bfd', 'N X'),
        ('src/types/struct.md', 'type.struct.tuple', 16, 18, 'ee5a4da2c65ddbda09eec7c7cbf18f9ae49c489da5c16ed8dffe90de8fe2e221', 'N'),
        ('src/types/struct.md', 'type.struct.unit', 19, 26, '5e8d2259a782fa1cb124393d82dace528fa5339a6b0ed4a358a9912652d2b467', 'N X'),
        ('src/types/union.md', 'type.union', 1, 3, 'e76946eb5437cd7313fda12326d148be0d05c15e1ffaeb0051b9f656a1e7eee3', 'I'),
        ('src/types/union.md', 'type.union.intro', 4, 6, '782dd9a6ddc4c6281f8b7787b029f4444b005b4037bb1a18a4a67ae63ff1fecc', 'N X'),
        ('src/types/union.md', 'type.union.access', 7, 9, '4f8e3c686629fbebe12a2c770402dcd97ddba3b4e395ef980b534d87cc5c96d2', 'N'),
        ('src/types/union.md', 'type.union.safety', 10, 12, '7f3c2791de873cdb75ea51a864e82f9dcbd71de47c02d01dff9c68654e6a0848', 'N'),
        ('src/types/union.md', 'type.union.constraint', 13, 15, 'ba9e810e5e4f551072424d2f18e2730c4d4cee7c6f24eec33036606359a8e101', 'N X'),
        ('src/types/union.md', 'type.union.layout', 16, 20, '4725adc00dd22222b1c5dc5fe35275ad2ee0f0e7317ea32845e55a6dec467a86', 'N'),
    )
    PRIOR_HASHES = dict(InferredTypesProposalTests.PRIOR_HASHES, **{
        'inferred-types': 'acc2b0f1cfbd1ffc0901069700ba5bd38a3d76329bf7ca2161b3ce9e2313d5c8'})
    setUp = PrimitiveTypesProposalTests.setUp

    def artifact(self, source=None):
        return build(self.index, self.inventory, self.source if source is None else source, 'nominal-types')

    def verify(self, data, source=None):
        return validate(data, self.index, self.inventory, self.source if source is None else source, 'nominal-types')

    def test_independent_complete_pages_and_literal_row_pins(self):
        import re
        import nominal_types_reconciliation_proposals as proposals
        classes = {'N': 'normative-semantic', 'G': 'grammar', 'I': 'index',
                   'X': 'cross-reference', 'E': 'edition-context'}
        self.assertEqual(proposals.SELECTED, self.PAGES)
        self.assertEqual(tuple(sorted(self.PAGES)), self.PAGES)
        self.assertEqual(proposals.PAGE_SHA256, self.PAGE_HASHES)
        self.assertEqual(proposals.SOURCE_COMMIT, '86635e30bf861a038dc197d7e16fd09e7e514e7a')
        self.assertEqual(proposals.ALIASES, {
            'struct': 'reference-types-struct-nominal-structs',
            'enum': 'reference-types-enum-nominal-enums',
            'union': 'reference-types-union-union-identity'})
        self.assertEqual(proposals.CLASS_NAMES, classes)
        authored = [r.split('|') for r in proposals.ROWS.strip().splitlines()]
        self.assertEqual(len(authored), 19)
        self.assertTrue(all(len(r) == 7 and all(f.strip() for f in r) for r in authored))
        self.assertEqual(tuple((r[0], r[1]) for r in authored), tuple((r[1], r[5]) for r in self.PINS))
        self.assertEqual(reconciliation.SOURCE_ONLY_PAGE_COUNTS['nominal-types'], (6, 7, 6))
        data = self.artifact()
        counts = self.verify(data)
        self.assertEqual([counts[k] for k in ('selected_rule_rows', 'unselected_rule_identities',
                                             'global_rule_identities')], [19, 2946, 2965])
        self.assertEqual(counts['proposed_classification_counts'], {
            'normative-semantic': 16, 'grammar': 0, 'index': 3,
            'cross-reference': 10, 'edition-context': 0})
        self.assertEqual([(p['path'], p['sha256']) for p in data['selected_pages']],
                         [(p, self.PAGE_HASHES[p]) for p in self.PAGES])
        for page, length, expected in zip(self.PAGES, (22, 26, 20), (6, 7, 6)):
            raw = (self.source / page).read_bytes()
            self.assertEqual(digest(raw), self.PAGE_HASHES[page])
            lines = raw.splitlines(keepends=True)
            self.assertEqual(len(lines), length)
            markers = [(i, m.group(1).decode()) for i, line in enumerate(lines, 1)
                       if (m := re.fullmatch(rb'r\[([^\]\r\n]+)\]\r?\n?', line))]
            pins = [r for r in self.PINS if r[0] == page]
            self.assertEqual(len(markers), expected)
            self.assertEqual(markers, [(r[2], r[1]) for r in pins])
        self.assertEqual(tuple((r['page'], r['rule']) for r in data['rows']),
                         tuple((r[0], r[1]) for r in self.PINS))
        self.assertEqual(len({r['id'] for r in data['rows']}), 19)
        for row, authored_row, (page, label, start, end, block_hash, code) in zip(data['rows'], authored, self.PINS):
            with self.subTest(rule=label):
                lines = (self.source / page).read_bytes().splitlines(keepends=True)
                self.assertEqual((row['line'], row['end_line']), (start, end))
                self.assertEqual(digest(b''.join(lines[start - 1:end])), block_hash)
                self.assertEqual(row['source_block_sha256'], block_hash)
                self.assertEqual(row['classifications'], [classes[c] for c in code.split()])
                self.assertEqual(row['candidate_obligation_ids'], [proposals.ALIASES[authored_row[3]]])
                self.assertEqual(row['id'], 'reconcile-nominal-types-' + label.encode().hex())
                self.assertEqual(row['review_status'], 'unreviewed')
                self.assertEqual(row['relationship'], 'context-only' if code == 'I' else 'proposed-partial-decomposition')
                self.assertEqual(len(row['missing_test_needs']), 4)
                for need, kind in zip(row['missing_test_needs'], ('positive', 'invalid', 'interaction', 'end-to-end')):
                    self.assertTrue(need.startswith(kind + ': Missing '))
                for phrase in ('stable1.98.1 eligibility', 'workers1/8', 'edition2024', 'Linux x86-64',
                               'CHECK must not demand codegen'):
                    self.assertIn(phrase, row['missing_test_needs'][-1])
        index = json.loads(self.index.read_bytes())
        universe = {(p['path'], r['label']) for p in index['pages'] + index['unlinked_markdown_files'] for r in p['rules']}
        selected = {(r['page'], r['rule']) for r in data['rows']}
        other = {(r['page'], r['rule']) for r in data['unselected_rules']}
        self.assertEqual((len(selected), len(other), len(universe)), (19, 2946, 2965))
        self.assertFalse(selected & other)
        self.assertEqual(selected | other, universe)
        for field in ('reviewed_rule_rows', 'rows_with_exact_executable_evidence'):
            self.assertEqual(counts[field], 0)

    test_each_row_omission_and_actual_changing_classification_forgery = InferredTypesProposalTests.test_each_row_omission_and_actual_changing_classification_forgery
    test_each_source_page_fault_preserves_exact_error_types = PrimitiveTypesProposalTests.test_each_source_page_fault_preserves_exact_error_types
    test_29_actual_changing_forgeries_rejected = InferredTypesProposalTests.test_29_actual_changing_forgeries_rejected

    def test_authored_omissions_duplicates_foreign_and_malformed_rows(self):
        import nominal_types_reconciliation_proposals as proposals
        baseline = self.artifact()
        rows = proposals.ROWS.strip().splitlines()
        cases = [[], rows + [rows[0]], ['type.foreign' + rows[0][len('type.enum'):], *rows[1:]],
                 ['|'.join(rows[0].split('|')[:-1]), *rows[1:]], [rows[0] + '|extra', *rows[1:]]]
        cases.extend(rows[:i] + rows[i + 1:] for i in range(19))
        for col in range(7):
            fields = rows[0].split('|')
            fields[col] = '   '
            cases.append(['|'.join(fields), *rows[1:]])
        for i, changed in enumerate(cases):
            self.assertNotEqual(changed, rows)
            with self.subTest(case=i), mock.patch.object(proposals, 'ROWS', '\n'.join(changed)):
                with self.assertRaises(ValueError):
                    self.artifact()
                with self.assertRaises(ValueError):
                    self.verify(baseline)

    def test_permuted_authored_rows_preserve_canonical_order(self):
        import nominal_types_reconciliation_proposals as proposals
        baseline = self.artifact()
        rows = proposals.ROWS.strip().splitlines()
        for changed in (rows[::-1], rows[2:] + rows[:2], rows[::2] + rows[1::2]):
            with mock.patch.object(proposals, 'ROWS', '\n'.join(changed)):
                self.assertEqual(self.artifact(), baseline)
                self.assertEqual(self.verify(self.artifact()), self.verify(baseline))

    def test_all_31_prior_canonical_bytes_without_new_module(self):
        self.assertEqual(len(self.PRIOR_HASHES), 31)
        self.assertEqual(set(self.PRIOR_HASHES), {p for p, _ in reconciliation.PROPOSAL_COUNTS if p not in ('trait-objects', 'type-parameters', 'nominal-types')})
        with mock.patch.dict(sys.modules, {'nominal_types_reconciliation_proposals': None}):
            for profile, sha in self.PRIOR_HASHES.items():
                with self.subTest(profile=profile):
                    data = build(self.index, self.inventory, self.source, profile)
                    validate(data, self.index, self.inventory, self.source, profile)
                    self.assertEqual(digest((json.dumps(data, indent=2) + '\n').encode()), sha)

    def test_global_unique_union_and_zero_credit(self):
        index = json.loads(self.index.read_bytes())
        pages = index['pages'] + index['unlinked_markdown_files']
        prior_pages = {p for profile, _ in reconciliation.PROPOSAL_COUNTS if profile not in ('trait-objects', 'type-parameters', 'nominal-types')
                       for p in reconciliation.profile_inputs(profile)[0]}
        prior = {(p['path'], r['label']) for p in pages if p['path'] in prior_pages for r in p['rules']}
        current = {(r['page'], r['rule']) for r in self.artifact()['rows']}
        self.assertEqual((len(prior), len(current), len(prior & current), len(prior | current)), (1000, 19, 0, 1019))
        summary = reconciliation.summarize_proposals(self.index, self.inventory, include_proposals=True)
        self.assertEqual((summary['proposed_rule_identities'], summary['rule_identities_without_proposals']), (1030, 1935))
        self.assertEqual(sum(p['proposed_rule_identities'] for p in summary['profiles'].values()), 1035)
        nominal = summary['profiles']['nominal-types']
        self.assertEqual(len(nominal['authored_proposals']), 19)
        for field in ('reviewed_rule_rows', 'rows_with_exact_executable_evidence', 'completed_rule_rows'):
            self.assertEqual(summary[field], 0)
            self.assertEqual(nominal[field], 0)
        self.assertIsNone(nominal['full_rust_completion_percent'])
        for page, count in zip(self.PAGES, (6, 7, 6)):
            result = reconciliation.query_rule_backlog(self.index, self.inventory, page=page, include_proposals=True)
            self.assertEqual((result['selected_rule_identities'], result['selected_authored_proposal_count']), (count, count))
            self.assertEqual(result['selected_rule_identities_without_proposals'], 0)
            self.assertTrue(all(r['proposal_profiles'] == ['nominal-types'] for r in result['rows']))
            self.assertEqual(reconciliation.query_rule_backlog(self.index, self.inventory, page=page, missing_only=True)['rows'], [])

    def test_source_only_restrictions_precede_io_and_execution(self):
        args = ['--profile', 'nominal-types', '--rule-index', '/absent/index', '--inventory', '/absent/inventory',
                '--source-root', '/absent/source', '--artifact', '/absent/artifact']
        extras = ([], ['--repo-root', '/absent/repo'], ['--evidence', '/absent/evidence'],
                  ['--repo-root', '/absent/repo', '--evidence', '/absent/evidence'])
        cases = [(a, e) for a in ('generate-witness-links', 'build-witness-links', 'validate-witness-links',
                                  'assess-witness-links') for e in extras]
        cases += [(a, e) for a in ('generate', 'validate') for e in extras[1:]]
        with contextlib.ExitStack() as stack:
            for name in ('read_bytes', 'read_text', 'open', 'is_symlink', 'is_file', 'write_bytes', 'write_text', 'mkdir'):
                stack.enter_context(mock.patch.object(Path, name, side_effect=AssertionError('IO')))
            stack.enter_context(mock.patch.object(subprocess, 'run', side_effect=AssertionError('execution')))
            for action, extra in cases:
                with self.subTest(action=action, extra=extra), self.assertRaises(ValueError):
                    main([action, *args, *extra])
            for call in (
                lambda: reconciliation.build_witness_links(self.index, self.inventory, self.source, self.source, 'nominal-types'),
                lambda: reconciliation.validate_witness_links({}, self.index, self.inventory, self.source, self.source, 'nominal-types'),
                lambda: reconciliation.assess_witness_links({}, self.index, self.inventory, self.source, self.source, (), 'nominal-types'),
            ):
                with self.assertRaises(ValueError):
                    call()

    def test_cli_exclusive_generation_validation_and_actual_process(self):
        with tempfile.TemporaryDirectory() as temp:
            artifact = Path(temp) / 'nominal-types.json'
            args = ['--profile', 'nominal-types', '--rule-index', str(self.index), '--inventory', str(self.inventory),
                    '--source-root', str(self.source), '--artifact', str(artifact)]
            expected = (json.dumps(self.artifact(), indent=2) + '\n').encode()
            with contextlib.redirect_stdout(io.StringIO()):
                main(['generate', *args])
            self.assertEqual(artifact.read_bytes(), expected)
            with contextlib.redirect_stdout(io.StringIO()), self.assertRaises(FileExistsError):
                main(['generate', *args])
            for action in ('validate',):
                with contextlib.redirect_stdout(io.StringIO()) as output:
                    main([action, *args])
                self.assertEqual(json.loads(output.getvalue()), self.verify(self.artifact()))
                self.assertEqual(artifact.read_bytes(), expected)
            forged = json.loads(expected)
            forged['counts']['reviewed_rule_rows'] = 1
            artifact.write_text(json.dumps(forged))
            before = artifact.read_bytes()
            with contextlib.redirect_stdout(io.StringIO()), self.assertRaises(ValueError):
                main(['validate', *args])
            self.assertEqual(artifact.read_bytes(), before)
            artifact.unlink()
            for action in ('generate', 'validate'):
                argv = [sys.executable, '-B', str(BASE / 'reconcile_token_rules.py'), action, *args]
                result = subprocess.run(argv, capture_output=True, text=True, check=False)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertEqual(json.loads(result.stdout), self.verify(self.artifact()))
                self.assertEqual(artifact.read_bytes(), expected)

    def test_caveats_remain_explicit_without_layout_or_unsafe_license(self):
        import nominal_types_reconciliation_proposals as proposals
        text = ' '.join(proposals.LIMITATIONS)
        for phrase in ('26 lines, 7 labels', '22 lines, 6 labels', '20 lines, 6 labels', 'UNREVIEWED',
                       'eligibility remains unknown', 'partial routing only', 'not structural',
                       'not an exact additive byte formula', 'fixed offsets', 'niche optimization',
                       'current stable ABI claim', 'version, representation and target authority',
                       'layout is unspecified', 'need not be at offset 0', 'READ differs from write',
                       'not a validity proof', 'no UB execution or transmute license', 'linked item allowlist',
                       'struct, tuple and unit variants', 'one inhabited value', 'not a promise about all layouts'):
            self.assertIn(phrase, text)
        rows = {r.split('|')[0]: r for r in proposals.ROWS.strip().splitlines()}
        for label, phrases in {
            'type.enum.value': ('historical', 'not a verified exact additive byte formula', 'niche optimization', 'target authority'),
            'type.union.safety': ('unsafe', 'READ', 'write', 'validity proof', 'never UB execution'),
            'type.union.constraint': ('linked item allowlist', 'without inventing a Copy-only'),
            'type.union.layout': ('need not be at offset 0', 'target', 'without universal zero offsets'),
            'type.enum.name': ('not structurally', 'declaration identity'),
            'type.struct.unit': ('only inhabited value', 'without a promise about all layouts'),
        }.items():
            for phrase in phrases:
                self.assertIn(phrase, rows[label])


class TypeParametersProposalTests(unittest.TestCase):
    """One mixed heading/body source label, not a generics support claim."""
    PAGES = ('src/types/parameters.md',)
    PAGE_HASHES = {'src/types/parameters.md': 'b57d9fa6ced214a50c5f25ae1fc1456d2858453c33901d800de940b2f606abb1'}
    PINS = (('type.generic', 1, 18,
             'b57d9fa6ced214a50c5f25ae1fc1456d2858453c33901d800de940b2f606abb1', 'N I'),)
    PRIOR_HASHES = dict(NominalTypesProposalTests.PRIOR_HASHES, **{
        'nominal-types': '70aa4e1a12ad7c509e98deafeab7673e6f64face95a1b172fc46d5a540194d43'})
    setUp = PrimitiveTypesProposalTests.setUp

    def artifact(self, source=None):
        return build(self.index, self.inventory, self.source if source is None else source, 'type-parameters')

    def verify(self, data, source=None):
        return validate(data, self.index, self.inventory, self.source if source is None else source, 'type-parameters')

    def test_independent_complete_page_rule_line_block_class_pins(self):
        import re
        import type_parameters_reconciliation_proposals as proposals
        self.assertEqual(proposals.SELECTED, self.PAGES)
        self.assertEqual(proposals.PAGE_SHA256, self.PAGE_HASHES)
        self.assertEqual(proposals.SOURCE_COMMIT, '86635e30bf861a038dc197d7e16fd09e7e514e7a')
        self.assertEqual(proposals.ALIASES, {'parameters': 'reference-types-parameters-rigid-parameters'})
        self.assertEqual(proposals.CLASS_NAMES, {'N': 'normative-semantic', 'G': 'grammar', 'I': 'index',
                                              'X': 'cross-reference', 'E': 'edition-context'})
        authored = [r.split('|') for r in proposals.ROWS.strip().splitlines()]
        self.assertEqual(len(authored), 1)
        self.assertTrue(all(len(r) == 7 and all(f.strip() for f in r) for r in authored))
        self.assertEqual((authored[0][0], authored[0][1], authored[0][3]), ('type.generic', 'N I', 'parameters'))
        self.assertEqual(reconciliation.SOURCE_ONLY_PAGE_COUNTS['type-parameters'], (1,))
        data = self.artifact()
        counts = self.verify(data)
        self.assertEqual([counts[k] for k in ('selected_rule_rows', 'unselected_rule_identities',
                                             'global_rule_identities')], [1, 2964, 2965])
        self.assertEqual(counts['proposed_classification_counts'], {
            'normative-semantic': 1, 'grammar': 0, 'index': 1, 'cross-reference': 0, 'edition-context': 0})
        raw = (self.source / self.PAGES[0]).read_bytes()
        self.assertEqual(digest(raw), self.PAGE_HASHES[self.PAGES[0]])
        lines = raw.splitlines(keepends=True)
        self.assertEqual(len(lines), 18)
        markers = [(i, m.group(1).decode()) for i, line in enumerate(lines, 1)
                   if (m := re.fullmatch(rb'r\[([^\]\r\n]+)\]\r?\n?', line))]
        self.assertEqual(markers, [(1, 'type.generic')])
        self.assertIn(b'# Type parameters', raw)
        self.assertIn(b'the names of its type parameters are types:', raw)
        self.assertEqual([(p['path'], p['sha256']) for p in data['selected_pages']],
                         [(self.PAGES[0], self.PAGE_HASHES[self.PAGES[0]])])
        self.assertEqual(len(data['rows']), 1)
        row = data['rows'][0]
        label, start, end, block_hash, codes = self.PINS[0]
        self.assertEqual((row['page'], row['rule'], row['line'], row['end_line']),
                         (self.PAGES[0], label, start, end))
        self.assertEqual(digest(b''.join(lines[start - 1:end])), block_hash)
        self.assertEqual(row['source_block_sha256'], block_hash)
        self.assertEqual(row['classifications'], ['normative-semantic', 'index'])
        self.assertEqual(row['id'], 'reconcile-type-parameters-747970652e67656e65726963')
        self.assertEqual(row['candidate_obligation_ids'], ['reference-types-parameters-rigid-parameters'])
        self.assertEqual(row['relationship'], 'proposed-partial-decomposition')
        self.assertEqual(row['review_status'], 'unreviewed')
        self.assertEqual(len(row['missing_test_needs']), 4)
        for need, kind in zip(row['missing_test_needs'], ('positive', 'invalid', 'interaction', 'end-to-end')):
            self.assertTrue(need.startswith(kind + ': Missing '))
        for phrase in ('stable1.98.1 eligibility', 'workers1/8', 'edition2024', 'Linux x86-64',
                       'CHECK must not demand codegen', 'unused invalid', 'source-authority', 'forged-fact'):
            self.assertIn(phrase, row['missing_test_needs'][-1])
        index = json.loads(self.index.read_bytes())
        universe = {(p['path'], r['label']) for p in index['pages'] + index['unlinked_markdown_files'] for r in p['rules']}
        selected = {(r['page'], r['rule']) for r in data['rows']}
        other = {(r['page'], r['rule']) for r in data['unselected_rules']}
        self.assertEqual((len(selected), len(other), len(universe)), (1, 2964, 2965))
        self.assertFalse(selected & other)
        self.assertEqual(selected | other, universe)
        self.assertEqual(counts['reviewed_rule_rows'], 0)
        self.assertEqual(counts['rows_with_exact_executable_evidence'], 0)
        self.assertEqual(self.artifact(), data)

    test_each_row_omission_and_actual_changing_classification_forgery = InferredTypesProposalTests.test_each_row_omission_and_actual_changing_classification_forgery
    test_each_source_page_fault_preserves_exact_error_types = PrimitiveTypesProposalTests.test_each_source_page_fault_preserves_exact_error_types

    def test_singleton_actual_changing_forgery_matrix(self):
        # There is no distinct row order or second row ID to forge in this profile.
        baseline = self.artifact()
        mutations = (
            lambda d: d['rows'].clear(),
            lambda d: d['rows'].append(copy.deepcopy(d['rows'][0])),
            lambda d: d['rows'][0].update(page='src/types/inferred.md'),
            lambda d: d['rows'][0].update(rule='type.inferred'),
            lambda d: d['rows'][0].update(line=True),
            lambda d: d['rows'][0].update(line=1.0),
            lambda d: d['rows'][0].update(end_line=17),
            lambda d: d['rows'][0].update(source_block_sha256='0' * 64),
            lambda d: d['rows'][0].update(id='reconcile-foreign'),
            lambda d: d['rows'][0].update(classifications=['index']),
            lambda d: d['rows'][0].update(classifications=['normative-semantic']),
            lambda d: d['rows'][0].update(classifications=['grammar']),
            lambda d: d['rows'][0].update(relationship='context-only'),
            lambda d: d['rows'][0].update(relationship='exact-executable-evidence'),
            lambda d: d['rows'][0].update(review_status='reviewed'),
            lambda d: d['rows'][0].update(rationale='invented interpretation'),
            lambda d: d['rows'][0].update(candidate_obligation_ids=['foreign']),
            lambda d: d['rows'][0].update(missing_test_needs=[]),
            lambda d: d['rows'][0]['missing_test_needs'].__setitem__(0, 'positive: implemented'),
            lambda d: d['counts'].update(reviewed_rule_rows=False),
            lambda d: d['counts'].update(reviewed_rule_rows=1),
            lambda d: d['counts'].update(rows_with_exact_executable_evidence=1),
            lambda d: d['selected_pages'][0].update(sha256='0' * 64),
            lambda d: d['selected_pages'].clear(),
            lambda d: d['bindings'].update(reference_commit='0' * 40),
            lambda d: d['unselected_rules'].pop(),
            lambda d: d['unselected_rules'].append(copy.deepcopy(d['unselected_rules'][0])),
            lambda d: d['unselected_rules'][0].update(page=d['rows'][0]['page'], rule=d['rows'][0]['rule']),
        )
        self.assertEqual(len(mutations), 28)
        for i, mutate in enumerate(mutations):
            changed = copy.deepcopy(baseline)
            mutate(changed)
            self.assertNotEqual(json.dumps(changed, sort_keys=True), json.dumps(baseline, sort_keys=True))
            with self.subTest(mutation=i), self.assertRaises(ValueError):
                self.verify(changed)

    def test_authored_seven_fields_aliases_foreign_and_malformed_rows(self):
        import type_parameters_reconciliation_proposals as proposals
        baseline = self.artifact()
        rows = proposals.ROWS.strip().splitlines()
        fields = rows[0].split('|')
        cases = [[], rows + rows, ['type.foreign|' + '|'.join(fields[1:])],
                 ['|'.join(fields[:-1])], [rows[0] + '|extra']]
        for col in range(7):
            changed = list(fields)
            changed[col] = '   '
            cases.append(['|'.join(changed)])
        for col, value in ((1, 'UNKNOWN'), (1, 'N N'), (3, 'foreign'), (3, 'parameters parameters')):
            changed = list(fields)
            changed[col] = value
            cases.append(['|'.join(changed)])
        for i, changed in enumerate(cases):
            self.assertNotEqual(changed, rows)
            with self.subTest(case=i), mock.patch.object(proposals, 'ROWS', '\n'.join(changed)):
                with self.assertRaises(ValueError):
                    self.artifact()
                with self.assertRaises(ValueError):
                    self.verify(baseline)
        with mock.patch.object(proposals, 'ALIASES', {'parameters': 'foreign-obligation'}):
            with self.assertRaises(ValueError):
                self.artifact()
            with self.assertRaises(ValueError):
                self.verify(baseline)

    def test_all_32_prior_canonical_bytes_without_new_module(self):
        self.assertEqual(len(self.PRIOR_HASHES), 32)
        self.assertEqual(set(self.PRIOR_HASHES), {p for p, _ in reconciliation.PROPOSAL_COUNTS if p not in ('trait-objects', 'type-parameters')})
        with mock.patch.dict(sys.modules, {'type_parameters_reconciliation_proposals': None}):
            for profile, sha in self.PRIOR_HASHES.items():
                with self.subTest(profile=profile):
                    data = build(self.index, self.inventory, self.source, profile)
                    validate(data, self.index, self.inventory, self.source, profile)
                    self.assertEqual(digest((json.dumps(data, indent=2) + '\n').encode()), sha)

    def test_global_disjoint_union_and_zero_credit(self):
        index = json.loads(self.index.read_bytes())
        pages = index['pages'] + index['unlinked_markdown_files']
        prior_pages = {p for profile, _ in reconciliation.PROPOSAL_COUNTS if profile not in ('trait-objects', 'type-parameters')
                       for p in reconciliation.profile_inputs(profile)[0]}
        prior = {(p['path'], r['label']) for p in pages if p['path'] in prior_pages for r in p['rules']}
        current = {(r['page'], r['rule']) for r in self.artifact()['rows']}
        self.assertEqual((len(prior), len(current), len(prior & current), len(prior | current)), (1019, 1, 0, 1020))
        summary = reconciliation.summarize_proposals(self.index, self.inventory, include_proposals=True)
        self.assertEqual((summary['global_rule_identities'], summary['proposed_rule_identities'],
                          summary['rule_identities_without_proposals']), (2965, 1030, 1935))
        self.assertEqual(sum(p['proposed_rule_identities'] for p in summary['profiles'].values()), 1035)
        profile = summary['profiles']['type-parameters']
        self.assertEqual(len(profile['authored_proposals']), 1)
        for field in ('reviewed_rule_rows', 'rows_with_exact_executable_evidence', 'completed_rule_rows'):
            self.assertEqual(summary[field], 0)
            self.assertEqual(profile[field], 0)
        self.assertIsNone(profile['full_rust_completion_percent'])
        result = reconciliation.query_rule_backlog(self.index, self.inventory, page=self.PAGES[0], include_proposals=True)
        self.assertEqual((result['selected_rule_identities'], result['selected_authored_proposal_count']), (1, 1))
        self.assertEqual(result['selected_rule_identities_without_proposals'], 0)
        self.assertEqual(result['rows'][0]['proposal_profiles'], ['type-parameters'])
        self.assertEqual(reconciliation.query_rule_backlog(self.index, self.inventory, page=self.PAGES[0], missing_only=True)['rows'], [])

    def test_source_only_restrictions_precede_io_and_execution(self):
        args = ['--profile', 'type-parameters', '--rule-index', '/absent/index', '--inventory', '/absent/inventory',
                '--source-root', '/absent/source', '--artifact', '/absent/artifact']
        extras = ([], ['--repo-root', '/absent/repo'], ['--evidence', '/absent/evidence'],
                  ['--repo-root', '/absent/repo', '--evidence', '/absent/evidence'])
        cases = [(a, e) for a in ('generate-witness-links', 'build-witness-links', 'validate-witness-links',
                                  'assess-witness-links') for e in extras]
        cases += [(a, e) for a in ('generate', 'validate') for e in extras[1:]]
        with contextlib.ExitStack() as stack:
            for name in ('read_bytes', 'read_text', 'open', 'is_symlink', 'is_file', 'write_bytes', 'write_text', 'mkdir'):
                stack.enter_context(mock.patch.object(Path, name, side_effect=AssertionError('IO')))
            stack.enter_context(mock.patch.object(subprocess, 'run', side_effect=AssertionError('execution')))
            for action, extra in cases:
                with self.subTest(action=action, extra=extra), self.assertRaises(ValueError):
                    main([action, *args, *extra])
            for call in (
                lambda: reconciliation.build_witness_links(self.index, self.inventory, self.source, self.source, 'type-parameters'),
                lambda: reconciliation.validate_witness_links({}, self.index, self.inventory, self.source, self.source, 'type-parameters'),
                lambda: reconciliation.assess_witness_links({}, self.index, self.inventory, self.source, self.source, (), 'type-parameters'),
            ):
                with self.assertRaises(ValueError):
                    call()

    def test_generation_validation_never_execute(self):
        with mock.patch.object(subprocess, 'run', side_effect=AssertionError('execution')):
            self.verify(self.artifact())
            with tempfile.TemporaryDirectory() as temp:
                artifact = Path(temp) / 'parameters.json'
                args = ['--profile', 'type-parameters', '--rule-index', str(self.index), '--inventory', str(self.inventory),
                        '--source-root', str(self.source), '--artifact', str(artifact)]
                for action in ('generate', 'validate'):
                    with contextlib.redirect_stdout(io.StringIO()):
                        main([action, *args])

    def test_actual_process_cli_exclusive_generation_and_validation(self):
        with tempfile.TemporaryDirectory() as temp:
            artifact = Path(temp) / 'parameters.json'
            args = ['--profile', 'type-parameters', '--rule-index', str(self.index), '--inventory', str(self.inventory),
                    '--source-root', str(self.source), '--artifact', str(artifact)]
            expected = (json.dumps(self.artifact(), indent=2) + '\n').encode()
            for action in ('generate', 'validate'):
                argv = [sys.executable, '-B', str(BASE / 'reconcile_token_rules.py'), action, *args]
                result = subprocess.run(argv, capture_output=True, text=True, check=False)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertEqual(json.loads(result.stdout), self.verify(self.artifact()))
                self.assertEqual(artifact.read_bytes(), expected)
            with contextlib.redirect_stdout(io.StringIO()), self.assertRaises(FileExistsError):
                main(['generate', *args])
            forged = json.loads(expected)
            forged['counts']['reviewed_rule_rows'] = 1
            artifact.write_text(json.dumps(forged))
            before = artifact.read_bytes()
            with contextlib.redirect_stdout(io.StringIO()), self.assertRaises(ValueError):
                main(['validate', *args])
            self.assertEqual(artifact.read_bytes(), before)

    def test_partial_routing_and_owner_identity_caveats(self):
        import type_parameters_reconciliation_proposals as proposals
        text = ' '.join(proposals.LIMITATIONS)
        for phrase in ('18-line', 'one label', 'N I', 'partial routing only', 'universal body checking',
                       'implicit Sized', 'defaults', 'inference', 'monomorphization', 'trait solver',
                       'owning item', 'not declaration identity', 'capture an outer item', 'shadowing',
                       'standard-library', 'UNREVIEWED', 'zero semantic, executable and completion credit'):
            self.assertIn(phrase, text)
        for phrase in ('invalid Rust', 'unsupported valid source', 'resource limits', 'infrastructure faults',
                       'Source identity is not stable eligibility'):
            self.assertIn(phrase, proposals.END_TO_END)


class TraitObjectsProposalTests(unittest.TestCase):
    """Complete pinned source attribution, never trait-object implementation evidence."""
    PAGES = ('src/types/trait-object.md',)
    PAGE_HASHES = {'src/types/trait-object.md': 'f6bf246e7492f682b48085cdbf28e0417c7f96060a4709972d442898f7944daa'}
    PINS = (
        ('type.trait-object', 1, 3, '8c66c043db34170991bfbdc981df90ac0fa29b0b1d215c26c859a0d17861fbf7', 'I'),
        ('type.trait-object.syntax', 4, 13, 'f9ea4f2772c090965167af30cc033f4a505713a3809e2a358ad43a2931af3488', 'G X E'),
        ('type.trait-object.intro', 14, 16, '48dcec65793fa99638239b119b32849a83c37d34aaebc36d5de3eb116ba218f5', 'N X'),
        ('type.trait-object.impls', 17, 19, '7e595f73ce16e2fbfb035e7f779bbdf211cfb9fdd90158b07664addce3ffcd78', 'N X'),
        ('type.trait-object.bounds', 20, 33, 'e300a0eef81ea07e3d0b5788f6d3319be65b50bcff497c2a58a91e0f5724f537', 'N'),
        ('type.trait-object.syntax-edition2021', 34, 37, 'bbcc7a6b74c246b2c7d7494c9463004c6fa5b913e17a5b81a5070a8cbb60553b', 'N E'),
        ('type.trait-object.syntax-edition2018', 38, 45, 'dc5c5fbea1c82649b92953a83fc3cd1e1960fdf15d88204354444f5813224bb3', 'N E X'),
        ('type.trait-object.alias', 46, 48, '601e721e536c2572ebc1282e31e9a40b2ef986e519fe0c62121de312a4c71a1d', 'N'),
        ('type.trait-object.unsized', 49, 78, '2ed6c06920dcbaf5f61b63c54b34642fda58f11b53b47319f91dcc742153ad9e', 'N X'),
        ('type.trait-object.lifetime-bounds', 79, 88, 'cc7361625dec2e9141c8b15bd2d7f330857d59c79c7cbe0ce01b7a05548693da', 'N I X'),
    )
    PRIOR_HASHES = dict(TypeParametersProposalTests.PRIOR_HASHES, **{
        'type-parameters': 'e48984336382af57c80ca83c60314f691ae369c2dda139a001294ac098e51111'})
    setUp = PrimitiveTypesProposalTests.setUp

    def artifact(self, source=None):
        return build(self.index, self.inventory, self.source if source is None else source, 'trait-objects')

    def verify(self, data, source=None):
        return validate(data, self.index, self.inventory, self.source if source is None else source, 'trait-objects')

    def test_independent_complete_page_label_block_classification_pins(self):
        import re
        import trait_objects_reconciliation_proposals as proposals
        self.assertEqual(proposals.SELECTED, self.PAGES)
        self.assertEqual(proposals.PAGE_SHA256, self.PAGE_HASHES)
        self.assertEqual(proposals.SOURCE_COMMIT, '86635e30bf861a038dc197d7e16fd09e7e514e7a')
        self.assertEqual(proposals.ALIASES, {
            'trait-object': 'reference-types-trait-object-dyn-values'})
        classes = {'N': 'normative-semantic', 'G': 'grammar', 'I': 'index',
                   'X': 'cross-reference', 'E': 'edition-context'}
        self.assertEqual(proposals.CLASS_NAMES, classes)
        authored = [r.split('|') for r in proposals.ROWS.strip().splitlines()]
        self.assertTrue(all(len(r) == 7 and all(f.strip() for f in r) for r in authored))
        self.assertEqual(tuple((r[0], r[1]) for r in authored), tuple((r[0], r[4]) for r in self.PINS))
        self.assertEqual(reconciliation.SOURCE_ONLY_PAGE_COUNTS['trait-objects'], (10,))
        data = self.artifact()
        counts = self.verify(data)
        self.assertEqual([counts[k] for k in ('selected_rule_rows', 'unselected_rule_identities',
                                             'global_rule_identities')], [10, 2955, 2965])
        self.assertEqual(counts['proposed_classification_counts'], {
            'normative-semantic': 8, 'grammar': 1, 'index': 2,
            'cross-reference': 6, 'edition-context': 3})
        page = self.PAGES[0]
        raw = (self.source / page).read_bytes()
        self.assertEqual(digest(raw), self.PAGE_HASHES[page])
        lines = raw.splitlines(keepends=True)
        self.assertEqual(len(lines), 88)
        markers = [(i, m.group(1).decode()) for i, line in enumerate(lines, 1)
                   if (m := re.fullmatch(rb'r\[([^\]\r\n]+)\]\r?\n?', line))]
        self.assertEqual(markers, [(r[1], r[0]) for r in self.PINS])
        self.assertEqual(tuple(r['rule'] for r in data['rows']), tuple(r[0] for r in self.PINS))
        self.assertEqual(len({r['id'] for r in data['rows']}), 10)
        self.assertEqual([(p['path'], p['sha256']) for p in data['selected_pages']],
                         [(page, self.PAGE_HASHES[page])])
        for row, authored_row, (label, start, end, block_hash, codes) in zip(data['rows'], authored, self.PINS):
            with self.subTest(rule=label):
                self.assertEqual((row['page'], row['line'], row['end_line']), (page, start, end))
                self.assertEqual(digest(b''.join(lines[start - 1:end])), block_hash)
                self.assertEqual(row['source_block_sha256'], block_hash)
                self.assertEqual(row['classifications'], [classes[c] for c in codes.split()])
                self.assertEqual(row['candidate_obligation_ids'], [proposals.ALIASES[a] for a in authored_row[3].split()])
                self.assertEqual(row['id'], 'reconcile-trait-objects-' + label.encode().hex())
                self.assertEqual(row['review_status'], 'unreviewed')
                self.assertEqual(row['relationship'], 'context-only' if codes == 'I'
                                 else 'proposed-partial-decomposition')
                self.assertEqual(len(row['missing_test_needs']), 4)
                for need, kind in zip(row['missing_test_needs'], ('positive', 'invalid', 'interaction', 'end-to-end')):
                    self.assertTrue(need.startswith(kind + ': Missing '))
                for phrase in ('stable1.98.1 eligibility', 'workers1/8', 'edition2024', 'Linux x86-64',
                               'CHECK must not demand codegen'):
                    self.assertIn(phrase, row['missing_test_needs'][-1])
        for field in ('reviewed_rule_rows', 'rows_with_exact_executable_evidence'):
            self.assertEqual(counts[field], 0)
        index = json.loads(self.index.read_bytes())
        universe = {(p['path'], r['label']) for p in index['pages'] + index['unlinked_markdown_files']
                    for r in p['rules']}
        selected = {(r['page'], r['rule']) for r in data['rows']}
        other = {(r['page'], r['rule']) for r in data['unselected_rules']}
        self.assertEqual((len(selected), len(other), len(universe)), (10, 2955, 2965))
        self.assertFalse(selected & other)
        self.assertEqual(selected | other, universe)
        self.assertEqual(self.artifact(), data)


    test_each_row_omission_and_actual_changing_classification_forgery = InferredTypesProposalTests.test_each_row_omission_and_actual_changing_classification_forgery
    test_29_actual_changing_forgeries_rejected = InferredTypesProposalTests.test_29_actual_changing_forgeries_rejected
    test_each_source_page_fault_preserves_exact_error_types = PrimitiveTypesProposalTests.test_each_source_page_fault_preserves_exact_error_types

    def test_authored_omissions_duplicates_foreign_and_malformed_rows(self):
        import trait_objects_reconciliation_proposals as proposals
        baseline = self.artifact()
        rows = proposals.ROWS.strip().splitlines()
        cases = [[], rows + [rows[0]], ['type.foreign' + rows[0][len('type.trait-object'):], *rows[1:]],
                 ['|'.join(rows[0].split('|')[:-1]), *rows[1:]], [rows[0] + '|extra', *rows[1:]]]
        cases.extend(rows[:i] + rows[i + 1:] for i in range(10))
        for col in range(7):
            fields = rows[0].split('|')
            fields[col] = '   '
            cases.append(['|'.join(fields), *rows[1:]])
        for i, changed in enumerate(cases):
            self.assertNotEqual(changed, rows)
            with self.subTest(case=i), mock.patch.object(proposals, 'ROWS', '\n'.join(changed)):
                with self.assertRaises(ValueError):
                    self.artifact()
                with self.assertRaises(ValueError):
                    self.verify(baseline)


    def test_permuted_authored_rows_preserve_canonical_order(self):
        import trait_objects_reconciliation_proposals as proposals
        baseline = self.artifact()
        rows = proposals.ROWS.strip().splitlines()
        for changed in (rows[::-1], rows[2:] + rows[:2], rows[::2] + rows[1::2]):
            with mock.patch.object(proposals, 'ROWS', '\n'.join(changed)):
                self.assertEqual(self.artifact(), baseline)
                self.assertEqual(self.verify(self.artifact()), self.verify(baseline))


    def test_all_33_prior_canonical_bytes_without_new_module(self):
        self.assertEqual(len(self.PRIOR_HASHES), 33)
        self.assertEqual(set(self.PRIOR_HASHES), {p for p, _ in reconciliation.PROPOSAL_COUNTS if p != 'trait-objects'})
        with mock.patch.dict(sys.modules, {'trait_objects_reconciliation_proposals': None}):
            for profile, sha in self.PRIOR_HASHES.items():
                with self.subTest(profile=profile):
                    data = build(self.index, self.inventory, self.source, profile)
                    validate(data, self.index, self.inventory, self.source, profile)
                    self.assertEqual(digest((json.dumps(data, indent=2) + '\n').encode()), sha)


    def test_global_disjoint_union_and_zero_credit(self):
        index = json.loads(self.index.read_bytes())
        pages = index['pages'] + index['unlinked_markdown_files']
        prior_pages = {p for profile, _ in reconciliation.PROPOSAL_COUNTS if profile != 'trait-objects'
                       for p in reconciliation.profile_inputs(profile)[0]}
        prior = {(p['path'], r['label']) for p in pages if p['path'] in prior_pages for r in p['rules']}
        current = {(r['page'], r['rule']) for r in self.artifact()['rows']}
        self.assertEqual((len(prior), len(current), len(prior & current), len(prior | current)), (1020, 10, 0, 1030))
        summary = reconciliation.summarize_proposals(self.index, self.inventory, include_proposals=True)
        self.assertEqual((summary['global_rule_identities'], summary['proposed_rule_identities'],
                          summary['rule_identities_without_proposals']), (2965, 1030, 1935))
        self.assertEqual(sum(p['proposed_rule_identities'] for p in summary['profiles'].values()), 1035)
        profile = summary['profiles']['trait-objects']
        self.assertEqual(len(profile['authored_proposals']), 10)
        for field in ('reviewed_rule_rows', 'rows_with_exact_executable_evidence', 'completed_rule_rows'):
            self.assertEqual(summary[field], 0)
            self.assertEqual(profile[field], 0)
        self.assertIsNone(profile['full_rust_completion_percent'])
        result = reconciliation.query_rule_backlog(self.index, self.inventory, page=self.PAGES[0], include_proposals=True)
        self.assertEqual((result['selected_rule_identities'], result['selected_authored_proposal_count']), (10, 10))
        self.assertEqual(result['selected_rule_identities_without_proposals'], 0)
        self.assertTrue(all(row['proposal_profiles'] == ['trait-objects'] for row in result['rows']))
        self.assertEqual(reconciliation.query_rule_backlog(self.index, self.inventory, page=self.PAGES[0], missing_only=True)['rows'], [])


    def test_source_only_restrictions_precede_io_and_execution(self):
        args = ['--profile', 'trait-objects', '--rule-index', '/absent/index', '--inventory', '/absent/inventory',
                '--source-root', '/absent/source', '--artifact', '/absent/artifact']
        extras = ([], ['--repo-root', '/absent/repo'], ['--evidence', '/absent/evidence'],
                  ['--repo-root', '/absent/repo', '--evidence', '/absent/evidence'])
        cases = [(a, e) for a in ('generate-witness-links', 'build-witness-links', 'validate-witness-links',
                                  'assess-witness-links') for e in extras]
        cases += [(a, e) for a in ('generate', 'validate') for e in extras[1:]]
        with contextlib.ExitStack() as stack:
            for name in ('read_bytes', 'read_text', 'open', 'is_symlink', 'is_file', 'write_bytes', 'write_text', 'mkdir'):
                stack.enter_context(mock.patch.object(Path, name, side_effect=AssertionError('IO')))
            stack.enter_context(mock.patch.object(subprocess, 'run', side_effect=AssertionError('execution')))
            for action, extra in cases:
                with self.subTest(action=action, extra=extra), self.assertRaises(ValueError):
                    main([action, *args, *extra])
            for call in (
                lambda: reconciliation.build_witness_links(self.index, self.inventory, self.source, self.source, 'trait-objects'),
                lambda: reconciliation.validate_witness_links({}, self.index, self.inventory, self.source, self.source, 'trait-objects'),
                lambda: reconciliation.assess_witness_links({}, self.index, self.inventory, self.source, self.source, (), 'trait-objects'),
            ):
                with self.assertRaises(ValueError):
                    call()


    def test_generation_validation_never_execute(self):
        with mock.patch.object(subprocess, 'run', side_effect=AssertionError('execution')):
            self.verify(self.artifact())
            with tempfile.TemporaryDirectory() as temp:
                artifact = Path(temp) / 'trait-objects.json'
                args = ['--profile', 'trait-objects', '--rule-index', str(self.index), '--inventory', str(self.inventory),
                        '--source-root', str(self.source), '--artifact', str(artifact)]
                for action in ('generate', 'validate'):
                    with contextlib.redirect_stdout(io.StringIO()):
                        main([action, *args])


    def test_actual_process_cli_exclusive_generation_and_validation(self):
        with tempfile.TemporaryDirectory() as temp:
            artifact = Path(temp) / 'trait-objects.json'
            args = ['--profile', 'trait-objects', '--rule-index', str(self.index), '--inventory', str(self.inventory),
                    '--source-root', str(self.source), '--artifact', str(artifact)]
            expected = (json.dumps(self.artifact(), indent=2) + '\n').encode()
            for action in ('generate', 'validate'):
                argv = [sys.executable, '-B', str(BASE / 'reconcile_token_rules.py'), action, *args]
                result = subprocess.run(argv, capture_output=True, text=True, check=False)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertEqual(json.loads(result.stdout), self.verify(self.artifact()))
                self.assertEqual(artifact.read_bytes(), expected)
            with contextlib.redirect_stdout(io.StringIO()), self.assertRaises(FileExistsError):
                main(['generate', *args])
            forged = json.loads(expected)
            forged['counts']['reviewed_rule_rows'] = 1
            artifact.write_text(json.dumps(forged))
            before = artifact.read_bytes()
            with contextlib.redirect_stdout(io.StringIO()), self.assertRaises(ValueError):
                main(['validate', *args])
            self.assertEqual(artifact.read_bytes(), before)


    def test_semantic_edition_lifetime_and_representation_caveats(self):
        import trait_objects_reconciliation_proposals as proposals
        self.assertEqual(digest((BASE / 'trait_objects_reconciliation_proposals.py').read_bytes()),
                         'a796044efbf6d75bd99bf40cb8ead83f4bd2ec20865e81acdeb02864c48bddc1')
        text = ' '.join(proposals.LIMITATIONS)
        for phrase in ('complete pinned 88-line', 'UNREVIEWED', 'partial routing only',
                       'resolved declaration identities', 'semantic requirement for a trait bound',
                       'Edition-2015/2018/2021', 'edition2024', 'not an exact vtable schema',
                       'function-address uniqueness', 'mandatory emitted instruction shape',
                       'not universally static', 'not the lifetime of every enclosing reference',
                       'not permission to erase captured-reference validity',
                       'not complete allocation', 'variance, upcasting or every coercion'):
            self.assertIn(phrase, text)
        rows = {row.split('|')[0]: row for row in proposals.ROWS.strip().splitlines()}
        for label, phrases in {
            'type.trait-object.syntax': ('bare and dyn grammar alternatives', 'semantic minimum bounds'),
            'type.trait-object.bounds': ('at most one non-auto trait', 'one lifetime', 'opt-out', 'use bounds'),
            'type.trait-object.syntax-edition2021': ('semantically required', 'earlier editions may omit'),
            'type.trait-object.syntax-edition2018': ('edition2015', 'edition2018', 'parenthesized-path'),
            'type.trait-object.alias': ('base traits alias', 'auto-trait sets match', 'lifetime bounds match', 'resolved identity'),
            'type.trait-object.unsized': ('virtual dispatch', 'without fixed metadata layout', 'unique function addresses'),
            'type.trait-object.lifetime-bounds': ('heading also states', 'linked defaults', 'without universal static defaults'),
        }.items():
            for phrase in phrases:
                self.assertIn(phrase, rows[label])
        lifetime = next(row for row in self.artifact()['rows'] if row['rule'] == 'type.trait-object.lifetime-bounds')
        self.assertEqual(lifetime['classifications'], ['normative-semantic', 'index', 'cross-reference'])
        self.assertEqual(lifetime['relationship'], 'proposed-partial-decomposition')

    def test_authored_invalid_classes_aliases_and_each_field_are_rejected(self):
        import trait_objects_reconciliation_proposals as proposals
        baseline = self.artifact()
        rows = proposals.ROWS.strip().splitlines()
        for column, value in ((1, 'UNKNOWN'), (1, 'N N'), (3, 'foreign'), (3, 'trait-object trait-object')):
            fields = rows[0].split('|')
            fields[column] = value
            changed = ['|'.join(fields), *rows[1:]]
            self.assertNotEqual(changed, rows)
            with self.subTest(column=column, value=value), mock.patch.object(proposals, 'ROWS', '\n'.join(changed)):
                with self.assertRaises(ValueError):
                    self.artifact()
                with self.assertRaises(ValueError):
                    self.verify(baseline)
        with mock.patch.object(proposals, 'ALIASES', {'trait-object': 'foreign-obligation'}):
            with self.assertRaises(ValueError):
                self.artifact()
            with self.assertRaises(ValueError):
                self.verify(baseline)


if __name__ == '__main__':
    unittest.main()
