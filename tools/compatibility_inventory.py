#!/usr/bin/env python3
"""Validate unreviewed obligation inventories, never execution or compatibility evidence.

Only the requirement IDs are read from the current scope. Scope execution-evidence
validation remains the responsibility of compatibility_scope.py and conformance.py.
"""
from __future__ import annotations

import argparse
from datetime import date
import hashlib
import io
import json
import re
from pathlib import Path, PurePosixPath
import sys
import tarfile
from urllib.parse import urlsplit

CONTRACT = {"edition": "2024", "target": "x86_64-unknown-linux-gnu"}
OFFICIAL_HOSTS = frozenset({"rust-lang.org", "www.rust-lang.org", "doc.rust-lang.org",
                            "blog.rust-lang.org", "rust-lang.github.io"})


class InvalidInventory(ValueError):
    """Malformed or unsupported inventory input."""


def require(condition, message):
    if not condition:
        raise InvalidInventory(message)


def fields(value, names, where):
    require(isinstance(value, dict) and set(value) == set(names),
            f"{where}: expected exactly {', '.join(names)}")


def text(value, where):
    require(isinstance(value, str) and bool(value.strip()), f"{where}: nonempty string required")
    return value


def rows(value, where, nonempty=True):
    require(isinstance(value, list) and (bool(value) or not nonempty),
            f"{where}: {'nonempty ' if nonempty else ''}array required")
    return value


def strings(value, where, nonempty=False):
    result = [text(item, where) for item in rows(value, where, nonempty)]
    require(len(result) == len(set(result)), f"{where}: duplicate value")
    return result


def identity(row, seen, where):
    ident = text(row.get("id"), f"{where}.id")
    require(re.fullmatch(r"[a-z][a-z0-9]*(?:-[a-z0-9]+)*", ident) is not None,
            f"{where}: canonical kebab-case id required")
    require(ident not in seen, f"{where}: duplicate id {ident}")
    seen.add(ident)
    return ident


def official_url(value):
    text(value, "source.url")
    require(not any(c.isspace() or ord(c) < 32 for c in value) and '\\' not in value,
            "source.url: whitespace, controls and backslashes forbidden")
    try:
        parsed = urlsplit(value)
        port = parsed.port
    except ValueError as exc:
        raise InvalidInventory("source.url: malformed URL") from exc
    host_ok = parsed.hostname in OFFICIAL_HOSTS
    # rust-lang's official repositories are primary sources too, not arbitrary GitHub users.
    if parsed.hostname == "github.com":
        host_ok = parsed.path.startswith("/rust-lang/") and len(parsed.path.split("/")) >= 3
    require(parsed.scheme == "https" and host_ok and parsed.username is None
            and parsed.password is None and port is None,
            "source.url: HTTPS official Rust primary source required")


def scope_ids(scope):
    require(isinstance(scope, dict) and type(scope.get("schema_version")) is int
            and scope["schema_version"] == 1, "scope: schema_version must be 1")
    contract = scope.get("scope")
    require(isinstance(contract, dict) and all(contract.get(k) == v for k, v in CONTRACT.items()),
            "scope: edition/target mismatch")
    seen, areas = set(), set()
    for area in rows(scope.get("areas"), "scope.areas"):
        require(isinstance(area, dict), "scope.area: object required")
        identity(area, areas, "scope.area")
        for requirement in rows(area.get("requirements"), "scope.requirements"):
            require(isinstance(requirement, dict), "scope.requirement: object required")
            identity(requirement, seen, "scope.requirement")
    return seen


def validate(inventory, existing, obligation_ids=None):
    """Validate one inventory. Share obligation_ids across files for global uniqueness."""
    fields(inventory, ("schema_version", "scope", "review_status", "limitations", "sources", "sections"), "inventory")
    require(type(inventory["schema_version"]) is int and inventory["schema_version"] == 1,
            "inventory: schema_version must be 1")
    require(inventory["scope"] == CONTRACT, "inventory: edition/target mismatch")
    require(inventory["review_status"] == "unreviewed", "inventory: review_status must be unreviewed")
    strings(inventory["limitations"], "limitations", True)
    sources = set()
    for source in rows(inventory["sources"], "sources"):
        fields(source, ("id", "url", "version_note"), "source")
        identity(source, sources, "source")
        official_url(source["url"])
        text(source["version_note"], "source.version_note")
    sections = set()
    obligations = obligation_ids if obligation_ids is not None else set()
    mapped = set()
    count = needs = 0
    for section in rows(inventory["sections"], "sections"):
        fields(section, ("id", "title", "source_refs", "obligations"), "section")
        identity(section, sections, "section")
        text(section["title"], "section.title")
        refs = set()
        for ref in rows(section["source_refs"], "source_refs"):
            fields(ref, ("source", "section"), "source_ref")
            source = text(ref["source"], "source_ref.source")
            anchor = text(ref["section"], "source_ref.section")
            require(source in sources, f"source_ref: unknown source {source}")
            require((source, anchor) not in refs, "source_ref: duplicate reference")
            refs.add((source, anchor))
        for obligation in rows(section["obligations"], "obligations"):
            fields(obligation, ("id", "title", "existing_requirements", "test_needs"), "obligation")
            identity(obligation, obligations, "obligation")
            text(obligation["title"], "obligation.title")
            mappings = strings(obligation["existing_requirements"], "existing_requirements")
            require(set(mappings) <= existing, f"existing_requirements: unknown IDs {sorted(set(mappings) - existing)}")
            mapped.update(mappings)
            needs += len(strings(obligation["test_needs"], "test_needs"))
            count += 1
    return {"sources": len(sources), "sections": len(sections), "obligations": count,
            "test_needs": needs}, mapped


