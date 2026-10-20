# Native overflow profiles

Status: implemented and validated for the bounded contract below (2026-10-20).
The reconciliation run `20261020-035159955-3782949` passed all 67 selected tests,
including the full arithmetic matrix, authority and graph gates.

## Frozen contract

`SourceOptions::overflow_checks: bool` is an explicit native compilation input,
defaulting to `true`. It is independent of `cfg(debug_assertions)` and
`OptMode::{Off, Adaptive, Final}`. Project profile resolution supplies the
resolved value: dev/test defaults checked, release/bench defaults unchecked,
with explicit profile overrides supported by the project layer.

For all admitted signed and unsigned integer widths, including i128/u128:

| Operation | Checked | Unchecked |
| --- | --- | --- |
| `+`, `-`, `*`, signed unary `-` | Trap on overflow | Wrap modulo declared width |
| `<<`, `>>` | Trap if count is outside width | Mask count by width minus one |
| `/`, `%` | Trap on zero and signed MIN / -1 or MIN % -1 | Same traps |
| Indexing, slice/pointer bounds | Checked | Still checked |
| Explicit panic | Abort | Still abort |

Unsigned unary negation remains an invalid Rust operand operation. Literal
range/type validation, invalid unused items and constant initializer CTFE
errors are unchanged. Static arithmetic-overflow diagnostics remain active in
both profiles, matching stock Rust's default lint. Runtime operands are not
constant initializers: their operations use the selected overflow policy.
True constant initializers always use the distinct checked constant evaluator.

## Graph and authority

The snapshot freezes `SourceOptions` before graph execution. `PreparedSource`
privately captures the overflow value.
Embedded assembly provenance validates the overflow value alongside original
files, roots, origins and debug assertions, rejecting a transplanted policy.
No global or mutable policy carrier was added.

Native object producers read the snapshot policy, including physical opaque
objects. SSA lowering encodes `Overflow::Trap` or `Overflow::Wrap` directly in
each relevant typed operation. Those operations are part of the existing SSA
canonical content hash, so different arithmetic semantics cannot share SSA
content identities. Graph keys remain scoped to their immutable snapshot;
this engine does not yet have cross-snapshot persistent caching. A future
persistent source/object key must include this input.

The stack backend normalizes unchecked word arithmetic to the declared width.
The two-limb backend retains low 128-bit arithmetic and applies masked shift
counts without removing division, bounds or explicit-panic guards. SSA's
existing evaluator, optimizer and native emitter implement the same per-op
policy. Unsupported SSA shapes continue to use the native stack backend.

## Acceptance gates

The primitive numeric and graph matrices explicitly disable the optional
embedded prelude. They still use the real public compiler CHECK/BUILD entry
points, both native backends and executable output, not synthetic IR or mocks.
`primitive_no_prelude_smoke` checks the same native println path against stock.
The opaque physical-body matrix and source/invalid-operand authority tests keep
the default prelude enabled. Project profile integration also retains defaults.
This isolates integer policy coverage from repeated unrelated library checking
without dropping any integer operations, widths, optimizer modes or workers.

Stock metadata-only CHECK omits the MIR `arithmetic_overflow` lint for unused
bodies, while stock BUILD rejects them. Static-lint rejection controls therefore
compare native's eager CHECK diagnostics with stock BUILD. Actual constant
initializer CTFE failures remain separate and are never relaxed by the profile.

- `tests/overflow_profiles.rs`: stock-rustc differential CHECK/BUILD/run at
  workers 1 and 8 and optimizer off/adaptive/final, including all-width wrapping,
  masked shifts, checked overflow and unconditional trap controls.
- `tests/overflow_profiles_authority.rs`: source-option snapshot independence,
  forged prepared stream rejection and invalid unused operand controls.
- Source unit test `changed_overflow_policy_cannot_reuse_assembly_authority`:
  mutation of the private policy cannot reuse embedded source authority.
- `tests/overflow_profiles_graph.rs`: CHECK does not demand code generation,
  checked/unchecked builds remain snapshot-isolated, SSA arithmetic differs.
- `justrust test --test graph_integrity`: unchanged structural ratchets.

This slice does not complete full Rust compatibility or change panic unwinding,
optimization scheduling, integer feature admission, or persistent caching.
No speedup claim is made.
