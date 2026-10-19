# Read-only optimizer accounting review, supplemental findings I-N

## Authority and limits

- Frozen source: `67e1906364533fbd5c5c8f2968d5ade3dcded359`, parent `11fc496ea`, following `368101dd1`.
- Checkout inspected: `/home/jeremy/justrust-opt`.
- Audit contract: `data/runs/graph-recovery/OPTIMIZER-ACCOUNTING-AUDIT.md`, especially priorities 2-5 and proposed acceptance evidence.
- Scope: `src/compiler/ssa/{lower,verify,opt,inline,machine,hash}.rs`, actual parent-to-repair diffs and local source call paths. `ssa.rs` operand/RPO helpers and the meter/store interfaces were read only to establish contract interactions. Hare owns meter/store/mod envelope review. Already-relayed unrestricted machine appends, symbol admission, panic growth delta, object envelopes and store capacity issues are not repeated as findings here.
- Inspection only. No builds, tests, fixture execution, source edits, commits, pushes, agent spawning, speed measurements or full-Rust claims. This Markdown is written only to external scratch after the coordinator expressly authorized it. The active worktree changed during review through other work, so all cited lines are the frozen commit's lines, not a moving working tree.

## Blocking dimension and transient-allocation findings

### I. Unreachable type propagation pays pending instructions, not repeatedly scanned call operands

**Priority 1.** `src/compiler/ssa/verify.rs:329-348`, especially `333-336`; nested traversal at `518-532`.

The fixed-point loop charges `meter.work(before)`, where `before = pending.len()`, and then invokes `inst_type` for every pending instruction. `inst_type`'s Call arm loops over every `(arg, param)` at lines 528-532. That per-argument traversal is repeated every retry, without an argument charge inside the fixed point. The one initial operand charge at lines 320-326 does not pay repeated retries.

A distinguishing valid SSA shape is an unreachable block with calls `c0, c1, ..., cN-1`, followed by a constant base. Each call has M-1 already typed block-parameter arguments and, as its last argument, the result of the next call (the last call uses the trailing base). Unreachable uses are explicitly allowed without dominance/order restriction by lines 270-271. Each round types only the next call from the end, and every earlier call walks all the typed prefix before failing on the last argument. Actual argument visits are Theta(M*N^2), while these paths charge Theta(M*N + N^2). The ratio grows with M. This is not merely an omitted constant multiplier.

Required follow-through: meter each argument/type probe on every retry, or pre-admit the checked operand total for the actual pending round before invoking the traversal. Keep resource failures distinct through the retain/fixed-point path. This construction was derived from source, not executed.

### J. Branch folding allocates an extra edge-argument vector outside the paid peak

**Priority 1.** `src/compiler/ssa/opt.rs:255-268`, especially `267`; `store::try_clone_target` is the allocation path.

`fold` starts with an owned, paid clone of the entire input at line 123. For a known branch, line 267 evaluates `try_clone_target(taken)` before assigning a Jump to `data.term`. While that fallible copy allocates and copies the chosen target's argument vector, both of the old Branch target vectors are still live. There is no extra `Owned::admit` or scratch lease for the copied vector. The input-shaped lease pays the old branch, not old branch plus the new target. Assignment drops the old branch only after the copy has been created.

A branch with M arguments on each edge makes the missing transient allocation M * sizeof(SsaValue), independent of instruction count. This is a peak-admission failure even though the final output shrinks and its old lease may subsequently over-cover the result. Prefer moving the selected target out of the old terminator or explicitly admit and reconcile the transient copy before constructing it.

### K. Inlining copies panic bytes without charging the copied-byte dimension

**Priority 1.** `src/compiler/ssa/inline.rs:157-161, 194-199, 221-233, 266, 358-362, 374-375`.

