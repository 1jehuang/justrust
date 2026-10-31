# Resident library-root cache investigation (v38)

## Decision: rejected hypothesis, no production candidate

The proposed `RA_JIT_LIBRARY_ROOT_CACHE=1` extension is **not implemented**.
The relevant demand-standard-library roots already satisfy the existing
`StableNames::of_checkout` guard. Adding an origin-based alternative to that
guard would not make those roots newly eligible. No roots, exports, native ABI,
cache keys, refusal rules, or mode behavior were changed. `main.rs`, `cached.rs`,
`stable.rs`, and `later.rs` are unmodified. This remains licensed Blaze-derived
work, not a clean-room implementation. No `RA_JIT_DEMAND_EXPORTS` mode exists.

This is a source investigation and rejection record, not a speedup claim or an
accepted cache implementation. No compiler build, test, or native workload was
run by the investigator. The native observations below were produced by the
coordinator's frozen v36 binary before this investigation's decision.

## Why the checkout-only guard already includes these roots

At revision `4614b6d4a2de0ab141cc810b4a9e0ef42b97a63d`:

1. `ra-jit/src/main.rs`, in the root collection near line 5276, extends `own`
   with `library_unit(db, &local)` when the exact demand-std opt-in is enabled
   and no prebuilt unit was supplied. The library selection is unchanged.
2. The selected linked crates contribute their `listed.rooted` and
   `listed.provided` functions. The existing partition near line 5320 puts each
   function in `Roots.entries` or `Roots.on_demand`. Exported functions remain
   entries. Nothing is dropped to improve timing.
3. `CodegenUnit::compile_or_hold` in `hir-ty-codegen/mod.rs` defines `all()` as
   **both** `roots.entries` and `roots.on_demand`. Its `root_crates` is the set of
   crates of all of these functions, not merely local workspace crates.
4. `OwnStatics::new` retains that `crates` set without filtering language crates.
   `ObjectCodegen::new` passes this ownership set to `StableNames::new`.
5. `stable.rs::StableNames::of_the_checkout` returns
   `self.owned.contains(&krate) || krate.data(self.db).origin.is_local()`.
   Consequently a selected language crate with roots is named by content with
   the checkout marker `~` even though its actual origin is `CrateOrigin::Lang`.
6. `find_stored_roots` names these root instances using the same ownership set.
   Its prefetch and transitive `Entries::usable` checks therefore already admit
   their `~` names. `stored_root` and the fallback root lookup in `compile_ahead`
   use the same eligibility convention.

Here, “of checkout” is a stable-name ownership classification, not a promise
that the crate's source is local or mutable. Reading the name guard in isolation
led to the rejected hypothesis. This conclusion follows from the complete source
path. The existing logs do not print a per-origin name inventory, so they cannot
independently establish the number of language roots among cache hits.

## Existing persistence, ABI and safety contracts

`later.rs::Keeper::make` handles `Of::Root` by keeping
`(root_instance(db, func), func.krate(db))` whenever `later.keeps` is true.
`define_function` already sets `later.keeps` from the existing
`roots_from_store()` conditions for an untiered root. There is no additional
“immutable roots cannot be persisted” restriction there.

A root and an ordinary function instance can share the same stable name and
entry key when the instance has empty generic arguments and the same overflow
setting. `root_instance` records the function's crate overflow setting, defaulting
to false for language crates. `instance_code` and `root_code` both use the
function's declared ABI, variadic setting, and `track_caller` setting and feed
these through `compile_as`. However, `root_body` uses an empty parameter
environment and resolves opaque locals as needed, while `instance_body`
monomorphizes in the instance's supplied environment. This source audit is **not**
a native differential proof of equality for every such instance. A future
extension beyond currently owned roots must prove this collision safe, not
assume that a matching name alone certifies root ABI equivalence.

