# Honest native if-let contract

Status: **bounded native slice accepted**. Full Rust remains open.

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
Chains, guards, or-patterns, refutable nested leaves and reference
pattern ergonomics stay explicitly unsupported.

The checker may lower a genuine source conditional pattern to existing typed
Match IR and a semantic wildcard fallback. This is not a fabricated source
Match. Preserve exact variant ownership, Downcast/Field/ConstantIndex plans,
fresh branch locals, exhaustive checking and selected-edge move behavior.
An omitted else contributes genuine Unit semantics. Require exact branch type
joins and divergence, including dead branch checking. Evaluate the scrutinee
once and do not consume omitted fields or bindings on a failed pattern edge.

## Measured bounded evidence

The retained corpus contains 27 exact stock-first sources: 20 stock-valid and
seven stock-invalid, with native outcomes of 18 runtime successes, seven
ordinary rejections and two explicit unsupported boundaries. Workers 1 and 8
exercise CHECK and BUILD for 108 cells per bootstrap profile. The runtime
cases execute 36 real native images per profile, including macro-introduced
and captured binders, imported and generic variants, else-if syntax, outer
shadowing, conditional non-Copy moves and once-only scrutinees. Bootstrap
debug/release are not a claim of a native optimization-profile interface.

Twenty public authority controls use actual parsed owners and genuine checked
plans. The original two then-reference transplants into else/following code
initially passed planning incorrectly. Universal admission now tracks actual
sealed BindingIds in a charged temporary live-declaration environment, with
source-ordered initializers/declarations and block/arm/then restoration. Older
live shadowed IDs remain available to real definition-site macro captures.
No spelling fallback, duplicate-occurrence heuristic or reconstructed frontier
is used. Unused functions and dead branches receive the same checks.

New parser boxes admit exact fixed Layout<T> storage before fallible global
allocation and retain conservative accounting with the final NameStore owner.
Alignment/drop, pre-allocation byte denial, zero-size allocation avoidance and
last-owner release are tested. Allocator-null behavior is checked in code, not
claimed as an injected runtime experiment. Clone preflight separately admits
the new boxes and actual written pattern-child vectors before copying.

## Accepted frozen gates

All-target check `20261017-120523973-2041306`, release native/authority25
`20261017-120526163-2041393`, strict all-target clippy with `-D warnings`
`20261017-120541569-2041171`, formatting and diff checks passed. The final
complete hosted suite `20261017-120554793-2043511` passed **2,447 tests in
122 binaries**, with zero failures, ignored or filtered tests, including
973 library tests and unchanged nesting/cast/macro resource witnesses.
Sixteen exact failed and passing redacted summaries are retained in
`data/runs/xps14/2026-10-17-if-let.jsonl`. The earlier authority failure
`20261017-115958716-2017571` is retained alongside its unchanged passing
controls `20261017-120319513-2032373`. No old source corpus was promoted or
weakened for this milestone.

## Continuing regression requirements

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
