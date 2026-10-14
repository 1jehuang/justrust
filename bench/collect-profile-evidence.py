#!/usr/bin/env python3
"""Collect compact capture provenance, without promoting profiling to a speed claim."""
import argparse
import hashlib
import json
from pathlib import Path


def collect(directory, legacy_launcher_source_label=False):
    directory = Path(directory)
    raw = (directory / "capture.json").read_bytes()
    capture = json.loads(raw)
    env = dict(capture["env"])
    # Older on-remote launchers supplied the source HEAD as a binary revision.
    # Preserve that label explicitly as unverified, never infer executable source.
    if legacy_launcher_source_label and env.get("benchmark_repo_rev") is None and env.get("justrust_binary_revision"):
        env["unverified_reported_binary_revision"] = env["justrust_binary_revision"]
        env["benchmark_repo_rev"] = env["justrust_binary_revision"]
        env["justrust_binary_revision"] = None
        env["justrust_revision_source"] = "legacy launcher source label, executable revision unknown"
    profiles = []
    for path in sorted(directory.glob("t*/*.mm_profdata")):
        with path.open("rb") as stream:
            digest = hashlib.file_digest(stream, "sha256").hexdigest()
        profiles.append({"frontend_threads": int(path.parent.name[1:]), "name": path.name,
                         "bytes": path.stat().st_size, "sha256": digest})
    return {"scenario": capture["scenario"], "toolchain": capture["toolchain"],
            "profile_events": capture["profile_events"], "env": env,
            "capture_sha256": hashlib.sha256(raw).hexdigest(),
            "wrapper_sha256": hashlib.sha256((directory / "prof-wrapper.sh").read_bytes()).hexdigest(),
            "profiles": profiles, "runs": capture["runs"],
            "interpretation": "local controlled host profiling, not hosted-service latency or a speedup comparison"}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directories", nargs="+")
    parser.add_argument("--out", required=True)
    parser.add_argument("--legacy-launcher-source-label", action="store_true",
                        help="mark a known old on-remote source-HEAD label as unverified executable provenance")
    args = parser.parse_args()
    result = {"schema_version": 1, "captures": [collect(path, args.legacy_launcher_source_label) for path in args.directories],
              "limitations": ["Profiled runs are not interchangeable with normal benchmark runs.",
                              "Retain noisy flags, first-run warmup rebuilds and unknown binary revisions."]}
    Path(args.out).write_text(json.dumps(result, indent=2) + "\n")


if __name__ == "__main__":
    main()