The existing entry key separates backend, target, MIR optimization, frame-pointer
and link-dead-code settings, debug mode, program identity where required, and
stable instance name. The store additionally salts keys by producer executable
and configured toolchain. Stable names distinguish overflow-checked instances.
The resolver's library content identity includes sorted cfg options and dependency
identities when that content path is used. These existing boundaries must not be
replaced with arbitrary crate-name-based immutability assumptions or treated as
a new audit of every toolchain/configuration combination.

The untouched safety path includes:

- `Entries::usable` checks the named-entry dependency closure, including cycles,
  and checked native imports through the current symbol resolver's
  `provides && linked` predicate. Inventory counts are not its certificate.
- `define_function` retains its scalable-vector native-caller ABI rejection
  before root hydration, and exported symbols remain exported.
- Stored-entry stitch failures retain the misfit ban and restart behavior.
- Denied and previously rejected roots retain their existing treatment.
- The `hydrate`, base, tier, entries, and `onto_rustcs` gates remain unchanged,
  as do the program-specific cache namespace and retained-base behavior.

A real implementation would require actual native fixtures for language-origin
eligibility/naming, root-versus-instance ABI and behavior, successful transitive
cache hydration, missing entries, checked-import mismatch, and refusal/rejection.
No synthetic positive fixture or fake compiler was added to support a rejected
no-op extension. Those implementation acceptance requirements remain unsatisfied,
not silently waived.

## Coordinator baseline evidence

Evidence directory:
`/home/jeremy/.jcode/scratch/blaze-native-engine-20261030/library-root-cache-v38-before/`.
The coordinator's `state.json` reports the scoped transition matrix passed.
It is not a comprehensive workload or a before/after candidate comparison.

| Transition log | Stored-root lookup | Native codegen observation |
| --- | --- | --- |
| `00-cold-prepare-worker-0.log` | 0 of 41,783, 72.6 ms | 56,223 compiled, 0 reused in the final stats |
| `02-manifest-feature-add-worker-0.log` | 6,361 of 41,783, 193.5 ms | 10,833 compiled, 44,995 reused |
| `04-type-recovery-worker-0.log` | 6,929 of 41,783, 275.2 ms | 178 compiled, 55,650 reused |

The recovery log additionally records `base from the store: 6924 roots, 48751
other functions of the checkout`. Lookup candidates, usable roots, final base
roots, and final compiled-function counts are different populations. They must
not be subtracted and labeled saved time, nor do these aggregate counts identify
how many hits came from a particular crate origin.

The recovery timeline records 2,450,152 us in root discovery, 533,707 us defining
code, 173,676 us writing the object, and 178,115 us linking. The Cranelift total
is 30.0 ms for 178 functions, which is not wall time for the entire compile.
Listing telemetry includes a scope traversal taking 1.4 s for a crate with
25,307 bodies, plus other listing work. These timings overlap or describe
subphases and must not be added as independent costs.

A measured next investigation is retaining or reusing correct root-discovery and
listing work across worker recovery, with exact cfg/manifest/source invalidation,
while preserving the complete root/export set. The manifest transition also
needs per-entry identity/miss attribution before blaming a particular guard:
it already hydrates thousands of roots but compiles 10,833 functions. This is a
proposed diagnostic direction, not a claim that listing can safely be skipped or
that any particular time reduction has been demonstrated.

## Freeze and ownership release

`ROOT` is `/home/jeremy/.jcode/scratch/blaze-native-engine-20261030`.
`ROOT/library-root-cache-v38/` preserves full HEAD originals of the five source
files read for the cache contract, the exact unchanged candidate `mod.rs`, an
empty source patch, this document's bytes/patch, and SHA-256 hashes.
The new document did not exist in HEAD. The freeze record explicitly releases
source ownership without authorizing an implementation or claiming acceptance.
No commits were made. There is no v38 binary to build or benchmark, and no
meaningful off/on differential for a flag that was deliberately not introduced.