def _object(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result, f"JSON: duplicate key {key}")
        result[key] = value
    return result


def load(path):
    try:
        return json.loads(Path(path).read_text(encoding="utf-8"), object_pairs_hook=_object,
                          parse_constant=lambda token: require(False, f"JSON: invalid constant {token}"))
    except (OSError, UnicodeError, ValueError, RecursionError) as exc:
        raise InvalidInventory(f"{path}: {exc}") from exc


def summarize(inventories, scope):
    existing = scope_ids(scope)
    totals = {"inventories": 0, "sources": 0, "sections": 0, "obligations": 0, "test_needs": 0}
    seen, mapped = set(), set()
    for inventory in inventories:
        counts, mappings = validate(inventory, existing, seen)
        totals["inventories"] += 1
        for key, value in counts.items():
            totals[key] += value
        mapped.update(mappings)
    require(totals["inventories"] > 0, "at least one inventory required")
    return {"schema_version": 1, "scope": CONTRACT, "review_status": "unreviewed",
            "full_rust_completion_percent": None,
            "counts": dict(totals, existing_requirements=len(existing), mapped_existing_requirements=len(mapped),
                           obligations_lacking_direct_exact_test_links=totals["obligations"],
                           obligations_with_executable_evidence=0),
            "unmapped_existing_requirements": sorted(existing - mapped),
            "limitations": ["Unreviewed inventories are planning obligations, not execution evidence.",
                            "Existing requirement links do not cover new obligations or supply direct exact test links.",
                            "Counts and mappings are not a full Rust compatibility percentage."]}


def validate_contract(contract, verifier_text):
    fields(contract, ("schema_version", "contract_id", "edition", "target", "oracle", "dimensions",
                      "evidence_policy", "exclusions", "readiness", "source_provenance_policy"), "contract")
    require(type(contract["schema_version"]) is int and contract["schema_version"] == 1,
            "contract: schema_version must be 1")
    require(all(contract[k] == v for k, v in CONTRACT.items()), "contract: edition/target mismatch")
    oracle = contract["oracle"]
    fields(oracle, ("release", "commit", "verification", "upgrade_policy"), "contract.oracle")
    require(oracle["verification"] == "tests/support/stock_rustc.rs", "contract: unexpected oracle verifier")
    constants = {}
    for name, value in re.findall(r'^const (RELEASE|COMMIT|HOST|LLVM): &str = "([^"\n]+)";', verifier_text, re.M):
        require(name not in constants, f"oracle verifier: duplicate {name}")
        constants[name] = value
    require(set(constants) == {"RELEASE", "COMMIT", "HOST", "LLVM"}, "oracle verifier: missing exact stable constants")
    release = text(oracle["release"], "oracle.release")
    commit = text(oracle["commit"], "oracle.commit")
    require(re.fullmatch(r"\d+\.\d+\.\d+", release) is not None
            and re.fullmatch(r"[0-9a-f]{40}", commit) is not None, "contract: malformed stable oracle")
    require(constants["RELEASE"] == "release: " + release
            and constants["COMMIT"] == "commit-hash: " + commit
            and constants["HOST"] == "host: " + CONTRACT["target"], "contract: oracle constants mismatch")
    require(contract["contract_id"] == "rust-2024-linux-x86-64-stock-" + release.replace(".", "-"),
            "contract: oracle requires matching contract identity")
    text(oracle["upgrade_policy"], "oracle.upgrade_policy")
    expected = {"language", "libraries", "projects", "execution", "ecosystem"}
    seen = set()
    for dimension in rows(contract["dimensions"], "dimensions"):
        fields(dimension, ("id", "includes"), "dimension")
        identity(dimension, seen, "dimension")
        text(dimension["includes"], "dimension.includes")
    require(seen == expected, "contract: missing or unknown compatibility dimensions")
    fields(contract["evidence_policy"], ("positive", "negative", "undefined_behavior", "unspecified_behavior",
                                       "provenance", "graph", "pending"), "evidence_policy")
    for value in contract["evidence_policy"].values():
        text(value, "evidence_policy")
    seen = set()
    for exclusion in rows(contract["exclusions"], "exclusions"):
        fields(exclusion, ("id", "reason"), "exclusion")
        identity(exclusion, seen, "exclusion")
        text(exclusion["reason"], "exclusion.reason")
    require(seen == {"nightly-extensions", "other-targets", "rustc-internals", "performance-parity"},
            "contract: missing or unknown exclusions")
    fields(contract["readiness"], ("inventory_review", "full_rust_completion_percent", "rule"), "readiness")
    require(contract["readiness"]["inventory_review"] == "unreviewed"
            and contract["readiness"]["full_rust_completion_percent"] is None,
            "contract: unreviewed inventory cannot assert full Rust completion")
    text(contract["readiness"]["rule"], "readiness.rule")
    text(contract["source_provenance_policy"], "source_provenance_policy")
    return contract


def read_verifier(path):
    try:
        return Path(path).read_text(encoding="utf-8")
    except (OSError, UnicodeError) as exc:
        raise InvalidInventory(f"{path}: {exc}") from exc


REFERENCE_RULES = "data/compiler-language-reference-rules.json"
_ABSENT = object()


