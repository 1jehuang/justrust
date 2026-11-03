#!/usr/bin/env python3
"""Generate/validate a bounded unreviewed pinned-token rule reconciliation.

Classifications are explicitly authored in token_reconciliation_proposals.py.
The validator checks identities, hashes, row completeness and structural claims,
not whether the proposed interpretation of normative Rust is correct.
"""
import argparse
import hashlib
import json
import re
from collections import Counter
from pathlib import Path

from extract_reference_rules import SCOPE, parse_markdown
from token_reconciliation_proposals import ALIASES, CLASS_NAMES, ROWS

SELECTED = ('src/tokens.md',)
CLASSES = set(CLASS_NAMES.values())
TOP_FIELDS = {'schema_version','review_status','purpose','scope','bindings','selected_pages','rows','unselected_rules','counts','limitations'}
ROW_FIELDS = {'id','page','rule','line','end_line','source_block_sha256','classifications','rationale','candidate_obligation_ids','relationship','missing_test_needs','review_status'}


def digest(data):
    return hashlib.sha256(data).hexdigest()


def require(condition, message):
    if not condition:
        raise ValueError(message)


def load_inputs(index_path, candidate_path, source_root):
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
    for path in SELECTED:
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


def compute_counts(rows, universe_size):
    classes = Counter(c for row in rows for c in row['classifications'])
    return {
        'global_rule_identities': universe_size,
        'selected_rule_rows': len(rows),
        'unselected_rule_identities': universe_size - len(rows),
        'proposed_classification_counts': {c: classes[c] for c in sorted(CLASSES)},
        'reviewed_rule_rows': 0,
        'rows_with_exact_executable_evidence': 0,
    }


def build(index_path, candidate_path, source_root):
    records, identities, obligations, bindings, pages = load_inputs(index_path, candidate_path, source_root)
    rows = []
    for line in ROWS.strip().splitlines():
        fields = line.split('|')
        require(len(fields) == 7, 'proposal requires seven explicit fields')
        label, classes, rationale, aliases, positive, invalid, interaction = fields
        identity = ('src/tokens.md', label)
        require(identity in records, 'foreign proposal rule: ' + label)
        classifications = [CLASS_NAMES[c] for c in classes.split()]
        executable = 'normative-semantic' in classifications or 'grammar' in classifications
        row = {
            'id': 'reconcile-' + re.sub('[^a-z0-9]+', '-', label),
            'page': identity[0], 'rule': label, **records[identity],
            'classifications': classifications,
            'rationale': rationale,
            'candidate_obligation_ids': [ALIASES[a] for a in aliases.split()],
            'relationship': 'proposed-partial-decomposition' if executable else 'context-only',
            'missing_test_needs': [
                'positive: Missing direct exact test link. ' + positive,
                'invalid: Missing direct exact test link. ' + invalid,
                'interaction: Missing direct exact test link. ' + interaction,
                ('end-to-end: Missing authored original-source CHECK/BUILD/run or stage-specific rejection witnesses, named test identities, pinned-stock outcomes and immutable receipts at workers1/8. Lexical-invalid rejection is distinct from semantic body rejection, and no execution is inferred from legacy IDs.' if executable else 'end-to-end: No standalone runtime feature is claimed by this context row. Missing source-review links and any concrete child-rule test attribution; do not grant executable credit for index/cross-reference text.')
            ],
            'review_status': 'unreviewed',
        }
        rows.append(row)
    rows.sort(key=lambda r: (r['page'], r['line']))
    selected = {(r['page'], r['rule']) for r in rows}
    result = {
        'schema_version': 1,
        'review_status': 'unreviewed',
        'purpose': 'bounded-token-rule-reconciliation-candidate',
        'scope': dict(SCOPE),
        'bindings': bindings,
        'selected_pages': pages,
        'rows': rows,
        'unselected_rules': [{'page': page, 'rule': rule} for page, rule in sorted(set(identities) - selected)],
        'counts': compute_counts(rows, len(identities)),
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
    validate(result, index_path, candidate_path, source_root)
    return result


def validate(data, index_path, candidate_path, source_root):
    require(isinstance(data, dict) and set(data) == TOP_FIELDS, 'unexpected sidecar fields')
    require(type(data['schema_version']) is int and data['schema_version'] == 1, 'schema version')
    require(data['scope'] == SCOPE and data['review_status'] == 'unreviewed', 'reviewed or foreign scope claim')
    require(data['purpose'] == 'bounded-token-rule-reconciliation-candidate', 'purpose mismatch')
    records, identities, obligations, bindings, pages = load_inputs(index_path, candidate_path, source_root)
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
        require(isinstance(classifications, list) and classifications and all(isinstance(c, str) for c in classifications) and len(set(classifications)) == len(classifications) and set(classifications) <= CLASSES, 'invalid classifications')
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
    require(json.dumps(data['counts'], sort_keys=True) == json.dumps(compute_counts(data['rows'], len(identities)), sort_keys=True), 'counts or evidence claim mismatch')
    require(isinstance(data['limitations'], list) and data['limitations'] and all(isinstance(t, str) and t.strip() for t in data['limitations']), 'missing limitations')
    return data['counts']


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=['generate','validate'])
    parser.add_argument('--rule-index', type=Path, required=True)
    parser.add_argument('--inventory', type=Path, required=True)
    parser.add_argument('--source-root', type=Path, required=True)
    parser.add_argument('--artifact', type=Path, required=True)
    args = parser.parse_args(argv)
    if args.action == 'generate':
        data = build(args.rule_index, args.inventory, args.source_root)
        with args.artifact.open('x', encoding='utf-8') as output:
            output.write(json.dumps(data, indent=2) + '\n')
    else:
        data = json.loads(args.artifact.read_text())
    print(json.dumps(validate(data, args.rule_index, args.inventory, args.source_root), sort_keys=True))


if __name__ == '__main__':
    main()
