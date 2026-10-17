# Checked structural token foundations

This checkpoint exposes the unstable `compiler::token_primitives` utility API.
It is not a Rust lexer, hygienic resolver, macro matcher, expansion driver or
production replacement for the parser's existing lexer. It does not establish
full Rust support, a new edition contract or a performance improvement.

## Ownership and provenance

`SourceSnapshot` takes a source map by value and seals it behind immutable shared
ownership. Import checks the canonical source file-count and aggregate original
byte limits before scanning contents or validating line tables. Import does not
perform filesystem operations, cfg selection or source-language validation.
File handles carry their snapshot owner. Original ranges are checked for file
ownership, bounds, ordering and UTF-8 boundaries. The old raw `SourceSpan` is a
rendering result, not an input that can mint owned token provenance.

Sessions issue private owner-qualified symbols, spans, syntax contexts, source
root references and expansion records. Checked atomic identity allocation never
wraps. Source, session and stream handles from another owner are rejected.
Opaque IDs are local handles, not persistent cache identities or semantic
capabilities. They cannot grant source-probe metadata or canonical replay rights.

Root syntax context is explicit. Mark/context interning and earlier-record-only
parent/anchor references provide an acyclic structural representation, not Rust
mixed-site name resolution. Provenance walking is iterative, bounded and visibly
truncated. Original and synthetic origins remain distinct.

## Flat grouping

Grouping consumes supplied structural atoms, validates every referenced owner
and creates immutable flat nodes and child-edge arrays. Delimiter origins remain
independent. Tree IDs and ranges include session and stream ownership. Sharing a
stream shares its retained storage lease; dropping deep trees is nonrecursive.
Original-source and generated grouping use separately tagged depth policies.
The caller's policy choice is not proof that an input originated from a compiler
expansion. Input spelling, Unicode and literal semantics are not validated here.

Grouping charges work, storage and depth. The future lexer and expansion producer
must charge logical input/output occurrences exactly once. Grouping tests do not
constitute emitted-macro acceptance, and a kind-tagged token is not proof of
lexical validity.

## Resource accounting

The expansion authority is shared across clones. Defaults follow the explicit
new-session table in `MACRO-EXPANSION.md`, separate from generic planning caps.
Requested higher limits create an explicitly poisoned authority, not silently
raised or clamped caps. `check` and all admissions return the latched failure.
Zero limits are valid deterministic rejection configurations.

Work and produced counts are monotonic. Retained plus temporary bytes share one
ceiling. Invocation output/state admissions validate their local and session
resources atomically. Live lease/state storage is released even after poison,
without clearing the first failure or refunding produced work. Lock ordering is
session before invocation. Typed limit, overflow, allocation and infrastructure
failures remain distinct.

`resource_accounting.rs` extracts the existing logical reservation telemetry from
IR without changing the `ir::ReservationAccounting` public path or planning
policy. Both budgets reuse it. Resize/promotion do not create phantom leases or
charge bytes twice. Telemetry can saturate independently of exact admission and
is not process RSS or allocator instrumentation.

Bulk vector/string capacity is leased before fallible reservation and reconciled
against actual capacity. If reconciliation fails, unpublished backing storage
and its lease are dropped before the exact original error returns. Standard
library shared-control allocation behavior remains distinct from deterministic
resource-policy rejection. No process-wide OOM-recovery claim is made.

## Remaining integration

The production parser continues using the delivered immutable cursor over the
old lexer. Actual lexical validation, version-pinned Unicode XID classification,
prepared-source occurrence projection, contextual AST identifiers, fragment
parsing, matching/transcription, cfg-before-I/O generated discovery and graph
completion remain separate required gates. Structural utility tests cannot close
those language or full-compatibility requirements.

## Frozen acceptance, 2026-10-17

- All-target hosted check: `20261017-061711723-464224`.
- External public-facade tests: `20261017-061716625-464709`, 20 tests.
- Session/source-import/capacity tests: `20261017-061723730-465322`, 12 tests.
- Shared expansion budget: `20261017-061732087-465732`, 22 tests.
- Extracted accounting: `20261017-061732530-465765`, 2 tests.
- Existing planning telemetry: `20261017-061733341-465974`, 11 tests.
- Complete hosted regression: `20261017-061735037-466211`, 1828 tests in 94
  binaries, no failures, ignored or filtered tests.
- Strict all-target lint: `20261017-061750141-467025`.

Format and whitespace checks pass. Independent review covered resource accounting
and source/session/tree ownership. Review exposed an initially unbounded public
source-map import and retained backing after failed capacity reconciliation.
Both were repaired before publication, with exact/one-over import tests and a
deterministic rounded-capacity cleanup regression. External tests also prove a
caller's cloned source-map mutation cannot change sealed files or locations.

The first strict-lint run `20261017-061011912-431049` failed on deprecated atomic
allocation API use and a test initializer. A portable checked compare/exchange
loop preserves exhaustion behavior; no warning suppression was added. That failed
receipt and the eight final successful receipts are retained in
`data/runs/xps14/2026-10-17-token-primitives.jsonl`. They establish utility and
regression gates, not a compiler speed comparison or native macro acceptance.
