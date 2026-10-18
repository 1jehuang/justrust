#!/usr/bin/env python3
"""Freeze stock Drop observations. Never invokes the experimental compiler.

freeze records immutable inputs before compiling. observe checks those hashes,
then records stock compiler output and exact byte stdout. verify repeats stock
execution and compares stdout/status bytes without rewriting frozen receipts.
"""
import argparse
import base64
import datetime
import hashlib
import json
import pathlib
import subprocess
import tempfile

ROOT = pathlib.Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "tests/fixtures/drop_normal"
RUSTC = pathlib.Path("/usr/bin/rustc")
FLAGS = ["--edition=2024", "-C", "panic=abort", "-A", "warnings"]


def digest(data):
    return hashlib.sha256(data).hexdigest()


def stamp():
    return datetime.datetime.now(datetime.timezone.utc).isoformat()


def write_new(path, value):
    with path.open("x") as stream:
        json.dump(value, stream, indent=2, sort_keys=True)
        stream.write("\n")


def sources():
    return sorted(FIXTURES.glob("*/*.rs"))


def validated_sources(manifest):
    """Validate frozen membership independently of later corpus additions."""
    seen = set()
    result = []
    for item in manifest["sources"]:
        name = item["path"]
        relative = pathlib.PurePosixPath(name)
        if (relative.is_absolute() or len(relative.parts) != 2
                or relative.parts[0] not in {"positive", "negative", "unsupported"}
                or relative.suffix != ".rs" or str(relative) != name
                or item["classification"] != relative.parts[0] or name in seen):
            raise ValueError("unsafe or duplicate frozen source: " + name)
        path = FIXTURES / name
        path.resolve().relative_to(FIXTURES.resolve())
        if digest(path.read_bytes()) != item["sha256"]:
            raise ValueError("frozen source changed: " + name)
        seen.add(name)
        result.append(path)
    return result


def freeze(directory):
    directory.mkdir(parents=True, exist_ok=True)
    manifest = {
        "schema": 1, "frozen_at": stamp(), "compiler": str(RUSTC),
        "compiler_sha256": digest(RUSTC.read_bytes()),
        "version": subprocess.check_output([str(RUSTC), "-Vv"], text=True),
        "flags": FLAGS,
        "sources": [{"path": str(p.relative_to(FIXTURES)),
                     "sha256": digest(p.read_bytes()),
                     "classification": p.parent.name} for p in sources()],
    }
    write_new(directory / "manifest.json", manifest)
    print("FROZEN", len(manifest["sources"]), "sources", digest((directory / "manifest.json").read_bytes()))


def invoke(command):
    result = subprocess.run(command, capture_output=True, timeout=30)
    return {"command": [str(x) for x in command], "status": result.returncode,
            "stdout_base64": base64.b64encode(result.stdout).decode(),
            "stderr_base64": base64.b64encode(result.stderr).decode()}


def observe(directory, verify):
    manifest_path = directory / "manifest.json"
    manifest = json.loads(manifest_path.read_text())
    assert digest(RUSTC.read_bytes()) == manifest["compiler_sha256"], "compiler changed"
    assert manifest["flags"] == FLAGS, "flags changed"
    frozen = validated_sources(manifest)
    if not verify:
        assert sources() == frozen, "corpus changed after freeze"
    observations = []
    with tempfile.TemporaryDirectory(prefix="stock-bin-", dir=directory) as temporary:
        for index, item in enumerate(manifest["sources"]):
            path = FIXTURES / item["path"]
            binary = pathlib.Path(temporary) / ("fixture_" + str(index))
            result = {**item, "compile": invoke([RUSTC, *FLAGS, "--crate-name", "drop_fixture", path, "-o", binary])}
            if result["compile"]["status"] == 0:
                result["run"] = invoke([binary])
            negative = item["classification"] == "negative"
            assert (result["compile"]["status"] != 0) == negative, "classification mismatch: " + item["path"]
            if not negative:
                assert result["run"]["status"] == 0, "runtime failed: " + item["path"]
            observations.append(result)
            output = base64.b64decode(result.get("run", {}).get("stdout_base64", "")).decode()
            print(item["path"], "compile=" + str(result["compile"]["status"]), repr(output))
    receipt = {"schema": 1, "observed_at": stamp(), "manifest_sha256": digest(manifest_path.read_bytes()),
               "observations": observations}
    if verify:
        original = json.loads((directory / "stock.json").read_text())
        assert original["manifest_sha256"] == receipt["manifest_sha256"]
        for old, new in zip(original["observations"], observations, strict=True):
            assert old["path"] == new["path"]
            assert old["sha256"] == new["sha256"]
            assert old["compile"]["status"] == new["compile"]["status"]
            if "run" in old:
                for key in ("status", "stdout_base64", "stderr_base64"):
                    assert old["run"][key] == new["run"][key], new["path"] + " " + key
        print("VERIFIED exact stock stdout, stderr and exit status")
    else:
        write_new(directory / "stock.json", receipt)


def export(directory):
    """Publish a dependency-free Rust-test index from already observed bytes."""
    manifest_path = directory / "manifest.json"
    manifest = json.loads(manifest_path.read_text())
    receipt = json.loads((directory / "stock.json").read_text())
    validated_sources(manifest)
    assert receipt["manifest_sha256"] == digest(manifest_path.read_bytes())
    assert len(receipt["observations"]) == len(manifest["sources"])
    lines = ["# manifest_sha256=" + receipt["manifest_sha256"] + "\n"]
    for item, observed in zip(manifest["sources"], receipt["observations"], strict=True):
        assert item["path"] == observed["path"]
        assert item["sha256"] == observed["sha256"]
        assert digest((FIXTURES / item["path"]).read_bytes()) == item["sha256"]
        runtime = observed.get("run", {})
        fields = [item["classification"], item["path"], item["sha256"],
                  str(observed["compile"]["status"]), str(runtime.get("status", "none")),
                  base64.b64decode(runtime.get("stdout_base64", "")).hex() or "-",
                  base64.b64decode(runtime.get("stderr_base64", "")).hex() or "-"]
        lines.append("\t".join(fields) + "\n")
    with (directory / "fixture-index.tsv").open("x") as stream:
        stream.writelines(lines)
    print("EXPORTED observed stock bytes for", len(manifest["sources"]), "fixtures")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["freeze", "observe", "verify", "export"])
    parser.add_argument("--receipts", type=pathlib.Path, default=FIXTURES / "receipts-v9")
    args = parser.parse_args()
    if args.action == "freeze":
        freeze(args.receipts)
    elif args.action == "export":
        export(args.receipts)
    else:
        observe(args.receipts, args.action == "verify")


if __name__ == "__main__":
    main()
