# Genuine let-else and persistent binding authority

Status: implementation pending acceptance. This milestone is not full Rust support.

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
