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

`coverage --json` also exposes `selector_reconciliation_backlog`: sorted suites
and lexical test candidates, their source lines, explicit requirement/layer and
interaction-scenario memberships, and unmapped candidates. This includes tests
inside a partly mapped suite, which a suite-only backlog hides. The discovery
reuses literal `#[test]` leads and the existing `conformance_generated` `probes!`
convention, not Rust parsing or authenticated libtest enumeration. Comments,
embedded source, nested modules, conditional attributes and other macros require
review. No ignore, execution, semantic eligibility or passing state is inferred.
Untriaged candidates may be utilities or unsupported boundaries, not missing
language features. Neither an empty backlog nor its mapped fraction establishes
full Rust compatibility.

For a bounded check of actual runnable names, use the existing executor in
inventory-only listing mode. Explicit suites are required; other coverage/case
filters are rejected. The target is fixed to `x86_64-unknown-linux-gnu`.

```sh
python3 tools/conformance.py catalog --suite conformance_generated \
  --suite source_configuration --dry-run --out "$JCODE_SCRATCH_DIR/catalog-plan"
python3 tools/conformance.py catalog --suite conformance_generated \
  --suite source_configuration --timeout 300 \
  --out "$JCODE_SCRATCH_DIR/test-catalog-current"
python3 tools/conformance.py assess-catalog \
  --out "$JCODE_SCRATCH_DIR/test-catalog-current"
# Combine separately listed suite groups or a retry, retaining all attempts.
python3 tools/conformance.py catalog-coverage \
  --evidence "$JCODE_SCRATCH_DIR/test-catalog-current" \
  --evidence "$JCODE_SCRATCH_DIR/test-catalog-other-suites"
```

`catalog.json` retains source/client/environment identities, exact `--list`
commands and original hashed logs. Assessment reparses one complete libtest
listing per suite, including its count footer. Duplicate or truncated names,
test-execution output and replay diagnostics are not accepted as listings.
Current names are compared with lexical candidates and mapped selectors;
`listed_only`, `lexical_only` and `mapped_not_listed` expose differences requiring
review. Unfinalized or altered receipts are rejected, and stale receipts cannot
establish current names. Listing exits successfully only when every selected
suite has a current complete listing and every selected mapped test appears.
The raw executor deliberately reports these zero-execution invocations as
`unverified` conformance, while the catalog can report inventory status `listed`.
Its executed compatibility count is always zero. Catalog receipts are not
`track` evidence and must not be passed to `coverage --evidence`.

`catalog-coverage` requires retained catalog directories and accounts for **all**
current integration suites, not just the selected suites in one receipt. It exits
zero only when every suite has a current authenticated listing and all mapped
selectors appear. Canonical duplicate directories and identical listings count
once. A successful current retry may supply names after a failed or stale attempt,
but those earlier attempts remain visible. Differing current successful listings
for one suite are `conflicting` and supply no names. Missing, stale, failed,
unverified or conflicting inventories cannot make the complete gate pass.
This is a complete test-name inventory gate, not an execution or compatibility
gate. Its executed compatibility count remains zero and full Rust percentage
remains null, even when the inventory is complete.

```sh
# Inventory and denominator, with missing and unexecuted tests visible.
python3 tools/conformance.py coverage
python3 tools/conformance.py coverage --json
# Inspect before running. Exact selectors must execute one test, never zero.
python3 tools/conformance.py track --area interactions --dry-run
python3 tools/conformance.py track --area memory --dry-run
# Zero-based exact-case partitions after area/case filters, sorted by selected ID.
# Shard i/n selects sorted positions i, i+n, ...; empty shards are errors.
python3 tools/conformance.py track --area memory --shard 0/2 --dry-run
# All mapped scenarios, or use --area / --case suite::test for a bounded run.
python3 tools/conformance.py track --timeout 600 \
  --out "$JCODE_SCRATCH_DIR/compatibility-current"
python3 tools/conformance.py coverage \
  --evidence "$JCODE_SCRATCH_DIR/compatibility-current"
# Automation gate for the entire mapped exact-case manifest, not one shard.
# Repeat --evidence for every independently run shard directory.
python3 tools/conformance.py coverage --require-complete --json \
  --evidence "$JCODE_SCRATCH_DIR/compatibility-current"
```

