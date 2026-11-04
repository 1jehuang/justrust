#!/usr/bin/env python3
"""Bounded literal documentation observations, never an API denominator.

build/verify authenticate retained sidecar bytes, not original HTML bytes.
validate_metadata checks recorded metadata only. It cannot freshly verify any
external bytes or recompute candidate observations without the retained sidecar.
"""
import argparse
import copy
from datetime import date
from functools import wraps
import hashlib
import json
from pathlib import Path
import re
import sys

MAX_BYTES = 100_000
MAX_SURFACE_BYTES = 64_000_000
MAX_QUERY_BYTES = 1_000_000
CRATES = ('alloc', 'core', 'proc_macro', 'std')
COUNTS = frozenset(('literal_candidate_records', 'unique_candidate_page_paths',
    'downloaded_page_records', 'downloaded_index_all_pages',
    'downloaded_other_detail_pages', 'missing_unique_paths',
    'candidate_records_collapsed_by_page_path'))
FIELDS = frozenset(('schema_version', 'inventory_kind', 'source_identity', 'scope',
    'surface_artifact', 'downloaded_pages', 'observations', 'missing_path_sample',
    'missing_path_sample_truncated', 'review_credit', 'execution_credit',
    'completion_credit', 'api_denominator', 'full_rust_denominator', 'interpretation'))
INTERPRETATION = ('Literal documentation observations only, not a stable public target API '
    'denominator. Broad API enumeration remains open. No review, execution or completion '
    'credit. Metadata-only validation does not recompute per-crate candidate observations '
    'or missing samples and is not fresh external-byte verification. Sidecar verification '
    'authenticates retained JSON bytes only, not original HTML source bytes.')
QUERY_INTERPRETATION = ('Authenticated retained-sidecar literal candidates only, not stable public '
    'target API identities, reachability or implementation support. Byte authentication is '
    'relative to the supplied recorded provenance, not fresh source-index or official HTML '
    'verification. A downloaded page flag '
    'means recorded page acquisition, not anchor existence, fresh HTML verification or API '
    'coverage. Missing-page filtering means page bytes were not acquired in the recorded '
    'snapshot, never that Rust lacks the API. Navigation pages, aliases, unstable and '
    'other-target items remain unclassified. Review, execution and completion credit stay '
    'zero; API and full Rust denominators remain unavailable.')
PAGE_FIELDS = frozenset(('path', 'url', 'byte_length', 'content_sha256', 'rustdoc_version'))
ARTIFACT_FIELDS = frozenset(('id', 'filename', 'content_sha256', 'byte_length', 'role'))
IDENTITY_FIELDS = frozenset(('commit', 'library_tree_sha1', 'official_documentation_root',
    'release', 'repository', 'root_tree_sha1', 'source_index_content_sha256', 'verification'))


def value_errors(function):
    """Public validators normalize malformed nested JSON to ValueError."""
    @wraps(function)
    def checked(*args, **kwargs):
        try:
            return function(*args, **kwargs)
        except (KeyError, TypeError, AttributeError, IndexError, OverflowError) as error:
            raise ValueError('malformed input metadata: ' + str(error)) from error
    return checked


def need(ok, message):
    if not ok:
        raise ValueError(message)


def fields(value, keys, label):
    need(type(value) is dict and set(value) == keys, 'incorrect ' + label + ' fields')


def integer(value):
    return type(value) is int and value >= 0


def digest(value, size=64):
    return type(value) is str and re.fullmatch('[0-9a-f]{%d}' % size, value) is not None


def no_duplicates(pairs):
    result = {}
    for key, value in pairs:
        need(key not in result, 'duplicate JSON key: ' + key)
        result[key] = value
    return result


def read_json(path, limit):
    with path.open('rb') as stream:
        raw = stream.read(limit + 1)
    need(len(raw) <= limit, 'input exceeds byte limit')
    return raw, json.loads(raw, object_pairs_hook=no_duplicates,
                           parse_constant=lambda s: (_ for _ in ()).throw(ValueError(s)))


