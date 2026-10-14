#!/usr/bin/env python3
"""Deterministic offline coarse Cargo package hints, not unbiased cost estimates.

Only explicit package/version identities are used. No compiler or hosted calls.
The output is Cargo's name -> integer milliseconds table, with fallback 100 for
unknown names supplied by existing Cargo behavior, not entries in this table.
"""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import re
import stat
import statistics

MAX_PACKAGES = 1024
MAX_BYTES = 65536
MAX_COST = 120000
MAX_INPUT_BYTES = 8 * 1024 * 1024


def digest(raw):
    return hashlib.sha256(raw).hexdigest()


def encode(value):
    return (json.dumps(value, sort_keys=True, indent=2, ensure_ascii=True,
                       allow_nan=False) + "\n").encode("utf-8")


def open_safe(path, flags, mode=0o400):
    """Walk every ancestor via dirfd, refusing symlinks including the leaf."""
    parts = Path(os.path.abspath(path)).parts
    directory = os.open(parts[0], os.O_RDONLY | os.O_DIRECTORY)
    try:
        for part in parts[1:-1]:
            child = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW,
                            dir_fd=directory)
            os.close(directory)
            directory = child
        fd = os.open(parts[-1], flags | os.O_NOFOLLOW | os.O_NONBLOCK,
                     mode, dir_fd=directory)
        if not stat.S_ISREG(os.fstat(fd).st_mode):
            os.close(fd)
            raise ValueError("input/output must be a regular non-symlink file")
        return fd
    finally:
        os.close(directory)


def read_regular(path):
    try:
        with os.fdopen(open_safe(path, os.O_RDONLY), "rb") as handle:
            if os.fstat(handle.fileno()).st_size > MAX_INPUT_BYTES:
                raise ValueError("input exceeds 8MiB bound")
            raw = handle.read(MAX_INPUT_BYTES + 1)
            if len(raw) > MAX_INPUT_BYTES:
                raise ValueError("input exceeds 8MiB bound")
            return raw
    except OSError as error:
        raise ValueError("cannot read regular non-symlink input") from error


def write_new(path, raw):
    with os.fdopen(open_safe(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL), "wb") as handle:
        handle.write(raw)


def identity(value):
    return isinstance(value, str) and bool(value.strip())


def aggregate(captures):
    versions, samples = {}, {}
    total = eligible = 0
    for rows in captures:
        maxima = {}
        for row in rows:
            total += 1
            if not isinstance(row, dict):
                continue
            name, version = row.get("package"), row.get("version")
            if not identity(name) or not identity(version):
                continue
            # Never normalize hyphens/underscores or infer from crate/paths.
            # Reject path-bearing identities rather than leaking them.
            if any(c in name + version for c in "/\\\n\r\x00"):
                continue
            versions.setdefault(name, set()).add(version)
            wall = row.get("wall")
            cache = row.get("depcache", "")
            if (type(row.get("exit")) is not int or row["exit"] != 0
                    or type(wall) not in (int, float) or not math.isfinite(wall)
                    or wall <= 0 or row.get("cache_hit")
                    or cache not in (None, "", "miss", "local-miss")):
                continue
            eligible += 1
            maxima[name] = max(maxima.get(name, 0), wall)
        for name, wall in maxima.items():
            samples.setdefault(name, []).append(wall)
    ambiguous = sorted(name for name, values in versions.items() if len(values) > 1)
    table, clamped = {}, []
    for name in sorted(samples):
        if name in ambiguous:
            continue
        seconds = statistics.median(samples[name])
        # Compare before multiplication to avoid overflow for huge finite walls.
        if seconds > MAX_COST / 1000:
            cost = MAX_COST
            clamped.append(name)
        else:
            cost = max(1, math.ceil(seconds * 1000))
        table[name] = cost
    raw = encode(table)
    if len(table) > MAX_PACKAGES or len(raw) > MAX_BYTES:
        raise ValueError("model exceeds 1024 packages or 64KiB")
    return raw, dict(total_units=total, eligible_units=eligible,
                     excluded_units=total - eligible, packages=len(table),
                     ambiguous_packages=len(ambiguous), ambiguous_names=ambiguous,
                     clamped_packages=clamped,
                     capture_samples={name: len(samples[name]) for name in table})


def public_text(text):
    if not isinstance(text, str):
        raise ValueError("confounds must be strings")
    return re.sub(r"(?<!\w)(?:/[^\s,;]+|~[/\\][^\s,;]+)", "[path-redacted]", text)


