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
        self.assertEqual(result['proposed_rule_identities'], 396)
        self.assertEqual(result['rule_identities_without_proposals'], 2569)
        for name, count in (('tokens', 115), ('const-eval', 44), ('unsafe', 27), ('abi', 24), ('visibility', 18), ('branching', 58), ('destructors', 50), ('loops', 60)):
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
        selected = {path for name in ('tokens', 'const-eval', 'unsafe', 'abi', 'visibility', 'branching', 'destructors', 'loops')
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
                          result['rule_identities_without_proposals']), (2965, 396, 2569))
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
        other = next(page for page in self.index['pages'] if page['path'] == 'src/types.md')
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
            self.assertEqual(sum(row['proposed_rule_identities'] for row in result['backlog']['pages']), 352)
        self.assertEqual(result['proposed_rule_identities'], 352)
        self.assertEqual(result['rule_identities_without_proposals'], 2613)

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
        for key, value in (('global_rule_identities', 2965), ('proposed_rule_identities', 396),
                           ('rule_identities_without_proposals', 2569), ('total_pages', 122),
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
        self.assertEqual(len(result['rows']), 2569)
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
            self.assertEqual(result['selected_proposed_rule_identities'], 396)
            self.assertEqual(result['selected_authored_proposal_count'], 511)
            result = self.query(page='src/tokens.md', include_proposals=True)
            self.assertEqual(result['selected_proposed_rule_identities'], 115)
            self.assertEqual(result['selected_authored_proposal_count'], 230)
            self.assertTrue(all([detail['profile'] for detail in row['authored_proposals']] ==
                                ['tokens', 'zz_tokens_overlap'] for row in result['rows']))

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

if __name__ == '__main__':
    unittest.main()