def path_ok(path):
    need(type(path) is str and len(path) <= 2048 and
         re.fullmatch(r'(alloc|core|proc_macro|std)/(?:[A-Za-z0-9_.-]+/)*[A-Za-z0-9_.-]+\.html', path)
         is not None and all(part not in ('.', '..') for part in path.split('/')),
         'noncanonical or foreign documentation path')
    return path


def identity_ok(identity):
    fields(identity, IDENTITY_FIELDS, 'identity')
    need(type(identity['release']) is str and re.fullmatch(r'[0-9]+\.[0-9]+\.[0-9]+', identity['release']),
         'noncanonical release')
    need(identity['official_documentation_root'] == 'https://doc.rust-lang.org/' + identity['release'] + '/',
         'noncanonical documentation root')
    need(identity['repository'] == 'https://github.com/rust-lang/rust', 'foreign repository')
    for key in ('commit', 'library_tree_sha1', 'root_tree_sha1'):
        need(digest(identity[key], 40), 'invalid source identity digest')
    need(digest(identity['source_index_content_sha256']), 'invalid source sidecar digest')
    need(type(identity['verification']) is str and bool(identity['verification']), 'invalid verification metadata')


def artifact_ok(artifact):
    fields(artifact, ARTIFACT_FIELDS, 'sidecar descriptor')
    need(artifact['id'] == 'pinned-html-link-anchor-candidate' and
         artifact['filename'] == 'pinned-html-api-index.json' and
         digest(artifact['content_sha256']) and integer(artifact['byte_length']) and
         0 < artifact['byte_length'] <= MAX_SURFACE_BYTES and
         type(artifact['role']) is str and bool(artifact['role']), 'invalid sidecar descriptor')


def pages_ok(pages, identity):
    need(type(pages) is list, 'pages must be a list')
    paths = []
    versions = set()
    for page in pages:
        fields(page, PAGE_FIELDS, 'downloaded page')
        path = path_ok(page['path'])
        need(page['url'] == identity['official_documentation_root'] + path,
             'incorrect page URL')
        need(integer(page['byte_length']) and page['byte_length'] > 0 and digest(page['content_sha256']),
             'invalid page byte fingerprint')
        need(type(page['rustdoc_version']) is str and re.fullmatch(
            re.escape(identity['release']) + r' \(' + re.escape(identity['commit'][:9]) +
            r' [0-9]{4}-[0-9]{2}-[0-9]{2}\)', page['rustdoc_version']) is not None,
            'page version mismatch')
        date.fromisoformat(page['rustdoc_version'][-11:-1])
        versions.add(page['rustdoc_version'])
        paths.append(path)
    need(len(versions) == 1, 'inconsistent rustdoc version markers')
    need(paths == sorted(set(paths)), 'duplicate or unsorted downloaded pages')
    need({crate + '/' + name for crate in CRATES for name in ('index.html', 'all.html')} <= set(paths),
         'missing required eight index/all pages')
    return set(paths)


def page_counts(paths):
    index = sum(path in {crate + '/' + name for crate in CRATES
                        for name in ('index.html', 'all.html')} for path in paths)
    return {'downloaded_page_records': len(paths), 'downloaded_index_all_pages': index,
            'downloaded_other_detail_pages': len(paths) - index}


