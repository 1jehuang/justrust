#!/usr/bin/env python3
"""Generate/validate a bounded unreviewed pinned-page rule reconciliation.

Token proposals are the unchanged default. The explicit const-eval profile uses
const_reconciliation_proposals.py, including explicit partial witness links.
Unsafe, ABI and visibility profiles cover pinned pages and are generate/validate only.
Classifications are explicitly authored in the selected proposal module.
The validator checks identities, hashes, row completeness and structural claims,
not whether the proposed interpretation of normative Rust is correct.
"""
import argparse
import copy
import hashlib
import json
import re
from collections import Counter
from pathlib import Path

from extract_reference_rules import SCOPE, parse_markdown
from token_reconciliation_proposals import ALIASES, CLASS_NAMES, ROWS
from token_reconciliation_proposals import DIRECT_WITNESSES, WITNESS_SOURCE_CONTRACT

SELECTED = ('src/tokens.md',)
CLASSES = set(CLASS_NAMES.values())
TOP_FIELDS = {'schema_version','review_status','purpose','scope','bindings','selected_pages','rows','unselected_rules','counts','limitations'}
ROW_FIELDS = {'id','page','rule','line','end_line','source_block_sha256','classifications','rationale','candidate_obligation_ids','relationship','missing_test_needs','review_status'}
PROPOSAL_COUNTS = (('tokens', 115), ('const-eval', 44), ('unsafe', 27), ('abi', 24), ('visibility', 18))
SOURCE_ONLY_PAGE_COUNTS = {'unsafe': (16, 11), 'abi': (24,), 'visibility': (18,)}


def digest(data):
    return hashlib.sha256(data).hexdigest()


def require(condition, message):
    if not condition:
        raise ValueError(message)


def source_only_proposals(profile):
    require(profile in SOURCE_ONLY_PAGE_COUNTS, 'unknown source-only profile')
    if profile == 'unsafe':
        import unsafe_reconciliation_proposals as proposals
    elif profile == 'abi':
        import abi_reconciliation_proposals as proposals
    else:
        import visibility_reconciliation_proposals as proposals
    return proposals


def profile_inputs(profile):
    """Only explicitly selected pages and authored proposals are admitted."""
    require(profile in dict(PROPOSAL_COUNTS), 'unknown reconciliation profile')
    if profile == 'tokens':
        return SELECTED, ALIASES, CLASS_NAMES, ROWS
    if profile in SOURCE_ONLY_PAGE_COUNTS:
        proposals = source_only_proposals(profile)
        return proposals.SELECTED, proposals.ALIASES, proposals.CLASS_NAMES, proposals.ROWS
    # Token callers and witness APIs do not depend on the optional module.
    import const_reconciliation_proposals as proposals
    return ('src/const_eval.md',), proposals.ALIASES, proposals.CLASS_NAMES, proposals.ROWS


def profile_purpose(profile):
    return 'bounded-' + ('token' if profile == 'tokens' else profile) + '-rule-reconciliation-candidate'


def proposal_backlog(pages, proposed):
    """Index-only work queue, not a semantic coverage or review assessment.

    The caller has validated the complete pinned index and authored proposals.
    Keep every page, including unlinked and zero-rule pages. Only the global
    exact-rule sample is capped, so totals never silently discard missing work.
    """
    page_rows, missing = [], []
    for page in sorted(pages, key=lambda row: row['path']):
        total = len(page['rules'])
        count = sum((page['path'], rule['label']) in proposed for rule in page['rules'])
        page_rows.append({
            'page': page['path'], 'global_rule_identities': total,
            'proposed_rule_identities': count, 'rule_identities_without_proposals': total - count,
        })
        missing.extend({
            'page': page['path'], 'rule': rule['label'], 'line': rule['line'],
            'page_sha256': page['sha256'],
        } for rule in page['rules'] if (page['path'], rule['label']) not in proposed)
    missing.sort(key=lambda row: (row['page'], row['rule']))
    limit = 100
    return {
        'pages': page_rows, 'total_pages': len(page_rows),
        'pages_without_proposals': sum(row['global_rule_identities'] > 0
                                       and row['proposed_rule_identities'] == 0 for row in page_rows),
        'pages_with_partial_proposals': sum(0 < row['proposed_rule_identities']
                                           < row['global_rule_identities'] for row in page_rows),
        'missing_rule_sample': missing[:limit], 'missing_rule_sample_limit': limit,
        'missing_rule_sample_truncated': len(missing) > limit,
    }


