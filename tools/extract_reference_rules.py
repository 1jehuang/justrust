#!/usr/bin/env python3
"""Extract unreviewed Reference rule metadata after exact archive verification.

No network, compiler invocation, source mutation, prose copying or semantic
coverage inference. The caller supplies an official source archive and its
expected SHA256. Matching that hash and every local file proves input consistency,
not independent authentication of the expected hash or of the compiler binary.
"""
import argparse
import hashlib
import json
import re
import tarfile
from pathlib import Path, PurePosixPath

RULE = re.compile(r'^r\[([^\]\s]+)\]\s*$')
HEADING = re.compile(r'^(#{1,6})\s+(.+?)\s*#*\s*$')
LINK = re.compile(r'\[[^\]]+\]\(([^)]+\.md)(?:#[^)]*)?\)')
EDITION = re.compile(r'^>\s*\[!EDITION-([0-9]{4})\]\s*$', re.IGNORECASE)
COMMIT = re.compile(r'^[0-9a-f]{40}$')
DIGEST = re.compile(r'^[0-9a-f]{64}$')
SCOPE = {'edition': '2024', 'target': 'x86_64-unknown-linux-gnu'}


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def parse_markdown(text):
    rules, headings, editions = [], [], []
    fence = None
    for line_number, line in enumerate(text.splitlines(), 1):
        marker = re.match(r'^(`{3,}|~{3,})', line.lstrip())
        if marker:
            token = marker.group(1)
            if fence is None:
                fence = token
            elif token[0] == fence[0] and len(token) >= len(fence):
                fence = None
            continue
        if fence is not None:
            continue
        match = RULE.fullmatch(line)
        if match:
            rules.append({'label': match.group(1), 'line': line_number})
        match = HEADING.match(line)
        if match:
            headings.append({'level': len(match.group(1)), 'title': match.group(2), 'line': line_number})
        match = EDITION.match(line)
        if match:
            editions.append({'edition': match.group(1), 'line': line_number})
    return rules, headings, editions


def verify_archive(source_root, archive_path, expected_sha256, reference_commit):
    if not DIGEST.fullmatch(expected_sha256):
        raise ValueError('expected archive SHA256 must be an exact lowercase digest')
    if source_root.is_symlink() or archive_path.is_symlink():
        raise ValueError('source root and archive must not be symlinks')
    archive_bytes = archive_path.read_bytes()
    if sha256(archive_bytes) != expected_sha256:
        raise ValueError('source archive SHA256 mismatch')
    root = source_root.resolve()
    expected_prefix = 'reference-' + reference_commit
    local = {}
    for path in sorted(source_root.rglob('*')):
        if path.is_symlink():
            raise ValueError('symlink in local source: ' + path.relative_to(source_root).as_posix())
        if not path.resolve().is_relative_to(root):
            raise ValueError('local source path escapes root')
        if path.is_file():
            local[path.relative_to(source_root).as_posix()] = sha256(path.read_bytes())
        elif not path.is_dir():
            raise ValueError('nonregular local source entry')
    archived = {}
    with tarfile.open(archive_path, 'r:*') as archive:
        for member in archive.getmembers():
            path = PurePosixPath(member.name)
            if path.is_absolute() or '..' in path.parts or not path.parts or path.parts[0] != expected_prefix:
                raise ValueError('unsafe or unexpected archive path: ' + member.name)
            if member.issym() or member.islnk():
                raise ValueError('symlink or hardlink in source archive')
            if member.isdir():
                continue
            if not member.isfile() or len(path.parts) < 2:
                raise ValueError('nonregular source archive entry')
            relative = PurePosixPath(*path.parts[1:]).as_posix()
            if relative in archived:
                raise ValueError('duplicate source archive member')
            handle = archive.extractfile(member)
            if handle is None:
                raise ValueError('unreadable archive member')
            with handle:
                archived[relative] = sha256(handle.read())
    if not archived:
        raise ValueError('source archive contains no regular files')
    if set(local) != set(archived):
        raise ValueError('local/archive file set mismatch: missing=' + repr(sorted(set(archived) - set(local))) + ' extra=' + repr(sorted(set(local) - set(archived))))
    mismatches = sorted(path for path in archived if local[path] != archived[path])
    if mismatches:
        raise ValueError('local source bytes differ from archive: ' + repr(mismatches))
    return len(archived)