@value_errors
def validate_structure(data):
    """Validate recorded structure/arithmetic only, without authenticating bytes."""
    fields(data, FIELDS, 'backlog')
    need(type(data['schema_version']) is int and data['schema_version'] == 1 and
         data['inventory_kind'] == 'literal-documentation-observation-backlog', 'incorrect backlog kind/version')
    need(data['scope'] == {'edition': '2024', 'target': 'x86_64-unknown-linux-gnu'}, 'incorrect scope')
    identity_ok(data['source_identity'])
    artifact_ok(data['surface_artifact'])
    paths = pages_ok(data['downloaded_pages'], data['source_identity'])
    need(data['interpretation'] == INTERPRETATION, 'incorrect interpretation')
    for key in ('review_credit', 'execution_credit', 'completion_credit'):
        need(type(data[key]) is int and data[key] == 0, 'nonzero/invalid credit')
    need(data['api_denominator'] is None and data['full_rust_denominator'] is None,
         'API/full Rust denominators must remain null')
    observations = data['observations']
    fields(observations, {'global', 'per_crate'}, 'observations')
    fields(observations['per_crate'], set(CRATES), 'per-crate observations')
    for name, counts in [('global', observations['global']), *observations['per_crate'].items()]:
        fields(counts, COUNTS, 'counts')
        need(all(integer(value) for value in counts.values()), 'invalid nonnegative literal count')
        need(counts['literal_candidate_records'] == counts['unique_candidate_page_paths'] +
             counts['candidate_records_collapsed_by_page_path'], 'incorrect candidate dedup aggregate')
        need(counts['missing_unique_paths'] == counts['unique_candidate_page_paths'] -
             counts['downloaded_page_records'], 'incorrect missing aggregate')
        subset = paths if name == 'global' else {p for p in paths if p.startswith(name + '/')}
        need(all(counts[k] == value for k, value in page_counts(subset).items()),
             'downloaded counts differ from recorded page fingerprints')
    need(all(observations['global'][key] == sum(observations['per_crate'][c][key] for c in CRATES)
             for key in COUNTS), 'incorrect global aggregate')
    sample = data['missing_path_sample']
    need(type(sample) is list, 'sample must be a list')
    missing = observations['global']['missing_unique_paths']
    need(len(sample) == min(100, missing), 'incorrect sample length')
    sample_paths = []
    for item in sample:
        fields(item, {'path', 'url'}, 'missing path sample')
        path = path_ok(item['path'])
        need(item['url'] == data['source_identity']['official_documentation_root'] + path,
             'incorrect missing sample URL')
        need(path not in paths, 'downloaded page in missing sample')
        sample_paths.append(path)
    need(sample_paths == sorted(set(sample_paths)), 'duplicate or unsorted missing sample')
    for crate in CRATES:
        need(sum(p.startswith(crate + '/') for p in sample_paths) <=
             observations['per_crate'][crate]['missing_unique_paths'], 'sample exceeds per-crate missing count')
    need(type(data['missing_path_sample_truncated']) is bool and
         data['missing_path_sample_truncated'] == (missing > 100), 'incorrect truncation flag')
    need(len(encode(data)) < MAX_BYTES, 'backlog output must be less than 100KB')
    return data


@value_errors
def provenance_parts(provenance):
    need(type(provenance) is dict and type(provenance.get('schema_version')) is int and
         provenance['schema_version'] == 1 and provenance.get('review_status') == 'unreviewed',
         'incorrect provenance schema/review state')
    need({'source_identity', 'scope', 'artifacts', 'pages', 'observations'} <= provenance.keys(),
         'missing required provenance fields')
    identity_ok(provenance['source_identity'])
    need(provenance['scope'] == {'edition': '2024', 'target': 'x86_64-unknown-linux-gnu'},
         'incorrect provenance scope')
    need(type(provenance['artifacts']) is list, 'incorrect provenance artifacts')
    descriptors = [a for a in provenance['artifacts'] if type(a) is dict and
                   a.get('id') == 'pinned-html-link-anchor-candidate']
    need(len(descriptors) == 1, 'missing/duplicate HTML sidecar descriptor')
    artifact_ok(descriptors[0])
    pages_ok(provenance['pages'], provenance['source_identity'])
    need(type(provenance['observations']) is dict and
         {'downloaded_page_records', 'literal_index_link_anchor_candidate_records',
          'missing_detail_page_records'} <= provenance['observations'].keys(),
         'missing required provenance observations')
    for key in ('downloaded_page_records', 'literal_index_link_anchor_candidate_records', 'missing_detail_page_records'):
        need(integer(provenance['observations'][key]), 'invalid provenance observation')
    need(provenance['observations']['downloaded_page_records'] == len(provenance['pages']),
         'inconsistent provenance page count')
    return descriptors[0]


