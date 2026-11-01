# Executable Rust compatibility program

The compatibility target is edition 2024, Linux x86-64, with the exact stock
oracle identified in `tests/support/stock_rustc.rs`. The full language, libraries
and project workflow remain the goal. A green subset, a legal-unsupported test,
a large test count, or an empty discovery queue does not establish full Rust.

## Three different questions

1. **What must work?** `data/compiler-conformance.json` indexes the language,
   ownership, abstraction, expansion/CTFE, runtime/ABI, upstream libraries,
   projects, graph, optimizer, evidence and completion obligations. Each row
   records remaining scope and runnable gates. These are area-level obligations,
   not an exhaustive rule-by-rule specification. The semantic inventory is
   explicitly incomplete.
2. **Does this revision satisfy the selected checks?** `tools/conformance.py`
   runs existing real acceptance suites through the installed stable `justrust`
   client. Existing suites remain responsible for stock/native CHECK/BUILD/run,
   workers 1/8, invalid unused items, forged facts, no fallback and graph shape.
   The runner does not substitute its own translation or infer language success
   from a rejected program. The full profile includes *all* test targets,
   including newly added ones, strict Clippy and formatting.
3. **Is full Rust established?** Not yet. Reports keep this false, even if every
   selected gate passes. Closing it requires a complete independent semantic
   inventory, unchanged upstream core/alloc/std and ecosystem evidence, target
   and edition review, sustained soundness/wrong-code campaigns and review of
   the remaining obligations in `RUST-COMPATIBILITY.md`. No test harness can
   prove an unbounded language correct merely by testing finite examples.

## Commands

From the checkout, with the stable client and the normal test environment:
Python 3.10 or newer is required, with no third-party Python packages.

```sh
python3 tools/conformance.py validate
python3 tools/conformance.py plan --profile full
python3 tools/conformance.py discover > "$JCODE_SCRATCH_DIR/discovery.json"
python3 tools/conformance.py run --profile smoke \
  --out "$JCODE_SCRATCH_DIR/conformance-smoke"
python3 tools/conformance.py run --gate references \
  --out "$JCODE_SCRATCH_DIR/conformance-references"
python3 tools/conformance.py run --profile full --timeout 21600 \
  --out "$JCODE_SCRATCH_DIR/conformance-full"
python3 tools/conformance.py assess \
  --out "$JCODE_SCRATCH_DIR/conformance-full"
```

Use a new output directory per invocation. A full run can be expensive. Run
`plan` first. The runner never retries a gate or stops after the first failure.
It cannot disable internal retries in the installed stable client. Exact client
diagnostics reporting failed-test replay or ignored failures make a zero-exit
gate unverified, never passed, even with a positive test count. Full logs, exit
status, timeout status, commands, source revision/input digest, suite inventory
and manifest digest accompany each run.
Changing compiler/test/runner inputs during a run invalidates its assessment.
An interrupted run retains partial evidence but cannot pass assessment. Reports
from a different revision cannot establish the current revision's result.
Routine results stay in scratch/CI, not committed Markdown receipts.

The runner classifies suite outcomes as passed, failed, timeout, infrastructure
or unverified. It deliberately does **not** guess whether arbitrary failed test
output means invalid Rust, legal unsupported input, resource exhaustion or wrong
code. That distinction belongs to the test oracle. Full failure output remains
available. Zero matched tests or silent successful test commands cannot pass.
Hashes detect accidental stale/tampered evidence, not a malicious operator who
can rewrite both a report and its logs. Reports are not signed attestations.

## Discovery and development loop

`discover` inventories every integration target and lexical unsupported/ignored/
wrong-code markers, plus direct stock-oracle selectors needing identity review.
Selectors can occur inside source strings or intentional bootstrap tests, so
they must not be automatically rewritten or counted as oracle failures.
Its markers are *triage leads*, not feature coverage or an
automatic semantic classification. Known real failures have explicit requirement
and test identities in the manifest. Never delete a valid unsupported source to
make a gate green. Promote it only after unchanged stock/native acceptance and
relevant invalid/forged/resource controls pass. A passing unsupported-boundary
regression is still an open compatibility gap.

`tests/conformance_generated.rs` supplies bounded deterministic cross-feature
programs with reproducible case identities and pinned stock/native comparisons.
It supplements, rather than replaces, unchanged real crates. Retain minimized
reproductions when generation finds a new failure. Its finite generator is not
an exhaustive grammar, fuzzer or soundness proof. Broader generators, automated
minimization and applicable upstream conformance imports remain explicit work.
Imported inputs must retain licenses, attribution and pinned source identity.

`tests/conformance_program.rs` integrates manifest validation and adversarial
runner tests into ordinary `justrust test`. The checks reject missing suites,
unknown scopes, duplicate/missing gate results, zero-test false success, changed
commands, stale source/manifest identities, escaped/missing/tampered logs and
resource/infrastructure failures. None of this adds compiler facts, graph joins,
shared compiler budgets or a second compilation orchestrator. This is an
external acceptance driver, not part of the demand-driven compiler engine.