def summarize_proposals(index_path, candidate_path):
    """Summarize authored module inputs, without Reference prose or sidecars.

    This deliberately does not call build/validate: those independently verify
    actual selected source bytes and remain the stronger reproduction path.
    """
    import compatibility_inventory as inventory

    index_path, candidate_path = Path(index_path), Path(candidate_path)
    missing = [str(path) for path in (index_path, candidate_path) if not path.is_file()]
    # A present malformed file must not be hidden by another missing input.
    index = inventory.load(index_path) if index_path.is_file() else None
    candidate = inventory.load(candidate_path) if candidate_path.is_file() else None
    try:
        if candidate_path.is_file():
            require(isinstance(candidate, dict), 'proposal candidate must be an object')
            existing = {r for section in candidate['sections'] for row in section['obligations']
                        for r in row['existing_requirements']}
            inventory.validate(candidate, existing)
        if index_path.is_file():
            require(isinstance(index, dict), 'proposal index must be an object')
            provenance = index['provenance']
            inventory.validate_reference_metadata(
                index, candidate, candidate_path if candidate is not None else None,
                provenance['compiler_commit'])
            pages = index['pages'] + index['unlinked_markdown_files']
            paths = {page['path'] for page in pages}
            identities = {(page['path'], rule['label']) for page in pages for rule in page['rules']}
        if missing:
            return {'readiness': 'missing', 'review_status': 'unreviewed', 'missing_files': missing,
                    'global_rule_identities': None, 'full_rust_completion_percent': None,
                    'limitations': ['Missing proposal inputs cannot establish a universal source denominator.']}
        selected_paths = {path for profile, _ in PROPOSAL_COUNTS
                          for path in profile_inputs(profile)[0]}
        if not selected_paths & paths:
            return {'readiness': 'missing', 'review_status': 'unreviewed',
                    'missing_files': [], 'missing_selected_pages': sorted(selected_paths),
                    'global_rule_identities': None, 'full_rust_completion_percent': None,
                    'limitations': ['This fixture has no selected proposal pages and cannot establish the pinned source denominator.']}
        require(len(identities) == 2965, 'proposal pinned global denominator mismatch')
        for name, actual in (
                ('reference_commit', provenance['reference_commit']),
                ('rule_index_sha256', digest(index_path.read_bytes())),
                ('candidate_inventory_sha256', digest(candidate_path.read_bytes()))):
            require(actual == WITNESS_SOURCE_CONTRACT[name], 'proposal source-contract ' + name)
        obligations = {row['id'] for section in candidate['sections'] for row in section['obligations']}
        profiles, union = {}, set()
        valid_classes = {'normative-semantic', 'grammar', 'index', 'cross-reference', 'edition-context'}
        for profile, expected in PROPOSAL_COUNTS:
            selected, aliases, classes, authored = profile_inputs(profile)
            require(len(selected) == len(set(selected)) and set(selected) <= paths,
                    'missing or duplicate selected proposal page')
            require(isinstance(aliases, dict) and aliases and set(aliases.values()) <= obligations,
                    'foreign candidate alias')
            require(isinstance(classes, dict) and classes and set(classes.values()) <= valid_classes,
                    'invalid proposal classes')
            selected_identities = {identity for identity in identities if identity[0] in selected}
            by_label = {label: (page, label) for page, label in selected_identities}
            seen, counts = set(), Counter()
            for line in authored.strip().splitlines():
                fields = line.split('|')
                require(len(fields) == 7 and all(field.strip() for field in fields),
                        'proposal requires seven nonempty explicit fields')
                label, classification, _, links, _, _, _ = fields
                require(label in by_label, 'foreign proposal rule: ' + label)
                identity = by_label[label]
                require(identity not in seen, 'duplicate proposal rule: ' + label)
                seen.add(identity)
                codes, names = classification.split(), links.split()
                require(len(codes) == len(set(codes)) and set(codes) <= set(classes),
                        'invalid proposal classification')
                require(len(names) == len(set(names)) and set(names) <= set(aliases),
                        'foreign or duplicate proposal alias')
                require(len({aliases[name] for name in names}) == len(names),
                        'duplicate proposal candidate identity')
                counts.update(classes[code] for code in codes)
            require(seen == selected_identities, 'omitted selected proposal rules')
            require(len(seen) == expected, 'proposal pinned profile denominator mismatch')
            union.update(seen)
            profiles[profile] = {
                'selected_pages': list(selected), 'proposed_rule_identities': len(seen),
                'proposed_classification_counts': {name: counts[name] for name in sorted(set(classes.values()))},
                'review_status': 'unreviewed', 'reviewed_rule_rows': 0,
                'rows_with_exact_executable_evidence': 0, 'completed_rule_rows': 0,
                'full_rust_completion_percent': None,
            }
        return {
            'readiness': 'review-required', 'review_status': 'unreviewed', 'profiles': profiles,
            'global_rule_identities': len(identities), 'proposed_rule_identities': len(union),
            'rule_identities_without_proposals': len(identities - union),
            'backlog': proposal_backlog(pages, union),
            'reviewed_rule_rows': 0, 'rows_with_exact_executable_evidence': 0,
            'completed_rule_rows': 0, 'full_rust_completion_percent': None,
            'source_block_byte_verification': 'not_reverified_in_this_run',
            'source_archive_byte_verification': 'not_reverified_in_this_run',
            'limitations': [
                'Authored proposed interpretation is not normative semantic coverage, review, execution or completion credit.',
                'Source blocks and archive bytes are NOT reverified in this offline module-input summary. Recorded hashes are not fresh proof.',
                'Exact source-backed generate/validate remains an independent stronger check. No reconciliation artifact is consumed.',
            ],
        }
    except (ValueError, KeyError, TypeError, AttributeError, OSError, ImportError, SyntaxError) as exc:
        raise inventory.InvalidInventory('rule_proposals: ' + str(exc)) from exc


