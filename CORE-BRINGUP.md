# Native core, alloc and std bring-up

## Evidence and source boundary

This is a dependency plan from static source inspection, not executed library
acceptance. Full Rust remains open in IMPLEMENTATION.md. Inspection used the
separate stable checkout's Rust library sources reporting version 1.101.0.
This alone does not establish source identity with a local or hosted stock oracle
toolchain. Record actual oracle version and source revision per run. The inventory
records imported revision d907fba494adcabfdd642a18a7918b0c9847d35b and upstream
1d81eb4ad9cd207e3e638bd32b17ec4fce8412a6. Pin exact file hashes and the target,
edition, cfg/features and profile before claiming unchanged-source acceptance.
The old compiler is reference material, not native compiler internals.

## First unchanged-source probes

The smallest whole module inspected is
`library/core/src/num/imp/flt2dec/estimator.rs`, SHA256
`2d109735cfff1317afd02e093012ee4d4c3b5bf39431da45f6afe99002b0f9b5`.
Its `estimate_scaling_factor(mant: u64, exp: i16) -> i16` requires a doc attribute,
primitive `leading_zeros`, arithmetic, casts and a signed shift. The original
provider expands a numeric macro and calls the ctlz intrinsic. ctlz(0) must
return the input width, so mant=1 is a critical case. Do not assume LZCNT CPU
support. A temporary primitive binding plus an authored client only proves this
module/client combination, not unchanged numeric-provider or core acceptance.
The original estimator test accepts either the exact expected exponent or one
less. An invented exact-equality oracle would be incorrect.

A smaller function-only probe is the original attributed const generic
`core::convert::identity`. Parsing its attributes and const-fn syntax and running
an authored generic client would not prove const evaluation or acceptance of
the whole convert module. Keep those claims separate.

## Dependency-ordered native gates

1. **Source lock and traceability.** Retain source hashes, the active module
   closure and lang/intrinsic inventory. Separate observed static blockers,
   executed first failures and actual unchanged-source successes.
2. **Source expansion.** Classify attributes and process cfg before loading
   modules. Implement original/expansion spans, token trees, macro_rules hygiene,
   repetition, builtins and derives. The current loader's coarse concatenated
   spans and unsupported path attributes are not sufficient for core.
3. **Crate and language identity.** Add library roots, extern crate/preludes/globs
   and source-owned cross-crate identities. Integrate authoritative source
   Copy/Clone/Sized/Drop identities rather than duplicating catalog definitions.
   Current core Sized supertraits exceed the delivered bounded catalog model.
4. **Typed abstraction, memory and const.** Complete trait parameters/associated
   types/projection normalization, ?Sized/coercions, const/lifetime parameters,
   pointers/function values/char/str/unions, recursive layout, aggregate regions,
   actual Drop and unsafe validity. MaybeUninit, ManuallyDrop and UnsafeCell are
   concrete core witnesses, not optional library conveniences.
5. **Typed intrinsic, CTFE and native artifact contracts.** Add signature-checked
   intrinsic/effect lowering, packed target layout/ABI/FFI, statics, drop/panic
   and own library artifacts. Current private eight-byte leaves, u8 stride eight,
   slice-result fence, direct exit-101 trap and executable-only linker are not
   platform or core ABI implementations. CHECK must not demand codegen.
6. **Unchanged core root and no_std client.** Compile its declared source closure
   and publish own crate facts/artifact before claiming core. Identity/estimator
   probes are stepping stones, not completion of this gate.
7. **Native alloc.** Resolve core and compiler_builtins source dependencies, own
   allocator selection/alignment/ZST/reallocation/OOM behavior and Box/Vec/DST
   destruction. Test allocation write/read/free and observable drop counts. Stock
   rlibs cannot mask missing native components.
8. **Native Linux std.** Own manifests/features/build scripts/native dependencies,
   core/alloc/panic/std_detect support, startup/IO/TLS/threads/atomics/synchronization
   and unwind paths. Abort hello/args/IO and thread/mutex/file clients are scoped
   milestones, not full std or full Rust.

## Ownership and acceptance

Freeze contracts before shared-file edits. Source/frontend owns attributes,
cfg/spans/expansion, the type world owns source lang identities and normalization,
IR/CFG plus CTFE owns effects/layout, ownership owns validity/drop cleanup, the
backend owns instruction/runtime/ABI/linking, the graph engine owns crate roots
and publication, instances own sealed cross-crate specialization, and the project
layer owns dependency/features/build orchestration. Independent conformance must
verify unchanged input bytes, native-only compilation and diagnostics/unused
invalid bodies at multiple workers. Current workload, closure-size and platform
coverage remain unknown until measured and executed.
