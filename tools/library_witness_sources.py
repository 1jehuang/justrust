#!/usr/bin/env python3
"""Authenticate six authored library witness sources, never execute them.

Fixed byte pins authorize this deliberately narrow literal extractor. Documentation
links are partial proposals, not normative review, method completeness or results.
"""
from __future__ import annotations

import argparse
from collections import Counter
import copy
import hashlib
from html.parser import HTMLParser
import json
from pathlib import Path
import re
import sys

import compatibility_inventory as inventory
from library_surface_backlog import provenance_parts, read_json, value_errors
from reconcile_token_rules import safe_path

FIXTURE = 'tests/conformance_generated.rs'
CANDIDATE = 'data/compiler-library-project-inventory.json'
PROVENANCE = 'data/compiler-library-surface-provenance.json'
PINS = {
    FIXTURE: '9e48f6798c36cdaded44cbd38babdea74a84c15b6da73e85cbc0f0eab9e0a0d8',
    'tests/support/closure_oracle.rs': 'ad21cc4daf6d4dbbded2b96ea4112ceefc45a3ff5526a05b3c7234f21bbecfea',
    'tests/support/stock_rustc.rs': '45a9473fa05601a672a1c4d5d98df72395961aea44bf73389e087282b757f238',
    'data/compiler-compatibility-scope.json': '88afac0397deb6ae82dd586c166f81ec1316c14a22c716d98a774d87b07b3f59',
    CANDIDATE: '39bb1f71140f07aa5bfa7cb6371894ce9ef46c299f5a8bc3ea887e5a412669f5',
    PROVENANCE: '6a1168581ae5c090ac739d40fc0cf3588f77ae655e9100157cf492e6423f888c',
}
OPTION = 'core/option/enum.Option.html'
VEC = 'alloc/vec/struct.Vec.html'
ITER = 'core/iter/trait.Iterator.html'
# Authored identities retained from the original source-item reference. No
# sidecar, Rust source prose, or external evidence is a production input.
ROWS = (
    ('library_option_replace_take_nested_owned_drop_source', 'obligation-core-option',
     '1597fdc1b7f8172d05287648e0afcab884c20dd1450ce1bf922bb3e4e9d348b3', 525,
     'Run', 'replaced\ndrop 7\nerror 9\nempty true\nrestored\ndrop 11\n',
     ((OPTION, 'method.replace'), (OPTION, 'method.take')),
     'Replacement returns the old nested owned payload and take empties the slot, with authored cleanup order.'),
    ('library_option_unused_replace_nested_payload_mismatch_source', 'obligation-core-option',
     '244e834cb68f611bf6c0fd43ae0faf371292876fc0f5e2b0bedf041dbf5d85a5', 143,
     'InvalidAfterParse', 'E0308', ((OPTION, 'method.replace'),),
     'An unused body still requires replace to preserve the nested success payload type.'),
    ('library_vec_swap_remove_truncate_owned_drop_source', 'obligation-alloc-vec-string',
     '9a7e085db375f20bbb644658fda549b7989edf16ed760f328b8dc12432bee8e9', 510,
     'Run', 'removed 1 remaining 3 2\ndrop 1\ndrop 2\nkept 3 len 1\ndrop 3\n',
     ((VEC, 'method.swap_remove'), (VEC, 'method.truncate')),
     'swap_remove moves the last owned payload and truncate cleans only the discarded tail.'),
    ('library_vec_unused_truncate_live_element_borrow_source', 'obligation-alloc-vec-string',
     'b9f9d1d7cd5185b491942602249b6213be7c894693d285f41db73d956b1f9b42', 186,
     'InvalidAfterParse', 'E0502', ((VEC, 'method.truncate'),),
     'An unused body cannot truncate while an element borrow is subsequently read.'),
    ('library_iterator_nth_owned_skipped_drop_source', 'obligation-core-iterators',
     '65b99e2edfb60a5309e0758577dd2f195a3952ba0426431af2032bf42efc05d4', 656,
     'Run', 'drop 1\nselected 2\ndrop 2\ntail 3\ndrop 3\nempty true\n',
     ((ITER, 'method.nth'), (ITER, 'tymethod.next'), (VEC, 'method.into_iter')),
     'Owned iteration drops skipped values and returns selected and remaining values once.'),
    ('library_iterator_unused_nth_non_usize_index_source', 'obligation-core-iterators',
     'e41032cfca751144ce24190794a9b120802266c383b21234b574f4dbe75b9bfb', 170,
     'InvalidAfterParse', 'E0308', ((ITER, 'method.nth'),),
     'An unused body must supply a usize index to nth, not bool.'),
)
LIMITS = [
    'Partial method-specific source links only, not full methods, types, families or library coverage.',
    'Normative semantic, stable-public and target-eligibility review is absent.',
    'Authored stdout and diagnostic codes are expectations, not imported execution evidence.',
    'Whole-page metadata pins do not freshly verify HTML bytes or anchors without html_root.',
]


