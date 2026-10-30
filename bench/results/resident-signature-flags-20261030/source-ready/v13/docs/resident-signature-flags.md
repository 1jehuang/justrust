# Resident signature comptime flag candidate

## Scope and mechanism

This bounded native resident candidate uses bit 15 of the existing `FnFlags: u16` as `RUSTC_COMPTIME`. `FunctionSignature::with_source_map` computes it using the unchanged original predicate `AttrFlags::has_word(db, id.into(), "rustc_comptime")`. `compile_time_only` reads `FunctionSignature::of(db, func).flags`. No new tracked query, tracked wrapper, custom AST scan, main.rs change, or clean-room change is involved.

The inspected consumers test individual flags rather than treating the complete bitset as a closed enumeration. Existing signature equality now includes the comptime bit. The u16 representation is unchanged. Function signature lowering already reads source and attribute state. The direct predicate executes within that existing tracked query, so Salsa records its original source and cfg dependencies there rather than in a new query node. Attribute edits must invalidate this result even when the compact general `AttrFlags` result itself backdates unchanged.

## Semantics and native differential gate

`AttrFlags::has_word` only accepts a matching single-segment path meta after cfg_attr expansion. Token-list, key-value, and qualified-path forms do not qualify. The implementation deliberately calls that predicate rather than approximating its syntax or trusting general attribute flags.

The new hir-def test `resident_signature_comptime_matches_direct_attributes_across_revisions` discovers functions through the native DefMap and trait/impl associated item collections. It compares the signature bit against an independent ORIGINAL DIRECT `AttrFlags::has_word` invocation, reading the signature first in every database revision. Coverage includes bare and absent attributes, enabled/disabled/nested cfg_attr, trait declarations/default bodies, inherent and trait impl methods, token-list/key-value/qualified forms, add/remove, cfg_attr toggles, form-to-word changes, restoration, and body-only edits. Initial expected values guard fixture meaning in addition to the differential oracle. Body-only edits must leave the signature value equal.

Root must run this test in the native hir-def test target, the existing native root classification/diagnostic and runtime semantic gates, and matched performance gates on frozen exact bytes. This worker runs source-only formatting and diff checks, not builds, tests, or workloads. A passing source check is not evidence that native tests pass. Diagnostics and useful source dependencies remain acceptance requirements, not assumed results.

## Cost and retention decision

There is no assumed speedup. A warm classification may reuse an already-needed signature instead of repeatedly collecting attributes. Conversely, a cold classification that previously only inspected attributes can now force complete signature lowering (generic parameters, parameter and return types, ABI and source maps). Every lowered function also pays one additional direct attribute collection, even if codegen never classifies it. Attribute or cfg changes can cause additional invalidations. The extra bit does not enlarge `FnFlags`, but this does not establish a memory or runtime win.

Retain only after root's frozen build/semantic checks and matched before/after native measurements establish correctness and an acceptable workload outcome. Inspect cold and incremental behavior separately. Reject or revise on semantic, diagnostic, dependency, or performance regressions without weakening the direct oracle. No commit or promotion is authorized before those gates.