def file_bytes(path):
    try:
        return Path(path).read_bytes()
    except OSError as exc:
        raise InvalidInventory(f"{path}: {exc}") from exc


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def exact_hex(value, length, where):
    require(isinstance(value, str) and re.fullmatch(r"[0-9a-f]{" + str(length) + "}", value) is not None,
            f"{where}: exact lowercase hexadecimal identity required")
    return value


def positive_integer(value, where):
    require(type(value) is int and value > 0, f"{where}: positive integer required")


def reference_path(value):
    text(value, "reference.path")
    path = PurePosixPath(value)
    require(not path.is_absolute() and path.as_posix() == value and '..' not in path.parts
            and len(path.parts) >= 2 and path.parts[0] == "src" and path.suffix == ".md"
            and re.fullmatch(r"[A-Za-z0-9_./-]+", value) is not None,
            "reference.path: canonical source Markdown path required")
    return value


def validate_reference_rules(root: Path, data=_ABSENT, *, candidate_path=None, contract=None,
                             source_archive=None, submodule_proof=None):
    """Validate static rule identities, not semantic coverage or authenticated receipts.

    Archive and submodule bytes are optional external reproduction inputs. Ordinary
    offline validation does not pretend to reauthenticate source bytes or GitHub.
    """
    root = Path(root)
    data = load(root / REFERENCE_RULES) if data is _ABSENT else data
    candidate_path = Path(candidate_path) if candidate_path else root / "data/compiler-language-inventory.json"
    candidate = load(candidate_path)
    contract = contract if contract is not None else load(root / "data/compiler-compatibility-contract.json")
    validate_contract(contract, read_verifier(root / "tests/support/stock_rustc.rs"))
    validate(candidate, scope_ids(load(root / "data/compiler-compatibility-scope.json")))
    return validate_reference_metadata(data, candidate, candidate_path, contract["oracle"]["commit"],
                                       source_archive=source_archive, submodule_proof=submodule_proof)


