# Exact usefulness for existing typed pattern decisions

Status: pending implementation acceptance. Full Rust compatibility remains open.

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
payloads, unsupported shapes or by-value cycles. Parameters/references remain
opaque under existing universal rules. This is not inferred hidden-field
uninhabitance. Metadata failures and every resource/allocation error propagate,
never become out-of-scope, non-exhaustive, useless or exhaustive.

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
