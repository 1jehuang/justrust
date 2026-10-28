#!/usr/bin/env python3
"""Check the bounded Lean model and reject three dependency mutations.

No packages, network access, or global toolchain installation required.
Run from the repository root:
    LEAN=/path/to/lean-4.24.0-linux/bin/lean python3 experiments/dependency_proof/check.py

Trust boundary: the Lean 4.24.0 kernel and its standard foundational axioms,
the specified model, and this runner. No custom axioms or admitted proofs are
accepted. Signature.lean is executable Lean, not generated from or linked to
the Rust compiler. Its request traces describe that model only. The scoped
lookup is an abstract pure input, not a verified Rust graph-provider chain.
Therefore this experiment does not certify the current compiler, establish
Rust semantic correctness, or demonstrate a speedup. It proves sufficiency
and exact modeled dependencies for the bounded header language, with necessity
witnesses for one concrete two-fact case, not universal execution minimality.
A next production step needs a verified implementation correspondence or a
verified generator plus checks of the graph adapter and supporting producers.
"""
import os
from pathlib import Path
import re
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parent
LEAN = os.environ.get("LEAN", "lean")


def check(path):
    result = subprocess.run(
        [LEAN, "-DwarningAsError=true", str(path)],
        text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=60,
    )
    print(result.stdout, end="")
    return result


def main():
    version = subprocess.check_output([LEAN, "--version"], text=True)
    print(version, end="")
    if not re.search(r"version 4\.24\.0(?:,|\s)", version):
        raise SystemExit("Use pinned Lean 4.24.0 for this pilot")
    source = (ROOT / "Signature.lean").read_text()
    result = check(ROOT / "Signature.lean")
    if result.returncode:
        raise SystemExit("Positive proof check failed")
    audit = re.findall(r"depends on axioms: \[([^]]*)\]", result.stdout)
    if len(re.findall(r"^'DependencyPilot\.", result.stdout, re.M)) != 8:
        raise SystemExit("Missing theorem axiom audit")
    for axioms in audit:
        unexpected = set(axioms.split(", ")) - {"propext", "Quot.sound", "Classical.choice"}
        if unexpected:
            raise SystemExit(f"Unexpected proof axioms: {unexpected}")
    # Mutate the actual adapter, leaving its stated contracts/proofs unchanged.
    # The extra-type case is allowed by the coarse role policy, but violates
    # exact_dependencies. This checks more than just a ban on global roots.
    original = "(answer.1, .header owner :: answer.2.map Request.key)"
    if source.count(original) != 1:
        raise SystemExit("Mutation target changed")
    mutations = {
        "global-table": ".allSignatures",
        "unrelated-body": ".body 99",
        "unreferenced-type": ".typeDecl 99",
    }
    with tempfile.TemporaryDirectory(prefix="dependency-proof-", dir=os.environ.get("JCODE_SCRATCH_DIR")) as tmp:
        for name, key in mutations.items():
            path = Path(tmp) / f"{name}.lean"
            path.write_text(source.replace(original,
                f"(answer.1, {key} :: .header owner :: answer.2.map Request.key)"))
            result = check(path)
            if result.returncode == 0 or "error:" not in result.stdout:
                raise SystemExit(f"Mutation {name} was not rejected by the checker")
            if "exact_dependencies" not in result.stdout:
                # Lean reports a source location rather than theorem name.
                line = source[:source.index("theorem exact_dependencies")].count("\n") + 1
                error_lines = [int(x) for x in re.findall(r"\.lean:(\d+):\d+: error:", result.stdout)]
                if not any(line <= x <= line + 6 for x in error_lines):
                    raise SystemExit(f"Mutation {name} failed outside the intended contract")
            print(f"PASS: rejected {name} mutation")
    print("PASS: 8 audited theorems and 3 rejected dependency mutations")


if __name__ == "__main__":
    main()