def validate_reference_metadata(data, candidate, candidate_path, compiler_commit, *,
                                source_archive=None, submodule_proof=None):
    """Validate pinned metadata only. The caller owns candidate/scope validation."""
    fields(data, ("schema_version", "review_status", "scope", "provenance", "counts",
                  "pinned_pages_without_candidate_section", "candidate_pages_absent_from_pinned_summary",
                  "unlinked_markdown_files", "limitations", "pages"), "reference_rules")
    require(type(data["schema_version"]) is int and data["schema_version"] == 1,
            "reference_rules: schema_version must be 1")
    require(data["scope"] == CONTRACT and data["review_status"] == "unreviewed",
            "reference_rules: scope mismatch or non-unreviewed status")
    strings(data["limitations"], "reference_rules.limitations", True)
    provenance = data["provenance"]
    fields(provenance, ("compiler_commit", "reference_commit", "compiler_submodule_api_url",
                        "compiler_submodule_response_sha256", "reference_repository_url", "source_archive_url",
                        "source_archive_sha256", "archive_verified_local_files", "local_source_bytes_match_archive",
                        "summary_sha256", "candidate_inventory_sha256"), "reference_rules.provenance")
    compiler = exact_hex(provenance["compiler_commit"], 40, "compiler_commit")
    reference = exact_hex(provenance["reference_commit"], 40, "reference_commit")
    require(compiler == compiler_commit, "reference_rules: compiler pin mismatch")
    for name in ("compiler_submodule_response_sha256", "source_archive_sha256", "summary_sha256", "candidate_inventory_sha256"):
        exact_hex(provenance[name], 64, name)
    if candidate_path is not None:
        require(provenance["candidate_inventory_sha256"] == sha256(file_bytes(candidate_path)),
                "reference_rules: candidate inventory bytes changed")
    api = "https://api.github.com/repos/rust-lang/rust/contents/src/doc/reference?ref=" + compiler
    repository = "https://github.com/rust-lang/reference/tree/" + reference
    require(provenance["compiler_submodule_api_url"] == api
            and provenance["reference_repository_url"] == repository
            and provenance["source_archive_url"] == "https://codeload.github.com/rust-lang/reference/tar.gz/" + reference,
            "reference_rules: immutable official provenance URLs required")
    positive_integer(provenance["archive_verified_local_files"], "archive_verified_local_files")
    require(provenance["local_source_bytes_match_archive"] is True,
            "reference_rules: extraction must record archive consistency, not a pass status")
    # The caller validates the complete candidate schema against the current scope.
    # Independently require unambiguous page identities for the adjacency relation.
    candidate_pages = {}
    for section in rows(candidate.get("sections"), "candidate.sections") if candidate is not None else ():
        require(isinstance(section, dict), "candidate.section: object required")
        refs = rows(section.get("source_refs"), "candidate.source_refs")
        first = refs[0]
        require(isinstance(first, dict), "candidate.source_ref: object required")
        page = text(first.get("section"), "candidate.source_ref.section")
        require(page.endswith(".html"), "candidate source section must be a page .html identity")
        path = reference_path("src/" + page.removesuffix(".html") + ".md")
        require(path not in candidate_pages, "candidate: ambiguous page adjacency")
        candidate_pages[path] = section
    paths, labels = set(), set()
    groups = []
    all_pages = []
    page_names = ("path", "url", "sha256", "headings", "edition_note_markers", "rules",
                  "candidate_section", "candidate_obligations", "rule_obligation_mapping_status")
    for group in ("pages", "unlinked_markdown_files"):
        group_pages = rows(data[group], "reference_rules." + group, group == "pages")
        group_paths = set()
        count = 0
        for page in group_pages:
            fields(page, page_names, "reference.page")
            path = reference_path(page["path"])
            require(path not in paths, "reference_rules: duplicate page path")
            paths.add(path)
            group_paths.add(path)
            require(page["url"] == "https://github.com/rust-lang/reference/blob/" + reference + "/" + path,
                    "reference.page: immutable official URL mismatch")
            exact_hex(page["sha256"], 64, "reference.page.sha256")
            require(page["rule_obligation_mapping_status"] == "unreviewed", "reference.page: mappings must remain unreviewed")
            expected = candidate_pages.get(path) if group == "pages" else None
            if candidate is not None:
                require(page["candidate_section"] == (expected["id"] if expected else None)
                        and page["candidate_obligations"] == ([item["id"] for item in expected["obligations"]] if expected else []),
                        "reference.page: candidate adjacency mismatch, not semantic mapping")
            else:
                require(page["candidate_section"] is None or isinstance(page["candidate_section"], str),
                        "reference.page: invalid candidate adjacency")
                strings(page["candidate_obligations"], "reference.page.candidate_obligations")
            for name, keys in (("headings", ("level", "title", "line")),
                               ("edition_note_markers", ("edition", "line")), ("rules", ("label", "line"))):
                prior = 0
                for item in rows(page[name], "reference.page." + name, False):
                    fields(item, keys, "reference.page." + name)
                    positive_integer(item["line"], "reference.line")
                    require(item["line"] > prior, "reference.page: line identities must be strictly ordered")
                    prior = item["line"]
                    if name == "headings":
                        require(type(item["level"]) is int and 1 <= item["level"] <= 6, "reference.heading: invalid level")
                        text(item["title"], "reference.heading.title")
                    elif name == "edition_note_markers":
                        require(isinstance(item["edition"], str) and re.fullmatch(r"[0-9]{4}", item["edition"]),
                                "reference.edition marker: four digits required")
                    else:
                        label = text(item["label"], "reference.rule.label")
                        require(re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]*", label) is not None
                                and label not in labels, "reference.rule: duplicate or malformed label")
                        labels.add(label)
                        count += 1
            all_pages.append(page)
        groups.append((group_paths, count))
    linked, linked_count = groups[0]
    unlinked, unlinked_count = groups[1]
    summary = next((page for page in all_pages if page["path"] == "src/SUMMARY.md"), None)
    require(summary is not None and "src/SUMMARY.md" in unlinked
            and summary["sha256"] == provenance["summary_sha256"], "reference_rules: summary hash missing or inconsistent")
    absent = sorted(set(candidate_pages) - linked)
    unmatched = sorted(linked - set(candidate_pages))
    if candidate is not None:
        require(data["pinned_pages_without_candidate_section"] == [path.removeprefix("src/") for path in unmatched]
                and data["candidate_pages_absent_from_pinned_summary"] == [path.removeprefix("src/") for path in absent],
                "reference_rules: missing-page accounting mismatch")
    else:
        strings(data["pinned_pages_without_candidate_section"], "reference_rules.unmatched")
        strings(data["candidate_pages_absent_from_pinned_summary"], "reference_rules.absent")
    computed = {"chapter_pages": len(linked), "linked_page_rule_labels": linked_count,
                "unlinked_page_rule_labels": unlinked_count, "unique_rule_labels_total": len(labels),
                "duplicate_rule_labels": 0, "candidate_pages": len(candidate_pages),
                "chapter_pages_without_candidate_section": len(unmatched),
                "candidate_pages_absent_from_pinned_summary": len(absent),
                "rules_with_reviewed_obligation_mapping": 0, "rules_requiring_mapping_review": len(labels)}
    if candidate is None:
        for name in ("candidate_pages", "chapter_pages_without_candidate_section", "candidate_pages_absent_from_pinned_summary"):
            value = data["counts"].get(name)
            require(type(value) is int and value >= 0, "reference_rules: invalid candidate count")
            computed[name] = value
    require(isinstance(data["counts"], dict) and set(data["counts"]) == set(computed)
            and all(type(data["counts"][key]) is int and data["counts"][key] == value for key, value in computed.items()),
            "reference_rules: declared counts disagree with exact page/rule inventory")
    archive_state = "not_reverified_in_this_run"
    if source_archive is not None:
        verify_reference_archive(source_archive, provenance, all_pages, linked)
        archive_state = "reproduced-from-provided-archive"
    pin_state = "not_reverified_in_this_run"
    if submodule_proof is not None:
        require(sha256(file_bytes(submodule_proof)) == provenance["compiler_submodule_response_sha256"],
                "reference_rules: submodule response bytes mismatch")
        pin = load(submodule_proof)
        require(isinstance(pin, dict) and pin.get("sha") == reference and pin.get("path") == "src/doc/reference"
                and pin.get("type") == "submodule" and pin.get("url") == api and pin.get("html_url") == repository
                and pin.get("submodule_git_url") == "https://github.com/rust-lang/reference.git",
                "reference_rules: compiler submodule identity mismatch")
        pin_state = "reproduced-from-provided-response"
    return {"readiness": "review-required", "review_status": "unreviewed", "counts": computed,
            "full_rust_completion_percent": None, "source_archive_byte_verification": archive_state,
            "compiler_submodule_byte_verification": pin_state,
            "limitations": ["Page adjacency is not a reviewed rule mapping or executable evidence.",
                            "Offline metadata validation does not authenticate GitHub, compiler binaries, or signed receipts.",
                            "Recorded extraction byte consistency is not independently reproduced without external source inputs."]}


