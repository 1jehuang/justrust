# B3a design checkpoint: shared reference returns

Status: read-only architecture proposal, not implemented and not authorized for
implementation yet. B2 must finish its native acceptance and evidence first.
Full Rust, mutable escaping references and general regions remain open.

## Proposed bounded first slice

Admit shared reference results that point to whole shared-reference parameter
referents. Returning-reference functions initially use only shared reference
inputs, with explicit declared lifetime parameters and explicit lifetimes on
reference parameters/results. Existing noescape B2 functions keep their mutable
capabilities. No mutable, projected/subfield, local, temporary, static, nested or
aggregate reference results, HRTB, callbacks, unsafe/FFI, general type generics or
full lifetime elision in this first slice. Valid deferred forms must diagnose
unsupported rather than be classified as invalid Rust.

`id<'a>(x: &'a T) -> &'a T` and selecting either of two same-lifetime shared
parameters are the core cases. Lifetime names alpha-normalize by declaration
order. Declared `'a: 'b` means points of b are a subset of points of a.

## Declaration contract, not recursive body dependencies

Keep Ty::Ref kind and referent structural. Add a shared signature sidecar with
region IDs, parameter regions, result region, declared outlives edges and a
conservative return contract. Proposed API names:

- RegionParamId(u16)
- RefContract { region_count, param_regions, result_region, outlives, returns }
- ReturnContract::{NoReference, SharedWholeParams(Arc<[usize]>)}

Use one canonical contract through Signature, checked function and CFG, not
independently reconstructed metadata. Eligible returned parameters come from the
declared region/type relation. Every body proves its returns satisfy that
contract. Calls depend on declarations, never callee-body completion, so
recursive calls add no graph fact cycle or optimistic summary.

## Provenance and lifetime dependencies are different

A returned reference's possible target roots are not all its lifetime inputs.
For `f<'a>(s: &'a S, x: &'a T) -> &'a T`, only x is a possible target, but both
s and x constrain the lifetime of the result. Separate candidate-specific
`authority_parents` from `lifetime_deps`. Pin all applicable input region
obligations through result uses without granting authority over unrelated roots.
Do not union candidate authority exemptions.

At a normal return, resolve every returned target to an allowed external
ParameterReferent root with empty projection. Reject owned frame storage and
unrelated shorter input lifetimes. Returning a private reborrow is possible only
when its canonical external root and all transitive region dependencies satisfy
the advertised output region. Handle death is distinct from referent death.

## Calls and fixed points

Instantiate signature regions freshly at each static call site. Actual reference
validity bounds instantiated regions, declared outlives adds subset constraints,
and returned-carrier uses constrain the result region. Solve bounded finite
constraints before diagnostics. Same-region arguments constrain the result even
when their referent types differ. Independent region parameters allow unrelated
arguments to end earlier.

Add a private CallReturn origin per reference-result Call terminator. Capture
argument targets and dependency origins before moving argument carriers. Freeze
returned target alternatives by substituting only eligible parameters, with
per-candidate source authority. Preserve lifetime dependencies from every input
region entailed to outlive the result, including upstream dependencies. Seed the
outgoing destination carrier with that origin and retain its target/dependencies
after argument temporaries die. Never expose callee-private loan IDs to callers.

Origin terminator transfer must both consume moved arguments and define result
origins. Backward carrier liveness must define call-result destinations. Model
true outgoing/write-phase states, not aliases of pre-call snapshots. CallReturn
births occur at terminators. Generalize target/dependency fixed points to calls,
branches and reborrows with fail-closed cycles. The existing SCC-born origin
backedge precision boundary includes returned-call origins.

## Native and trust boundary

Shared reference returns remain thin 64-bit scalar pointers, without copying a
referent or returning a callee frame address. Pointer transport becomes legal
only after return provenance and region verification. Keep the whole-program
all-body gate and standalone/trusted emission distinction. Mutable results must
remain rejected until explicit unique authority transfer and consuming return
semantics are implemented.

## Proposed shared resource gates

Initial bounds: 64 declared regions, 4096 combined site/parameter/CallReturn
origins, one million total retained target/authority/dependency/region
memberships, existing depth 128, 131072 paths, 128 MiB peak state and 32-million
shared work. Charge region closure, region-by-CFG constraints and returned-target
substitution before allocation. No recursive summary unrolling or unbounded
origin path enumeration. Exhaustion is incomplete and fail-closed.

## Required acceptance before promotion

- Identity and alpha-renamed lifetime signatures, two same-region candidates.
- Independent short-lived input versus same-region different-pointee dependency.
- Explicit outlives shortening and missing-bound rejection.
- Local owner return rejection and unused bad callee publication failure.
- Forwarding and recursive calls preserve canonical roots and dependencies.
- Argument temporary death allowed, owner death and early mutation rejected.
- Last use permits later owner mutation and branch aliases remain conservative.
- Malformed summaries, region IDs, signatures and outgoing origins reject.
- Thin full-width result transport and all origin/region/resource caps.

After B3a: mutable reference returns, bounded projected input summaries, aggregate
reference provenance, nested references, broader elision and general lifetime
inference. This checkpoint does not claim those features or full Rust completion.
