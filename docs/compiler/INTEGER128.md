# Native 128-bit integer implementation contract

## Status

Implementation adds native i128/u128 to the bounded Linux x86-64 compiler after
source-alias checkpoint `24ea35b7a`. The independent 92-source stock oracle and
all 368 native CHECK/BUILD cells pass, including executable runtime parity and
intentional traps. All 1,677 tests across 89 binaries, hosted all-target check,
strict all-target clippy, formatting and diff hygiene pass.
This is not full Rust compatibility. No speedup claim follows from adding
arithmetic semantics.

## Shared semantic representation

- Append `IntTy::I128` and `IntTy::U128`, preserving existing primitive identities.
- Keep `IntTy::bits() -> u32`, extend `mask() -> u128`, and retain exact names and
  signedness. A width-128 mask never shifts by 128.
- `IntValue { ty: IntTy, bits: u128 }` stores canonical raw low bits. Existing
  narrow values must not acquire high bits. `usize(u64)` remains source-compatible.
- Frontend nonnegative literal magnitudes use checked u128 accumulation, with
  exact contextual bounds and signed-minimum grouped-literal handling.
- Integer constants, patterns, recipe keys and equality preserve both limbs.
  Wide integers remain semantic scalar Copy values, not aggregates or references.

## Private native transport

A wide scalar occupies 16 bytes at private alignment 8, low limb at offset 0 and
high limb at offset 8. Its expression result is a snapshot address in rax. A call
passes one snapshot-pointer word and copies both limbs into the callee's own
local storage. Returns use existing caller-owned sret transport. This private
layout is not repr(C), platform Rust ABI, or interoperable FFI support.

Wide scratch is allocated even without any aggregate constructor. Both operands
are snapshotted before the next constant or expression can reuse scratch. Every
load/store, projection, reference, index, array/repeat, mixed aggregate, enum
payload, argument and result path must preserve all 128 bits.

## Arithmetic and casts

The existing checked native arithmetic policy remains: admitted runtime
arithmetic failures terminate with status 101. Signed/unsigned add/sub/mul,
signed negation, comparisons, bitwise operations, shifts, div and rem must be
exact at 128 bits. Guard zero divisors and signed MIN/-1 for both division and
remainder. Shift counts require full source-width validation before narrowing,
with admitted counts 0 through 127. Never encode comparison with 128 through a
sign-extended x86 imm8 encoding.

Integer casts extend according to source signedness, then retain destination
low bits and normalize. The complete 12-by-12 primitive cast matrix is a gate.
Float-to-wide casts decode IEEE values and truncate/saturate, with NaN to zero.
Wide-to-float casts round directly to destination precision using exact guard,
round and sticky information. A f64 intermediate is not acceptable for f32.
No external compiler, assembler, linker or compiler-rt delegation is introduced.

Compile-time counts cannot use host i128 as a wider-than-all-values accumulator.
Use checked arithmetic, exact signed/unsigned boundaries and full sign/high-limb
checks before usize conversion. CHECK and BUILD must admit the same operators.
Any unimplemented endpoint needs an explicit semantic unsupported fence rather
than a legacy 64-bit fallback.

## Backend ownership seam

`backend.rs` owns layout, snapshots, constants, routing and integer-to-integer
casts. `backend/wide_int.rs` owns emitted unary/binary integer math.
`backend/wide_float.rs` owns emitted wide IEEE conversions. Child entry methods
are `pub(super)` inherent methods on Emitter, with distinct helper prefixes.

The rbp-relative `wide_scratch` region has 256 bytes: lhs 0..15, rhs 16..31,
result 32..47, math workspace 48..255. Input integer limbs are canonical sign- or
zero-extended to 128 bits, float inputs use raw low IEEE bits and high zero.
Wide results write both result limbs and return its address. Comparisons return
normalized bool in rax. Narrow integer/float casts return their raw scalar rax
convention. All registers except rbp/rsp may be clobbered, rsp must be balanced,
and there is no nested operand evaluation in arithmetic helpers.

## Acceptance and remaining boundaries

`tests/wide_integers.rs` first classifies all 92 exact sources with an executable
stock compiler, then checks native workers 1 and 8 in CHECK and BUILD: 60 ordinary
valid sources, 18 declared runtime-101 cases, 13 invalid sources and one separately
legal unsupported bool-to-wide cast. Successful BUILD fixtures require native ELF
and exact runtime status/stdout. CHECK and rejected inputs publish no executable.
The corpus includes the unchanged historical 128-bit type, cast, bitwise, shift
and compound-assignment sources, promoted only after exact source parity.

Native emitted-code unit oracles independently compare full-width integer
arithmetic and roughly 25,000 IEEE cast results with bootstrap Rust. Wide integer
oracle execution uses bounded OS-26-only retries before process creation, never
rerunning an executed image. A concurrent 8-by-8 executable test and deterministic
retry/error policy tests cover the transient writable-inode race.

The source suite includes mutable-array-reference indexed compounds and an
index expression that rebinds the reference. Stock and native both select the
current carrier after RHS and index effects. Every dereference must confer
mutable authority, including dead CFG statements and stored field carriers.
Whole-array initialization and live shared-loan conflicts remain checked.
This does not generalize all aggregate-reference language forms.

Canonical graph checks preserve alias/native identity while distinguishing
constants with equal low limbs and different high limbs. Unused malformed wide
values, full-width static counts and stable instance keys are checked. Physical
backend scratch planning consumes sealed operand layout without re-proving
nominal Copy with weaker standalone facts.

Retained profiles in `data/runs/xps14/2026-10-17-integer128.jsonl`:

- all-target check: `20261017-045445276-83224`;
- complete tests: `20261017-045447149-83291`;
- strict all-target clippy: `20261017-045502213-84341`.

The complete test execution took 12.90 seconds; the slowest stock-first native
corpus test took 12.32 seconds. These are observed validation costs, not a
before/after compiler speedup. General const
evaluation, profile-exact panic/unwind semantics, bool/char/enum/pointer casts,
platform Rust/C layout/FFI, broader inference, core/alloc/std and existing-project
acceptance remain open in IMPLEMENTATION.md. The private wide transport does not
establish those contracts.
