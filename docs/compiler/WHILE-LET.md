# Genuine while-let: bounded native acceptance

Status: accepted for the bounded statement slice below on October 17, 2026. Full Rust compatibility remains open.

## Source and authority

- Genuine `Stmt::WhileLet { label, pattern: Box<Pattern>, scrutinee: Box<Expr>, body, has_semicolon }` retains ordinary while unchanged. The parser records the actual trailing semicolon. This first slice is statement-position only.
- Parse and resolve the scrutinee before introducing real body-only BindingIds. Restore lexical and active-declaration frontiers before following source and on failure. Runtime iterations reuse static source identities but initialize successful pattern locals anew.
- New boxes use the charged fallible parser allocator. Source clone preflight accounts for both fixed boxes and actual written pattern children. No omitted wildcard, fake source match/break, callable or body identity is fabricated.
- Inventory tag 22 records actual scrutinee child 0, pattern child 1 and body child 2. A repeated runtime condition retains one original source call site. Pattern locals do not acquire specialized root-reference replay eligibility.
- Universal validation checks every actual child, including unused/dead code, with actual BindingId availability restricted to the body.

## Checked loop semantics

- Enter a real non-value loop and install its checked label before typing the scrutinee. Preserve the condition-target barrier, then clear it before checking the body.
- Existing typed IR Loop contains genuine checked Match. The selected arm executes the actual body; semantic fallback breaks that exact loop. Continue targets the header and re-evaluates the scrutinee. No new IR or ABI is required.
- Reuse checked pattern domains, source constructor certificates, enum owner/variant downcasts and physical indices. Only success initializes/moves selected bindings. No enum masquerades as Aggregate.
- Source Unit obligations remain separate from runtime divergence. A final unsemicolonized while-let retains its Unit tail obligation even after an earlier return, whereas the actual semicolon and following statements preserve their measured source behavior. Blocks without while-let and ordinary while are unchanged.
- The emitted loop records intrinsic exit completion even in outer unreachable source. A Never scrutinee suppresses runtime completion, not dead-body checking. Preserve cleanup, backedge ownership/refinement and indexed non-Copy move fences.
- New checked IR body storage is precharged before fallible reservation, reconciles actual capacity and latches allocator failure. No injected allocation-null execution claim is made.

## Acceptance evidence

- Stock oracle: 33 exact sources, 23 accepted and 10 rejected by the measured stock compiler. The initial 29 source bytes/order/categories are unchanged; four appended controls retain the measured trailing-semicolon distinction.
- Native partition: 18 runtime, 10 ordinary invalid and 5 stock-valid explicit unsupported. Workers 1/8 CHECK/BUILD produce 132 cells per bootstrap debug/release profile. Each profile executes 36 actual native main images. Six are deliberately empty-main executions for three exact compile-completion controls, not execution of their divergent value functions.
- Public authority: 23 controls use real scoped source identities and actual canonical enum/loop metadata. They reject condition/after/sibling binding escapes, foreign unused children, forged downcasts, payload indices, locals and sibling loop targets at their real IR/CFG boundaries.
- Controls include zero/multiple iterations, N+1 condition evaluations, continue/header behavior, nested labels, actual macros/imports, failed-edge reuse, non-Copy reinitialization versus an uninitialized backedge, Never scrutinees and checked dead bodies.
- First library run `20261017-121351810-2080055` found three new fixture grammar errors (unsupported mutable function parameters). Genuine immutable parameters plus local mutable bindings preserve the intended stock-checked ownership witnesses.
- Actual unchanged stock-invalid trailing-while control failed in `20261017-121835617-2101988`. Recording the actual semicolon and keeping source typing separate from runtime divergence makes it pass unchanged in `20261017-121949871-2107998`; explicit-semicolon positives remain accepted.
- Final debug authority/native gate `20261017-122031682-2110919`, release gate `20261017-122040822-2111523` and strict all-target lint `20261017-122056912-2112724` pass. Formatting and diff checks pass.
- Complete hosted gate `20261017-122101838-2110918`: 2,495 passed across 124 binaries, with zero failed, ignored or filtered tests, including 992 library tests. Exact failed/passing receipts are retained in `data/runs/xps14/2026-10-17-while-let.jsonl`.

Expression-position while-let, chains, guards/or-patterns, reference ergonomics and general refutable payload matrices remain open. Existing array Copy-only extraction and other native boundaries are unchanged. No complete Rust compatibility or speedup claim follows from this milestone.