def generate(capture_dir, audit_path, runs):
    if (not runs or len(set(runs)) != len(runs)
            or any(not re.fullmatch(r"\d{8}-\d{9}-\d+", run) for run in runs)):
        raise ValueError("unique explicit run IDs required")
    audit_raw = read_regular(audit_path)
    audit = json.loads(audit_raw)
    if not isinstance(audit, dict):
        raise ValueError("audit must be an object")
    provenance = {}
    for key in ("cargo", "rustc"):
        value = audit.get("identities", {}).get(key)
        if not isinstance(value, str) or not re.fullmatch(r"[0-9a-f]{64}", value):
            raise ValueError("audited compiler/candidate identity required")
        provenance[key + "_sha256"] = value
    captures, receipts = [], []
    for run in sorted(runs):
        matches = [record for record in audit.get("records", []) if record.get("id") == run]
        if len(matches) != 1:
            raise ValueError("run must have one audit record")
        record = matches[0]
        if (record.get("candidate_cargo_selected") is not True
                or record.get("pinned_compiler_selected") is not True
                or type(record.get("exit")) is not int or record["exit"] != 0):
            raise ValueError("successful audited candidate/compiler capture required")
        confounds = record.get("known_confounds")
        if not isinstance(confounds, list):
            raise ValueError("explicit audited training confounds required")
        receipt = dict(id=run, known_confounds=[public_text(x) for x in confounds],
                       candidate_cargo_selected=True, pinned_compiler_selected=True,
                       toolchain=public_text(record.get("toolchain", "unknown")))
        # Verify BOTH original hashes before opening or parsing the units.
        for kind in ("meta", "summary"):
            raw = read_regular(Path(capture_dir) / f"{run}-{kind}.json")
            actual = digest(raw)
            if actual != record.get(kind + "_sha256"):
                raise ValueError(kind + " hash mismatch")
            receipt[kind + "_sha256"] = actual
        units_raw = read_regular(Path(capture_dir) / f"{run}-units.jsonl")
        receipt["units_sha256"] = digest(units_raw)
        captures.append([json.loads(line) for line in units_raw.splitlines() if line.strip()])
        receipts.append(receipt)
    model, counts = aggregate(captures)
    receipt = dict(schema=1, audit_sha256=digest(audit_raw), captures=receipts,
                   compiler_candidate_provenance=provenance,
                   model_sha256=digest(model), model_bytes=len(model), counts=counts,
                   unknown_package_fallback=100,
                   method="max successful non-cache unit wall per package per capture, then median across available captures, ceil milliseconds, clamp 1..120000",
                   limitations=[
                       "Coarse package hints only, not unbiased costs. All audited training confounds apply.",
                       "Contended wall time, check-mode profile and hardware specific. Only observed captures, no independent validation.",
                       "Build scripts, proc macros and targets share package names. Multiple versions dropped. Missing capture samples are not imputed.",
                       "Units hashes record captured bytes but audit authenticates only original meta and summary hashes.",
                       "No DAG span, speed improvement or performance ceiling inferred. Unknown packages retain existing Cargo fallback 100."])
    return model, receipt


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture-dir", type=Path, required=True)
    parser.add_argument("--audit", type=Path, required=True)
    parser.add_argument("--run", action="append", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--receipt", type=Path, required=True)
    args = parser.parse_args(argv)
    try:
        model, receipt = generate(args.capture_dir, args.audit, args.run)
        receipt_raw = encode(receipt)
        # Reserve both exclusive leaves before writing either payload.
        with os.fdopen(open_safe(args.output, os.O_WRONLY | os.O_CREAT | os.O_EXCL), "wb") as output:
            with os.fdopen(open_safe(args.receipt, os.O_WRONLY | os.O_CREAT | os.O_EXCL), "wb") as proof:
                output.write(model)
                proof.write(receipt_raw)
        print(json.dumps(dict(model_sha256=receipt["model_sha256"],
                              packages=receipt["counts"]["packages"], bytes=len(model)), sort_keys=True))
    except (OSError, ValueError, TypeError, KeyError) as error:
        # Do not echo private input/output paths from OSError messages.
        parser.exit(2, "scheduler costs: " + (str(error) if isinstance(error, ValueError)
                                             and not isinstance(error, json.JSONDecodeError)
                                             else "invalid input or unavailable exclusive output") + "\n")


if __name__ == "__main__":
    main()
