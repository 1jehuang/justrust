#!/usr/bin/env python3
"""Validate unreviewed obligation inventories, never execution or compatibility evidence.

Only the requirement IDs are read from the current scope. Scope execution-evidence
validation remains the responsibility of compatibility_scope.py and conformance.py.
"""
from __future__ import annotations

import argparse
import json
import re
from pathlib import Path
import sys
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


def load_and_summarize(root: Path):
    """Return honest missing readiness for partial fixtures, reject malformed present inputs."""
    root = Path(root)
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
        return {"schema_version": 1, "scope": CONTRACT, "readiness": "missing",
                "review_status": "unreviewed", "missing_files": missing,
                "full_rust_completion_percent": None,
                "limitations": ["Missing inventory inputs cannot establish executable evidence or full compatibility."]}
    result = summarize([present[name] for name in names[2:4]], present[names[1]])
    result.update(contract_id=contract["contract_id"], readiness="unreviewed", missing_files=[])
    return result


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--inventory", action="append", type=Path)
    parser.add_argument("--scope", type=Path)
    parser.add_argument("--contract", type=Path)
    parser.add_argument("--oracle-verifier", type=Path)
    args = parser.parse_args(argv)
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
    except InvalidInventory as exc:
        print(json.dumps({"error": str(exc)}), file=sys.stderr)
        return 2
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    sys.exit(main())
