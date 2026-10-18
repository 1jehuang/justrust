#!/usr/bin/env python3
"""Read-only static fixture ledger. Never import/execute a corpus or compiler."""
import argparse
import ast
from collections import Counter
import csv
import hashlib
import json
from pathlib import Path
import re

GROUPS = {
    "positive": "declared-valid", "valid": "declared-valid",
    "negative": "declared-invalid", "invalid": "declared-invalid",
    "legalunsupported": "legal-unsupported", "legal_unsupported": "legal-unsupported",
    "legal-excluded": "legal-unsupported", "legal-unsupported": "legal-unsupported",
}


def classification(value):
    return GROUPS.get(value, "unknown") if isinstance(value, str) else "unknown"


def digest(data):
    return hashlib.sha256(data).hexdigest()


def row(source, locator, category="unknown", **extra):
    return {"source_path": source, "locator": locator, "classification": category,
            "classification_basis": "declaration only, not current native behavior",
            "contract": "unknown", "evidence": "unavailable", **extra}


def literal(node, env):
    """Only data literals, previously read constants and json.loads(data)."""
    if isinstance(node, ast.Name):
        return env[node.id]
    if (isinstance(node, ast.Call) and isinstance(node.func, ast.Attribute)
            and isinstance(node.func.value, ast.Name)
            and node.func.value.id == "json" and node.func.attr == "loads"
            and len(node.args) == 1 and not node.keywords):
        return json.loads(literal(node.args[0], env))
    return ast.literal_eval(node)


def python_records(path, relative):
    """Extract literal FIXTURES only. Dynamic corpus() remains explicitly unknown."""
    tree = ast.parse(path.read_text(encoding="utf-8"), filename=relative)
    env, records = {}, []
    for node in tree.body:
        if not isinstance(node, ast.Assign) or len(node.targets) != 1:
            continue
        target = node.targets[0]
        if not isinstance(target, ast.Name):
            continue
        try:
            env[target.id] = literal(node.value, env)
        except (ValueError, TypeError, KeyError, SyntaxError, json.JSONDecodeError):
            continue
        if target.id != "FIXTURES":
            continue
        fixtures = env[target.id]
        if isinstance(fixtures, dict):
            grouped = [(group, item) for group, items in fixtures.items()
                       if isinstance(items, list) for item in items]
        elif isinstance(fixtures, list):
            grouped = [(None, item) for item in fixtures]
        else:
            grouped = []
        for index, (group, item) in enumerate(grouped):
            if not isinstance(item, dict):
                records.append(row(relative, f"FIXTURES[{index}]",
                                   note="unrecognized fixture record shape"))
                continue
            label = item.get("group", group)
            source = item.get("full_source", item.get("source"))
            complete = isinstance(item.get("full_source"), str)
            expected_hash = item.get("full_source_sha256" if complete else "source_sha256")
            actual_hash = digest(source.encode()) if isinstance(source, str) else None
            records.append(row(relative, f"FIXTURES:{item.get('name', index)}",
                               classification(label), declared_group=label,
                               source_sha256=actual_hash,
                               source_form="full_source" if complete else "fragment-or-unverified-source",
                               contract="embedded fixture and runner",
                               evidence="embedded declaration, not a fresh run",
                               declared_hash_match=(actual_hash == expected_hash) if expected_hash else None,
                               expected_stdout_available="expected_stdout" in item,
                               note="group classification does not execute corpus() promotions or wrappers"))
    # Literal records cannot prove the runner has no transformations/promotions.
    if any(isinstance(n, (ast.FunctionDef, ast.AsyncFunctionDef)) and n.name == "corpus"
           for n in tree.body):
        records.append(row(relative, "corpus()", note=(
            "dynamic corpus evaluation not executed; generated cases, overrides, wrappers "
            "and current effective classifications remain unknown")))
    if not records:
        records.append(row(relative, "module", note="unrecognized static fixture format"))
    return records


def manifest_records(path, relative):
    records = []
    for number, fields in enumerate(csv.reader(path.read_text().splitlines(), delimiter="\t"), 1):
        if not fields:
            continue
        if len(fields) != 4 or not re.fullmatch(r"[A-Za-z0-9_-]+", fields[0]):
            records.append(row(relative, f"line:{number}", note="unknown TSV row schema"))
            continue
        name, category, stock, sha = fields
        source = path.parent / f"{name}.rs"
        receipt = path.parent / "stock-receipt.json"
        evidence = "unavailable"
        if receipt.is_file():
            try:
                data = json.loads(receipt.read_text())
                cases = data.get("cases", []) if isinstance(data, dict) else []
                matches = [c for c in cases if isinstance(c, dict) and c.get("name") == name]
                if len(matches) == 1:
                    case = matches[0]
                    agrees = (case.get("category"), case.get("stock"), case.get("source_sha256")) == (category, stock, sha)
                    evidence = "receipt-record-agrees" if agrees else "receipt-record-conflicts"
                else:
                    evidence = "receipt-present-record-missing-or-duplicate"
            except (ValueError, OSError, TypeError):
                evidence = "receipt-unreadable"
        records.append(row(str(Path(relative).parent / f"{name}.rs"), f"{relative}:{number}",
                           classification(category), declared_group=category,
                           contract=relative, stock_declaration=stock, evidence=evidence,
                           evidence_path=str(Path(relative).parent / receipt.name) if receipt.exists() else None,
                           source_exists=source.is_file(),
                           source_hash_match=digest(source.read_bytes()) == sha if source.is_file() else None,
                           expected_stdout_available=source.with_suffix(".stdout").is_file()))
    return records