def verify_reference_archive(path, provenance, pages, linked):
    """Read, never extract, optional external archive bytes. No network or shared writes."""
    try:
        from extract_reference_rules import LINK, parse_markdown
    except ImportError as exc:
        raise InvalidInventory("archive reproduction requires tools/extract_reference_rules.py") from exc
    path = Path(path)
    require(not path.is_symlink(), "reference source archive must not be a symlink")
    archive_bytes = file_bytes(path)
    require(sha256(archive_bytes) == provenance["source_archive_sha256"], "reference source archive SHA256 mismatch")
    prefix = "reference-" + provenance["reference_commit"]
    files, markdown = set(), {}
    consumed = 0
    try:
        with tarfile.open(fileobj=io.BytesIO(archive_bytes), mode="r:*") as archive:
            for member in archive:
                name = PurePosixPath(member.name)
                require(not name.is_absolute() and '..' not in name.parts and name.parts
                        and name.parts[0] == prefix, "reference source archive: unsafe member path")
                require(not member.issym() and not member.islnk(), "reference source archive: links forbidden")
                if member.isdir():
                    continue
                require(member.isfile() and len(name.parts) >= 2, "reference source archive: nonregular member")
                relative = PurePosixPath(*name.parts[1:]).as_posix()
                require(relative not in files, "reference source archive: duplicate member")
                files.add(relative)
                require(member.size >= 0, "reference source archive: negative file size")
                consumed += member.size
                require(consumed <= 64 * 1024 * 1024 and len(files) <= 10000,
                        "reference source archive: validation resource limit")
                if relative.startswith("src/") and relative.endswith(".md"):
                    handle = archive.extractfile(member)
                    require(handle is not None, "reference source archive: unreadable member")
                    with handle:
                        markdown[relative] = handle.read()
    except (OSError, tarfile.TarError, EOFError) as exc:
        raise InvalidInventory(f"reference source archive: {exc}") from exc
    require(len(files) == provenance["archive_verified_local_files"], "reference source archive: file count mismatch")
    require(set(markdown) == {page["path"] for page in pages}, "reference source archive: Markdown page set mismatch")
    for page in pages:
        raw = markdown[page["path"]]
        require(sha256(raw) == page["sha256"], "reference source archive: page hash mismatch")
        try:
            rules, headings, editions = parse_markdown(raw.decode("utf-8"))
        except UnicodeError as exc:
            raise InvalidInventory("reference source archive: invalid source encoding") from exc
        require(page["rules"] == rules and page["headings"] == headings and page["edition_note_markers"] == editions,
                "reference source archive: extracted metadata differs from index")
    summary = markdown["src/SUMMARY.md"]
    require(sha256(summary) == provenance["summary_sha256"], "reference source archive: summary hash mismatch")
    require({"src/" + path for path in LINK.findall(summary.decode("utf-8"))} == linked,
            "reference source archive: linked page set differs from summary")


def reference_readiness(root, *, data=_ABSENT, candidate_path=None, contract=None,
                        source_archive=None, submodule_proof=None):
    if data is _ABSENT and not (Path(root) / REFERENCE_RULES).is_file():
        return {"readiness": "missing", "review_status": "unreviewed", "missing_files": [REFERENCE_RULES],
                "full_rust_completion_percent": None,
                "limitations": ["Pinned reference rule inventory is missing, so inventory readiness remains blocked."]}
    return validate_reference_rules(root, data, candidate_path=candidate_path, contract=contract,
                                    source_archive=source_archive, submodule_proof=submodule_proof)


LIBRARY_PROVENANCE = "data/compiler-library-surface-provenance.json"
LIBRARY_TOOLS = ("acquire_pinned_library_sources.py", "acquire_pinned_api_html.py",
                 "compact_library_surface_provenance.py")
LIBRARY_FIELDS = ("schema_version", "review_status", "scope", "inventory_kind", "source_identity",
                  "artifacts", "observations", "pages", "reproduction", "limitations", "review_needs")


