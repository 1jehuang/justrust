# Honest native if-let contract

Status: **implementation in progress, not accepted**. Full Rust remains open.

## Source and binding authority

Retain a genuine conditional-pattern source node. Do not parser-lower to a
fabricated Match, synthesize source binders or mint callable/body identities.
The frozen source API is `Expr::IfLet { pattern: Box<Pattern>,
scrutinee: Box<Expr>, then_block: Block, else_branch: Option<Box<Expr>> }`.
The pattern is boxed rather than enlarging recursive expression frames with
inline pattern storage. `None` means absent else; `Some` retains a genuine
Block, If or IfLet expression, never a fabricated wrapper. New parser boxes
use fallible fixed-layout allocation with storage admitted before allocation
and retained by the same NameStore owner as the AST.

Parse and resolve the scrutinee before introducing real then-only pattern
bindings. Enter a dedicated checked lexical frontier for the pattern and then
block, restoring it before else and after the expression, including error
paths. Actual source occurrences, constructor-use certificates and BindingIds
remain authoritative. No spelling/NameKey fallback repairs a contextual miss.

Source inventory, depth, clone, name validation and replay walkers visit the
actual scrutinee, pattern, then block and optional else in stable source-node
paths. Pattern locals do not gain direct-root reference replay eligibility.
All branches, including unused and unreachable syntax, remain universally
validated.

## Checked conditional semantics

The first slice reuses already-supported enum variant, scalar and irrefutable
aggregate pattern semantics, without general refutable payload matrices.
Existing array Copy-only restrictions and all nominal/privacy gates remain.
Chains, guards, or-patterns, while-let, refutable nested leaves and reference
pattern ergonomics stay explicitly unsupported.

The checker may lower a genuine source conditional pattern to existing typed
Match IR and a semantic wildcard fallback. This is not a fabricated source
Match. Preserve exact variant ownership, Downcast/Field/ConstantIndex plans,
fresh branch locals, exhaustive checking and selected-edge move behavior.
An omitted else contributes genuine Unit semantics. Require exact branch type
joins and divergence, including dead branch checking. Evaluate the scrutinee
once and do not consume omitted fields or bindings on a failed pattern edge.

## Acceptance gates

Require stock-first exact sources, native workers1/8 CHECK/BUILD without compiler
fallback, exact executable outputs and bootstrap debug/release. Cover outer-name
shadowing in the scrutinee, then-only visibility, restoration in else and after,
actual macro binders, nominal/privacy failures, conditional non-Copy moves,
side-effect ordering, omitted else and divergent branches. Public authority
controls must use real parsed owners and genuine checked plans. Preserve all
existing source corpora except individually justified newly-supported witness
promotions. Run complete hosted regression, strict all-target lint, formatting
and diff checks, retain exact failed/passing receipts, then commit/push the
bounded milestone. No full Rust, ABI or speed claim follows from this slice.