def load_inputs(index_path, candidate_path, source_root, profile='tokens'):
    selected, _, _, _ = profile_inputs(profile)
    index_bytes, candidate_bytes = index_path.read_bytes(), candidate_path.read_bytes()
    index, candidate = json.loads(index_bytes), json.loads(candidate_bytes)
    require(index['scope'] == candidate['scope'] == SCOPE, 'scope mismatch')
    require(index['provenance']['candidate_inventory_sha256'] == digest(candidate_bytes), 'index candidate binding mismatch')
    pages = index['pages'] + index['unlinked_markdown_files']
    identities = [(page['path'], rule['label']) for page in pages for rule in page['rules']]
    require(len(identities) == len(set(identities)), 'duplicate index rule identity')
    require(len({label for _, label in identities}) == len(identities), 'duplicate index rule label')
    require(index['counts']['unique_rule_labels_total'] == len(identities), 'index total mismatch')
    all_obligations = {o['id'] for section in candidate['sections'] for o in section['obligations']}
    require(len(all_obligations) == sum(len(s['obligations']) for s in candidate['sections']), 'duplicate candidate obligation')
    records = {}
    selected_pages = []
    require(not source_root.is_symlink(), 'symlink source root')
    for path in selected:
        matching = [page for page in pages if page['path'] == path]
        require(len(matching) == 1, 'selected page missing or duplicated in index')
        page = matching[0]
        target = source_root / path
        # Reject symlink traversal through every component, not merely the leaf.
        current = source_root
        for component in Path(path).parts:
            current = current / component
            require(not current.is_symlink(), 'symlink in selected source path')
        require(target.resolve().is_relative_to(source_root.resolve()), 'source page escapes root')
        raw = target.read_bytes()
        require(digest(raw) == page['sha256'], 'selected page hash mismatch')
        actual_rules, _, _ = parse_markdown(raw.decode('utf-8'))
        require(actual_rules == page['rules'], 'selected page labels/lines mismatch')
        lines = raw.splitlines(keepends=True)
        for position, rule in enumerate(actual_rules):
            start = rule['line']
            end = actual_rules[position + 1]['line'] - 1 if position + 1 < len(actual_rules) else len(lines)
            records[(path, rule['label'])] = {'line': start, 'end_line': end, 'source_block_sha256': digest(b''.join(lines[start - 1:end]))}
        selected_pages.append({'path': path, 'sha256': page['sha256'], 'url': page['url']})
    bindings = {
        'compiler_commit': index['provenance']['compiler_commit'],
        'reference_commit': index['provenance']['reference_commit'],
        'source_archive_sha256': index['provenance']['source_archive_sha256'],
        'rule_index_sha256': digest(index_bytes),
        'candidate_inventory_sha256': digest(candidate_bytes),
    }
    return records, identities, all_obligations, bindings, selected_pages


def compute_counts(rows, universe_size, profile='tokens'):
    _, _, class_names, _ = profile_inputs(profile)
    classes = Counter(c for row in rows for c in row['classifications'])
    return {
        'global_rule_identities': universe_size,
        'selected_rule_rows': len(rows),
        'unselected_rule_identities': universe_size - len(rows),
        'proposed_classification_counts': {c: classes[c] for c in sorted(set(class_names.values()))},
        'reviewed_rule_rows': 0,
        'rows_with_exact_executable_evidence': 0,
    }


