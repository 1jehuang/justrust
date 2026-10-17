# Genuine while-let: bounded native acceptance

Status: implementation in progress, not accepted. Full Rust compatibility remains open.

## Source and authority

- Add genuine `Stmt::WhileLet { label, pattern: Box<Pattern>, scrutinee: Box<Expr>, body }`, retaining ordinary while unchanged. This first slice is statement-position only.
- Parse and resolve the scrutinee before introducing real body-only BindingIds. Restore the lexical frontier and active declarations before following source and on failure. Runtime iterations reuse static source identities but initialize successful pattern locals anew.
- New boxes use the charged fallible parser allocator. Source clone preflight accounts for both fixed boxes and actual pattern children. No omitted wildcard, fake source match, source break, callable, or body identity may be fabricated.
- Source inventory uses distinct tag 22: actual scrutinee child 0, actual pattern child 1, body child 2. A repeated runtime condition retains one original source call site. Pattern locals do not acquire specialized root-reference replay eligibility.
- Universal name validation checks every actual child, including unused/dead code, and limits actual BindingId availability to the body.

## Checked loop semantics

- Enter a real non-value loop and install its checked label before typing the scrutinee. Preserve the existing condition-target barrier, then clear it before checking the body.
- Emit existing typed IR Loop containing genuine checked Match. The selected arm contains the actual body; semantic fallback breaks that exact loop. Continue targets the loop header and re-evaluates the scrutinee. No new IR or native ABI is required.
- Reuse checked pattern domains, source constructor certificates, enum owner/variant downcasts and physical projection indices. Success alone initializes/moves selected bindings. No enum may masquerade as Aggregate.
- Source statement completion retains ordinary while semantics: whether the condition completes. The emitted IR loop separately records actual exit completion. Irrefutable infinite patterns must not erase a missing return obligation.
- Both successful and fallback bodies and all source obligations remain checked under a Never scrutinee. Preserve cleanup, backedge ownership/refinement and indexed non-Copy move fences.

## Required evidence before acceptance

- Stock-first sources covering zero/one/multiple iterations, N+1 condition evaluations, continue, nested labels, actual macro hygiene and imported constructors.
- Real scope transplants into condition/after/sibling loops must be rejected, including unused functions and foreign handles, before publication where applicable.
- Conditional non-Copy moves, failure-edge reuse, per-iteration reinitialization versus missing replenishment, sparse suffix projections, body cleanup and Never/dead-code validation.
- Body Unit obligation, valued-break rejection and source-versus-IR completion controls. Chains, guards, or-patterns, reference ergonomics and refutable payload leaves remain explicit existing boundaries.
- Native CHECK and BUILD with workers 1 and 8, actual executed images in bootstrap debug/release, public checked-authority negatives, full hosted regression, strict lint and formatting.
- Retain real failed and passing receipts. Commit only after frozen final gates. No performance or complete Rust claim follows.