def validate_library_surface_provenance(root: Path, data=_ABSENT, *, contract=None):
    """Validate compact acquisition metadata, never a library API denominator.

    Recorded HTML/source sidecars remain external inputs. Only local reproduction
    tool bytes and metadata consistency are rechecked here, not remote content.
    """
    root = Path(root)
    data = load(root / LIBRARY_PROVENANCE) if data is _ABSENT else data
    fields(data, LIBRARY_FIELDS, "library_provenance")
    require(type(data["schema_version"]) is int and data["schema_version"] == 1,
            "library_provenance: schema_version must be 1")
    require(data["review_status"] == "unreviewed" and data["scope"] == CONTRACT,
            "library_provenance: unreviewed scope mismatch")
    require(data["inventory_kind"] == "pinned-library-source-and-documentation-input-provenance",
            "library_provenance: input provenance is not API coverage")
    strings(data["limitations"], "library_provenance.limitations", True)
    strings(data["review_needs"], "library_provenance.review_needs", True)
    contract = contract if contract is not None else load(root / "data/compiler-compatibility-contract.json")
    validate_contract(contract, read_verifier(root / "tests/support/stock_rustc.rs"))
    identity = data["source_identity"]
    fields(identity, ("commit", "release", "repository", "official_documentation_root", "root_tree_sha1",
                      "library_tree_sha1", "source_index_content_sha256", "verification"), "library_provenance.source_identity")
    require(identity["commit"] == contract["oracle"]["commit"] and identity["release"] == contract["oracle"]["release"],
            "library_provenance: stable oracle release/commit mismatch")
    base = "https://doc.rust-lang.org/" + identity["release"] + "/"
    require(identity["repository"] == "https://github.com/rust-lang/rust"
            and identity["official_documentation_root"] == base,
            "library_provenance: official pinned repository and documentation URLs required")
    for name in ("root_tree_sha1", "library_tree_sha1"):
        exact_hex(identity[name], 40, "library_provenance." + name)
    exact_hex(identity["source_index_content_sha256"], 64, "source_index_content_sha256")
    text(identity["verification"], "source_identity.verification")
    artifact_names = {"pinned-html-link-anchor-candidate": "pinned-html-api-index.json",
                      "pinned-library-source-path-index": "library-source-index.json"}
    artifacts, filenames = {}, set()
    for artifact in rows(data["artifacts"], "library_provenance.artifacts"):
        fields(artifact, ("id", "filename", "content_sha256", "byte_length", "role"), "library_provenance.artifact")
        ident = text(artifact["id"], "artifact.id")
        require(ident in artifact_names and ident not in artifacts, "library_provenance: duplicate or unknown artifact ID")
        require(artifact["filename"] == artifact_names[ident] and artifact["filename"] not in filenames,
                "library_provenance: unexpected or duplicate artifact filename")
        filenames.add(artifact["filename"])
        exact_hex(artifact["content_sha256"], 64, "artifact.content_sha256")
        positive_integer(artifact["byte_length"], "artifact.byte_length")
        text(artifact["role"], "artifact.role")
        artifacts[ident] = artifact
    require(set(artifacts) == set(artifact_names), "library_provenance: exactly two external artifacts required")
    require(artifacts["pinned-library-source-path-index"]["content_sha256"] == identity["source_index_content_sha256"],
            "library_provenance: source artifact hash disagrees with source identity")
    pages, versions = set(), set()
    for page in rows(data["pages"], "library_provenance.pages"):
        fields(page, ("path", "url", "rustdoc_version", "content_sha256", "byte_length"), "library_provenance.page")
        path = text(page["path"], "library_provenance.page.path")
        parts = PurePosixPath(path)
        require(not parts.is_absolute() and parts.as_posix() == path and '..' not in parts.parts
                and len(parts.parts) >= 2 and parts.parts[0] in {"core", "alloc", "std", "proc_macro"}
                and re.fullmatch(r"[A-Za-z0-9_./-]+\.html", path) is not None,
                "library_provenance.page: canonical crate HTML path required")
        require(path not in pages, "library_provenance: duplicate page path")
        pages.add(path)
        require(page["url"] == base + path, "library_provenance.page: official versioned URL mismatch")
        exact_hex(page["content_sha256"], 64, "page.content_sha256")
        positive_integer(page["byte_length"], "page.byte_length")
        marker = text(page["rustdoc_version"], "page.rustdoc_version")
        prefix = identity["release"] + " (" + identity["commit"][:9] + " "
        require(marker.startswith(prefix) and re.fullmatch(re.escape(prefix) + r"[0-9]{4}-[0-9]{2}-[0-9]{2}\)", marker),
                "library_provenance.page: rustdoc release/commit marker mismatch")
        try:
            date.fromisoformat(marker[len(prefix):-1])
        except ValueError as exc:
            raise InvalidInventory("library_provenance.page: malformed rustdoc date") from exc
        versions.add(marker)
    require(len(versions) == 1, "library_provenance: inconsistent rustdoc version markers")
    required = {crate + "/" + name for crate in ("core", "alloc", "std", "proc_macro") for name in ("index.html", "all.html")}
    require(required <= pages, "library_provenance: eight crate index/all pages required")
    observations = data["observations"]
    fields(observations, ("downloaded_page_records", "literal_index_link_anchor_candidate_records",
                          "missing_detail_page_records", "interpretation"), "library_provenance.observations")
    for name in ("downloaded_page_records", "literal_index_link_anchor_candidate_records", "missing_detail_page_records"):
        require(type(observations[name]) is int and observations[name] >= 0,
                "library_provenance: nonnegative observed record counts required")
    require(observations["downloaded_page_records"] == len(pages), "library_provenance: downloaded page count mismatch")
    require(observations["missing_detail_page_records"] <= observations["literal_index_link_anchor_candidate_records"],
            "library_provenance: missing detail records exceed observed candidate records")
    text(observations["interpretation"], "observations.interpretation")
    reproduction = data["reproduction"]
    fields(reproduction, ("tools", "working_directory", "commands", "requirements"), "library_provenance.reproduction")
    text(reproduction["working_directory"], "reproduction.working_directory")
    strings(reproduction["commands"], "reproduction.commands", True)
    strings(reproduction["requirements"], "reproduction.requirements", True)
    tools = set()
    for tool in rows(reproduction["tools"], "reproduction.tools"):
        fields(tool, ("filename", "content_sha256"), "reproduction.tool")
        filename = text(tool["filename"], "reproduction.tool.filename")
        require(filename in LIBRARY_TOOLS and filename not in tools, "library_provenance: duplicate or unexpected reproduction tool")
        tools.add(filename)
        exact_hex(tool["content_sha256"], 64, "reproduction.tool.content_sha256")
        require(tool["content_sha256"] == sha256(file_bytes(root / "tools" / filename)),
                "library_provenance: reproduction tool bytes changed: " + filename)
    require(tools == set(LIBRARY_TOOLS), "library_provenance: exactly three reproduction tools required")
    return {"readiness": "review-required", "review_status": "unreviewed",
            "inventory_kind": data["inventory_kind"], "full_rust_completion_percent": None,
            "api_denominator": None, "observations": dict(observations), "external_artifacts": len(artifacts),
            "required_index_pages": len(required), "locally_bound_reproduction_tools": len(tools),
            "external_source_byte_verification": "not_reverified_in_this_run",
            "limitations": ["Input provenance only, not a stable public target API denominator or executable evidence.",
                            "Literal links, anchors, downloaded pages and missing records are not API or support counts.",
                            "External artifact hashes and recorded page metadata are not source-byte authentication."]}


