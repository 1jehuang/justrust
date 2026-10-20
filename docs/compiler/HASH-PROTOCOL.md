# Hash protocol (prerequisite for HashMap)

Status: offline candidate, not built. Owner: gorilla. Approved scope:
`libraries/prelude/hash.rs`, `src/compiler/source_derive.rs::gen_hash` only,
plus focused tests and this document. No other compiler edits.

## Audit of current users (on ce1db978a)

| User | Uses |
|---|---|
| `libraries/prelude/hash.rs` | `trait Hash { fn hash(&self, state: &mut DefaultHasher); }`, `trait Hasher { finish, write_u64 }`, `DefaultHasher` (xorshift over u64 bytes), impls for i8..usize, bool, char, 1-4 tuples |
| `src/compiler/source_derive.rs::gen_hash` | emits `impl std::hash::Hash for T { fn hash(&self, state: &mut std::hash::DefaultHasher) { ... } }`, discriminant via `<DefaultHasher as Hasher>::write_u64` |
| `libraries/prelude/lib.rs` `core_surface::hash` | re-exports `Hash`, `Hasher` |
| full_programs 20 | `#[derive(..., Hash, ...)]` on a user enum (via derive) |
| tests | no test exercises `Hash`/`Hasher`/`DefaultHasher` directly; no test derives `Hash` |
| unactivated `collections.rs` (gorilla clone) | `DefaultHasher::new`, `Hash::hash`, `Hasher::finish` |

No other prelude, compiler or test code names the protocol.

## Frozen target contract (std-shaped)

```rust
pub trait Hasher {
    fn finish(&self) -> u64;
    fn write(&mut self, bytes: &[u8]);
    fn write_u8(&mut self, i: u8) { self.write(&[i]) }
    // Provided, matching core::hash dispatch exactly:
    // write_u16/u32/u64/u128/usize: ONE `write` call with the value's
    //   native-endian bytes (2/4/8/16/8 bytes, little-endian on x86-64).
    //   Never split, never routed through another overridable write_*.
    // write_i8/i16/i32/i64/i128/isize: delegate to the same-width unsigned
    //   method (isize -> write_usize, not write_u64).
}
pub trait Hash {
    fn hash<H: Hasher>(&self, state: &mut H);
}
```

Observable consequence, gated by `provided_write_methods_dispatch_like_core`:
a user `Hasher` overriding `write_u32`/`write_u64` sees i32 and i64 reach
those overrides, while u16/u128/usize/isize/bool/char and derived enum
discriminants reach `write` with exactly one call of native width. The
expected output is stock rustc's output for that exact source (verified).

`DefaultHasher` keeps its existing mixing, moved behind `write(&[u8])` (one
byte per step). For one eight-byte `write` call this equals the old
`write_u64` loop.
No SipHash claim; std's `DefaultHasher` output is unspecified, so no
stock-differential test may print hash values.

Impls in this slice: the existing primitives and tuples, re-expressed through
`write_*`. `str`/`String`/references/arrays/slices/`Option`/`Vec` are a later
slice (per coordinator). No `BuildHasher`/`RandomState` in this slice. `RandomState` with fixed keys is not acceptable and will not be added.

## Boundary (why it cannot land prelude-only)

The derive generator emits the non-generic signature. Once the trait method is
generic, every derived impl fails to match the trait (E0049/E0053 class), which
would regress `#[derive(Hash)]`. Required compiler change, one function:

- `gen_hash`: `fn hash<__H: std::hash::Hasher>(&self, state: &mut __H)` and
  the discriminant as `std::hash::Hasher::write_isize(&mut *state, d)`.

The prelude half and this edit land together. Drafted `gen_hash`:

- method binder `__JrHasher`, extended with `_` until it differs from every
  type, lifetime and const parameter of the input (unit test covers a
  `struct Q<__JrHasher>` collision);
- enum discriminant: same `isize` variant index as before, written through
  `<__JrHasher as std::hash::Hasher>::write_isize`; field order, field bounds
  (`T: Hash` via the header) and pattern binders (`a0..`) unchanged;
- the derived impl no longer names `DefaultHasher`.

Impact: hash VALUES of derived enums change (discriminant was
`write_u64(d as u64)`, now `write_isize(d)`, the same 8 LE bytes, so for
DefaultHasher the value is in fact unchanged). No test or program prints hash
values. `BuildHasher`: only an explicit `BuildHasherDefault` in a later slice;
`RandomState` stays absent until there is genuine random seed authority.

## Gates (when unblocked and a slot is granted)

- `tests/hash_protocol.rs`, workers 1 and 8, stock-differential run output:
  manual std-form `impl Hash` (generic `H`), derived `Hash` on struct/enum,
  `Hash` via a user `Hasher` that records the byte stream (stock and native
  must print the same recorded bytes: this IS comparable, unlike
  DefaultHasher output), equal values hash equal through `DefaultHasher`.
- Invalid: old non-generic `fn hash(&self, &mut DefaultHasher)` impl rejects
  (as stock); `Hash` on a type with a non-`Hash` field rejects.
- graph_integrity unchanged; full_programs ratchet unchanged.

## Trait-call closedness (traits.rs `resolve_trait_call`)

Derived `hash<__JrHasher>` bodies call `<Field as Hash>::hash::<__JrHasher>`:
a concrete `Self` with an open method actual. Previously the Universal branch
tested only the trait goal's closedness, so such calls fell through to the
closed re-resolution in the world's empty environment, which then rejected
the caller's own parameter ("type parameter outside authoritative metadata
environment"). Contract now:

- Universal phase and (goal open OR any ordered method actual open, judged by
  the same `validate_ty_inner`/`TypeWalk` used by `goal_is_closed`): resolve in
  the caller's environment and phase, `TraitItem` (or `Virtual` for `dyn`)
  callee, caller signature/topology, `require_in` evidence in the caller
  environment. Concrete callee selection happens at genuine closed
  instantiation; no obligation is dropped, since the caller signature and
  predicate are discharged before the branch, as before.
- Goal and all method actuals closed: unchanged empty-environment closed
  dispatch with the same exact-evidence checks.
- Closed-phase facts with open arguments still reject; validate_ty_metadata,
  goal_is_closed and the projection branch are unchanged.

Gates: `forwarded_open_method_actuals_keep_caller_environment` (open forwarded
actual under a genuine bound runs as stock; fully closed `emit::<Log>` call
runs; the same forwarding without the bound rejects natively as stock E0277).
Wrong-owner/unknown-parameter metadata validation is unchanged and still
covered by the existing authority suites. The temporary diagnostic in
associated_types.rs is removed before promotion.
