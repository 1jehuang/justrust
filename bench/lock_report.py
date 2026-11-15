#!/usr/bin/env python3
"""Read-only lock inventory over existing run summaries. Writes only stdout.

Cargo phase lock_wait is a stderr-to-next-output estimate, potentially including
unrelated work. E2e build/host.lock_wait repeats that estimate, not another cost.
Percentiles use nearest rank over observed records, including observed zeros.
"""
import argparse
from datetime import datetime, timezone
import json
import math
import os
from pathlib import Path
import re


NOTES = [
    "Cargo lock durations are estimated from stderr until subsequent output and may overestimate actual lock ownership waits.",
    "Phase and e2e build/host.lock_wait overlap: only phases (or e2e fallback) enter the Cargo inventory.",
    "Sync queue and admission are separate views, not additive to Cargo or one another without a verified waterfall.",
    "remote.sync.lock_ms is measured daemon queue time; e2e sync.lock_wait may be scaled to the client span. Admission is an end-to-end segment, not proof of a lock.",
    "Label-specific statistics are conditional on an unambiguous aggregate label; other records are unallocated or outside that label, never inferred zero.",
    "Missing telemetry is unknown, not zero. Empty e2e segments do not prove zero. Percentiles are nearest rank over covered records.",
    "Scratch exclusion is a path heuristic, not proof of production provenance. Custom roots can be supplied with --exclude-root.",
]


def number(value):
    return isinstance(value, (int, float)) and not isinstance(value, bool) and math.isfinite(value) and value >= 0


def mapping(value):
    return value if isinstance(value, dict) else {}


def since_time(value, now=None):
    """Accept epoch seconds, ISO-8601 (naive means UTC), or relative 2h/7d."""
    match = re.fullmatch(r"(\d+(?:\.\d+)?)([smhdw])", value)
    if match:
        now = datetime.now(timezone.utc).timestamp() if now is None else now
        return now - float(match[1]) * dict(s=1, m=60, h=3600, d=86400, w=604800)[match[2]]
    try:
        result = float(value)
    except ValueError:
        dt = datetime.fromisoformat(value.replace("Z", "+00:00"))
        result = dt.replace(tzinfo=timezone.utc).timestamp() if dt.tzinfo is None else dt.timestamp()
    if not number(result):
        raise ValueError("since must be a finite nonnegative timestamp")
    return result


def scratch(record, exclude_roots=()):
    for raw in (record.get("cwd"), mapping(record.get("git")).get("root")):
        if not isinstance(raw, str):
            continue
        path = Path(raw)
        parts = path.parts
        if "justrust-bench" in parts or any(
            parts[i:i + 2] == ("bench", name)
            for i in range(len(parts)) for name in ("work", "synth", "snapshots")
        ):
            return True
        if any(("bench" in part.lower() or "synth" in part.lower()) for part in parts) and "scratch" in parts:
            return True
        for root in exclude_roots:
            if path == root or root in path.parents:
                return True
    return False


def stats(observations, total, samples):
    values = sorted(v for _, v in observations)
    return {
        "covered_records": len(values), "missing_records": total - len(values),
        "positive_records": sum(v > 0 for v in values),
        "total_secs": sum(values) if values else None,
        "p50_secs": values[math.ceil(len(values) * .50) - 1] if values else None,
        "p95_secs": values[math.ceil(len(values) * .95) - 1] if values else None,
        "sample_record_ids": [rid for rid, _ in sorted(observations, key=lambda item: (-item[1], item[0]))[:samples]],
    }


