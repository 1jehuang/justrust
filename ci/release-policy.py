#!/usr/bin/env python3
"""Fail-closed release policy snapshot and compiler provenance, stdlib only."""
import argparse
import hashlib
import json
import subprocess
from pathlib import Path
import re
import sys

# Bootstrap is separate from both service-owned and compatibility compilers.
BOOTSTRAP = "1.99.0"
TARGETS = ("x86_64-unknown-linux-musl", "aarch64-unknown-linux-musl",
           "x86_64-apple-darwin", "aarch64-apple-darwin", "x86_64-pc-windows-msvc")
EXACT = re.compile(r"(?:jr-[0-9a-fA-F]{7,40}|(?:nightly|beta)-[0-9]{4}-[0-9]{2}-[0-9]{2}|[0-9]+\.[0-9]+\.[0-9]+)\Z")


def pairs(items):
    result = {}
    for key, value in items:
        if key in result:
            raise ValueError("duplicate JSON key: " + key)
        result[key] = value
    return result


def parse(text):
    p = json.loads(text, object_pairs_hook=pairs)
    if not isinstance(p, dict) or set(p) != {"toolchain", "protocol", "release"}:
        raise ValueError("expected toolchain, protocol and release mapping")
    if not isinstance(p["toolchain"], str) or len(p["toolchain"]) > 128 or not EXACT.fullmatch(p["toolchain"]):
        raise ValueError("policy must name an exact toolchain Spec")
    if type(p["protocol"]) is not int or p["protocol"] != 1:
        raise ValueError("unsupported toolchain protocol (release driver supports 1)")
    release = p["release"]
    if not isinstance(release, dict) or set(release) != {"schema", "targets"} or type(release["schema"]) is not int or release["schema"] != 1 or not isinstance(release["targets"], dict) or set(release["targets"]) != set(TARGETS):
        raise ValueError("release policy must cover exactly the supported targets")
    for target, entry in release["targets"].items():
        if target.endswith("linux-musl"):
            if entry != {"toolchain": p["toolchain"], "protocol": p["protocol"], "kind": "hosted-policy"} or type(entry.get("protocol")) is not int:
                raise ValueError("Linux release compiler must match active service compiler")
        elif not isinstance(entry, dict) or set(entry) != {"toolchain", "kind"} or entry["kind"] != "stock-compatibility" or not isinstance(entry["toolchain"], str) or not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", entry["toolchain"]):
            raise ValueError("unsupported platform requires exact service-selected stock compatibility compiler")
    return p


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"))


def digest(value):
    return hashlib.sha256(canonical(value).encode()).hexdigest()


def freeze(policy):
    return {"schema": 1, "policy": policy, "bootstrap": BOOTSTRAP}


def validate(snapshot, expected):
    if not isinstance(snapshot, dict) or type(snapshot.get("schema")) is not int or set(snapshot) != {"schema", "policy", "bootstrap"} or snapshot != freeze(parse(canonical(snapshot["policy"]))):
        raise ValueError("invalid release snapshot")
    if digest(snapshot) != expected:
        raise ValueError("release policy identity drift")
    return snapshot


def check_protocol_source(policy, source):
    text = source.read_text()
    bounds = re.findall(r"pub const SUPPORTED:\s*std::ops::RangeInclusive<u32>\s*=\s*([0-9]+)\.\.=([0-9]+)\s*;", text)
    if len(bounds) != 1 or not int(bounds[0][0]) <= policy["protocol"] <= int(bounds[0][1]):
        raise ValueError("selected protocol unsupported by release candidate")


def selected(snapshot, target):
    if target not in TARGETS:
        raise ValueError("unsupported release target")
    return snapshot["policy"]["release"]["targets"][target]["toolchain"]


def environment(snapshot, target):
    # Deliberately overwrites ambient overrides/project pins. Never ask the service
    # again in a build job. Required mode prevents system-compiler fallback.
    return {"JUSTRUST_TOOLCHAIN": selected(snapshot, target),
            "JUSTRUST_TOOLCHAIN_REQUIRED": "1", "JUSTRUST_TOOLCHAIN_POLICY": "off",
            "JUSTRUST_REMOTE_REQUIRED": "1", "RUSTUP_TOOLCHAIN": snapshot["bootstrap"] if target.endswith("linux-musl") else selected(snapshot, target)}