def need(ok, message):
    if not ok:
        raise ValueError(message)


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


class Anchors(HTMLParser):
    """Count exact IDs and version attributes, including duplicate attributes."""
    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.ids = Counter()
        self.versions = []

    def handle_starttag(self, tag, attrs):
        for key, value in attrs:
            if key == 'id':
                self.ids[value] += 1
            if key == 'data-rustdoc-version':
                self.versions.append(value)

    handle_startendtag = handle_starttag


def verify_html(raw, page, anchors):
    need(len(raw) == page['byte_length'] and sha(raw) == page['content_sha256'],
         'documentation page byte mismatch: ' + page['path'])
    parsed = Anchors()
    parsed.feed(raw.decode('utf-8'))
    parsed.close()
    need(parsed.versions == [page['rustdoc_version']], 'documentation version mismatch')
    need(all(parsed.ids[anchor] == 1 for anchor in anchors),
         'missing or duplicate referenced documentation anchor')


def source_binding(raw, row):
    selector, _, expected_sha, length, kind, observation, _, _ = row
    text = raw.decode('utf-8')
    need(text.count('#[path = "support/closure_oracle.rs"]\nmod oracle;\nuse oracle::{Case, Expect};') == 1,
         'missing original oracle alias')
    starts = list(re.finditer(r'^fn ' + re.escape(selector) + r'\(\) \{\n', text, re.M))
    need(len(starts) == 1, 'missing or duplicate authored selector: ' + selector)
    start = starts[0]
    need(text[:start.start()].endswith('#[test]\n'), 'selector is not a top-level test')
    match = re.match(
        r'    oracle::check\(\n        "' + re.escape(selector.replace('_', '-')) +
        r'",\n        &\[Case \{\n            name: "[a-z0-9_]+",\n'
        r'            source: r#"(?P<source>.*?)"#,\n'
        r'            expect: Expect::(?P<kind>Run|InvalidAfterParse)\('
        r'(?P<observation>"(?:[^"\\]|\\.)*")\),\n'
        r'        \}\],\n    \);\n\}', text[start.end():], re.S)
    need(match is not None, 'selector is not an exact single literal Case')
    source = match['source'].encode('utf-8')
    need(b'"#' not in source, 'multiple raw literals in source')
    need(sha(source) == expected_sha and len(source) == length,
         'authored exact source identity mismatch: ' + selector)
    need(match['kind'] == kind and json.loads(match['observation']) == observation,
         'authored expectation mismatch: ' + selector)
    return {'source_sha256': expected_sha, 'source_byte_length': length,
            'oracle_expect': kind, 'expected_observation': observation,
            'source_file': FIXTURE, 'selector_line': text.count('\n', 0, start.start()) + 1,
            'selector_body_sha256': sha(match[0].encode('utf-8'))}


def _summary():
    return {'schema_version': 1, 'readiness': 'missing', 'review_status': 'unreviewed',
            'source_files': [], 'links': [], 'missing_files': [],
            'direct_witness_selectors': 0, 'positive_selectors': 0,
            'semantic_invalid_selectors': 0, 'direct_documentation_links': 0,
            'distinct_documentation_references': 0, 'documentation_page_count': 0,
            'candidate_obligations_with_partial_links': 0,
            'reviewed_obligations': 0, 'completed_obligations': 0,
            'obligations_with_executable_evidence': 0, 'full_rust_completion_percent': None,
            'api_denominator': None, 'execution_evidence_imported': False,
            'source_file_byte_verification': 'not_reverified_in_this_run',
            'documentation_page_byte_verification': 'not_reverified_in_this_run',
            'documentation_anchor_verification': 'not_reverified_in_this_run',
            'limitations': list(LIMITS)}


@value_errors
def summarize(root: Path, html_root: Path | None = None) -> dict:
    """Only the six named local inputs and optional three bound pages are read."""
    try:
        return _summarize(root, html_root)
    except (OSError, UnicodeError, RecursionError) as error:
        raise ValueError('library witness sources: ' + str(error)) from error