The optional `coverage --require-complete` gate exits zero only when every
mapped exact case has current, nonconflicting passing evidence. It requires
`--evidence` and is invalid for other actions. The full summary is still emitted
on exit 1 for incomplete or failing campaigns. Malformed or interrupted receipts
are rejected with exit 2 rather than patched or silently imported.
`campaign.complete_current_measurement` distinguishes a complete pass/fail
measurement from an incomplete campaign; a complete measurement containing
failed cases still has `campaign.all_cases_passed=false` and fails the gate.
Unmeasured, stale, conflicting, timed-out, infrastructure and unverified cases
remain unresolved. Completion here covers only the mapped exact tests. It does
not fill missing layer obligations or certify the full Rust inventory.

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
remaining an actual failing test in progress runs. New semantic-invalid probes
use `InvalidAfterParse`: they require healthy complete capture, the original
input parse publication and complete declaration facts, not merely a parser
refusing an unsupported construct. Header parsing alone is insufficient because
body parsing is lazy. These are necessary admission checks, not a claim that
every native diagnostic is already typed or matches stock exactly. Neither ignored nor
expected-unsupported tests count as feature support.

Coverage also counts unique exact tests separately from layer gates, so a test
mapped to several gates is not counted as several independent executions.
Failed differential tests can emit versioned, diagnostic-only annotations:
`valid-source-rejected`, `invalid-source-accepted`, `semantic-stage-unproven`,
and `rejection-proof-unproven`. A stage witness can fail even when an early
diagnostic correctly rejects invalid Rust. A rejected valid source can reflect
unsupported input or resource exhaustion rather than wrong-code behavior.
These log annotations are triage leads, not authenticated diagnostic categories
or compatibility proof. They never turn a failed test green. Stale, conflicting,
and successful executions do not supply current failed-case annotations, and
unannotated failures remain explicit rather than being guessed from prose.

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
pass. `interaction_matrix.combination_accounting` separately reports authored
pair and triple partitions, their authenticated outcome counts, and unauthored
combinations. Each combination requires all tests in every matching authored
scenario to pass. Adding a passing scenario cannot hide another scenario's
failure or missing evidence. Missing-combination lists are lexicographic samples
capped at 100 entries, with exact total counts and explicit truncation flags.

These are combinatorial counts over the declared axes, not a reviewed denominator
of applicable language requirements. Applicability remains unreviewed, and an
unauthored combination is not automatically a normative obligation. No pair or
triple count establishes exhaustive coverage of values, orders, programs, or
higher-order interactions. Keep individual feature, rejection, pairwise,
higher-order and whole-project evidence visible separately.

As features advance, add unchanged valid and invalid oracle scenarios, map their
exact test identities, and add interactions and representative programs. Do not
shrink the denominator or relabel a valid rejection to inflate progress. Both
Python adversarial tests and the ordinary `conformance_program` suite validate
the tracker. CI validates mappings and accounting without claiming an unrun
compiler acceptance result. Reports are local/CI evidence, not signed proofs.

## Discovery and development loop

### Versioned compatibility target

`data/compiler-compatibility-contract.json` defines the target independently of
the current test inventory: Edition 2024, Linux x86-64, and the exact stock
compiler verified by `tests/support/stock_rustc.rs`. Language, core/alloc/std,
project behavior, execution correctness and ecosystem coverage are distinct
dimensions. Nightly extensions and other targets need separate contracts.
Changing the oracle requires a new contract identity and fresh measurements.

Source-referenced inventories are discovery work, not passing test evidence.
References to moving official documentation must disclose that they are not
frozen documentation for the pinned oracle. Mapping an inventory obligation to
an existing broad requirement does not establish that all its behavior is
tested. Inventories remain unreviewed until omissions and version differences
have been checked, independently of compiler pass rates.

`python3 tools/compatibility_inventory.py` validates both source-referenced
inventories and the pinned contract, then reports their obligation counts and
unmapped existing requirements. `conformance.py coverage --json` includes the
same readiness summary. The inventories are
`data/compiler-language-inventory.json` and
`data/compiler-library-project-inventory.json`. Their requirement links are
triage associations, not direct test links or execution evidence. Every new
obligation initially lacks direct exact-test evidence, even when its broader
existing requirement has passing scenarios. Ordinary tests and CI reject
malformed references, unknown mappings, duplicate identities and an oracle
contract that disagrees with the actual stock verifier.

