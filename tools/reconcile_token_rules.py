#!/usr/bin/env python3
"""Generate/validate a bounded unreviewed pinned-page rule reconciliation.

Token proposals are the unchanged default. The explicit const-eval profile uses
const_reconciliation_proposals.py, including explicit partial witness links.
Unsafe, ABI, visibility, branching, destructor, loop, operator, closure, call and aggregate profiles cover pinned pages and are generate/validate only.
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
PROPOSAL_COUNTS = (('tokens', 115), ('const-eval', 44), ('unsafe', 27), ('abi', 24), ('visibility', 18), ('branching', 58), ('destructors', 50), ('loops', 60), ('operators', 112), ('closures', 17), ('calls', 23), ('aggregates', 38))
SOURCE_ONLY_PAGE_COUNTS = {'unsafe': (16, 11), 'abi': (24,), 'visibility': (18,), 'branching': (19, 34, 5), 'destructors': (50,), 'loops': (60,), 'operators': (112,), 'closures': (17,), 'calls': (11, 12), 'aggregates': (22, 16)}


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
    elif profile == 'visibility':
        import visibility_reconciliation_proposals as proposals
    elif profile == 'branching':
        import branching_reconciliation_proposals as proposals
    elif profile == 'destructors':
        import destructors_reconciliation_proposals as proposals
    elif profile == 'loops':
        import loops_reconciliation_proposals as proposals
    elif profile == 'operators':
        import operators_reconciliation_proposals as proposals
    elif profile == 'closures':
        import closures_reconciliation_proposals as proposals
    elif profile == 'calls':
        import calls_reconciliation_proposals as proposals
    else:
        import aggregates_reconciliation_proposals as proposals
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


def proposal_row_id(label, profile):
    # Dots and hyphens both occur in distinct loop labels, for example
    # expr.loop.break-label and expr.loop.break.label. A lossy slug merges them.
    # Use reversible UTF-8 bytes for newer profiles, without changing
    # previously published profile artifact identities or canonical bytes.
    if profile in ('loops', 'operators', 'closures', 'calls', 'aggregates'):
        return 'reconcile-' + profile + '-' + label.encode('utf-8').hex()
    return 'reconcile-' + re.sub('[^a-z0-9]+', '-', label)


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


def summarize_proposals(index_path, candidate_path, *, include_proposals=False):
    """Summarize authored module inputs, without Reference prose or sidecars.

    This deliberately does not call build/validate: those independently verify
    actual selected source bytes and remain the stronger reproduction path.
    """
    import compatibility_inventory as inventory

    index_path, candidate_path = Path(index_path), Path(candidate_path)
    require(type(include_proposals) is bool, 'include_proposals must be a boolean')
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
            seen, counts, details = set(), Counter(), []
            for line in authored.strip().splitlines():
                fields = line.split('|')
                require(len(fields) == 7 and all(field.strip() for field in fields),
                        'proposal requires seven nonempty explicit fields')
                label, classification, rationale, links, positive, invalid, interaction = fields
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
                if include_proposals:
                    classifications = sorted(classes[code] for code in codes)
                    executable = bool({'normative-semantic', 'grammar'} & set(classifications))
                    details.append({
                        'page': identity[0], 'rule': label, 'profile': profile,
                        'classifications': classifications, 'rationale': rationale,
                        'candidate_obligation_ids': sorted(aliases[name] for name in names),
                        'relationship': 'proposed-partial-decomposition' if executable else 'context-only',
                        'review_status': 'unreviewed',
                        'missing_test_needs': {'positive': positive, 'invalid': invalid,
                                               'interaction': interaction},
                    })
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
            if include_proposals:
                profiles[profile]['authored_proposals'] = sorted(
                    details, key=lambda row: (row['page'], row['rule']))
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


def query_rule_backlog(index_path, candidate_path, *, page=None, rule=None, missing_only=False,
                       include_proposals=False):
    """Query every indexed label without treating labels as normative coverage.

    The existing pinned metadata and complete authored proposal validators run
    before filtering. Original Reference prose, archives and receipts are not read.
    """
    import compatibility_inventory as inventory

    try:
        for name, value in (('page', page), ('rule', rule)):
            require(value is None or (type(value) is str and bool(value)),
                    name + ': nonempty exact string required')
        require(type(missing_only) is bool, 'missing_only must be a boolean')
        require(type(include_proposals) is bool, 'include_proposals must be a boolean')
        summary = (summarize_proposals(index_path, candidate_path, include_proposals=True)
                   if include_proposals else summarize_proposals(index_path, candidate_path))
        require(summary['readiness'] == 'review-required', 'rule query requires complete pinned metadata inputs')
        # Authenticate the exact bytes used for rows, not an unchecked second load.
        raw = Path(index_path).read_bytes()
        require(digest(raw) == WITNESS_SOURCE_CONTRACT['rule_index_sha256'], 'rule query index pin drift')
        index = json.loads(raw)
        linked = {entry['path'] for entry in index['pages']}
        pages = index['pages'] + index['unlinked_markdown_files']
        paths = {entry['path'] for entry in pages}
        require(page is None or page in paths, 'unknown exact rule page: ' + str(page))
        selected = [entry for entry in pages if page is None or entry['path'] == page]
        if rule is not None:
            selected = [entry for entry in selected if any(row['label'] == rule for row in entry['rules'])]
            require(bool(selected), 'unknown exact rule in selected pages: ' + rule)
        profiles = {name: set(data['selected_pages']) for name, data in summary['profiles'].items()}
        page_rows, rows = [], []
        for entry in sorted(selected, key=lambda value: value['path']):
            names = sorted(name for name, paths in profiles.items() if entry['path'] in paths)
            rules = [row for row in entry['rules'] if (rule is None or row['label'] == rule)
                     and (not missing_only or not names)]
            page_rows.append({'page': entry['path'], 'page_sha256': entry['sha256'],
                              'linked': entry['path'] in linked, 'rule_identities': len(entry['rules']),
                              'selected_rule_identities': len(rules)})
            rows.extend({'page': entry['path'], 'rule': row['label'], 'line': row['line'],
                         'page_sha256': entry['sha256'], 'linked': entry['path'] in linked,
                         'proposal_profiles': list(names),
                         'proposal_status': 'authored-unreviewed' if names else 'missing',
                         'review_status': 'unreviewed'} for row in rules)
        proposed = sum(bool(row['proposal_profiles']) for row in rows)
        report = {'readiness': 'review-required', 'review_status': 'unreviewed', 'scope': dict(SCOPE),
                'bindings': {key: WITNESS_SOURCE_CONTRACT[key] for key in
                             ('reference_commit', 'rule_index_sha256', 'candidate_inventory_sha256')},
                'global_rule_identities': summary['global_rule_identities'],
                'proposed_rule_identities': summary['proposed_rule_identities'],
                'rule_identities_without_proposals': summary['rule_identities_without_proposals'],
                'total_pages': len(pages), 'selected_page_count': len(page_rows),
                'selected_rule_identities': len(rows), 'selected_proposed_rule_identities': proposed,
                'selected_rule_identities_without_proposals': len(rows) - proposed,
                'selection': {'page': page, 'rule': rule, 'missing_only': missing_only},
                'pages': page_rows, 'rows': sorted(rows, key=lambda row: (row['page'], row['rule'])),
                'reviewed_rule_rows': 0, 'completed_rule_rows': 0,
                'rows_with_exact_executable_evidence': 0, 'full_rust_completion_percent': None,
                'normative_inventory_complete': False,
                'source_block_byte_verification': 'not_reverified_in_this_run',
                'source_archive_byte_verification': 'not_reverified_in_this_run',
                'limitations': ['Indexed labels only. Unlabeled prose, grammar and tables may impose additional requirements.',
                                'Authored proposals remain unreviewed. Missing proposals are not compiler failures.',
                                'Pinned metadata bytes are checked, but source blocks, archives and execution evidence are not read.',
                                'No normative completeness, reviewed coverage or full Rust percentage is established.']}
        if include_proposals:
            by_identity = {}
            for profile in sorted(summary['profiles']):
                for detail in summary['profiles'][profile]['authored_proposals']:
                    payload = {key: value for key, value in detail.items() if key not in ('page', 'rule')}
                    by_identity.setdefault((detail['page'], detail['rule']), []).append(payload)
            for row in report['rows']:
                row['authored_proposals'] = by_identity.get((row['page'], row['rule']), [])
            report['include_proposals'] = True
            report['selected_authored_proposal_count'] = sum(
                len(row['authored_proposals']) for row in report['rows'])
            report['limitations'].append(
                'These three authored missing needs do not replace full profile generate/validate '
                'end-to-end requirements. Context-only rows earn no standalone executable credit.')
        return report
    except (ValueError, KeyError, TypeError, AttributeError, OSError) as error:
        raise inventory.InvalidInventory('rule_query: ' + str(error)) from error


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
        require(all(field.strip() for field in fields), 'proposal fields must be nonempty')
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
            'id': proposal_row_id(label, profile),
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
            require(all(field.strip() for field in fields), 'proposal fields must be nonempty')
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
            require(row['id'] == proposal_row_id(row['rule'], profile),
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


def summarize_witness_sources(index_path, candidate_path, repo_root):
    """Read authored partial attribution, not Reference prose or execution evidence."""
    try:
        return _summarize_witness_sources(index_path, candidate_path, repo_root)
    except (KeyError, TypeError, AttributeError, OSError, ImportError, SyntaxError) as exc:
        raise ValueError('witness sources: ' + str(exc)) from exc


def _summarize_witness_sources(index_path, candidate_path, repo_root):
    index_path, candidate_path, repo_root = map(safe_path, (index_path, candidate_path, repo_root))
    for path in (index_path, candidate_path):
        require(not path.exists() or path.is_file(), 'present witness input is not a file')
    proposal = summarize_proposals(index_path, candidate_path)
    index = json.loads(index_path.read_bytes()) if index_path.is_file() else None
    candidate = json.loads(candidate_path.read_bytes()) if candidate_path.is_file() else None
    pages = index['pages'] + index['unlinked_markdown_files'] if index is not None else []
    obligations = {row['id'] for section in candidate['sections']
                   for row in section['obligations']} if candidate is not None else None
    selected_paths = {path for profile in ('tokens', 'const-eval')
                      for path in profile_inputs(profile)[0]}
    witness_context = any(page['path'] in selected_paths for page in pages) or (
        obligations is not None and len(obligations) == 205)
    if proposal.get('missing_selected_pages') or not witness_context:
        return {
            'readiness': 'missing', 'review_status': 'unreviewed',
            'missing_files': proposal.get('missing_files', []),
            'missing_selected_pages': proposal.get('missing_selected_pages', []),
            'global_rule_identities': None, 'profiles': {}, 'links': [], 'source_files': [],
            'direct_witness_selectors': 0, 'direct_rule_links': 0,
            'distinct_direct_rule_identities': 0, 'candidate_obligation_ids': [],
            'candidate_obligations_with_partial_links': 0,
            'reviewed_rule_rows': 0, 'completed_rule_rows': 0,
            'rows_with_exact_executable_evidence': 0, 'full_rust_completion_percent': None,
            'execution_evidence_imported': False,
            'source_file_byte_verification': 'not_reverified_in_this_run',
            'source_block_byte_verification': 'not_reverified_in_this_run',
            'source_archive_byte_verification': 'not_reverified_in_this_run',
            'limitations': ['Required attribution metadata or selected proposal pages are missing. No pinned witness-source claim or attribution is established.'],
        }
    missing = list(proposal.get('missing_files', []))
    profiles, files, all_links = {}, {}, []
    credit = {'reviewed_rule_rows': 0, 'completed_rule_rows': 0,
              'rows_with_exact_executable_evidence': 0, 'full_rust_completion_percent': None}
    # Lexical discovery is bounded to the two authorized source streams. Neither
    # unrelated suites nor their discovery, receipts, or runner are dependencies.
    scope_path = safe_path(repo_root / 'data/compiler-compatibility-scope.json')
    scope, scoped = None, None

    def counts(links):
        ids = sorted({identity for link in links for identity in link['candidate_obligation_ids']})
        return {'direct_witness_selectors': len({(row['suite'], row['test']) for row in links}),
                'direct_rule_links': len({(row['suite'], row['test'], rule['page'], rule['rule'])
                                          for row in links for rule in row['rule_links']}),
                'distinct_direct_rule_identities': len({(rule['page'], rule['rule'])
                                                        for row in links for rule in row['rule_links']}),
                'candidate_obligation_ids': ids, 'candidate_obligations_with_partial_links': len(ids)}

    for profile in ('tokens', 'const-eval'):
        pin, witnesses, aliases, suite, source_file = witness_profile(profile)
        selected = profile_inputs(profile)[0]
        authored_labels = {line.split('|')[0] for line in profile_inputs(profile)[3].strip().splitlines()}
        for field, actual in (
                ('reference_commit', index['provenance']['reference_commit'] if index is not None else None),
                ('rule_index_sha256', digest(index_path.read_bytes()) if index is not None else None),
                ('candidate_inventory_sha256', digest(candidate_path.read_bytes()) if candidate is not None else None)):
            if actual is not None:
                require(actual == pin[field], 'witness source-contract ' + field)
        profile_files = []
        for name, expected in pin['files'].items():
            require(isinstance(name, str) and Path(name).as_posix() == name
                    and not Path(name).is_absolute(), 'noncanonical source-contract path')
            path = safe_path(repo_root / name)
            require(path.is_relative_to(repo_root), 'source-contract path escapes repository')
            if not path.exists():
                missing.append(str(path))
                continue
            require(path.is_file(), 'source-contract path is not a file')
            actual = digest(path.read_bytes())
            require(actual == expected, 'witness source-contract file mismatch: ' + name)
            record = {'path': name, 'sha256': actual}
            profile_files.append(record)
            require(name not in files or files[name] == record, 'conflicting source-contract hash')
            files[name] = record
        if scope is None and scope_path.is_file():
            scope = json.loads(scope_path.read_bytes())
            scoped = {(case['suite'], case['test']) for area in scope['areas']
                      for requirement in area['requirements'] for gate in requirement['gates']
                      for case in gate['tests']}
        path = safe_path(repo_root / source_file)
        discovered = []
        if path.is_file():
            pending_test = False
            for number, line in enumerate(path.read_text().splitlines(), 1):
                if re.match(r'\s*#\[test\]', line):
                    pending_test = True
                function = re.match(r'\s*(?:pub\s+)?fn\s+(\w+)\s*\(', line)
                if pending_test and function:
                    discovered.append({'file': source_file, 'line': number, 'test': function.group(1)})
                    pending_test = False
        require(isinstance(aliases, dict) and aliases, 'invalid witness aliases')
        require(all(isinstance(key, str) and isinstance(value, str) for key, value in aliases.items()),
                'invalid witness alias identity')
        require(len(set(aliases.values())) == len(aliases), 'duplicate witness candidate alias')
        if obligations is not None:
            require(set(aliases.values()) <= obligations and len(obligations) == 205,
                    'foreign witness candidate alias')
        require(isinstance(witnesses, tuple) and len(witnesses) == 6, 'authored witness cardinality mismatch')
        rows = {rule['label']: {'page': page['path'], 'rule': rule['label'], 'line': rule['line']}
                for page in pages if page['path'] in selected for rule in page['rules']}
        links, seen = [], set()
        for authored in witnesses:
            require(isinstance(authored, tuple) and len(authored) == 6, 'invalid authored witness tuple')
            selector, expectation, labels, names, rationale, limitations = authored
            require(isinstance(selector, str) and re.fullmatch(r'[A-Za-z_][A-Za-z0-9_]*', selector),
                    'invalid exact witness selector')
            require(selector not in seen, 'duplicate direct selector')
            seen.add(selector)
            require(expectation in ('positive', 'semantic-invalid'), 'foreign expectation class')
            require(all(isinstance(text, str) and text.strip() for text in (rationale, limitations)),
                    'missing authored witness rationale or limitations')
            for values in (labels, names):
                require(isinstance(values, tuple) and values and all(isinstance(value, str) and value.strip()
                        for value in values) and len(set(values)) == len(values),
                        'empty or duplicate witness labels/aliases')
            require(set(names) <= set(aliases), 'foreign direct candidate alias')
            require(set(labels) <= authored_labels, 'foreign direct rule identity')
            ids = [aliases[name] for name in names]
            require(len(set(ids)) == len(ids), 'duplicate direct candidate identity')
            if index is not None:
                require(set(labels) <= set(rows), 'foreign direct rule identity')
            path = safe_path(repo_root / source_file)
            matches = [row for row in discovered if row['file'] == source_file and row['test'] == selector]
            if path.is_file():
                require(len(matches) == 1, 'missing or duplicate exact top-level selector: ' + selector)
                text = path.read_text()
                require(len(re.findall(r'^fn ' + re.escape(selector) + r'\(\) \{\s*$', text, re.M)) == 1,
                        'selector is not a unique literal top-level function')
            if scoped is not None:
                require((suite, selector) in scoped, 'exact selector absent from compatibility scope: ' + selector)
            if index is None or not path.is_file() or scope is None:
                continue
            link = {'suite': suite, 'test': selector, 'source_file': source_file,
                    'selector_line': matches[0]['line'], 'expectation_class': expectation,
                    'relationship': 'unreviewed-direct-partial-link',
                    'rule_links': [dict(rows[label]) for label in labels],
                    'candidate_obligation_ids': ids, 'rationale': rationale, 'limitations': limitations}
            if profile == 'const-eval':
                link.update(scalar_source_binding(path.read_bytes(), selector, expectation))
            links.append(link)
        links.sort(key=lambda row: (row['suite'], row['test']))
        profiles[profile] = {'selected_pages': list(selected), 'source_files': profile_files,
                             'links': links, **counts(links), **credit, 'review_status': 'unreviewed'}
        all_links.extend(links)
    return {'readiness': 'missing' if missing or proposal['readiness'] == 'missing' else 'review-required',
            'review_status': 'unreviewed', 'missing_files': sorted(set(missing)),
            'global_rule_identities': None if missing or proposal['readiness'] == 'missing' else 2965,
            'profiles': profiles, 'links': all_links, 'source_files': [files[name] for name in sorted(files)],
            **counts(all_links), **credit, 'execution_evidence_imported': False,
            'source_file_byte_verification': 'partial' if missing else 'reverified_in_this_run',
            'source_block_byte_verification': 'not_reverified_in_this_run',
            'source_archive_byte_verification': 'not_reverified_in_this_run',
            'limitations': [
                'Partial authored attribution only, not full obligation coverage, normative review, execution or completion credit.',
                'Inventory obligations contain no embedded exact links. Their missing-link counts remain unchanged.',
                'Selector discovery is lexical, not libtest discovery or execution. Only authorized fixture streams and linked scope memberships are read.',
                'Local source files are reverified where present. Reference blocks and archive bytes are NOT reverified in this run.',
                'Source-backed build_witness_links remains an independent stronger verification path.']}


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
