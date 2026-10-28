#!/usr/bin/env python3
"""Verify the real SUMMARY request protocol, not a rewritten Rust model.

Usage (from the repository root):
  python3 experiments/dependency_proof/check_summary.py --verus /path/to/verus
Optional --rustup-home and --cargo-home select a scratch-only verifier toolchain.
Pinned verifier: Verus 0.2026.10.26.d17fbd0, official Rust 1.99.1, bundled Z3.
No verifier libraries are production Cargo dependencies.

Proof boundary: Verus checks src/compiler/summary_node/requests.rs directly,
including the implementation of its constructor and request transition, with
--no-cheating. The client theorem proves exactly one emission of the original
instance words followed by terminal silence. Runtime selection of LOCAL_OPT
output zero, async graph execution, SSA summarization, and upstream dependencies
are NOT formally verified here. tests/graph_integrity.rs exercises the adapter's
real direct edges. Neither check establishes necessity or transitive minimality.
The adapter check below is only a fail-closed textual integration tripwire, not
an implementation-correspondence proof or a general Rust effect analysis.
"""
import argparse
import os
from pathlib import Path
import re
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent
PROTOCOL = ROOT / "src/compiler/summary_node/requests.rs"
ADAPTER = ROOT / "src/compiler/summary_node.rs"
HARNESS = HERE / "summary.rs"
VERSION = "0.2026.10.26.d17fbd0"


def run(command, env):
    result = subprocess.run(command, env=env, text=True, stdout=subprocess.PIPE,
                            stderr=subprocess.STDOUT, timeout=60)
    print(result.stdout, end="")
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--verus", default=os.environ.get("VERUS", "verus"))
    parser.add_argument("--rustup-home", type=Path)
    parser.add_argument("--cargo-home", type=Path)
    args = parser.parse_args()
    env = os.environ.copy()
    if args.rustup_home:
        env["RUSTUP_HOME"] = str(args.rustup_home.resolve())
    if args.cargo_home:
        env["CARGO_HOME"] = str(args.cargo_home.resolve())
        env["PATH"] = str(args.cargo_home.resolve() / "bin") + os.pathsep + env["PATH"]
    version = run([args.verus, "--version"], env)
    if version.returncode or VERSION not in version.stdout:
        raise SystemExit(f"Expected pinned Verus {VERSION}")
    adapter = ADAPTER.read_text()
    # These anchors deliberately require review when the tiny adapter changes.
    if (adapter.count("cx.request(") != 1 or adapter.count("cx.read_fact") != 1
            or "keys::words(LOCAL_OPT, instance.to_vec())?" not in adapter
            or "cx.request(input).get(0)" not in adapter
            or "while let Some(instance) = requests.next()" not in adapter):
        raise SystemExit("SUMMARY adapter changed: review the unverified bridge")
    original = run([args.verus, "--no-cheating", str(HARNESS)], env)
    if original.returncode or not re.search(r"3 verified, 0 errors", original.stdout):
        raise SystemExit("Actual-source protocol proof failed or coverage changed")
    source = PROTOCOL.read_text()
    harness = HARNESS.read_text()
    needle = '#[path = "../../src/compiler/summary_node/requests.rs"]'
    if harness.count(needle) != 1:
        raise SystemExit("Harness no longer imports the production source directly")
    mutations = {
        "repeat-request": ("self.requested = true;", "self.requested = false;"),
        "wrong-instance": ("Some(self.instance)", "Some(&[])"),
        "missing-request": ("requested: false,", "requested: true,"),
    }
    with tempfile.TemporaryDirectory(prefix="summary-proof-", dir=env.get("JCODE_SCRATCH_DIR")) as tmp:
        directory = Path(tmp)
        for name, (old, new) in mutations.items():
            if source.count(old) != 1:
                raise SystemExit(f"Mutation target changed: {name}")
            mutant = directory / "requests.rs"
            mutant.write_text(source.replace(old, new))
            client = directory / "summary.rs"
            client.write_text(harness.replace(needle, '#[path = "requests.rs"]'))
            result = run([args.verus, "--no-cheating", str(client)], env)
            if result.returncode == 0 or "postcondition not satisfied" not in result.stdout:
                raise SystemExit(f"Expected contract violation for {name}")
            print(f"PASS: rejected {name} mutation")
    print("PASS: actual Rust protocol, client theorem, adapter tripwire, 3 rejected mutations")


if __name__ == "__main__":
    main()
