# Next Rust boundary: source-ordered reference inference

Status: **Measured blocker and proposed next architecture, not admission GO.**
The completed explicit generic-carrier stage remains unchanged. Inferred
structural reference actuals remain Unsupported, including frozen32 X29.

## Actual public compiler experiments

These are separate sources, not repairs of the frozen generic-carrier inventory.
Before first execution, the five-source manifest was frozen. The raw record is
`experiments/results/20261016-reference-inference-source-order.jsonl`; every
source and process stream has a verified SHA256. The native compiler was built
from implementation `2a3b022ff24b8f3246af96381dce99058a93e768`, binary SHA256
`6f4883e31238983e74df0bb5a9b2d92f626215c0cbe938e8b66ef0e9ceb6ca76`.
Stock is `/usr/bin/rustc` 1.98.1, exact `-vV` retained in the raw record. Stock
BUILD emits a linked executable, CHECK uses genuine `--emit=metadata` only.
Native uses the actual public file-based `jr-rust` CLI, workers 1 and 8.

| Witness | Stock BUILD / CHECK / runtime | Current native BUILD / CHECK |
|---|---|---|
| `mutable_explicit_reborrow`: `take::<&mut i32>(reference); *reference=2` | 0 / 0 / 0, exact `inference witness ok\n` | 0 / 0 for workers1/8, real runtime0 with the same stdout |
| `mutable_first_move`: `take(reference); *reference=2` | 1 / 1, genuine E0382 use after move, no artifact | 1 / 1, explicit inferred-reference Unsupported, no artifact |
| `mutable_first_after_pair`: `pair(a,b); *a=3` | 1 / 1, genuine E0382 use after move, no artifact | 1 / 1, explicit inferred-reference Unsupported, no artifact |
| `mutable_second_reborrow`: `pair(a,b); *b=3` | 0 / 0 / 0, exact `inference witness ok\n` | 1 / 1, explicit inferred-reference Unsupported, no artifact |
| `shared_identity`: `id(&value)` | 0 / 0 / 0, exact `inference witness ok\n` | 1 / 1, explicit inferred-reference Unsupported, no artifact |

A sixth independently added witness is retained in
`experiments/results/20261016-reference-inference-result-expectation.jsonl`:

```rust
fn id<T>(value: T) -> T { value }
fn main() {
    let mut value = 1;
    let reference = &mut value;
    let returned: &mut i32 = id(reference);
    *returned = 2;
    *reference = 3;
    if value == 3 { println!("inference witness ok"); }
}
```

Stock BUILD/CHECK and actual runtime all return 0 with the literal stdout above.
Native CHECK remains explicit Unsupported. The first execution of this additional
witness completed but the coordinator's JSON formatter tried to parse an already
decoded object; the recorded second trial reran the unchanged source into distinct
artifacts. That tooling retry is transparent in the receipt. No source or expected
outcome was repaired. All recorded processes completed without timeout/truncation.

The native Unsupported results are **not** claimed as correct semantic rejection
of the two E0382 cases or acceptance of the three legal inferred cases. They
measure the precise remaining boundary.

## Hypotheses rejected by observation

1. **Remove the Ref fence and use the solved signature for all argument checking.**
   Rejected. `take(reference)` moves an opaque first argument, while its explicit
   `take::<&mut i32>` form reborrows. The final type alone is identical.
2. **All arguments of a generic call have the same firm-reference classification.**
   Rejected. The first `pair(a,b)` argument moves, but the second is reborrowed:
   using `a` afterwards is E0382, while using `b` is legal and executes correctly.
3. **Only earlier arguments can establish firm expectations.** Rejected. The
   expected-result witness shows a source return context can establish the mutable
   reference expectation before the first argument is checked.
4. **A shared-only unary carveout needs only a new expectation enum or bool.**
   Not authorized. Although a first shared-local domain may be semantically
   tractable, the current final checked Call does not retain original omitted vs
   explicit actuals, result expectation or inference frontier. A role flag would
   not independently validate those facts. Use the common replay record below,
   even if its first admitted domain is small.

## Actual implementation seam

- `frontend.rs::Inference::finish` intentionally rejects newly inferred structural
  Ref actuals before ordinary argument validation.
- `Checker::infer_call_args` constrains the expected result first, then collects
  every argument's evidence and solves all actuals before normal checking.
- `finish_checked_call` currently derives traces from the resulting source-stage
  signature. This is correct under the existing ref-free/rigid inference domain,
  not justification to expand that domain without retaining earlier stages.
- `call_argument` plus `expr_without_cast` can create mutable or mutable-to-shared
  reborrows under a firm expected Ref. Therefore both paths must consume the
  selected original-stage expectation, never a later inferred or monomorphic Ty.
- `ir::validate_call_evidence` validates the canonical signature and derives firm
  expectations from its parameter heads. The original inference frontier is not
  an available input today.
- Primary owned rustc source `~/justrust/toolchain/rustc/compiler/
  rustc_hir_typeck/src/coercion.rs` documents the same source-order distinction.
  The six executable witnesses above, rather than that comment alone, establish
  the externally observed behavior.

## One proposed architecture, no competing shortcut

Retain a metered original-call boundary record before inference changes:

1. Canonical declaration/binder and exact original call/source identities.
2. Explicit versus omitted type-actual form, retaining original terms.
3. Original expected-result term and whether it was firm or unknown.
4. Ordered argument identities and independently available preadjustment evidence.
5. The substitution frontier before checking each argument.
6. The actual chosen Local/Borrow/Deref adjustment and BorrowTiming.

A single replay validator uses an authoritative original callable schema accessor
from the exact world/facts context. It validates the original result constraint
and each argument transition, exact canonical targets and binder identities,
selected adjustments, region evidence and all dead metadata. A private validated
record is then sealed into the existing `ValidatedCallEvidence` lifecycle.
`VerifiedTemplateCfg` and `BodySubstitutionProof` carry that original evidence
unchanged across the existing Universal-to-Closed bridge. Closed Ty equality must
not manufacture source-stage authority. No second borrow solver, loose success
flag, world-to-body callback or new budget is proposed.

Before publication, shared sizing kernels must precharge term/path/frontier
storage, every replay transition, repeated scans, old/new allocation peaks and
retained evidence on the same planning ledger and local limits. Exhaustion stays
Resource. Foreign or inconsistent evidence stays Invalid. Explicit Unsupported
may bound unimplemented inference shapes, never mask a demonstrated E0382 case
once that shape has been admitted.

## Next implementation gate

The current concrete blockers are missing original application/result-context
provenance and missing narrow authoritative schema access for IR replay. Freeze
the exact replay record/accessor contract and independently review it before
editing admission. A first slice may limit itself to unary shared-local `(T)->T`
argument inference with reference-free, Param-free referent and no result-context
inference, but it must use the same replay architecture, not a special waiver.
The original frozen32 and their effective native classifications must remain
unchanged unless a separately evidenced, explicit promotion is authorized.

This investigation advances a specific Full Rust prerequisite. It does not close
Full Rust, claim new accepted syntax, or claim a speedup.