`data/compiler-language-reference-rules.json` separately indexes the Reference
revision linked by the pinned compiler's source submodule. It retains page
hashes, source positions and rule labels, including unlinked Markdown pages.
Chapter adjacency is not a reviewed rule-to-obligation mapping. Rule labels
are also not a complete semantic inventory: unlabeled prose, grammar and
tables can impose additional requirements.

To reproduce the index, use `tools/extract_reference_rules.py` with
`--source-root`, `--pin`, `--compiler-commit`, `--inventory`,
`--source-archive`, `--archive-sha256` and a new `--output` path. The extractor
checks every local source file against the supplied archive and rejects links,
path escapes, changed files and duplicate rules. Official source URLs and
archive hashes are retained, not the full Reference prose. A supplied expected
hash establishes byte consistency, not independent authentication. Ordinary
metadata validation must not claim that external source bytes were reverified
when the archive was not supplied.

`data/compiler-library-surface-provenance.json` records bounded pinned library
source and rustdoc input provenance, not public API coverage. Its link/anchor
observations include unstable and other-target entries and must not become a
compatibility denominator. Reproduction commands and tool digests are retained
in that file. Full downloaded inputs remain external. Ordinary no-network tests
exercise synthetic rejection controls and explicitly skip unavailable real-input
fixtures. To repeat the real-input checks, set `JR_COMPAT_LIBRARY_FIXTURES` to
the acquisition directory containing `pinned-source-final` and
`pinned-html-final`, then run `tools/test_pinned_acquisition.py` and
`tools/test_compact_library_surface_provenance.py`. A skipped fixture check is
not proof of source-byte verification or API completeness.

`tools/classify_proc_macro_details.py` provides a bounded literal review of
retained `proc_macro` detail pages. It scopes stability annotations to their
item header or method summary, keeps const instability separate, and leaves
missing or conflicting evidence ambiguous. Public reachability, target
availability and semantic API identity remain unresolved. Its fresh scratch
output is not consumed as compatibility evidence or an API denominator. Run
`tools/test_classify_proc_macro_details.py` with the same fixture environment
to repeat exact-page and corruption controls. Full extracted sidecars are not
vendored into this repository.

`tools/token_reconciliation_proposals.py` contains explicit, unreviewed
classifications and missing-test needs for all 115 labeled blocks in the pinned
token chapter. Generate an exact-source-bound candidate after reproducing the
Reference inputs:

```sh
python3 tools/reconcile_token_rules.py generate \
  --rule-index data/compiler-language-reference-rules.json \
  --inventory data/compiler-language-inventory.json \
  --source-root "$REFERENCE_SOURCE_ROOT" \
  --artifact "$JCODE_SCRATCH_DIR/token-reconciliation.json"
```

Use `validate` with the same arguments to check a retained candidate. Generation
requires a fresh artifact path and cannot overwrite source inputs. The validator
requires all selected rule rows and all 2,850 unselected identities to remain
accounted for. Its tests reject omissions, foreign mappings and invented review
or execution credit. Structural validation does not establish that a proposed
normative interpretation is correct. All 2,965 rule mappings still require
review. The default candidate intentionally contains no direct test links.

For the pinned constant-evaluation chapter, add `--profile const-eval` to the
`generate` or `validate` command above and use a separate scratch artifact.
`tools/const_reconciliation_proposals.py` explicitly accounts for all 44 labeled
blocks in `src/const_eval.md`, including the distinction between promotion and
temporary lifetime extension, evaluated versus skipped faults, target-based
interpretation, and the specification's unresolved const-function type list.
The profile retains all 2,921 other rule identities, missing exact-test needs,
and zero review or execution credit. Its separate `build-witness-links`,
`validate-witness-links`, and `assess-witness-links` actions use the same explicit
`--profile const-eval` and a separate external scratch artifact. Six original
scalar-const selectors have 15 partial links across three rule identities.
Whole fixture/helper hashes and the pinned single-case literal/body hashes bind
the sources. These are deliberately narrow: observable positive values do not
prove evaluation timing, and three unused overflow controls do not establish
all const-error behavior. All missing needs remain open, even with passing
current receipts. Default token artifacts remain unchanged, and an artifact
from one profile cannot validate as the other.
Both profiles refer to the same 2,965-rule universe, not additive language
denominators. Proposed page decomposition is not tested Rust compatibility.