def build(index_path, candidate_path, source_root, profile='tokens'):
    selected_pages, aliases_by_name, class_names, proposals = profile_inputs(profile)
    records, identities, obligations, bindings, pages = load_inputs(index_path, candidate_path, source_root, profile)
    rows = []
    for line in proposals.strip().splitlines():
        fields = line.split('|')
        require(len(fields) == 7, 'proposal requires seven explicit fields')
        label, classes, rationale, aliases, positive, invalid, interaction = fields
        identity = (selected_pages[0], label)
        if profile in SOURCE_ONLY_PAGE_COUNTS:
            matches = [key for key in records if key[1] == label]
            require(len(matches) == 1, 'foreign proposal rule: ' + label)
            identity = matches[0]
        require(identity in records, 'foreign proposal rule: ' + label)
        classifications = [class_names[c] for c in classes.split()]
        executable = 'normative-semantic' in classifications or 'grammar' in classifications
        row = {
            'id': 'reconcile-' + re.sub('[^a-z0-9]+', '-', label),
            'page': identity[0], 'rule': label, **records[identity],
            'classifications': classifications,
            'rationale': rationale,
            'candidate_obligation_ids': [aliases_by_name[a] for a in aliases.split()],
            'relationship': 'proposed-partial-decomposition' if executable else 'context-only',
            'missing_test_needs': [
                'positive: Missing direct exact test link. ' + positive,
                'invalid: Missing direct exact test link. ' + invalid,
                'interaction: Missing direct exact test link. ' + interaction,
                ('end-to-end: Missing authored original-source CHECK/BUILD/run or stage-specific rejection witnesses, named test identities, pinned-stock outcomes and immutable receipts at workers1/8. Lexical-invalid rejection is distinct from semantic body rejection, and no execution is inferred from legacy IDs.' if executable else 'end-to-end: No standalone runtime feature is claimed by this context row. Missing source-review links and any concrete child-rule test attribution; do not grant executable credit for index/cross-reference text.')
            ],
            'review_status': 'unreviewed',
        }
        if profile == 'const-eval' and executable:
            row['missing_test_needs'][-1] = (
                'end-to-end: Missing authored original-source CHECK/BUILD/run or stage-specific rejection witnesses, named test identities, pinned-stock outcomes and immutable receipts at workers1/8. Invalid CTFE, unsupported input, resource limits and infrastructure faults require distinct outcomes; no execution is inferred from legacy IDs.')
        if profile in SOURCE_ONLY_PAGE_COUNTS:
            row['missing_test_needs'][-1] = source_only_proposals(profile).END_TO_END
        rows.append(row)
    rows.sort(key=lambda r: (r['page'], r['line']))
    selected = {(r['page'], r['rule']) for r in rows}
    result = {
        'schema_version': 1,
        'review_status': 'unreviewed',
        'purpose': profile_purpose(profile),
        'scope': dict(SCOPE),
        'bindings': bindings,
        'selected_pages': pages,
        'rows': rows,
        'unselected_rules': [{'page': page, 'rule': rule} for page, rule in sorted(set(identities) - selected)],
        'counts': compute_counts(rows, len(identities), profile),
        'limitations': [
            'All classifications and mappings are proposed interpretations after reading the exact pinned token page. They remain unreviewed by the coordinator and grant no reviewed-readiness credit.',
            'The bounded selection is all115 labeled blocks on src/tokens.md only. All other2850 source identities remain explicitly unselected; none are removed from the2965-label denominator.',
            'Grammar is normative syntax, not a nonsemantic exclusion. Mixed index/semantic and edition/semantic blocks carry multiple classifications. Context-only rows cannot be used as executable feature completions.',
            'Candidate IDs are precise references to partial existing obligation buckets, not exact test cases or proof of semantic equivalence. No legacy requirement selector or adjacent page placement supplies executable evidence.',
            'Rule block boundaries run from a marker through the byte before the next marker, or EOF. Hashes bind complete original blocks including examples/links, but full source prose is not copied into this artifact.',
            'Cross-page definitions, unlabeled normative prose, Unicode versions, stabilization histories and continuation rules can impose additional obligations beyond the chosen page and remain open.',
            'Earlier-edition comparisons are explanatory controls only. The tracked conformance scope remains edition2024 on x86_64-unknown-linux-gnu.',
            'The source archive and global index require independent byte-validation prerequisites. This sidecar verifies selected-page hashes against the index and never upgrades source-authentication or full language completeness.',
            'All test needs are explicitly missing direct exact links and measured execution evidence. No Rust builds/tests or compiler executions were performed to produce the reconciliation candidate.',
        ],
    }
    if profile == 'const-eval':
        result['limitations'][:2] = [
            'All classifications and mappings are proposed interpretations of the exact pinned src/const_eval.md page. They remain unreviewed and grant no reviewed-readiness credit.',
            f'The bounded selection is all {len(records)} labeled blocks on src/const_eval.md only. All other {len(identities) - len(records)} source identities remain explicitly unselected; none are removed from the {len(identities)}-label denominator.',
        ]
    if profile in SOURCE_ONLY_PAGE_COUNTS:
        result['limitations'] = list(source_only_proposals(profile).LIMITATIONS)
    validate(result, index_path, candidate_path, source_root, profile)
    return result