The counting pass includes panic string lengths in storage preflight, but its string loops only query `.len()`. The output-wide work charge at line 233 is `x.insts + x.blocks`. Construction copies the caller panic strings through `try_clone_panics` at line 266 and copies every inlined callee panic message/location with unmetered `try_string` at lines 358-362. There is no charge by `x.panic_bytes`, and the per-callee-block build charge at line 375 counts instructions plus terminator operands, not copied string bytes. Verification validates panic indices, not text bytes.

Consequently S call sites copying P bytes of panic text can perform Theta(S*P) copied-byte work while work charges remain independent of P. Byte storage admission alone does not pay the work dimension. This is especially relevant to the existing instruction-only inline policy: a leaf with a large panic string can still have zero or very few instructions.

Related source evidence in `hash.rs:99-103`: canonicalization also copies all panic strings with no text-byte work charge. Later hash encoding eventually charges encoded bytes, but that does not account for canonicalization byte copying as its own operation or repair inline's independent work accounting.

### L. Lowering charges structural target comparisons as constant-time comparisons

**Priority 1.** `src/compiler/ssa/lower.rs:484-492`; target representation at `src/compiler/ir.rs:167-171`.

`Ctx::callee` charges only `self.callees.len()` before a linear `position(|c| c.target == *call)` search. `CallTarget` derives structural PartialEq/Eq and contains `type_args: Arc<[Ty]>`, so equal-declaration targets with distinct type-argument allocations can require recursively traversing long equal prefixes. An Arc shares storage but does not make comparisons between distinct argument trees constant time.

For C distinct instances of one declaration whose argument types contain K-node common prefixes and differ late, the callee search can visit Theta(C^2*K) type nodes. This code charges Theta(C^2), plus the separately metered symbol generation for new instances. Symbol generation after the search does not cover the extra K factor on every prior comparison. A scalar-signature function with an unused generic type parameter can carry such type arguments without non-scalar runtime locals.

Use canonical instance identity or meter actual structural comparison/type traversal. This finding concerns the lower-owned callee search, not the instances subsystem's own accounting.

### N. Inline panic-table growth reserves final storage but not the old-plus-new header peak

**Priority 1.** `src/compiler/ssa/inline.rs:221-239, 265-269`.

The preflight includes final `x.panics * sizeof(SsaPanicText)` once. Build first allocates a caller-sized cloned panic table at line 266, then calls `panics.try_reserve_exact(x.panics - panics.len())` at lines 267-269. If inlining adds entries, a relocating growth can keep the old table allocation and the new final-sized table allocation simultaneously live. No extra admission precedes that reserve. This directly contradicts the file's claim of no clone-then-extend and the audit's old/new overlap requirement.

Many caller panic records plus one added inline panic distinguish this from a small fixed overhead: the extra old header buffer scales with the caller panic count and can exceed other still-unused preflight slack. Allocate the final header capacity first and fill it directly, or admit the old/new overlap. This is independent of the already known machine code growth issue.

## Excess lifetime, not under-accounting

### M. Shadowed Owned generations remain alive through the round

**Priority 2.** `src/compiler/ssa/opt.rs:60-95`, especially `74-88`.

`let folded = ...` at line 76 shadows the earlier owned fold result at line 74 instead of consuming or dropping it. The same pattern occurs for merged at 77/79 and trimmed at 80/82. Explicit `drop(folded)`, `drop(merged)` and `drop(trimmed)` address the newer bindings only. Earlier shadowed owners retain their storage and leases until the round scope ends.

While the next canonical result is constructed at line 87, the round can still hold current, pre-canonical folded, pre-canonical merged, pre-canonical trimmed, swept and next, rather than the claimed at-most-two generations. This is correctly charged storage with excessive lifetime, not a missing charge. It can cause avoidable task-byte exhaustion and makes the source/commit two-generation claim false. Consume/rebind explicitly and drop each prior owner once its final consumer finishes. The end-of-call scratch-refund test cannot detect an unnecessarily high mid-round peak.