def extract(source_root, pin_path, compiler_commit, inventory_path, archive_path, archive_sha256):
    if not COMMIT.fullmatch(compiler_commit):
        raise ValueError('compiler commit must be an immutable 40-character lowercase SHA')
    pin_bytes = pin_path.read_bytes()
    pin = json.loads(pin_bytes)
    reference_commit = pin.get('sha', '')
    if not COMMIT.fullmatch(reference_commit):
        raise ValueError('Reference submodule commit must be immutable')
    if pin.get('path') != 'src/doc/reference' or pin.get('type') != 'submodule':
        raise ValueError('pin must describe compiler src/doc/reference submodule')
    if pin.get('submodule_git_url') != 'https://github.com/rust-lang/reference.git':
        raise ValueError('pin is not the official Reference repository')
    expected_api = 'https://api.github.com/repos/rust-lang/rust/contents/src/doc/reference?ref=' + compiler_commit
    if pin.get('url') != expected_api:
        raise ValueError('pin API identity does not match compiler commit')
    expected_html = 'https://github.com/rust-lang/reference/tree/' + reference_commit
    if pin.get('html_url') != expected_html:
        raise ValueError('pin HTML identity does not match Reference commit')
    candidate_bytes = inventory_path.read_bytes()
    candidate = json.loads(candidate_bytes)
    if candidate.get('scope') != SCOPE:
        raise ValueError('invalid candidate scope: exact edition2024 and x86_64-unknown-linux-gnu required')
    verified_files = verify_archive(source_root, archive_path, archive_sha256, reference_commit)
    src = source_root / 'src'
    summary_bytes = (src / 'SUMMARY.md').read_bytes()
    paths = sorted(set(LINK.findall(summary_bytes.decode('utf-8'))))
    if not paths:
        raise ValueError('no Markdown chapter links in SUMMARY.md')
    candidate_pages = {}
    for section in candidate['sections']:
        path = section['source_refs'][0]['section'].removesuffix('.html') + '.md'
        if path in candidate_pages:
            raise ValueError('duplicate candidate page')
        candidate_pages[path] = section
    seen = set()

    def record(path, linked):
        target = src / path
        if not target.resolve().is_relative_to(src.resolve()):
            raise ValueError('chapter escapes source root: ' + path)
        data = target.read_bytes()
        rules, headings, editions = parse_markdown(data.decode('utf-8'))
        for rule in rules:
            if rule['label'] in seen:
                raise ValueError('duplicate Reference rule label: ' + rule['label'])
            seen.add(rule['label'])
        section = candidate_pages.get(path) if linked else None
        return {
            'path': 'src/' + path,
            'url': 'https://github.com/rust-lang/reference/blob/' + reference_commit + '/src/' + path,
            'sha256': sha256(data), 'headings': headings,
            'edition_note_markers': editions, 'rules': rules,
            'candidate_section': section['id'] if section else None,
            'candidate_obligations': [o['id'] for o in section['obligations']] if section else [],
            'rule_obligation_mapping_status': 'unreviewed',
        }

    pages = [record(path, True) for path in paths]
    all_md = sorted(p.relative_to(src).as_posix() for p in src.rglob('*.md'))
    linked = set(paths)
    unlinked = [record(path, False) for path in all_md if path not in linked]
    linked_rules = sum(len(p['rules']) for p in pages)
    unlinked_rules = sum(len(p['rules']) for p in unlinked)
    return {
        'schema_version': 1, 'review_status': 'unreviewed', 'scope': SCOPE,
        'provenance': {
            'compiler_commit': compiler_commit, 'reference_commit': reference_commit,
            'compiler_submodule_api_url': expected_api,
            'compiler_submodule_response_sha256': sha256(pin_bytes),
            'reference_repository_url': expected_html,
            'source_archive_url': 'https://codeload.github.com/rust-lang/reference/tar.gz/' + reference_commit,
            'source_archive_sha256': archive_sha256,
            'archive_verified_local_files': verified_files,
            'local_source_bytes_match_archive': True,
            'summary_sha256': sha256(summary_bytes),
            'candidate_inventory_sha256': sha256(candidate_bytes),
        },
        'counts': {
            'chapter_pages': len(paths), 'linked_page_rule_labels': linked_rules,
            'unlinked_page_rule_labels': unlinked_rules, 'unique_rule_labels_total': len(seen),
            'duplicate_rule_labels': 0, 'candidate_pages': len(candidate_pages),
            'chapter_pages_without_candidate_section': len(linked - set(candidate_pages)),
            'candidate_pages_absent_from_pinned_summary': len(set(candidate_pages) - linked),
            'rules_with_reviewed_obligation_mapping': 0,
            'rules_requiring_mapping_review': len(seen),
        },
        'pinned_pages_without_candidate_section': sorted(linked - set(candidate_pages)),
        'candidate_pages_absent_from_pinned_summary': sorted(set(candidate_pages) - linked),
        'unlinked_markdown_files': unlinked,
        'limitations': [
            'The expected archive digest is supplied by the caller. Matching it and every local regular file establishes exact input consistency, not independent authentication of the supplied digest, GitHub response or compiler binary.',
            'Rule labels and page links are extracted syntactically, not semantically reviewed. Candidate IDs adjacent to a page establish no individual rule-to-obligation coverage.',
            'All linked and unlinked rule mappings remain unreviewed. Zero reviewed mappings is not an implementation unsupported count.',
            'Unlabeled normative prose, tables, grammar, examples, notes and cross-links can impose additional obligations. Label enumeration is not semantic completeness.',
            'Edition markers are syntactic discovery evidence only. Stabilization histories and pinned compiler discrepancies still require semantic and executable review.',
            'Duplicate rule labels across all linked and unlinked Markdown files are rejected, not silently deduplicated.',
            'The frozen205-obligation candidate is unchanged and not silently reattributed to this pinned source. No executable or support evidence follows from this sidecar.',
        ],
        'pages': pages,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source-root', required=True, type=Path)
    parser.add_argument('--pin', required=True, type=Path)
    parser.add_argument('--compiler-commit', required=True)
    parser.add_argument('--inventory', required=True, type=Path)
    parser.add_argument('--source-archive', required=True, type=Path)
    parser.add_argument('--archive-sha256', required=True)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    data = extract(args.source_root, args.pin, args.compiler_commit, args.inventory, args.source_archive, args.archive_sha256)
    with args.output.open('x', encoding='utf-8') as output:
        output.write(json.dumps(data, indent=2, ensure_ascii=False) + '\n')
    print(json.dumps(data['counts'], sort_keys=True))


if __name__ == '__main__':
    main()
