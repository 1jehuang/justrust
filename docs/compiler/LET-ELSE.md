# Genuine let-else and persistent binding authority

Status: accepted bounded milestone on 2026-10-17. This is not full Rust support.

## Frozen source contract

`Stmt::LetElse { pattern: Box<Pattern>, ty: Type, value: Box<Expr>, else_block: Block }` retains real source syntax and requires the actual terminating semicolon. The initializer and else block are parsed and resolved under the preceding lexical frontier. Genuine pattern BindingIds are published into the enclosing scope only after the else block and semicolon. They are not visible in their own initializer or else block.

Initializer grammar is checked from actual consumed typed tokens, not a printed or expanded expression. Unparenthesized closing braces and top-level lazy boolean operators retain Rust's restrictions. Parenthesized and opaque macro grouping must not be guessed from expanded AST shape.

## Explicit checked IR and storage

`ir::Stmt::LetElse { value: ExprId, ty: Ty, pattern: MatchPattern, else_branch: ExprId }` is a real persistent-binding statement. No source Match, synthetic identifiers, source body identities or arm-local lifetime promotion are permitted.

The checked domain is explicit even for a Never initializer. Existing pattern schema, exact nominal owner, variant, projection, freshness, privacy and Copy-only array policies remain authoritative. The actual else expression must have both Never type and Never completion. Source Unit with noncompletion is not sufficient.

The checker collects a bounded metered pending plan without publishing LocalIds, checks the initializer and else under the old environment, then publishes genuine successful bindings in the containing block. Existing match-like constructs reuse the same checked pattern planning without changing arm lifetimes.

## CFG and failure divergence

The initializer is evaluated exactly once. Genuine scalar or variant tests precede all selected binding consumption. Successful binding StorageLive and assignments belong to the enclosing scope and survive the statement. The failure branch has no successful binding initialization or selected moves.

Else metadata alone is not proof of divergence. A bounded metered actual CFG reachability check must reject a live path from the failure entry to the post-else continuation. A default Unreachable terminator, forged Never metadata or an empty block must not be accepted as proof. Return, break, continue and true divergence preserve their real targets. Existing ownership, refinement and non-Copy indexed-move fences remain unchanged.

## Acceptance gates

Require stock-first byte-preserved grammar and runtime witnesses, worker-1/8 native CHECK/BUILD cells with no compiler fallback, executable successful and failing-pattern paths, actual binding use after the statement, initializer side effects once, macro and imported constructor identity, else return/break/continue and Never distinctions.

Public authority controls must reject escaped bindings in initializer/else/earlier statements, foreign unused children, wrong actual nominal/downcast/payload/local metadata, nondivergent else, forged empty Never blocks and premature successful storage cleanup. Validate all children even for irrefutable, dead or Never-initializer cases.

Complete debug and release corpus/authority gates, strict all-target lint, formatting, full unchanged regressions and retained successful and failing run receipts are required before acceptance. Refutable payload matrices, reference ergonomics, or-patterns and non-Copy array element moves remain outside this slice.

## Observed acceptance

- Stock33 sources: 23 valid and 10 invalid, with the original first16 source bytes/order preserved. Native partition: 19 Runtime, 10 Invalid, 4 stock-valid Unsupported. Worker1/8 CHECK/BUILD yields132 cells per debug/release bootstrap profile,264 total. Each profile executes38 main images, including4 deliberate empty-main executions for2 exact Never-initializer controls. Divergent value functions are not executed by those controls.
- Public authority26 validates genuine continuation storage, oldscope binding identity, actual enum ownership and payload paths, failure Never metadata and forged empty Never blocks. Structural IR acceptance is not treated as a substitute for CFG/ownership validation.
- The historical irrefutable tuple let-else witness retains identical source and empty output, with parsing, checked IR, actual ELF execution and exit status now required. The historical nondivergent else witness retains identical source and moves from parser rejection to its actual checker rejection.
- Temporary pending rows, copied projections, scratch paths and CFG reachability backing retain actual-capacity leases until backing storage drops. New allocation failure is latched; no injected allocator-null execution claim is made.
- Final all-target check20261017-123801483-2189937; debug89 tests20261017-123803698-2190285; release89 tests20261017-123810200-2190624; strict lint20261017-123826007-2191780. These89 include the new32 source/authority tests and57 historical pattern tests.
- Full unchanged regression20261017-123833686-2189766:2546 passed across126 binaries, zero failed, ignored or filtered. This includes1011 library tests. Formatting and diff checks pass.
- All16 cited successful and failing receipts are retained in data/runs/xps14/2026-10-17-let-else.jsonl. No compiler speedup or full-Rust compatibility claim follows from these tests.