For the six explicitly authored token witnesses, use `build-witness-links`
with the same arguments and a new external scratch artifact path. Check that
separate artifact with `validate-witness-links`. These actions bind the exact
fixture, oracle, stock verifier and scope bytes, verify selector discovery and
scope membership, and retain every original rule and missing-test need. Source
changes require explicit review of the pinned source contract before regeneration.
The links are partial, unreviewed attribution, not passing results or completed
rules. They import no execution receipts and leave review and execution credit
at zero, including for the retained failing tuple-index witness.

Set `JUSTRUST_WITNESS_REFERENCE_ROOT="$REFERENCE_SOURCE_ROOT"` when running
`tools/test_reconcile_token_rules.py` to exercise actual pinned-source linkage
as well as the synthetic adversarial controls. Without that input, the actual
source test is explicitly skipped.

Use `assess-witness-links` with the same artifact arguments and repeated
`--evidence <track-directory>` to observe executions for those exact selectors.
It reuses the ordinary authenticated receipt and log checks, deduplicates report
directories, and keeps failed, stale, conflicting and untested states distinct.
Without evidence it reports all six selectors as untested and does not need an
installed runner. This read-only action reports observations, not an acceptance
gate: even a passing selector does not complete a rule or review an obligation.
Review and completed-rule credit remain zero and the full-Rust percentage stays
unavailable. Source or runner changes make earlier execution receipts stale.

Undefined behavior is not a runtime-output parity obligation. Unsafe
preconditions and compiler-enforced restrictions must be tracked separately.
Where documented behavior permits multiple outcomes, a differential oracle
must check the allowed outcomes rather than enshrine an accidental stock
result. Performance is a separate measurement, not semantic compatibility.

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
Its six seeded families attempt all four fixed seeds without retries, retain
seed/expected-output/full-source diagnostics, and fail if any attempt fails.
Three separate utility tests check generation and failure retention, not Rust
compatibility. It supplements, rather than replaces, unchanged real crates.
Its finite generator is not an exhaustive grammar, fuzzer or soundness proof.

`tools/bounded_failure_reduce.py` compares explicitly authored candidate files
using a trusted checker command, retaining the original source and every
attempt. For example:

```bash
python3 tools/bounded_failure_reduce.py \
  --original original.rs --candidate smaller.rs \
  --scratch-root "$JCODE_SCRATCH_DIR" --output "$JCODE_SCRATCH_DIR/new-reduction" \
  --max-attempts 8 --timeout 30 -- python3 /absolute/path/checker.py
```

The checker receives the attempted source path as its last argument. It must
validate stock acceptance and emit the exact JSON protocol described by the
tool's `--help`. Only the baseline's exact failure classification is retained.
Timeouts, malformed results, changed classifications and modified inputs do not
preserve a failure. Results describe the **best observed reproducer**, never a
globally minimal source. This utility does not parse or automatically shrink
Rust, authenticate tracker evidence, or sandbox checker commands. Attempt and
wall-time bounds do not bound checker disk use. Use trusted commands and bounded
input lists. A checker with the wrong stock version cannot establish pinned
compatibility evidence. The ordinary `conformance_program` gate supplies the
verified stock compiler and Cargo-built native binary to a separate reduction
smoke test at workers 1 and 8. Direct Python runs skip that smoke test unless
`JR_REDUCER_STOCK` and `JR_REDUCER_NATIVE` are explicitly supplied; synthetic
utility checks alone never substitute for it. Broader generators, automatic
candidate generation, sustained campaigns and applicable upstream conformance
imports remain open.
Imported inputs must retain licenses, attribution and pinned source identity.

`tests/conformance_program.rs` integrates manifest validation and adversarial
runner tests into ordinary `justrust test`. The checks reject missing suites,
unknown scopes, duplicate/missing gate results, zero-test false success, changed
commands, stale source/manifest identities, escaped/missing/tampered logs and
resource/infrastructure failures. None of this adds compiler facts, graph joins,
shared compiler budgets or a second compilation orchestrator. This is an
external acceptance driver, not part of the demand-driven compiler engine.