def validate(data, index_path, candidate_path, source_root, profile='tokens'):
    _, _, class_names, _ = profile_inputs(profile)
    require(isinstance(data, dict) and set(data) == TOP_FIELDS, 'unexpected sidecar fields')
    require(type(data['schema_version']) is int and data['schema_version'] == 1, 'schema version')
    require(data['scope'] == SCOPE and data['review_status'] == 'unreviewed', 'reviewed or foreign scope claim')
    require(data['purpose'] == profile_purpose(profile), 'purpose mismatch')
    records, identities, obligations, bindings, pages = load_inputs(index_path, candidate_path, source_root, profile)
    require(data['bindings'] == bindings, 'stale source/index/candidate bindings')
    require(data['selected_pages'] == pages, 'selected page binding mismatch')
    require(isinstance(data['rows'], list) and data['rows'], 'rule rows must be a nonempty list')
    seen, ids = set(), set()
    for row in data['rows']:
        require(isinstance(row, dict) and set(row) == ROW_FIELDS, 'unexpected rule row fields')
        identity = (row['page'], row['rule'])
        require(identity in records, 'foreign rule row')
        require(identity not in seen, 'duplicate rule row')
        seen.add(identity)
        require(isinstance(row['id'], str) and re.fullmatch('[a-z][a-z0-9]*(?:-[a-z0-9]+)*', row['id']), 'invalid row ID')
        require(row['id'] not in ids and row['id'] not in obligations, 'duplicate or colliding row ID')
        ids.add(row['id'])
        for key, value in records[identity].items():
            require(row[key] == value and type(row[key]) is type(value), 'stale source block binding')
        classifications = row['classifications']
        require(isinstance(classifications, list) and classifications and all(isinstance(c, str) for c in classifications) and len(set(classifications)) == len(classifications) and set(classifications) <= set(class_names.values()), 'invalid classifications')
        require(isinstance(row['rationale'], str) and row['rationale'].strip(), 'missing rationale')
        links = row['candidate_obligation_ids']
        require(isinstance(links, list) and links and all(isinstance(link, str) for link in links) and len(set(links)) == len(links) and set(links) <= obligations, 'foreign or duplicate candidate obligation')
        executable = bool(set(classifications) & {'normative-semantic','grammar'})
        require(row['relationship'] == ('proposed-partial-decomposition' if executable else 'context-only'), 'relationship would grant context execution credit')
        require(row['review_status'] == 'unreviewed', 'reviewed row claim forbidden')
        needs = row['missing_test_needs']
        require(isinstance(needs, list) and len(needs) == 4 and all(isinstance(n, str) and n.strip() for n in needs), 'missing test-needs strings')
        require(all(needs[i].startswith(prefix) for i, prefix in enumerate(('positive:','invalid:','interaction:','end-to-end:'))), 'missing evidence classes')
    require(seen == set(records), 'omitted selected rule rows')
    require(data['rows'] == sorted(data['rows'], key=lambda r: (r['page'], r['line'])), 'nondeterministic row order')
    unselected = data['unselected_rules']
    require(isinstance(unselected, list) and all(isinstance(r, dict) and set(r) == {'page','rule'} for r in unselected), 'invalid unselected identity')
    expected = [{'page': page, 'rule': rule} for page, rule in sorted(set(identities) - seen)]
    require(unselected == expected, 'omitted duplicate foreign or reordered unselected identities')
    require(json.dumps(data['counts'], sort_keys=True) == json.dumps(compute_counts(data['rows'], len(identities), profile), sort_keys=True), 'counts or evidence claim mismatch')
    require(isinstance(data['limitations'], list) and data['limitations'] and all(isinstance(t, str) and t.strip() for t in data['limitations']), 'missing limitations')
    if profile == 'const-eval':
        require(data['limitations'][:2] == [
            'All classifications and mappings are proposed interpretations of the exact pinned src/const_eval.md page. They remain unreviewed and grant no reviewed-readiness credit.',
            f'The bounded selection is all {len(records)} labeled blocks on src/const_eval.md only. All other {len(identities) - len(records)} source identities remain explicitly unselected; none are removed from the {len(identities)}-label denominator.',
        ], 'const-eval scope limitations mismatch')
    if profile in SOURCE_ONLY_PAGE_COUNTS:
        proposals = source_only_proposals(profile)
        require(len(identities) == 2965 and len(records) == dict(PROPOSAL_COUNTS)[profile],
                profile + ' pinned denominator mismatch')
        require(Counter(page for page, _ in records)
                == Counter(dict(zip(proposals.SELECTED, SOURCE_ONLY_PAGE_COUNTS[profile]))),
                profile + ' selected page cardinality mismatch')
        authored = {}
        for line in proposals.ROWS.strip().splitlines():
            fields = line.split('|')
            require(len(fields) == 7, 'proposal requires seven explicit fields')
            label, classes, rationale, aliases, positive, invalid, interaction = fields
            require(label not in authored, 'duplicate ' + profile + ' proposal rule')
            authored[label] = (
                [proposals.CLASS_NAMES[c] for c in classes.split()], rationale,
                [proposals.ALIASES[a] for a in aliases.split()],
                ['positive: Missing direct exact test link. ' + positive,
                 'invalid: Missing direct exact test link. ' + invalid,
                 'interaction: Missing direct exact test link. ' + interaction,
                 proposals.END_TO_END])
        require(set(authored) == {label for _, label in records}, profile + ' authored selection mismatch')
        for row in data['rows']:
            require((row['classifications'], row['rationale'], row['candidate_obligation_ids'],
                     row['missing_test_needs']) == authored[row['rule']],
                    profile + ' authored classification or proposal mismatch')
            require(row['id'] == 'reconcile-' + re.sub('[^a-z0-9]+', '-', row['rule']),
                    profile + ' authored row ID mismatch')
        require(data['limitations'] == list(proposals.LIMITATIONS), profile + ' scope limitations mismatch')
    return data['counts']


def safe_path(path):
    """Reject traversal and symlinks, including symlinked ancestors."""
    path = Path(path)
    require('..' not in path.parts, 'unsafe path traversal')
    absolute = path.absolute()
    for component in (absolute, *absolute.parents):
        require(not component.is_symlink(), 'symlink path forbidden')
    return absolute