## Liveness: dimension and fixed point repaired, literal visits still incompletely charged

**Priority 2 observation, constant-factor only.** `src/compiler/ssa/machine.rs:309-313, 360-395`.

The flattened matrix cell count uses checked multiplication, four liveness tables and two reused rows hold their leases, and the actual `while changed` loop charges each round at line 368. Every successor row OR is separately charged at line 373. There is no remaining fresh row allocation per block per round in this code. These are real repairs of the audit's missing dimension and fixed-point problems.

However line 368 charges B*W once per round, while each block clears W words at line 371, computes W words at 380-386, compares up to two W-word rows at 388, and copies two W-word rows on change at 389-390. Only successor OR visits are separately charged. Under the literal documented per-word/actual-comparison contract, distinct row scans and comparisons remain unpaid. This is a constant-factor omission, not the asymptotic missing-dimension failure in finding I and not a demonstrated liveness miscompile.

## Other checked boundaries and positive evidence

- Operand and target traversal in `ssa.rs:257-415` now uses fixed stack arrays and borrowed iterators/slices, including Call arguments. It no longer materializes a traversal Vec or clones Call operands per visit.
- Lowering's block-times-locals and edge-times-locals byte products are checked, and their work dimensions are charged at `lower.rs:633-636`. The local-to-parameter map removes the prior locals-times-params position search.
- Inline output values, blocks and panic counts are checked for u32 at `inline.rs:205-207`; final-shape byte products use checked helpers. This does not establish that every allocation/growth has a paid transient peak, as J and N show.
- Lowering fresh values use checked addition, but aggregate block/value/panic ID admissibility is not fully validated in the counting pass before constructing output. For example `lower.rs:641-673` allocates output buffers before `fresh` can report ID exhaustion; block conversion also occurs later. This is incomplete preflight evidence, not a reproduced overflow or a newly introduced miscompile.
- Machine spill-frame rounding still has unchecked `bytes + 15` at `machine.rs:525-528`. This exact expression exists in the parent, so it is inherited unresolved overflow debt, not a repair-introduced regression. A sufficiently large allowed spill count can pass checked multiplication and overflow the rounding addition. No such large allocation was attempted.
- `MAX_ARGS = 4096` and `INLINE_LIMIT = 24` are unchanged from the parent. No new fixed arbitrary cap replacing task-budget work was found in this lane.
- Hash encoding charges its accumulated byte count after the panic-text group and after each whole block (`hash.rs:230-238,329-330`). It eventually charges bytes, but a large text group or block can be processed before work exhaustion is observed. No strict preemption-before-every-byte claim is established.
- No new output semantic miscompile was established by this inspection. Accounting contract failures above are sufficient to block claiming audit completion independently of stock correctness.

## Evidence versus owner-reported validation

The repair commit message reports 22 SSA tests, optimizer end-to-end, graph_integrity, backend and clippy success, plus a reconciliation mutation check. These are owner-reported results only. This review did not run or independently verify those commands.

Source test coverage read:

- `ssa/tests.rs:1149-1164` calls its fixture many-block/few-instruction, but it creates two instructions per added block. It is not a zero/few-total-instruction many-block fixture.
- `ssa/tests.rs:1294-1364` has 40 inline sites into one tiny, one-block, text-free leaf. It tests reconciliation and a low-byte rejection, not large panic-byte work, long common-prefix target comparison, unreachable M-operand fixpoints, transient header peaks or shadowed-generation peak lifetime.
- The added `tests/optimizer.rs:389-404` test is named for tiny-budget fail-closed behavior but its body only compares default-limit optimizer-on/off builds and work totals. It does not actually set a tiny budget or assert a resource failure.
- The scoped SSA additions end at `ssa/tests.rs:1364`; no new long-liveness work-exhaustion fixture is present there.

No executable validation was substituted for these missing acceptance cases. Existing tests passing would not refute the distinguishing source paths above.