@value_errors
def validate_metadata(data, provenance):
    """Check recorded bindings/count consistency, NOT external-byte verification.

    Per-crate candidate observations and sample membership/order relative to the
    full missing set cannot be independently recomputed here. Use verify for that.
    """
    validate_structure(data)
    artifact = provenance_parts(provenance)
    need(data['source_identity'] == provenance['source_identity'] and
         data['scope'] == provenance['scope'] and data['surface_artifact'] == artifact and
         data['downloaded_pages'] == provenance['pages'], 'backlog provenance binding mismatch')
    obs = provenance['observations']
    counts = data['observations']['global']
    need(counts['literal_candidate_records'] == obs['literal_index_link_anchor_candidate_records'] and
         counts['missing_unique_paths'] == obs['missing_detail_page_records'], 'provenance aggregate mismatch')
    return {'readiness': 'review-required', 'review_status': 'unreviewed',
            'external_verification': 'recorded-not-recomputed', 'full_rust_completion_percent': None,
            **{key: copy.deepcopy(data[key]) for key in ('observations', 'missing_path_sample',
                'missing_path_sample_truncated', 'review_credit', 'execution_credit',
                'completion_credit', 'api_denominator', 'full_rust_denominator', 'interpretation')}}


@value_errors
def _build_snapshot(surface_path: Path, provenance_path: Path):
    """Authenticate and validate once, retaining the exact candidate snapshot."""
    _, provenance = read_json(provenance_path, 1_000_000)
    descriptor = provenance_parts(provenance)
    raw, surface = read_json(surface_path, MAX_SURFACE_BYTES)
    need(len(raw) == descriptor['byte_length'] and hashlib.sha256(raw).hexdigest() ==
         descriptor['content_sha256'], 'retained sidecar byte authentication failed')
    need(type(surface) is dict and type(surface.get('schema_version')) is int and
         surface['schema_version'] == 1 and surface.get('review_status') == 'unreviewed',
         'incorrect surface schema/review state')
    fields(surface, {'schema_version', 'review_status', 'scope', 'source_identity', 'pages',
                     'index_link_candidates', 'missing_detail_pages', 'limitations'}, 'surface')
    need(surface['source_identity'] == provenance['source_identity'] and surface['scope'] == provenance['scope'],
         'surface identity/scope mismatch')
    need(type(surface['pages']) is list, 'incorrect surface pages')
    pages = []
    for page in surface['pages']:
        need(type(page) is dict and PAGE_FIELDS <= page.keys(), 'missing surface page metadata')
        pages.append({key: page[key] for key in PAGE_FIELDS})
    pages.sort(key=lambda page: page['path'])
    paths = pages_ok(pages, provenance['source_identity'])
    need(pages == provenance['pages'], 'surface page metadata differs from compact provenance')
    candidates = surface['index_link_candidates']
    need(type(candidates) is list, 'incorrect candidates')
    pairs = set()
    candidate_paths = set()
    percrate_records = dict.fromkeys(CRATES, 0)
    for candidate in candidates:
        fields(candidate, {'anchor', 'index_pages', 'labels', 'path', 'stable_api_review',
                           'target_availability_review', 'url'}, 'candidate')
        path = path_ok(candidate['path'])
        anchor = candidate['anchor']
        need(type(anchor) is str and len(anchor) <= 2048 and
             re.fullmatch(r'[A-Za-z0-9_.:-]*', anchor) is not None, 'noncanonical candidate anchor')
        need(candidate['url'] == provenance['source_identity']['official_documentation_root'] + path,
             'incorrect candidate URL')
        need((path, anchor) not in pairs, 'duplicate (path,anchor) candidate')
        pairs.add((path, anchor))
        candidate_paths.add(path)
        percrate_records[path.split('/')[0]] += 1
        need(candidate['stable_api_review'] == 'required' and
             candidate['target_availability_review'] == 'required', 'candidate review state mismatch')
        for key in ('labels', 'index_pages'):
            need(type(candidate[key]) is list and all(type(x) is str for x in candidate[key]),
                 'incorrect candidate ' + key)
        need(bool(candidate['index_pages']) and len(set(candidate['index_pages'])) == len(candidate['index_pages']) and
             all(x in {c + '/' + n for c in CRATES for n in ('index.html', 'all.html')}
                 for x in candidate['index_pages']), 'incorrect candidate index pages')
    need(paths <= candidate_paths, 'downloaded page absent from candidate paths')
    missing = surface['missing_detail_pages']
    need(type(missing) is list, 'incorrect missing paths')
    for path in missing:
        path_ok(path)
    need(len(set(missing)) == len(missing), 'duplicate missing paths')
    derived_missing = sorted(candidate_paths - paths)
    need(set(missing) == set(derived_missing), 'missing set differs from candidate paths minus downloaded pages')
    percrate = {}
    for crate in CRATES:
        unique = sum(p.startswith(crate + '/') for p in candidate_paths)
        downloaded = {p for p in paths if p.startswith(crate + '/')}
        percrate[crate] = {'literal_candidate_records': percrate_records[crate],
            'unique_candidate_page_paths': unique, **page_counts(downloaded),
            'missing_unique_paths': unique - len(downloaded),
            'candidate_records_collapsed_by_page_path': percrate_records[crate] - unique}
    data = {'schema_version': 1, 'inventory_kind': 'literal-documentation-observation-backlog',
        'source_identity': provenance['source_identity'], 'scope': provenance['scope'],
        'surface_artifact': descriptor, 'downloaded_pages': pages,
        'observations': {'global': {key: sum(percrate[c][key] for c in CRATES) for key in COUNTS},
                         'per_crate': percrate},
        'missing_path_sample': [{'path': path, 'url': provenance['source_identity']['official_documentation_root'] + path}
                                for path in derived_missing[:100]],
        'missing_path_sample_truncated': len(derived_missing) > 100,
        'review_credit': 0, 'execution_credit': 0, 'completion_credit': 0,
        'api_denominator': None, 'full_rust_denominator': None, 'interpretation': INTERPRETATION}
    validate_metadata(data, provenance)
    return data, candidates


