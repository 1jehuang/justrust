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

## Test-suite progress, not a separate status checklist

`data/compiler-compatibility-scope.json` declares scoped test obligations grouped
by compiler area. Every requirement has four compulsory layers: feature behavior,
invalid-input rejection, feature interactions, and end-to-end execution. A layer
contains exact existing integration test identities, or an explicit empty array
when tests are missing. There are no manually maintained passing/support states.
An unsupported-valid rejection test must never be mapped as feature acceptance.
The original broad full-suite gates remain available independently.

```sh
# Inventory and denominator, with missing and unexecuted tests visible.
python3 tools/conformance.py coverage
python3 tools/conformance.py coverage --json
# Inspect before running. Exact selectors must execute one test, never zero.
python3 tools/conformance.py track --area interactions --dry-run
python3 tools/conformance.py track --area memory --dry-run
# All mapped scenarios, or use --area / --case suite::test for a bounded run.
python3 tools/conformance.py track --timeout 600 \
  --out "$JCODE_SCRATCH_DIR/compatibility-current"
python3 tools/conformance.py coverage \
  --evidence "$JCODE_SCRATCH_DIR/compatibility-current"
```

Use a new output directory per run. Run on a stable source state. `track --jobs 2`
runs at most two independent exact-case processes concurrently (allowed range
1 through 8, default 1), keeping separate full logs, every failure, and deterministic
receipt ordering. There are no retries. Gate `evidence_notes` retain reviewed
limitations such as utility-only assertions, narrow positive partitions, fixed
expected outputs rather than live stock comparison, and incomplete language coverage.
A green mapped scenario does not discharge those broader limitations. `track` uses
the installed stable justrust client, preserves exact commands and full logs,
and binds results to the manifest, source revision/input digest, client binary
and relevant environment. `coverage --evidence DIR` recomputes results from
those logs. Multiple `--evidence` directories combine independently run cases.
Conflicting current-source outcomes stay conflicting, never silently choose a
passing rerun. Source, runner, environment or suite changes make results stale.
Old suite runs without exact case provenance are not silently imported.
Inventory-only coverage works without Git in hosted source snapshots, but it
verifies no results. Retained executable evidence always requires provenance.
Tracked cases use `--include-ignored`: a newly discovered valid-but-rejected
scenario can be marked as a known gap in the ordinary regression suite while
remaining an actual failing test in progress runs. Neither ignored nor
expected-unsupported tests count as feature support.

Reports separately expose **test-mapping coverage** (declared layer obligations
with exact tests), **verified test obligations** (current passing evidence), and
**full Rust completion** (unavailable while the semantic inventory is incomplete).
`measurement_readiness` and `missing_test_obligations` provide a machine-readable
work queue. Merely adding mappings cannot improve verified support; merely
running tests cannot make an incomplete inventory complete.

The displayed test-pass percentage is **passing declared test obligations**, not percent
of all Rust. Missing layers stay in the denominator. A requirement only has all
declared gates green when every mapped case in all four layers passes. A green
row proves its precise test scenarios, not every semantic rule implied by its
title. Unmapped integration suites are listed for further mapping, not treated
as absent implementation. The scope is explicitly incomplete and expanding it
can lower the percentage. No completion date follows from this number.

The separate interaction matrix enumerates every pair of its declared bounded
semantic axes, including pairs without tests. Multi-feature scenarios execute
as written; pair coverage requires every declared scenario for that pair to
pass. This is a systematic denominator within those axes, not exhaustive
coverage of all language-feature pairs, values, orders or higher-order programs.
Adding an axis automatically exposes all its missing pairs. Keep individual
feature, rejection, pairwise and whole-project coverage visible separately.

As features advance, add unchanged valid and invalid oracle scenarios, map their
exact test identities, and add interactions and representative programs. Do not
shrink the denominator or relabel a valid rejection to inflate progress. Both
Python adversarial tests and the ordinary `conformance_program` suite validate
the tracker. CI validates mappings and accounting without claiming an unrun
compiler acceptance result. Reports are local/CI evidence, not signed proofs.

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
