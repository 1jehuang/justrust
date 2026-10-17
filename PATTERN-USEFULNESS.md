# Exact usefulness for existing typed pattern decisions

Status: accepted scoped implementation, 2026-10-17. Full Rust compatibility remains open.

## Scope and authority

Extend coverage/usefulness for the already checked Bool/Int/enum decision
conjunctions inside existing tuple, record, tuple-struct, enum payload and fixed
array patterns. No new syntax, source node, binding/callable identity, IR or CFG
variant is added. Guards, or/range/@/reference patterns and ordinary refutable
let remain outside this contract. Usefulness is not move or tag authority.

Every row is fully source/schema checked before coverage. The proof uses exact
typed physical projection paths and existing nominal application facts, never
spelling, field count or source-span guesses. Bindings erase to unconstrained
coverage positions only; actual source bindings and successful moves are not
rewritten. Every dead/subsumed arm still imposes all source and IR obligations.
Subsumption is not an ordinary semantic error merely because stock warns.

## Exact sparse partition proof

The kernel owns metered copies of actual tests, not synthesized omitted fields.
A row is a conjunction. Usefulness asks whether the candidate intersects a cell
not covered by earlier rows; exhaustiveness asks whether any root cell remains.
An explicit iterative DFS keeps reversible exact-path assignments. Query tests
are considered first, then viable prior rows. Selecting the first unassigned
atom requires every earlier test in that row to be true, so nested Downcasts
are inspected only under their actual successful tag prerequisites.

Split Bool into two values, enums into actual variants of the exact application,
and integers into distinct mentioned singleton values plus a symbolic OTHER
cell when nonempty. OTHER is not a fabricated IntValue. Width/signed bits and
full 128-bit cardinality must avoid overflow, truncation and premature default.
Equal paths share assignments; typed sibling paths remain independent product
components. Do not pre-expand arrays or a Cartesian product of omitted fields.

Exact admission applies once per domain. A separate explicit out-of-scope
inhabitance result retains the previous conservative proof for Never/empty-enum
payloads or unsupported shapes. By-value type cycles are contract errors, not
conservative admission, and cannot suppress validation of other metadata.
Parameters/references remain opaque under existing universal rules. This is not
inferred hidden-field uninhabitance. Metadata failures and every resource or
allocation error propagate, never become out-of-scope, non-exhaustive, useless
or exhaustive.

## Lifetime and resource contract

`insert_row` validates and copies every supplied test before any covered shortcut,
queries against prior rows only, then commits the complete row transactionally.
Failure leaves prior rows intact and the first latched failure unchanged.
All rows, paths, assignment trails, branch/frame storage and comparisons use
existing checked work/count/depth/byte limits and actual-capacity leases.
Payload backing drops before leases. Exponential worst-case search terminates
with a typed resource error, not a guessed semantic answer. No limit is raised.

Checker queries precede local publication and arm reachability selection.
Let-else still checks the real else in its old scope and requires actual Never
type/completion before publishing enclosing bindings. All runtime predicate,
refinement, cleanup, selected-Copy and non-Copy indexed-move fences are unchanged.

## Required acceptance

Freeze exact stock-first complete/holed/overlapping products, nested tags,
integer complements/widths, all four clients, dead invalid bodies, once-only
sources and non-Copy failure reuse. Public controls start from real checked
plans and canonical facts; malformed covered/dead rows must still reject.
Require workers 1/8 CHECK/BUILD/no-fallback ELF execution, debug/release profiles,
strict lint/formatting, complete unfiltered regressions and retained failed and
passing receipts. Earlier source witnesses may be promoted only byte-for-byte.
No speedup, public ABI, complete pattern grammar or full Rust claim follows.

## Accepted evidence

- Stock32 sources: 24 valid and 8 invalid. Native classification is 21 runtime,
  6 ordinary invalid, 3 stock-valid unsupported and 2 stock-invalid/native-
  unsupported policy boundaries. The latter are not ordinary-invalid parity.
- Workers 1/8 CHECK/BUILD with compiler fallback disabled exercise 128 cells
  and 42 nonempty-output ELF executions per debug/release bootstrap profile.
  All 32 source bytes/order independently match retained stock measurements.
- 24 public source/checked-plan authority controls retain malformed covered
  rows, prior-tag ownership, selected binding and canonical-facts checks.
- Four historical outcome transitions preserve all 133 source literals/order
  in pattern_decisions, payload_enums and match. Including these suites, each
  profile checks 660 native cells and 194 ELF executions; release runs 137 tests.
- Library: 1,046 passed. Complete hosted regression: 2,678 passed across 132
  binaries, zero failed/ignored/filtered. Strict all-target lint, formatting and
  diff checks passed. Full receipt: 20261017-132808398-2425432; release receipt:
  20261017-132824911-2425281. Failed and passing receipts are retained in
  data/runs/xps14/2026-10-17-pattern-usefulness.jsonl.
- Private controls cover full u8 cardinality under unchanged default budgets,
  symbolic i128/u128 OTHER, distinct payload variants, dead malformed rows,
  transactional row admission, byte cleanup and latched work failure. A failed
  work admission cannot later become a cheap successful coverage answer.