def header_record(path, relative):
    text = path.read_text(encoding="utf-8")
    stocks = re.findall(r"^// stock: (accept|reject)\s*$", text, re.MULTILINE)
    categories = re.findall(r"^// category: ([a-z-]+)\s*$", text, re.MULTILINE)
    category = "unknown"
    if len(stocks) == len(categories) == 1:
        if stocks[0] == "accept" and categories[0] == "excluded":
            category = "legal-unsupported"
        elif stocks[0] == "reject":
            category = "declared-invalid"
    return row(relative, "source headers", category,
               stock_declaration=stocks[0] if len(stocks) == 1 else "unknown",
               declared_group=categories[0] if len(categories) == 1 else "unknown",
               source_sha256=digest(path.read_bytes()),
               contract=str(Path(relative).parent / "README.md") if (path.parent / "README.md").is_file() else "unknown",
               evidence="README historical result table, not a fresh run" if (path.parent / "README.md").is_file() else "unavailable")


def drop_receipt_records(root):
    """Select the highest canonical numeric receipt version, never a stale fallback."""
    base = root / "tests/fixtures/drop_normal"
    versions = [(int(match.group(1)), path) for path in base.glob("receipts-v*")
                if (match := re.fullmatch(r"receipts-v([1-9][0-9]*)", path.name))
                and path.is_dir() and not path.is_symlink()]
    if not versions:
        return []
    _, selected = max(versions)
    manifest = selected / "manifest.json"
    relative = manifest.relative_to(root).as_posix()
    if not manifest.is_file() or manifest.is_symlink():
        return [row(relative, "manifest", note="latest Drop manifest missing or symlinked; no fallback")]
    try:
        data = json.loads(manifest.read_text())
        if not isinstance(data, dict) or data.get("schema") != 1 or not isinstance(data.get("sources"), list):
            raise ValueError("unknown manifest schema")
    except (ValueError, OSError):
        return [row(relative, "manifest", note="invalid or unknown latest Drop manifest schema; no fallback")]
    receipt = selected / "stock.json"
    observations, linked, receipt_state = [], None, "unavailable"
    if receipt.is_file() and not receipt.is_symlink():
        try:
            stock = json.loads(receipt.read_text())
            if not isinstance(stock, dict) or stock.get("schema") != 1 or not isinstance(stock.get("observations"), list):
                raise ValueError("unknown receipt schema")
            observations = stock["observations"]
            linked = stock.get("manifest_sha256") == digest(manifest.read_bytes())
            receipt_state = "receipt-record-missing"
        except (ValueError, OSError):
            receipt_state = "receipt-unreadable"
    if receipt.is_symlink():
        receipt_state = "receipt-symlink-rejected"
    records, visited = [], set()
    for index, item in enumerate(data["sources"]):
        name = item.get("path") if isinstance(item, dict) else None
        if (not isinstance(name, str) or not name or Path(name).is_absolute()
                or ".." in Path(name).parts or "\\" in name):
            records.append(row(relative, f"sources[{index}]", note="invalid source path or record"))
            continue
        source = base / name
        if not source.resolve().is_relative_to(base.resolve()):
            records.append(row(relative, f"sources[{index}]", note="source escapes corpus root"))
            continue
        source_relative = source.relative_to(root).as_posix()
        if source_relative in visited:
            continue
        visited.add(source_relative)
        duplicates = sum(isinstance(s, dict) and s.get("path") == name for s in data["sources"])
        label = item.get("classification")
        category = {"positive": "declared-valid", "negative": "declared-invalid",
                    "unsupported": "legal-unsupported"}.get(label, "unknown") if isinstance(label, str) else "unknown"
        if duplicates != 1:
            category = "unknown"
        matches = [o for o in observations if isinstance(o, dict) and o.get("path") == name]
        evidence, agrees, compile_status, run_status = receipt_state, None, None, None
        if len(matches) == 1:
            observed = matches[0]
            agrees = (observed.get("classification"), observed.get("sha256")) == (label, item.get("sha256"))
            evidence = "receipt-record-agrees" if agrees and linked else "receipt-record-conflicts"
            compile_status = observed.get("compile", {}).get("status") if isinstance(observed.get("compile"), dict) else None
            run_status = observed.get("run", {}).get("status") if isinstance(observed.get("run"), dict) else None
        elif len(matches) > 1:
            evidence = "receipt-record-duplicate"
        records.append(row(source_relative, f"{relative}:sources[{index}]", category,
                           declared_group=label, contract=relative,
                           evidence=evidence, evidence_path=receipt.relative_to(root).as_posix() if receipt.is_file() else None,
                           manifest_hash_match=linked, receipt_record_match=agrees,
                           source_exists=source.is_file(),
                           source_hash_match=digest(source.read_bytes()) == item.get("sha256") if source.is_file() else None,
                           stock_compile_status=compile_status, stock_run_status=run_status,
                           native_support="unknown; not evaluated",
                           note="Latest numeric Drop declared labels only; older versions ignored; duplicate declaration" if duplicates != 1 else
                           "Latest numeric Drop declared labels only; older versions ignored; receipt statuses are historical stock observations"))
    return records