def witness_bytes(data):
    return (json.dumps(data, indent=2, ensure_ascii=True, allow_nan=False) + '\n').encode('utf-8')


def witness_profile(profile):
    require(profile not in SOURCE_ONLY_PAGE_COUNTS, profile + ' profile supports generate/validate only')
    profile_inputs(profile)
    if profile == 'tokens':
        return WITNESS_SOURCE_CONTRACT, DIRECT_WITNESSES, ALIASES, 'conformance_generated', 'tests/conformance_generated.rs'
    import const_reconciliation_proposals as proposals
    return proposals.WITNESS_SOURCE_CONTRACT, proposals.DIRECT_WITNESSES, proposals.ALIASES, 'const_source', 'tests/const_source.rs'


def scalar_source_binding(raw, selector, expectation):
    """Extract the pinned single-case literal, never execute Rust or infer outcomes."""
    text = raw.decode('utf-8')
    require('#[path = "support/closure_oracle.rs"]\nmod scalar_const_differential;' in text,
            'missing original oracle alias')
    bodies = re.findall(r'^fn ' + re.escape(selector) + r'\(\) \{\n(.*?)^\}', text, re.M | re.S)
    require(len(bodies) == 1, 'missing or duplicate scalar source selector')
    body = bodies[0]
    require('scalar_const_differential::check(' in body and 'use scalar_const_differential::{Case, Expect};' in body,
            'scalar source must use original oracle')
    sources = re.findall(r'^\s*source: ("(?:[^"\\]|\\.)*"),$', body, re.M)
    require(len(sources) == 1, 'scalar source requires one literal case')
    source = json.loads(sources[0])
    expected = (r'Expect::Run\("(?:[^"\\]|\\.)*"\)' if expectation == 'positive'
                else r'Expect::InvalidAfterParse\("E0080"\)')
    require(len(re.findall(r'^\s*expect: ' + expected + r',$', body, re.M)) == 1,
            'scalar source expectation mismatch')
    return {'source_sha256': digest(source.encode('utf-8')),
            'selector_body_sha256': digest(body.encode('utf-8'))}


