#!/usr/bin/env python3
"""Generate/validate a bounded unreviewed pinned-token rule reconciliation.

Classifications are explicitly authored in token_reconciliation_proposals.py.
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


def build_witness_links(index_path, candidate_path, source_root, repo_root):
    """Direct partial attribution, never imported execution or reviewed coverage."""
    index_path, candidate_path, source_root, repo_root = map(
        safe_path, (index_path, candidate_path, source_root, repo_root))
    # Reuse every v1 source/page/line/block and candidate identity check. Keep the
    # entire reconciliation, including all missing needs and unselected rules.
    reconciliation = build(index_path, candidate_path, source_root)
    pin = copy.deepcopy(WITNESS_SOURCE_CONTRACT)
    for field in ('reference_commit', 'rule_index_sha256', 'candidate_inventory_sha256'):
        require(reconciliation['bindings'][field] == pin[field], 'witness source-contract ' + field)
    require(reconciliation['counts']['global_rule_identities'] == 2965
            and len(reconciliation['rows']) == 115
            and len(reconciliation['unselected_rules']) == 2850,
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
    require(len(DIRECT_WITNESSES) == 6, 'requires exactly six authored witnesses')
    for selector, expectation, labels, aliases, rationale, limitations in DIRECT_WITNESSES:
        matches = [row for row in discovered if row['file'] == 'tests/conformance_generated.rs'
                   and row['test'] == selector]
        require(len(matches) == 1, 'missing or duplicate exact top-level selector: ' + selector)
        require(('conformance_generated', selector) in scoped_selectors,
                'exact selector absent from compatibility scope: ' + selector)
        require(expectation in ('positive', 'semantic-invalid'), 'foreign expectation class')
        candidate_ids = [ALIASES[alias] for alias in aliases]
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
            'suite': 'conformance_generated', 'test': selector,
            'source_file': 'tests/conformance_generated.rs',
            'selector_line': matches[0]['line'],
            'expectation_class': expectation,
            'relationship': 'unreviewed-direct-partial-link',
            'rule_links': rule_links, 'candidate_obligation_ids': candidate_ids,
            'rationale': rationale, 'limitations': limitations,
        })
    require(len({link['test'] for link in links}) == 6, 'duplicate direct selector')
    links.sort(key=lambda row: (row['suite'], row['test']))
    return {
        'schema_version': 1, 'schema': 'bounded-token-direct-witness-links-v1',
        'purpose': 'scratch-only-unreviewed-token-direct-witness-links',
        'scope': dict(SCOPE), 'source_contract': pin,
        'source_files': files, 'reconciliation': reconciliation, 'links': links,
        'summary': {
            'global_rule_identities': 2965, 'selected_rule_rows': 115,
            'unselected_rule_identities': 2850, 'direct_witness_selectors': len(links),
            'direct_rule_links': sum(len(link['rule_links']) for link in links),
            'distinct_direct_rule_identities': len({row['rule'] for link in links for row in link['rule_links']}),
            'positive_selectors': sum(link['expectation_class'] == 'positive' for link in links),
            'semantic_invalid_selectors': sum(link['expectation_class'] == 'semantic-invalid' for link in links),
            'reviewed_rule_rows': 0, 'rows_with_exact_executable_evidence': 0,
            'full_rust_percent': None,
        },
        'limitations': [
            'Literal rule links and rationale are scoped unreviewed attribution, not normative review. No title-based coverage inference.',
            'Whole-file hashes bind actual fixture and helper source, not extracted program bytes. No authored program hashes or stale worker fixture hashes are imported.',
            'An exact selector link is not completion of a rule, candidate bucket, missing positive/invalid layer, or passing outcome. All v1 missing needs remain open.',
            'Positive and semantic-invalid are authored expectation classes, not results. The known ignored tuple .01 selector remains linked. No evidence import, compiler execution or passing credit.',
            'The source contract pins this bounded source snapshot only. It does not authenticate the full reference archive or certify full Rust support.',
        ],
    }


def validate_witness_links(data, index_path, candidate_path, source_root, repo_root):
    expected = build_witness_links(index_path, candidate_path, source_root, repo_root)
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


def assess_witness_links(data, index_path, candidate_path, source_root, repo_root, reports=()):
    """Observe exact linked executions, without upgrading source attribution."""
    validate_witness_links(data, index_path, candidate_path, source_root, repo_root)
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
        'schema_version': 1, 'schema': 'bounded-token-witness-observations-v1',
        'source_contract': data['source_contract'], 'source_contract_state': 'partial',
        'witness_artifact_sha256': digest(witness_bytes(data)),
        'source_identity': identity, 'runner_identity': runner,
        'summary': {
            'global_rule_identities': 2965, 'selected_rule_rows': 115,
            'unselected_rule_identities': 2850, 'direct_witness_selectors': 6,
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
    parser.add_argument('action', choices=['generate','validate','build-witness-links','validate-witness-links','assess-witness-links'])
    parser.add_argument('--rule-index', type=Path, required=True)
    parser.add_argument('--inventory', type=Path, required=True)
    parser.add_argument('--source-root', type=Path, required=True)
    parser.add_argument('--artifact', type=Path, required=True)
    parser.add_argument('--repo-root', type=Path, default=Path(__file__).resolve().parent.parent)
    parser.add_argument('--evidence', type=Path, action='append', default=[])
    args = parser.parse_args(argv)
    require(not args.evidence or args.action == 'assess-witness-links', '--evidence requires assess-witness-links')
    if args.action == 'assess-witness-links':
        artifact = witness_artifact_path(args.artifact, args.repo_root, args.source_root)
        raw = artifact.read_bytes()
        data = json.loads(raw)
        require(raw == witness_bytes(data), 'noncanonical or nondeterministic witness artifact bytes')
        print(json.dumps(assess_witness_links(data, args.rule_index, args.inventory,
                                            args.source_root, args.repo_root, args.evidence), sort_keys=True))
        return
    if args.action in ('build-witness-links', 'validate-witness-links'):
        artifact = witness_artifact_path(args.artifact, args.repo_root, args.source_root)
        if args.action == 'build-witness-links':
            data = build_witness_links(args.rule_index, args.inventory, args.source_root, args.repo_root)
            with artifact.open('xb') as output:
                output.write(witness_bytes(data))
        else:
            raw = artifact.read_bytes()
            data = json.loads(raw)
            require(raw == witness_bytes(data), 'noncanonical or nondeterministic witness artifact bytes')
        print(json.dumps(validate_witness_links(data, args.rule_index, args.inventory, args.source_root, args.repo_root), sort_keys=True))
        return
    if args.action == 'generate':
        data = build(args.rule_index, args.inventory, args.source_root)
        with args.artifact.open('x', encoding='utf-8') as output:
            output.write(json.dumps(data, indent=2) + '\n')
    else:
        data = json.loads(args.artifact.read_text())
    print(json.dumps(validate(data, args.rule_index, args.inventory, args.source_root), sort_keys=True))


if __name__ == '__main__':
    main()