def _summarize(root, html_root):
    root = safe_path(root)
    result = _summary()
    metadata, raw_inputs = {}, {}
    # Parse present metadata before deciding whether this is our pinned context.
    for name in (CANDIDATE, PROVENANCE):
        path = safe_path(root / name)
        need(not path.exists() or path.is_file(), 'input is not a file: ' + name)
        if not path.is_file():
            result['missing_files'].append(name)
            continue
        raw, data = read_json(path, 1_000_000)
        if name == CANDIDATE:
            need(type(data) is dict, 'candidate must be an object')
            existing = {r for section in data['sections'] for row in section['obligations']
                        for r in row['existing_requirements']}
            inventory.validate(data, existing)
        else:
            # Authoritative provenance accepts unordered page arrays. The
            # backlog helper wants canonical order, so adapt only its copy.
            # Original bytes still decide trusted context and pin enforcement.
            checked = copy.deepcopy(data)
            checked['pages'] = sorted(checked['pages'], key=lambda page: page['path'])
            provenance_parts(checked)
        metadata[name], raw_inputs[name] = data, raw
    if not any(sha(raw) == PINS[name] for name, raw in raw_inputs.items()):
        result['missing_context'] = 'No trusted library witness metadata pin is present.'
        return result
    # A missing companion must never conceal drift in any other present input.
    for name, pin in PINS.items():
        path = safe_path(root / name)
        need(not path.exists() or path.is_file(), 'input is not a file: ' + name)
        if not path.is_file():
            if name not in result['missing_files']:
                result['missing_files'].append(name)
            continue
        raw = raw_inputs.get(name)
        if raw is None:
            with path.open('rb') as stream:
                raw = stream.read(4_000_001)
            need(len(raw) <= 4_000_000, 'input exceeds byte limit')
        need(sha(raw) == pin, 'local input pin drift: ' + name)
        raw_inputs[name] = raw
        result['source_files'].append({'path': name, 'content_sha256': pin,
                                       'byte_length': len(raw)})
    result['source_file_byte_verification'] = 'partial'
    if result['missing_files']:
        return result
    result['source_file_byte_verification'] = 'reverified_in_this_run'
    scope = json.loads(raw_inputs['data/compiler-compatibility-scope.json'])
    scoped_tests = {(test['suite'], test['test']) for area in scope['areas']
                    for requirement in area['requirements'] for gate in requirement['gates']
                    for test in gate['tests']}
    need(all(('conformance_generated', row[0]) in scoped_tests for row in ROWS),
         'missing exact witness scope membership')
    obligations = {row['id'] for section in metadata[CANDIDATE]['sections']
                   for row in section['obligations']}
    need(len({row[0] for row in ROWS}) == 6, 'duplicate authored selector')
    need(tuple(row[1] for row in ROWS) == (
         'obligation-core-option', 'obligation-core-option',
         'obligation-alloc-vec-string', 'obligation-alloc-vec-string',
         'obligation-core-iterators', 'obligation-core-iterators'), 'foreign candidate mapping')
    need(all(row[1] in obligations for row in ROWS), 'missing candidate obligation')
    pages = {page['path']: page for page in metadata[PROVENANCE]['pages']}
    references = set()
    for row in ROWS:
        selector, obligation, _, _, _, _, refs, rationale = row
        binding = source_binding(raw_inputs[FIXTURE], row)
        docs = []
        for path, anchor in refs:
            need(path in pages, 'missing bound documentation page')
            page = copy.deepcopy(pages[path])
            docs.append(dict(page, anchor=anchor, reference_url=page['url'] + '#' + anchor))
            references.add((path, anchor))
        result['links'].append({'suite': 'conformance_generated', 'test': selector,
            'candidate_obligation_ids': [obligation], 'relationship': 'partial',
            'review_status': 'unreviewed', 'rationale': rationale, 'limitations': list(LIMITS),
            'documentation_references': docs, **binding})
    need(len(references) == 7 and sum(len(row[6]) for row in ROWS) == 10,
         'authored documentation edge counts differ')
    if html_root is not None:
        html_root = safe_path(html_root)
        for path in sorted({path for path, _ in references}):
            file = safe_path(html_root / path)
            need(file.is_file(), 'missing documentation page: ' + path)
            with file.open('rb') as stream:
                raw = stream.read(pages[path]['byte_length'] + 1)
            verify_html(raw, pages[path], {anchor for p, anchor in references if p == path})
        result['documentation_page_byte_verification'] = 'reverified_in_this_run'
        result['documentation_anchor_verification'] = 'reverified_in_this_run'
    result.update(readiness='review-required', direct_witness_selectors=6,
        positive_selectors=3, semantic_invalid_selectors=3, direct_documentation_links=10,
        distinct_documentation_references=7, documentation_page_count=3,
        candidate_obligations_with_partial_links=3)
    return result


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, default=Path(__file__).resolve().parent.parent)
    parser.add_argument('--html-root', type=Path)
    args = parser.parse_args(argv)
    try:
        print(json.dumps(summarize(args.root, args.html_root), indent=2, sort_keys=True,
                         allow_nan=False))
        return 0
    except ValueError as error:
        print('library witness sources: ' + str(error), file=sys.stderr)
        return 1


if __name__ == '__main__':
    sys.exit(main())