def build_witness_links(index_path, candidate_path, source_root, repo_root, profile='tokens'):
    """Direct partial attribution, never imported execution or reviewed coverage."""
    contract, witnesses, aliases_by_name, suite, source_file = witness_profile(profile)
    index_path, candidate_path, source_root, repo_root = map(
        safe_path, (index_path, candidate_path, source_root, repo_root))
    # Reuse every v1 source/page/line/block and candidate identity check. Keep the
    # entire reconciliation, including all missing needs and unselected rules.
    reconciliation = build(index_path, candidate_path, source_root, profile)
    pin = copy.deepcopy(contract)
    for field in ('reference_commit', 'rule_index_sha256', 'candidate_inventory_sha256'):
        require(reconciliation['bindings'][field] == pin[field], 'witness source-contract ' + field)
    selected_count = 115 if profile == 'tokens' else pin['selected_rule_rows']
    require(reconciliation['counts']['global_rule_identities'] == 2965
            and len(reconciliation['rows']) == selected_count
            and len(reconciliation['unselected_rules']) == 2965 - selected_count,
            'witness denominator mismatch')
    candidate = json.loads(candidate_path.read_bytes())
    obligations = {o['id'] for section in candidate['sections'] for o in section['obligations']}
    require(len(obligations) == 205, 'witness requires the actual 205 candidate inventory')
    files = []
    for name, expected in pin['files'].items():
        path = safe_path(repo_root / name)
        require(path.is_relative_to(repo_root), 'source-contract path escapes repository')
        actual = digest(path.read_bytes())
        require(actual == expected, 'witness source-contract file mismatch: ' + name)
        files.append({'path': name, 'sha256': actual})
    # Discovery is lexical, not evidence. Reuse the ordinary exact-selector
    # contract as well, but do not load/import any passing outcomes.
    import conformance
    import compatibility_scope
    for path in (repo_root / 'tests').rglob('*'):
        require(not path.is_symlink(), 'symlink in discovery source')
    scope = json.loads((repo_root / 'data/compiler-compatibility-scope.json').read_bytes())
    compatibility_scope.validate(scope, conformance, repo_root)
    discovered = conformance.discover(repo_root)['source_test_functions']
    scoped_selectors = {(case['suite'], case['test'])
                        for area in scope['areas'] for requirement in area['requirements']
                        for gate in requirement['gates'] for case in gate['tests']}
    rows = {row['rule']: row for row in reconciliation['rows']}
    links = []
    expected_count = 6 if profile == 'tokens' else pin['direct_witness_selectors']
    require(len(witnesses) == expected_count, 'authored witness cardinality mismatch')
    for selector, expectation, labels, aliases, rationale, limitations in witnesses:
        matches = [row for row in discovered if row['file'] == source_file
                   and row['test'] == selector]
        require(len(matches) == 1, 'missing or duplicate exact top-level selector: ' + selector)
        require((suite, selector) in scoped_selectors,
                'exact selector absent from compatibility scope: ' + selector)
        require(expectation in ('positive', 'semantic-invalid'), 'foreign expectation class')
        candidate_ids = [aliases_by_name[alias] for alias in aliases]
        require(set(candidate_ids) <= obligations and len(set(candidate_ids)) == len(candidate_ids),
                'foreign or duplicate direct candidate identity')
        require(len(set(labels)) == len(labels), 'duplicate direct rule identity')
        rule_links = []
        for label in labels:
            require(label in rows, 'foreign direct rule identity')
            row = rows[label]
            rule_links.append({key: row[key] for key in (
                'id', 'page', 'rule', 'line', 'end_line', 'source_block_sha256')})
        links.append({
            'suite': suite, 'test': selector,
            'source_file': source_file,
            'selector_line': matches[0]['line'],
            'expectation_class': expectation,
            'relationship': 'unreviewed-direct-partial-link',
            'rule_links': rule_links, 'candidate_obligation_ids': candidate_ids,
            'rationale': rationale, 'limitations': limitations,
        })
        if profile == 'const-eval':
            links[-1].update(scalar_source_binding((repo_root / source_file).read_bytes(), selector, expectation))
    require(len({link['test'] for link in links}) == expected_count, 'duplicate direct selector')
    links.sort(key=lambda row: (row['suite'], row['test']))
    return {
        'schema_version': 1, 'schema': 'bounded-' + ('token' if profile == 'tokens' else profile) + '-direct-witness-links-v1',
        'purpose': 'scratch-only-unreviewed-' + ('token' if profile == 'tokens' else profile) + '-direct-witness-links',
        'scope': dict(SCOPE), 'source_contract': pin,
        'source_files': files, 'reconciliation': reconciliation, 'links': links,
        'summary': {
            'global_rule_identities': 2965, 'selected_rule_rows': selected_count,
            'unselected_rule_identities': 2965 - selected_count, 'direct_witness_selectors': len(links),
            'direct_rule_links': sum(len(link['rule_links']) for link in links),
            'distinct_direct_rule_identities': len({row['rule'] for link in links for row in link['rule_links']}),
            'positive_selectors': sum(link['expectation_class'] == 'positive' for link in links),
            'semantic_invalid_selectors': sum(link['expectation_class'] == 'semantic-invalid' for link in links),
            'reviewed_rule_rows': 0, 'rows_with_exact_executable_evidence': 0,
            'full_rust_percent': None,
        },
        'limitations': [
            'Literal rule links and rationale are scoped unreviewed attribution, not normative review. No title-based coverage inference.',
            ('Whole-file hashes bind actual fixture and helper source, not extracted program bytes. No authored program hashes or stale worker fixture hashes are imported.' if profile == 'tokens' else 'Whole-file and extracted literal/body hashes bind actual scalar source and original oracle alias. No historical passing outcomes are imported.'),
            'An exact selector link is not completion of a rule, candidate bucket, missing positive/invalid layer, or passing outcome. All v1 missing needs remain open.',
            ('Positive and semantic-invalid are authored expectation classes, not results. The known ignored tuple .01 selector remains linked. No evidence import, compiler execution or passing credit.' if profile == 'tokens' else 'Positive and semantic-invalid are authored expectation classes, not results. Observable positive values alone cannot prove absence of runtime folding. No if/match link, compiler execution or passing credit.'),
            'The source contract pins this bounded source snapshot only. It does not authenticate the full reference archive or certify full Rust support.',
        ],
    }


def validate_witness_links(data, index_path, candidate_path, source_root, repo_root, profile='tokens'):
    expected = build_witness_links(index_path, candidate_path, source_root, repo_root, profile)
    # Canonical byte comparison also distinguishes bool/int and rejects every
    # arbitrary field, manual pass state, reordered list or weakened limitation.
    require(witness_bytes(data) == witness_bytes(expected), 'witness artifact differs from exact authored source-bound links')
    return expected['summary']


def witness_artifact_path(path, repo_root, source_root):
    path = safe_path(path)
    require(not path.is_relative_to(safe_path(repo_root))
            and not path.is_relative_to(safe_path(source_root))
            and not path.is_relative_to(safe_path(Path(__file__).absolute().parent.parent)),
            'scratch artifact must be outside repository and reference source')
    return path


