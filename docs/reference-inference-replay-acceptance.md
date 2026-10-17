# Narrow original-call reference inference: acceptance

Date: October 17, 2026. This closes the first slice of
`reference-inference-replay-contract.md`, not general reference inference or full Rust.
No compile-time speedup is claimed.

## Delivered domain

The native source graph publishes an immutable, exact-source capability after
original-call replay and mandatory structural validation. It admits omitted
actuals for an ordinary one-parameter identity call when its argument is a shared
reference variable from a parameter or an earlier explicitly annotated root
local, in a direct unannotated root-let initializer with no result expectation.
Public Program-taking legacy adapters do not gain this capability. Mutable
inference, result-context inference, inferred method actuals, nested contexts and
source borrow expressions remain outside this slice.

The implementation shares the inference state transition kernel with the legacy
checker. Original source occurrences, lexical binding origins, independent
statement-emission positions and the preadjustment frontier are retained and
validated. Every native Source body retains its publication, including unused
and zero-call bodies. Recipe handling remains on its separate existing typed path.
Generic substitution retains the original publication through the exact verified
template and body-substitution proof.

## Requirement-to-observation map

| Requirement | Executed observation |
|---|---|
| Shared parameter and annotated-local inference | Both frozen sources passed actual `jr-rust` CHECK and BUILD at workers 1 and 8. Each BUILD emitted a regular ELF executable whose run exited 0 with exactly `inference local ok\n` and empty stderr. |
| Lifetime safety is checked, not hidden as unsupported | Frozen eligible local escape failed actual CHECK and BUILD at workers 1 and 8 with `InvalidReference`, no executable, and neither Unsupported nor ResourceLimit. Its independently frozen stock result is E0515. |
| Exact owner, source Arc, facts, ledger, original bindings and unused metadata | Executed IR/CFG and frontend regression groups reject transplanted/edited source, raw forged traces, mismatched facts/ledger, missing zero-call publication, unused parameter/root-local swaps and a pre-capture mapping swap. |
| Generic original-to-Closed transport | Executed CFG group validates the exact original template/publication and body-substitution phase bridge. |
| Legacy adapters and explicit source behavior | Existing adapters and their cloned Program fixtures pass the full suite. New frontend groups retain the inferred-reference fence on raw adapters. |
| Existing 4096-argument limit | The unchanged backend argument-bound test passes with the original 128MiB and work/node caps. |
| Frozen carrier boundaries | All 32 unchanged cases pass: 16 accepted, 12 invalid, 4 Unsupported. This includes X29 `id(&v)`, which is not the newly admitted shared-Var argument form. |
| Source-order boundaries | All six unchanged witnesses were rerun through actual CLI workers 1 and 8. Explicit mutable reborrow passes and runs; the five omitted mutable/shared-expression/result-context witnesses remain Unsupported. |
| Existing source-to-native integration and packaging | Hosted `justrust test --no-fail-fast` passed 1,480 tests in 72 binaries. Strict `justrust clippy --all-targets -- -D warnings` passed. These include graph ordering, native output, existing corpus and failure-path tests. |

## Immutable public CLI receipts

The tested local CLI SHA-256 is
`50677dcc0cff723228bef7c43af26bcc29a2c09f48d8ffc291e77f73e54a1b91`.
It was built with `justrust build --local --bin jr-rust` specifically to exercise
the actual local executable, not a copied test implementation. Hosted checks,
tests and lint remained the normal validation path. The experimental CLI was
not installed over the machine's stable justrust command.

All receipts retain source bytes/hashes and raw process stdout/stderr, statuses,
worker counts and artifact observations:

- `experiments/results/20261017-reference-inference-shared-local-native.jsonl`
  SHA-256 `1f3f84e64debe9446467f68ed7865cf1eae5da53d3a4f806fd71e044ae654eac`.
- `experiments/results/20261017-reference-inference-eligible-escape-native.jsonl`
  SHA-256 `7e426aca055574b3c044eb7e786bec917fa21a71037c7c93b0cae7edc000843e`.
- `experiments/results/20261017-reference-inference-generic-carriers.jsonl`
  SHA-256 `0d8cd38d25e2e808cc4ca9065b779cb59b30b68518102ecfebd302ebc2582006`.
- `experiments/results/20261017-reference-inference-source-order-native.jsonl`
  SHA-256 `c26457a184e743597e7ae93e72052e788666dfa90d880cbd839953362c7cf7ed`.

The earlier stock-only/baseline receipts remain unchanged. No frozen case was
reclassified or repaired to obtain these results.

## Resource and compatibility corrections found by execution

The first integrated 4096-argument run exceeded the shared byte cap because
repeated publication validation rebuilt source inventories. A second run
exposed repeated structural/schema allocations for the same immutable source.
Both were corrected by reusing the privately validated exact immutable source
seal. The private factory still performs original replay and complete candidate
validation before publication. Reuse checks exact source/facts/owner/ledger;
mutable CFG payload snapshots and raw Legacy validation are unchanged. No cap
was raised. Temporary diagnostic instrumentation was removed.

The frozen X29 CLI check exposed missing public category text even though the
typed error was Unsupported and no artifact was emitted. The bounded root
renderer now includes the authoritative typed category, without substring
classification, and preserves diagnostic exhaustion as ResourceLimit.
An existing primitive-rvalue Clone test exposed its normal lexical block
wrapper: source call correspondence now points at that wrapper's actual tail
call while retaining the original single-evaluation wrapper in the IR.

New fixture setup errors were corrected only in new groups: missing main
functions and assuming an oversized failed budget charge consumed remaining
capacity. Existing corpus sources and assertions were not weakened.

## Independent static review

A fresh independent reviewer cleared the exact immutable publication/factory
seam, independent statement-emission binding witness, bounded typed diagnostic
renderer, and primitive-receiver wrapper correspondence. This was source
inspection only, not a claim that the reviewer executed the acceptance runs or
proved every compiler path.

## Recorded central runs

- Full suite: `20261016-182614825-2662754`, 1,480 passed, zero failed.
- Strict lint: `20261016-182619651-2662419`, passed with `-D warnings`.
- Original publication transport groups: `20261016-181411261-2508838`, three passed.
- Resource diagnosis: `20261016-181110392-2483090` and
  `20261016-181452119-2514494`, both retained as failed development evidence.

Build run IDs use the recorder's machine-local date convention. Raw acceptance
was performed on October 17 UTC. This document does not substitute earlier
stock results for current native behavior, and does not claim deferred domains
have been implemented.