def attest(snapshot, target, since, cwd, runs, binary=None):
    want = selected(snapshot, target)
    matches = []
    for path in runs.glob("*/meta.json"):
        # Historical malformed/test recordings cannot select this new build.
        if path.stat().st_mtime < since:
            continue
        try:
            m = json.loads(path.read_text())
        except (ValueError, OSError):
            # Runs can be live or older test fixtures, not release candidates.
            # None can attest anything without a complete matching receipt.
            continue
        if not isinstance(m, dict):
            continue
        args = m.get("args", [])
        if not isinstance(m.get("start"), (int, float)) or m["start"] < since or m.get("cwd") != str(cwd.resolve()):
            continue
        if "--target" not in args or args[args.index("--target") + 1] != target or "--release" not in args:
            continue
        s = json.loads(path.with_name("summary.json").read_text())
        if type(s.get("exit")) is not int or s["exit"] != 0 or m.get("place") != "hosted" or m.get("toolchain") != want:
            raise ValueError("release build did not use selected hosted compiler")
        compiler = m.get("real_rustc", "")
        if not isinstance(compiler, str) or not compiler.replace("\\", "/").endswith(f"/toolchains/{want}/bin/rustc") or ".." in compiler.replace("\\", "/").split("/"):
            raise ValueError("recorded actual compiler path does not match policy")
        matches.append({"run_id": m["id"], "compiler": compiler, "toolchain": want,
                        "target": target, "policy_sha256": digest(snapshot), "place": "hosted",
                        "selection": snapshot})
    if len(matches) != 1:
        raise ValueError("expected exactly one matching successful hosted release build")
    if binary is not None:
        matches[0]["binary_sha256"] = hashlib.sha256(binary.read_bytes()).hexdigest()
    return matches[0]


def main():
    p = argparse.ArgumentParser(__doc__)
    sub = p.add_subparsers(dest="command", required=True)
    f = sub.add_parser("freeze")
    f.add_argument("--url", default="https://build.jcode.sh/v1/toolchain/current?release=1")
    f.add_argument("--output", required=True)
    f.add_argument("--protocol-source", type=Path, required=True)
    for name in ("env", "attest"):
        a = sub.add_parser(name)
        a.add_argument("--snapshot", required=True)
        a.add_argument("--sha256", required=True)
        a.add_argument("--target", required=True, choices=TARGETS)
        a.add_argument("--output", required=True)
        a.add_argument("--protocol-source", type=Path, required=True)
        if name == "attest":
            a.add_argument("--since", type=float, required=True)
            a.add_argument("--binary", type=Path, required=True)
            a.add_argument("--source-sha", required=True)
            a.add_argument("--runs", type=Path, default=Path.home() / ".justrust/runs")
    args = p.parse_args()
    if args.command == "freeze":
        if not args.url.startswith("https://"):
            raise ValueError("release policy requires HTTPS")
        text = subprocess.run(["curl", "--fail", "--silent", "--show-error", "--proto", "=https", "--proto-redir", "=https", "--connect-timeout", "5", "--max-time", "15", "--max-filesize", "65536", args.url], check=True, stdout=subprocess.PIPE).stdout
        if len(text) > 65536:
            raise ValueError("oversized policy")
        snapshot = freeze(parse(text))
        check_protocol_source(snapshot["policy"], args.protocol_source)
        Path(args.output).write_text(canonical(snapshot) + "\n")
        print("policy_sha256=" + digest(snapshot))
    else:
        snapshot = validate(json.loads(Path(args.snapshot).read_text(), object_pairs_hook=pairs), args.sha256)
        check_protocol_source(snapshot["policy"], args.protocol_source)
        if args.command == "env":
            Path(args.output).write_text("".join(f"{k}={v}\n" for k, v in environment(snapshot, args.target).items()))
        else:
            if not re.fullmatch(r"[0-9a-f]{40}", args.source_sha):
                raise ValueError("release source must be an exact commit SHA")
            result = attest(snapshot, args.target, args.since, Path.cwd(), args.runs, args.binary)
            result["source_revision"] = args.source_sha
            Path(args.output).write_text(canonical(result) + "\n")


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError, KeyError, IndexError, subprocess.CalledProcessError) as error:
        sys.exit("release policy refused: " + str(error))