def inventory(root):
    root = Path(root).resolve()
    records = drop_receipt_records(root)
    seen = {record["source_path"] for record in records}
    for base in ("fixtures", "tests/fixtures"):
        directory = root / base
        for manifest in sorted(directory.rglob("manifest.tsv")):
            relative = manifest.relative_to(root).as_posix()
            if relative != "tests/fixtures/leading_zeros/manifest.tsv":
                records.append(row(relative, "manifest", note="unknown manifest schema; not parsed as leading-zero TSV"))
                continue
            parsed = manifest_records(manifest, relative)
            records.extend(parsed)
            seen.update(r["source_path"] for r in parsed)
        for source in sorted(directory.rglob("*.rs")):
            relative = source.relative_to(root).as_posix()
            if relative in seen:
                continue
            if source.parent.name == "slice-contract-rejection":
                records.append(header_record(source, relative))
            else:
                readme = source.parent / "README.md"
                if not readme.exists() and source.parent.name == "cases":
                    readme = source.parent.parent / "README.md"
                evidence, evidence_path, hash_match = "unavailable", None, None
                receipt = source.parent.parent / "stock-evidence.json"
                if source.parent.name == "cases" and receipt.is_file():
                    evidence_path = receipt.relative_to(root).as_posix()
                    try:
                        data = json.loads(receipt.read_text())
                        name = source.name.removesuffix(".oracle.rs").removesuffix(".rs")
                        matches = [c for c in data["cases"] if c.get("name") == name]
                        if len(matches) == 1:
                            key = "oracle_source_sha256" if source.name.endswith(".oracle.rs") else "source_sha256"
                            hash_match = digest(source.read_bytes()) == matches[0].get(key)
                            evidence = "historical-receipt-source-match" if hash_match else "historical-receipt-source-conflict"
                        else:
                            evidence = "receipt-present-record-missing-or-duplicate"
                    except (ValueError, OSError, KeyError, TypeError, AttributeError):
                        evidence = "receipt-unreadable"
                records.append(row(relative, "file", contract=(readme.relative_to(root).as_posix()
                                   if readme.exists() else "unknown"), evidence=evidence,
                                   evidence_path=evidence_path, source_hash_match=hash_match,
                                   note="no supported per-case classification schema; filenames are not classifications"))
    for path in sorted((root / "experiments").glob("*correctness.py")):
        relative = path.relative_to(root).as_posix()
        try:
            records.extend(python_records(path, relative))
        except (SyntaxError, UnicodeError, OSError) as error:
            records.append(row(relative, "module", note=f"parse error: {type(error).__name__}"))
    # These runners have a separate contract, not the correctness.py schema.
    for name in ("slice-contract-runtime.py", "slice-contract-rejection.py", "leading_zeros_oracle.py"):
        if (root / "experiments" / name).is_file():
            records.append(row(f"experiments/{name}", "runner",
                               note="runner not executed; linked file corpus inventoried separately"))
    records.sort(key=lambda r: (r["source_path"], r["locator"]))
    return {"schema": 1, "scope": "static declarations only; no compiler invocation",
            "native_support": "unknown; not evaluated", "g0_complete": False,
            "counts": dict(sorted(Counter(r["classification"] for r in records).items())),
            "records": records}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--format", choices=("json", "text"), default="text")
    args = parser.parse_args()
    report = inventory(args.root)
    if args.format == "json":
        print(json.dumps(report, indent=2, sort_keys=True))
    else:
        print(report["scope"])
        print("Native support unknown. G0 incomplete. Counts are records, not coverage.")
        print(json.dumps(report["counts"], sort_keys=True))
        for record in report["records"]:
            print(f"{record['classification']}\t{record['source_path']}\t{record['locator']}"
                  f"\tcontract={record['contract']}\tevidence={record['evidence']}")


if __name__ == "__main__":
    main()