def build(surface_path: Path, provenance_path: Path) -> dict:
    """Build the unchanged compact backlog from authenticated retained bytes."""
    return _build_snapshot(surface_path, provenance_path)[0]


def query_options(crate, page, anchor, missing_only, limit, offset):
    """Reject malformed selectors and bounds before opening any inputs."""
    need(crate is None or type(crate) is str and crate in CRATES, 'unknown query crate')
    if page is not None:
        path_ok(page)
        need(crate is None or page.startswith(crate + '/'), 'query crate/page mismatch')
    need(anchor is None or type(anchor) is str and len(anchor) <= 2048 and
         re.fullmatch(r'[A-Za-z0-9_.:-]*', anchor) is not None,
         'noncanonical query anchor')
    need(type(missing_only) is bool, 'missing_only must be a boolean')
    need(type(limit) is int and 1 <= limit <= 100, 'query limit must be 1..100')
    need(type(offset) is int and offset >= 0, 'query offset must be nonnegative')


@value_errors
def query(surface_path: Path, provenance_path: Path, *, crate=None, page=None,
          anchor=None, missing_only=False, limit=100, offset=0):
    """Page through exact literal candidates, not a semantic/public API census.

    All sidecar records are validated before filtering. Querying uses the same
    in-memory bytes that passed authentication, never a second unverified read.
    """
    query_options(crate, page, anchor, missing_only, limit, offset)
    data, candidates = _build_snapshot(surface_path, provenance_path)
    downloaded = {item['path'] for item in data['downloaded_pages']}
    matched = sorted((candidate for candidate in candidates
                      if (crate is None or candidate['path'].startswith(crate + '/'))
                      and (page is None or candidate['path'] == page)
                      and (anchor is None or candidate['anchor'] == anchor)
                      and (not missing_only or candidate['path'] not in downloaded)),
                     key=lambda item: (item['path'], item['anchor']))
    records = [{key: copy.deepcopy(candidate[key])
                for key in ('path', 'anchor', 'url', 'index_pages', 'labels',
                            'stable_api_review', 'target_availability_review')}
               for candidate in matched[offset:offset + limit]]
    for record in records:
        record['index_pages'].sort()
        record['page_downloaded'] = record['path'] in downloaded
    result = {
        'schema_version': 1, 'inventory_kind': 'literal-documentation-candidate-query',
        'source_identity': copy.deepcopy(data['source_identity']),
        'scope': copy.deepcopy(data['scope']),
        'surface_artifact': copy.deepcopy(data['surface_artifact']),
        'selection': {'crate': crate, 'page': page, 'anchor': anchor,
                      'missing_only': missing_only, 'limit': limit, 'offset': offset},
        'total_candidate_records': len(candidates),
        'matched_candidate_records': len(matched), 'returned_candidate_records': len(records),
        'has_more': offset + len(records) < len(matched), 'records': records,
        'external_verification': 'retained-sidecar-bytes-reverified',
        'original_html_verification': 'not_reverified_in_this_run',
        'review_credit': 0, 'execution_credit': 0, 'completion_credit': 0,
        'api_denominator': None, 'full_rust_completion_percent': None,
        'interpretation': QUERY_INTERPRETATION,
    }
    need(len(encode(result)) <= MAX_QUERY_BYTES, 'query output exceeds byte limit')
    return result