LIBRARY_BACKLOG = "data/compiler-library-documentation-backlog.json"


def library_documentation_backlog_readiness(root, *, provenance=_ABSENT):
    """Report recorded documentation gaps, never an authenticated API denominator."""
    root = Path(root)
    if not (root / LIBRARY_BACKLOG).is_file():
        return {"readiness": "missing", "review_status": "unreviewed",
                "missing_files": [LIBRARY_BACKLOG], "api_denominator": None,
                "full_rust_completion_percent": None,
                "limitations": ["Missing literal documentation backlog is not evidence of an enumerated public API surface."]}
    from library_surface_backlog import validate_metadata, validate_structure
    data = load(root / LIBRARY_BACKLOG)
    try:
        validate_structure(data)
        if provenance is _ABSENT:
            return {"readiness": "blocked-by-missing-inputs", "review_status": "unreviewed",
                    "missing_files": [LIBRARY_PROVENANCE], "api_denominator": None,
                    "full_rust_completion_percent": None,
                    "limitations": ["Recorded documentation backlog requires its acquisition provenance."]}
        return validate_metadata(data, provenance)
    except ValueError as exc:
        raise InvalidInventory(str(exc)) from exc


def library_surface_readiness(root, *, data=_ABSENT, contract=None):
    if data is _ABSENT and not (Path(root) / LIBRARY_PROVENANCE).is_file():
        return {"readiness": "missing", "review_status": "unreviewed", "missing_files": [LIBRARY_PROVENANCE],
                "full_rust_completion_percent": None, "api_denominator": None,
                "documentation_backlog": library_documentation_backlog_readiness(root),
                "limitations": ["Library acquisition input provenance is missing and does not establish API readiness."]}
    provenance = load(Path(root) / LIBRARY_PROVENANCE) if data is _ABSENT else data
    result = validate_library_surface_provenance(root, provenance, contract=contract)
    result["documentation_backlog"] = library_documentation_backlog_readiness(root, provenance=provenance)
    return result


def inventory_readiness_blockers(report):
    result = [path for name, path in (("reference_rules", REFERENCE_RULES),
                                      ("library_surface_inputs", LIBRARY_PROVENANCE))
              if report[name]["readiness"] == "missing"]
    backlog = report["library_surface_inputs"]["documentation_backlog"]
    if backlog["readiness"] == "missing":
        result.append(LIBRARY_BACKLOG)
    return result


def load_and_summarize(root: Path):
    """Return honest missing readiness for partial fixtures, reject malformed present inputs."""
    root = Path(root)
    proposals = rule_proposal_readiness(root)
    witness_sources = partial_witness_source_readiness(root)
    library_witnesses = library_witness_readiness(root)
    library_data = load(root / LIBRARY_PROVENANCE) if (root / LIBRARY_PROVENANCE).is_file() else _ABSENT
    if library_data is not _ABSENT:
        fields(library_data, LIBRARY_FIELDS, "library_provenance")
    reference_data = load(root / REFERENCE_RULES) if (root / REFERENCE_RULES).is_file() else _ABSENT
    if reference_data is not _ABSENT:
        fields(reference_data, ("schema_version", "review_status", "scope", "provenance", "counts",
                               "pinned_pages_without_candidate_section", "candidate_pages_absent_from_pinned_summary",
                               "unlinked_markdown_files", "limitations", "pages"), "reference_rules")
    names = ("data/compiler-compatibility-contract.json", "data/compiler-compatibility-scope.json",
             "data/compiler-language-inventory.json", "data/compiler-library-project-inventory.json",
             "tests/support/stock_rustc.rs")
    missing = [name for name in names if not (root / name).is_file()]
    # Validate every present JSON even when the fixture is incomplete.
    present = {name: load(root / name) for name in names[:-1] if name not in missing}
    contract = present.get(names[0])
    if names[0] in present:
        fields(contract, ("schema_version", "contract_id", "edition", "target", "oracle", "dimensions",
                          "evidence_policy", "exclusions", "readiness", "source_provenance_policy"), "contract")
    if names[0] in present and names[-1] not in missing:
        validate_contract(contract, read_verifier(root / names[-1]))
    existing = scope_ids(present[names[1]]) if names[1] in present else set()
    seen = set()
    for name in names[2:4]:
        if name in present:
            require(names[1] in present, "inventory validation requires current scope")
            validate(present[name], existing, seen)
    if missing:
        backlog = library_documentation_backlog_readiness(root, provenance=library_data)
        return {"schema_version": 1, "scope": CONTRACT, "readiness": "missing",
                "review_status": "unreviewed", "missing_files": missing,
                "full_rust_completion_percent": None,
                "rule_proposals": proposals,
                "partial_witness_sources": witness_sources,
                "library_witness_sources": library_witnesses,
                "inventory_readiness_blockers": missing + ([REFERENCE_RULES] if reference_data is _ABSENT else [])
                                                + ([LIBRARY_PROVENANCE] if library_data is _ABSENT else [])
                                                + ([LIBRARY_BACKLOG] if backlog["readiness"] == "missing" else []),
                "reference_rules": {"readiness": "missing" if reference_data is _ABSENT else "blocked-by-missing-inputs",
                                    "full_rust_completion_percent": None},
                "library_surface_inputs": {"readiness": "missing" if library_data is _ABSENT else "blocked-by-missing-inputs",
                                               "full_rust_completion_percent": None, "api_denominator": None,
                                               "documentation_backlog": backlog},
                "limitations": ["Missing inventory inputs cannot establish executable evidence or full compatibility."]}
    result = summarize([present[name] for name in names[2:4]], present[names[1]])
    result.update(contract_id=contract["contract_id"], readiness="unreviewed", missing_files=[])
    result["reference_rules"] = reference_readiness(root, data=reference_data, contract=contract)
    result["library_surface_inputs"] = library_surface_readiness(root, data=library_data, contract=contract)
    result["rule_proposals"] = proposals
    result["partial_witness_sources"] = witness_sources
    result["library_witness_sources"] = library_witnesses
    result["inventory_readiness_blockers"] = inventory_readiness_blockers(result)
    return result


