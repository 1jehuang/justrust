# Shared byte-slice rejection corpus

These are standalone Rust 2021 binary crates, not modules of a Cargo test target.
They freeze negative input for the bounded shared `&[u8]` milestone described in
`next-real-program-worker-proposal.md`. No native compiler acceptance or rejection
is claimed by this corpus or its stock validation.

Each source starts with machine-readable `// stock: accept|reject` and
`// category: ...` headers agreed with the public CLI rejection harness owner.
Both groups are expected to be rejected by the bounded native milestone, but for
very different reasons. `invalid_*` inputs violate Rust semantics. `unsupported_*`
inputs are valid Rust that deliberately cross the milestone's admission fence.
Do not describe the latter as Rust errors or silently accept them without their
required semantics. All function bodies must be validated, including unused ones.

## Exact cases and stock results

Line references below include the two classification header lines. Stock checks
were run on October 16, 2026 local time (October 17 UTC), with the command below.

| Source | Classification | Relevant lines | Stock exit | Observed diagnostic / boundary |
| --- | --- | --- | --- | --- |
| `invalid_wrong_lifetime.rs` | Rust-invalid, lifetime | 7-10 | 1 | E0597: local storage expires before the forwarded slice read |
| `invalid_unused_lifetime.rs` | Rust-invalid, unused body | 3-11, especially 7-9 | 1 | E0597 even though `main` never calls `unused` |
| `invalid_unused_unknown_name.rs` | Rust-invalid, unused body | 3-6, especially 4 | 1 | E0425: unknown value `missing_byte` |
| `invalid_unused_unknown_type.rs` | Rust-invalid, unused body | 3-6, especially 4 | 1 | E0425: unknown type `MissingType` |
| `invalid_bare_slice_parameter.rs` | Rust-invalid, unsized by-value parameter | 3 | 1 | E0277: `[u8]` is not `Sized` |
| `invalid_bare_slice_local.rs` | Rust-invalid, unsized local | 5 | 1 | E0308 and E0277: an array is not a by-value slice and the local is unsized |
| `unsupported_mutable_slice.rs` | Legal but unsupported | 3, 6 | 0 | Mutable slice parameter, mutable unsizing, mutation |
| `unsupported_slice_return.rs` | Legal but unsupported | 3, 6 | 0 | Input-derived slice return, explicitly tied lifetime |
| `unsupported_generic_slice.rs` | Legal but unsupported | 3, 6 | 0 | Generic slice parameter and instantiation |
| `unsupported_non_u8_slice.rs` | Legal but unsupported | 3, 6 | 0 | Shared slice of `u16`, not the admitted `u8` |
| `unsupported_slice_range.rs` | Legal but unsupported | 4 | 0 | Shared suffix subslice formed with a range |
| `unsupported_slice_iteration.rs` | Legal but unsupported | 5 | 0 | Slice iteration and reference dereference |

The bare slice cases are intentionally **Rust-invalid**, not merely a current
compiler limitation. The legal return fixture does not return local storage.
Conversely, the lifetime cases avoid slice-return contracts so they test a local
origin escaping its scope without relying on an already excluded return feature.
The unused name/type cases are independent of reachability and dead-code emission.

## Reproduce the stock oracle

From the repository root, with a fresh writable scratch directory:

```sh
out=$(mktemp -d /home/jeremy/.jcode/scratch/slice-rejection-stock.XXXXXX)
rustc -Vv > "$out/version.txt"
for source in tests/fixtures/slice-contract-rejection/*.rs; do
    name=$(basename "$source" .rs)
    rustc --edition=2021 --crate-type=bin --emit=metadata "$source" \
        -o "$out/$name.rmeta" > "$out/$name.stdout" 2> "$out/$name.stderr"
    status=$?
    printf '%s %s\n' "$name" "$status"
    cat "$out/$name.stderr"
done
printf 'OUTPUT_DIR=%s\n' "$out"
```

Do not enable `set -e` around this loop: rejection is the expected result for six
inputs. Inspect each exit status against the table or use the separately owned
rejection harness. This command only type-checks and borrows-checks the exact
source files. It does not execute binaries or invoke the native compiler.

Recorded `rustc -Vv`:

```text
rustc 1.98.1 (48a229cea 2026-09-01) (Arch Linux rust 1:1.98.1-1)
binary: rustc
commit-hash: 48a229ceaefd4985c50990b14116b6d856af0985
commit-date: 2026-09-01
host: x86_64-unknown-linux-gnu
release: 1.98.1
LLVM version: 22.1.8
```

The original stdout/stderr and metadata outputs are retained locally in
`/home/jeremy/.jcode/scratch/slice-rejection-stock.PxS3A0/`; they are not repository
artifacts. The durable evidence is the exact committed source, compiler identity,
reproduction command and result table above.

## Native integration and limits

The separate public CLI harness owns native `--check` and build matrices at
workers 1 and 8, graph assertions, and failed-output publication checks. A negative
corpus alone cannot establish that native slices work: require a positive admitted
slice control in that harness so an old parser rejecting every slice cannot pass
as an implementation. Match rejection and meaningful diagnostics rather than
requiring stock diagnostic text or stock error codes from the native compiler.

Unchecked here: native compilation, code generation, runtime bounds behavior,
positive Adler checksums, graph/cache behavior, mutation conflicts, uninitialized
reads, trait indexing, and a full lifetime/generic test matrix. No performance or
full-Rust-support claim. Arrays of `u8` are Copy, so copying an array while borrowed
is not a valid negative move test. These files intentionally make no such claim.