@value_errors
def verify(data, surface_path: Path, provenance_path: Path) -> dict:
    """Authenticate retained sidecar bytes and recompute every observation."""
    validate_structure(data)
    expected = build(surface_path, provenance_path)
    need(data == expected, 'backlog differs from full retained-sidecar recomputation')
    return data


def encode(data):
    return (json.dumps(data, indent=2, sort_keys=True, ensure_ascii=True, allow_nan=False) + '\n').encode('ascii')


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='command', required=True)
    for command in ('generate', 'verify', 'query'):
        p = sub.add_parser(command)
        p.add_argument('--surface-index', type=Path, required=True)
        p.add_argument('--provenance', type=Path, required=True)
        if command == 'query':
            p.add_argument('--crate', choices=CRATES)
            p.add_argument('--page')
            p.add_argument('--anchor')
            p.add_argument('--missing-pages-only', action='store_true')
            p.add_argument('--limit', type=int, default=100)
            p.add_argument('--offset', type=int, default=0)
        else:
            p.add_argument('--output' if command == 'generate' else '--backlog', type=Path, required=True)
    args = parser.parse_args(argv)  # All option validation precedes any input/output IO.
    try:
        if args.command == 'query':
            result = query(args.surface_index, args.provenance, crate=args.crate, page=args.page,
                           anchor=args.anchor, missing_only=args.missing_pages_only,
                           limit=args.limit, offset=args.offset)
            sys.stdout.write(encode(result).decode('ascii'))
        elif args.command == 'generate':
            data = build(args.surface_index, args.provenance)
            with args.output.open('xb') as stream:
                stream.write(encode(data))
        else:
            _, data = read_json(args.backlog, MAX_BYTES - 1)
            verify(data, args.surface_index, args.provenance)
        return 0
    except (ValueError, KeyError, TypeError, OSError, UnicodeError, RecursionError) as error:
        print('library surface backlog: ' + str(error), file=sys.stderr)
        return 1


if __name__ == '__main__':
    sys.exit(main())