def rule_proposal_readiness(root, *, index_path=None, candidate_path=None):
    from reconcile_token_rules import summarize_proposals
    root = Path(root)
    try:
        return summarize_proposals(index_path or root / REFERENCE_RULES,
                                   candidate_path or root / "data/compiler-language-inventory.json")
    except ValueError as exc:
        # The CLI's __main__ class and an imported module's class are distinct.
        raise InvalidInventory(str(exc)) from exc


def partial_witness_source_readiness(root, *, index_path=None, candidate_path=None):
    from reconcile_token_rules import summarize_witness_sources
    root = Path(root)
    try:
        return summarize_witness_sources(index_path or root / REFERENCE_RULES,
                                         candidate_path or root / "data/compiler-language-inventory.json", root)
    except ValueError as exc:
        raise InvalidInventory(str(exc)) from exc


def library_witness_readiness(root):
    from library_witness_sources import summarize as summarize_library_witnesses
    try:
        return summarize_library_witnesses(Path(root))
    except ValueError as exc:
        raise InvalidInventory(str(exc)) from exc


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--inventory", action="append", type=Path)
    parser.add_argument("--scope", type=Path)
    parser.add_argument("--contract", type=Path)
    parser.add_argument("--oracle-verifier", type=Path)
    parser.add_argument("--reference-rules", type=Path)
    parser.add_argument("--source-archive", type=Path)
    parser.add_argument("--submodule-proof", type=Path)
    parser.add_argument("--library-surface-provenance", type=Path)
    args = parser.parse_args(argv)
    if not any((args.inventory, args.scope, args.contract, args.oracle_verifier,
                args.reference_rules, args.source_archive, args.submodule_proof,
                args.library_surface_provenance)):
        try:
            report = load_and_summarize(args.root)
        except InvalidInventory as exc:
            print(json.dumps({"error": str(exc)}), file=sys.stderr)
            return 2
        print(json.dumps(report, indent=2, sort_keys=True))
        return 0
    args.scope = args.scope or args.root / "data/compiler-compatibility-scope.json"
    args.inventory = args.inventory or [args.root / "data/compiler-language-inventory.json",
                                       args.root / "data/compiler-library-project-inventory.json"]
    try:
        contract_path = args.contract or args.scope.with_name("compiler-compatibility-contract.json")
        verifier_path = args.oracle_verifier or contract_path.parent.parent / "tests/support/stock_rustc.rs"
        contract = validate_contract(load(contract_path), read_verifier(verifier_path))
        report = summarize([load(path) for path in args.inventory], load(args.scope))
        report["contract_id"] = contract["contract_id"]
        report["readiness"] = "unreviewed"
        report["reference_rules"] = reference_readiness(
            args.root, data=load(args.reference_rules) if args.reference_rules else _ABSENT,
            candidate_path=next((path for path in args.inventory if path.name == "compiler-language-inventory.json"), None),
            contract=contract, source_archive=args.source_archive, submodule_proof=args.submodule_proof)
        report["library_surface_inputs"] = library_surface_readiness(
            args.root, data=load(args.library_surface_provenance) if args.library_surface_provenance else _ABSENT,
            contract=contract)
        report["rule_proposals"] = rule_proposal_readiness(
            args.root, index_path=args.reference_rules,
            candidate_path=next((path for path in args.inventory
                                 if path.name == "compiler-language-inventory.json"), None))
        report["partial_witness_sources"] = partial_witness_source_readiness(
            args.root, index_path=args.reference_rules,
            candidate_path=next((path for path in args.inventory
                                 if path.name == "compiler-language-inventory.json"), None))
        report["library_witness_sources"] = library_witness_readiness(args.root)
        report["inventory_readiness_blockers"] = inventory_readiness_blockers(report)
    except InvalidInventory as exc:
        print(json.dumps({"error": str(exc)}), file=sys.stderr)
        return 2
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    sys.exit(main())