def assess_witness_links(data, index_path, candidate_path, source_root, repo_root, reports=(), profile='tokens'):
    """Observe exact linked executions, without upgrading source attribution."""
    validate_witness_links(data, index_path, candidate_path, source_root, repo_root, profile)
    import conformance as c
    import compatibility_scope as coverage
    root = safe_path(repo_root)
    scope = coverage.validate(c.load_json(root / 'data/compiler-compatibility-scope.json'), c, root)
    base = c.validate(c.load_json(root / c.MANIFEST), root)
    manifest = coverage.case_manifest(scope, base, c)
    directories = sorted({safe_path(directory) for directory in reports})
    identity = c.source_identity(root) if directories else None
    runner = coverage.runner_identity(c) if directories else None
    observations = {coverage.case_id(link): [] for link in data['links']}
    for directory in directories:
        # Parse every selected receipt row with the ordinary authenticated API,
        # even when it is not one of this artifact's six exact selectors.
        for key, value in coverage.evidence(scope, manifest, directory, identity, runner, c, root).items():
            if key in observations:
                observations[key].append(value)
    counts = dict.fromkeys(('passed', 'failed', 'timeout', 'infrastructure', 'unverified',
                            'untested', 'missing', 'stale', 'conflicting'), 0)
    links = []
    for link in data['links']:
        attempts = observations[coverage.case_id(link)]
        states = {attempt['status'] for attempt in attempts if attempt['status'] != 'stale'}
        require(all(attempt['status'] in counts for attempt in attempts), 'unknown evidence status')
        status = ('conflicting' if len(states) > 1 else next(iter(states)) if states
                  else 'stale' if attempts else 'untested')
        counts[status] += 1
        links.append({'suite': link['suite'], 'test': link['test'], 'status': status,
                      'evidence': attempts})
    return {
        'schema_version': 1, 'schema': 'bounded-' + ('token' if profile == 'tokens' else profile) + '-witness-observations-v1',
        'source_contract': data['source_contract'], 'source_contract_state': 'partial',
        'witness_artifact_sha256': digest(witness_bytes(data)),
        'source_identity': identity, 'runner_identity': runner,
        'summary': {
            'global_rule_identities': data['summary']['global_rule_identities'],
            'selected_rule_rows': data['summary']['selected_rule_rows'],
            'unselected_rule_identities': data['summary']['unselected_rule_identities'],
            'direct_witness_selectors': len(data['links']),
            'distinct_linked_selectors_observed': sum(bool(row['evidence']) for row in links),
            'evidence_directories': len(directories), 'status_counts': counts,
            'reviewed_rule_rows': 0, 'completed_rules': 0,
            'rows_with_exact_executable_evidence': 0, 'full_rust_percent': None,
        },
        'links': links,
        'limitations': [
            'Execution observations apply only to exact linked selectors, never rule completeness or reviewed inventory evidence.',
            'The source contract remains partial and unreviewed. All reconciliation missing needs remain open.',
        ],
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=['generate','validate','generate-witness-links','build-witness-links','validate-witness-links','assess-witness-links'])
    parser.add_argument('--profile', choices=[profile for profile, _ in PROPOSAL_COUNTS], default='tokens')
    parser.add_argument('--rule-index', type=Path, required=True)
    parser.add_argument('--inventory', type=Path, required=True)
    parser.add_argument('--source-root', type=Path, required=True)
    parser.add_argument('--artifact', type=Path, required=True)
    parser.add_argument('--repo-root', type=Path)
    parser.add_argument('--evidence', type=Path, action='append', default=[])
    args = parser.parse_args(argv)
    if args.profile in SOURCE_ONLY_PAGE_COUNTS:
        require(args.action in ('generate', 'validate'),
                args.profile + ' profile supports generate/validate only')
        require(args.repo_root is None and not args.evidence,
                args.profile + ' profile does not support --repo-root or --evidence')
    if args.profile == 'const-eval' and args.action in ('generate', 'validate'):
        require(args.repo_root is None and not args.evidence,
                'const-eval profile does not support --repo-root or --evidence')
    if args.repo_root is None:
        args.repo_root = Path(__file__).resolve().parent.parent
    require(not args.evidence or args.action == 'assess-witness-links', '--evidence requires assess-witness-links')
    if args.action == 'assess-witness-links':
        artifact = witness_artifact_path(args.artifact, args.repo_root, args.source_root)
        raw = artifact.read_bytes()
        data = json.loads(raw)
        require(raw == witness_bytes(data), 'noncanonical or nondeterministic witness artifact bytes')
        print(json.dumps(assess_witness_links(data, args.rule_index, args.inventory,
                                            args.source_root, args.repo_root, args.evidence, args.profile), sort_keys=True))
        return
    if args.action in ('build-witness-links', 'generate-witness-links', 'validate-witness-links'):
        artifact = witness_artifact_path(args.artifact, args.repo_root, args.source_root)
        if args.action in ('build-witness-links', 'generate-witness-links'):
            data = build_witness_links(args.rule_index, args.inventory, args.source_root, args.repo_root, args.profile)
            with artifact.open('xb') as output:
                output.write(witness_bytes(data))
        else:
            raw = artifact.read_bytes()
            data = json.loads(raw)
            require(raw == witness_bytes(data), 'noncanonical or nondeterministic witness artifact bytes')
        print(json.dumps(validate_witness_links(data, args.rule_index, args.inventory, args.source_root, args.repo_root, args.profile), sort_keys=True))
        return
    if args.action == 'generate':
        data = build(args.rule_index, args.inventory, args.source_root, args.profile)
        with args.artifact.open('x', encoding='utf-8') as output:
            output.write(json.dumps(data, indent=2) + '\n')
    else:
        data = json.loads(args.artifact.read_text())
    print(json.dumps(validate(data, args.rule_index, args.inventory, args.source_root, args.profile), sort_keys=True))


if __name__ == '__main__':
    main()