def report(runs, since=None, limit=1000, include_benchmarks=False, samples=5, exclude_roots=()):
    counts = dict(files_seen=0, malformed_files=0, excluded_benchmark=0,
                  excluded_since=0, missing_start=0, excluded_limit=0)
    records = []
    malformed = []
    for path in sorted(Path(runs).glob("*/summary.json")):
        counts["files_seen"] += 1
        try:
            record = json.loads(path.read_text())
            if not isinstance(record, dict):
                raise ValueError("summary is not an object")
        except (OSError, ValueError, UnicodeError):
            counts["malformed_files"] += 1
            if len(malformed) < samples:
                malformed.append(path.parent.name)
            continue
        if not include_benchmarks and scratch(record, exclude_roots):
            counts["excluded_benchmark"] += 1
            continue
        start = record.get("start")
        if not number(start):
            counts["missing_start"] += 1
            if since is not None:
                counts["excluded_since"] += 1
                continue
        elif since is not None and start < since:
            counts["excluded_since"] += 1
            continue
        rid = record.get("id")
        rid = rid if isinstance(rid, str) and rid else path.parent.name
        records.append((start if number(start) else -1, rid, record))
    records.sort(key=lambda row: (row[0], row[1]), reverse=True)
    counts["excluded_limit"] = max(0, len(records) - limit) if limit else 0
    records = records[:limit] if limit else records
    counts["selected_records"] = len(records)
    metrics = {name: [] for name in (
        "cargo_lock_estimated", "cargo_build_directory_estimated",
        "cargo_package_cache_estimated", "cargo_unallocated_estimated",
        "sync_queue_measured", "e2e_sync_lock_wait", "e2e_admission",
        "e2e_wall", "cargo_wall")}
    sources = dict(cargo_phase=0, cargo_e2e_fallback=0, e2e_derived=0,
                   mixed_or_unknown_lock_labels=0)
    label_counts = {}
    for _, rid, record in records:
        e2e = mapping(record.get("e2e"))
        derived = e2e.get("derived") is True
        sources["e2e_derived"] += int(derived)
        segments = e2e.get("segments")
        segments = segments if isinstance(segments, list) else []
        segments_by_key = {}
        for segment in segments:
            segment = mapping(segment)
            key, secs = segment.get("key"), segment.get("secs")
            if isinstance(key, str) and number(secs):
                segments_by_key[key] = segments_by_key.get(key, 0) + secs
        cargo = mapping(record.get("phases")).get("lock_wait")
        if number(cargo):
            sources["cargo_phase"] += 1
        else:
            fallback = [segments_by_key[key] for key in ("host.lock_wait", "build.lock_wait") if key in segments_by_key]
            # Both host and build labels in one record are ambiguous, not additive.
            cargo = fallback[0] if len(fallback) == 1 else None
            sources["cargo_e2e_fallback"] += int(cargo is not None)
        labels = record.get("locks_waited")
        labels = sorted(set(label for label in labels if isinstance(label, str))) if isinstance(labels, list) else []
        for label in labels:
            label_counts[label] = label_counts.get(label, 0) + 1
        category = "unallocated"
        if labels == ["build directory"]:
            category = "build_directory"
        elif labels == ["package cache"]:
            category = "package_cache"
        elif labels:
            sources["mixed_or_unknown_lock_labels"] += 1
        if number(cargo):
            metrics["cargo_lock_estimated"].append((rid, cargo))
            metrics[f"cargo_{category}_estimated"].append((rid, cargo))
        measured = mapping(mapping(record.get("remote")).get("sync")).get("lock_ms")
        if number(measured):
            metrics["sync_queue_measured"].append((rid, measured / 1000))
        for key, metric in (("sync.lock_wait", "e2e_sync_lock_wait"), ("remote.admission", "e2e_admission")):
            if key in segments_by_key and not derived:
                metrics[metric].append((rid, segments_by_key[key]))
        for value, metric in ((e2e.get("wall"), "e2e_wall"), (record.get("wall"), "cargo_wall")):
            if number(value):
                metrics[metric].append((rid, value))
    summarized = {key: stats(values, len(records), samples) for key, values in metrics.items()}
    priorities = sorted((key for key in metrics if key in (
        "cargo_lock_estimated", "sync_queue_measured", "e2e_admission")),
        key=lambda key: -(summarized[key]["total_secs"] or 0))
    return dict(schema=1, filters=dict(since=since, limit=limit, include_benchmarks=include_benchmarks,
                exclude_roots=[str(p) for p in exclude_roots]), counts=counts, sources=sources,
                malformed_sample_record_ids=malformed, lock_label_record_counts=label_counts,
                metrics=summarized, priority_views_not_additive=priorities, notes=NOTES)


def text_report(result):
    lines = ["Lock inventory (seconds, observed coverage only)", json.dumps(result["counts"], sort_keys=True),
             "Sources: " + json.dumps(result["sources"], sort_keys=True)]
    for key in result["priority_views_not_additive"] + [k for k in result["metrics"] if k not in result["priority_views_not_additive"]]:
        metric = result["metrics"][key]
        def fmt(value):
            return "unknown" if value is None else f"{value:.3f}"
        lines.append(f"{key}: total={fmt(metric['total_secs'])} p50={fmt(metric['p50_secs'])} p95={fmt(metric['p95_secs'])} covered={metric['covered_records']} missing={metric['missing_records']} positive={metric['positive_records']} samples={','.join(metric['sample_record_ids'])}")
    lines.append("Lock labels (record counts, not durations): " + json.dumps(result["lock_label_record_counts"], sort_keys=True))
    lines.extend(result["notes"])
    return "\n".join(lines)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--runs", type=Path, default=Path(os.environ.get("JUSTRUST_HOME", Path.home() / ".justrust")) / "runs")
    parser.add_argument("--since", help="ISO UTC, epoch seconds, or relative duration such as 7d")
    parser.add_argument("--limit", type=int, default=1000, help="newest records, 0 means unlimited (default 1000)")
    parser.add_argument("--samples", type=int, default=5)
    parser.add_argument("--include-benchmarks", action="store_true")
    parser.add_argument("--exclude-root", type=Path, action="append", default=[])
    parser.add_argument("--format", choices=("text", "json"), default="text")
    args = parser.parse_args(argv)
    if args.limit < 0 or args.samples < 0:
        parser.error("limit and samples must be nonnegative")
    try:
        since = since_time(args.since) if args.since else None
    except (ValueError, OverflowError) as exc:
        parser.error(str(exc))
    result = report(args.runs, since, args.limit, args.include_benchmarks, args.samples, args.exclude_root)
    print(json.dumps(result, indent=2, allow_nan=False) if args.format == "json" else text_report(result))


if __name__ == "__main__":
    main()
