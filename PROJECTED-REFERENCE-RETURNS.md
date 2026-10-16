# Projected input reference returns: design checkpoint

Status: implemented with the frozen 30-case stock corpus and four retained
workers1/8 graph tests passing. All 888 hosted tests pass, and both independent
actual-code soundness/resource reviews are clear. Final strict all-target lint
and formatting also pass. Tracked seventeen-corpus native verification remains
the delivery gate.
Mutable whole-parameter returns passed 844 hosted tests and all 662 tracked native
cases at implementation `49a1ae343`, with raw evidence pushed in `68c2b7258`.
Full Rust remains open.

## Declaration-derived contract

Replace `SharedWholeParams` and `MutWholeParams` with one canonical pair:

```rust
pub struct InputPath {
    pub param: usize,
    pub projection: Arc<[Projection]>,
}

pub enum ReturnContract {
    NoReference,
    SharedInputPaths(Arc<[InputPath]>),
    MutInputPaths(Arc<[InputPath]>),
}
```

`param` is the original Signature.params index, including nonreference slots.
The suffix is relative to that parameter's referent. Permit only Field and
ConstantIndex steps. Never permit Deref, Downcast, dynamic indices or native byte
offsets. Retain all other RefContract fields and canonical Arc propagation.
Migrate whole-parameter contracts to empty suffixes rather than retaining two
competing representations. NoReference and its B2 mixed-kind policy stay intact.
Shared-returning functions retain all-shared reference inputs, and mutable-returning
functions retain all-mutable reference inputs. Reference aggregates, nested
references, local/static/rvalue returns, lifetime elision and enum payload return
projections remain outside this slice. Source projected borrows retain explicit
one-Deref place syntax, such as `&(*p).field`; implicit field autoderef is not
introduced. Historical `&p.field` and `&mut p.field` boundary sources remain
byte-identical and are not promoted by this protocol.

For every same-kind input whose declared region outlives the output, enumerate
all static referent occurrences. Emit a candidate at every node whose type exactly
matches the output referent. Traverse tuple and struct fields and constant array
indices in ascending order. Include empty paths. Treat enums as opaque, permitting
an enum itself to match but never traversing variants. Zero-length arrays have no
element occurrences. Different occurrences of a shared declaration DAG must not
be skipped by global type-identity deduplication.

Canonical order is original parameter order followed by lexicographic path order.
The verifier recomputes and requires the complete exact list, rejecting omitted,
extra, duplicate, reordered, ill-typed or forbidden projection candidates. No body
summary is trusted to narrow the declaration-derived list. Empty candidate sets
retain current diverging-contract behavior, with normal returns still requiring
actual valid provenance.

Private fields participate in internal candidate enumeration. Public getters may
return fields they can access without callers naming those fields. Existing source
privacy checks remain authoritative when forming source borrows or field accesses.
Contract metadata grants no source access rights. Nominal type identity remains
exact, not structural layout equivalence.

## Body proof and call substitution

Every normal returned canonical target must equal an allowed external parameter
root plus its exact suffix. A prefix or matching root alone is insufficient.
Independently prove all transitive lifetime dependencies are formal-rooted and
universally outlive the output. Never infer missing universal bounds from a body.
Shared returns retain Copy semantics. Mutable returns prove provenance in the read
phase before moving the unique result carrier.

For each candidate, resolve only the pre-consumption actual origins at that
candidate's original parameter position. Append that candidate's suffix to each
of those targets. Never collect all parents and cross-product all suffixes over
all arguments. Preserve source-specific target authority, even for equal canonical
addresses. Lifetime dependencies still retain all region-related input origins,
including unrelated pointee types and upstream returned origins. Capture before
argument moves and seed the genuine result temporary on the outgoing call edge.

The existing overlap policy remains authoritative: disjoint tuple/struct fields
can be independent, but array indices conservatively overlap the whole array.
A returned projected reference does not weaken the input loan's lifetime freeze.
All may-target accesses require initialization and authority. Mutable writes kill
all affected proof subtrees without initializing alternatives. Calls continue to
invalidate proofs below every mutable actual, not only returned candidates.

## Resource and trust gates

Proposed per-signature limits are 4096 candidates, 131072 total stored projection
steps, 131072 expanded enumeration occurrences including nonmatches, and depth128.
Charge bounded type equality separately and check before allocation or growth.
Use a depth-first walker with one current path and O(depth) frames, not a queue
containing cloned paths for every expanded occurrence. Validate and charge supplied
candidate count and stored steps before traversing or cloning malformed metadata.
Peak scratch includes supplied and generated candidate path buffers. Check composed
actual canonical suffix plus declared suffix against depth128 before allocation.
The shared helper is `eligible_input_paths(params, result)`; `validation_work`
retains complete bounded metadata, enumeration and equality accounting.
Candidate substitution products, projection composition, exact deduplication,
authority memberships and scratch bytes share existing 32M work, 1M memberships
and 128MiB peak state caps. Combined explicit/formal/returned origins remain capped
at4096. Exhaustion fails closed and incomplete, never truncates a successful list.

Declarations produce contracts before BODY. Recursive and forwarding calls depend
on canonical declaration contracts, not recursive callee-body fact dependencies.
All bodies, including unused functions, must prove their own contract before
successful CHECK or BUILD. Each native OBJECT still requires exact ANALYZE.
Standalone checked emission continues rejecting reference calls without validated
whole-program evidence. Native reference transport stays thin64, with typed place
addressing and no snapshots or layout offsets embedded in summaries.

## Required gates before promotion

- Nested tuple, struct and static array shared/mutable projected returns.
- Private-field getter across modules, with forbidden direct access still rejected.
- Distinct nominal same-layout types never conflated.
- Different argument layouts expose candidate/argument cross-product mistakes.
- Forwarding and recursion retain exact targets and universal dependencies.
- High-bit usize and Copy aggregates mutate/read original caller storage.
- Unrelated lifetime inputs remain pinned without granting authority.
- Parent freeze/resume, same-address authority negatives and array alias rejection.
- Forged missing, extra, reordered, wrong-type, Deref and Downcast metadata rejected.
- Unused invalid callee blocks executable publication, workers1/8 fact parity.
- Shared-DAG occurrence expansion, candidate products and byte/work caps fail closed.

## Coordinated ownership

- IR/CFG: InputPath, canonical helpers, exact metadata and occurrence budgets.
- Frontend: declaration contract construction and unchanged source privacy.
- Borrow/ownership: correlated target substitution, exact return membership,
  universal dependencies and peak scratch/work preflight as one coordinated owner.
- Backend: thin-pointer trust audit and focused native regressions, no ABI redesign.
- Acceptance: one frozen new stock corpus and matching experiment harness only.
- Coordinator: graph acceptance, examples, docs, profiles and tracked native receipts.

All owners and independent reviewers confirmed correlated substitution, bounded
occurrence enumeration and privacy separation are feasible. Implementation must
retain these obligations and pass actual-code review before promotion. This
checkpoint is not full Rust completion.
