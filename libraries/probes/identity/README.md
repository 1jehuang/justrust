# Original identity function probe

This fixture is a function-level runtime probe, not acceptance of the whole
`core::convert` module, the `core` crate, or full Rust.

## Source provenance

`convert.rs` is a byte-for-byte copy of the original provider file
`library/core/src/convert/mod.rs` from the vendored Rust source tree at
`/home/jeremy/justrust/toolchain/rustc` when this probe was authored.

- Upstream revision: `d907fba494adcabfdd642a18a7918b0c9847d35b`.
- Upstream revision date: October 14, 2026.
- Vendored `src/version`: `1.101.0`.
- Local monorepo source import: `8f0a1aeef27a0bb43b0e405c86e8a9dcd2c2448c`.
- Full provider: 33,122 bytes, SHA256
  `0fd346eff31bfa6cc7c79fc4cba8367e98c579ccd7072dd141ce0bea76b602ac`.
- Selected original docs, attributes, and function: lines 48 through 107 inclusive,
  zero-based byte interval `[2258, 3893)`, 1,635 bytes, SHA256
  `c4187b524514fd5daf107703898a3489da0cdf7806eea13988f4f7a365c4d6a8`.
- The first original attribute starts at byte 3663, line 101.

The strict manifest is `../../source-lock.json`. Its path is relative to the
manifest directory. Both the full provider and the selected byte range are
verified before private metadata is admitted under the trusted probe policy.
The complete provider is retained for original-file source locations, but only
the selected unchanged function envelope is emitted into the client root.
No sibling declarations are claimed to compile.

The source is copyright The Rust Project Contributors, dual-licensed under
`MIT OR Apache-2.0`. Exact upstream `LICENSE-MIT`, `LICENSE-APACHE`, and
`COPYRIGHT` are preserved alongside the original provider. Attribution is not
replaced with the authored client or test harness license.

## Preserved original attributes

All four original attributes and the `const` qualifier remain unchanged:

```rust
#[stable(feature = "convert_id", since = "1.33.0")]
#[rustc_const_stable(feature = "const_identity", since = "1.33.0")]
#[inline(always)]
#[rustc_diagnostic_item = "convert_identity"]
pub const fn identity<T>(x: T) -> T {
    x
}
```

Probe metadata is inert. It must not register a source-owned diagnostic/language
identity, enable feature gates, grant const evaluation, or change ordinary client
attribute policy. Clients call the original root declaration as `identity(...)`.

## Oracle layers, measured separately

The initial installed stock toolchain was `rustc 1.98.1`, commit
`48a229ceaefd4985c50990b14116b6d856af0985`, dated September 1, 2026,
host `x86_64-unknown-linux-gnu`, LLVM 22.1.8. This is not the vendored source's
1.101.0 revision. Tests print `rustc -Vv` for each actual oracle environment.
All behavioral compilations pin edition 2021, Linux x86-64 target, optimization
level 0, overflow checks enabled, and panic abort.

The first hosted oracle run used the installed pinned toolchain
`nightly-2026-10-08`: `rustc 1.101.0-nightly`, commit
`1d81eb4ad9cd207e3e638bd32b17ec4fce8412a6`, commit date October 7, 2026,
LLVM 23.1.3. Its exact-source ordinary and bootstrap-wrapper policy rejections
were the same as the initial local stock experiment. Although both it and the
vendored source report 1.101.0, their source revisions differ. Hosted stock
behavior is not described as execution of the October 14 vendored provider.

1. **Exact original source, ordinary stock crate:** rejected with E0658 because
   stability attributes are outside the standard library and
   `rustc_diagnostic_item` is an internal attribute. This is a policy restriction,
   not evidence that the Rust standard-library function is invalid.
2. **Explicitly authorized stock-policy experiment:** the exact original source
   is placed in an authored module under `#![feature(staged_api, rustc_attrs)]`
   and `#![stable(feature="probe", since="1.0.0")]`, with `RUSTC_BOOTSTRAP=1`
   scoped only to that stock child. On the initial installed stock toolchain,
   this rejects a duplicate diagnostic item `convert_identity`, first defined
   by the installed `core`. This is not exact-source executable acceptance.
3. **Behavioral reference:** authored clients import the installed
   `core::convert::identity`, separately testing runtime primitives, non-Copy
   moves and forwarding, references, arrays/tuples, and invalid ownership/loans.
   This is not described as the same-source stock oracle.
4. **Native original-source probe:** authored clients have no stock import and
   explicitly request the locked original function. Native subprocess lookup
   of rustc is poisoned. No stock path is a native fallback.

The test suite keeps ten runtime-positive cases, six ownership/unused-invalid
cases, one stock-invalid const-body/native-unsupported boundary, and one
stock-valid const-required/native-unsupported boundary separate. Rejection of a
legal unsupported const-evaluation client is not a runtime acceptance result.
Rejecting a non-const call outside the constrained const-body grammar is not a
claim to diagnose all invalid Rust const semantics.

CLI and source-lock rejection cases separately test pairing and duplicate
arguments, absent items, malformed/unknown manifest fields, hash/range mismatch,
provider mutation, and attempted client internal-attribute spoofing. They are
protocol/authority tests, not additional Rust-language conformance cases.
