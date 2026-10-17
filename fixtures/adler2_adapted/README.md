# Adapted Adler2 recurrence acceptance

This is **not unchanged Adler2 compilation**. The native fixture is a serial
adaptation of the Adler recurrence documented in Adler2 2.0.1 `src/algo.rs:5-9`.
It tests the prerequisite shared-byte-slice capability through the public CLI.
No host checksum call is substituted inside the adapted program.

## Provenance and changes

`upstream/lib.rs` and `upstream/algo.rs` are byte-for-byte original source from
Adler2 2.0.1, copied from the local crates.io registry package. Their SHA-256 pins
are enforced by `experiments/adler2_adapted_acceptance.py` before any compilation:

- `lib.rs`: `4acd41668fe30daffa37084e7e223f268957b816afc1864ffb3f5d6d7adf0890`
- `algo.rs`: `932c2bc591d13fe4470185125617b5aaa660a3898f23b553acc85df0bf49dded`

The original is licensed under 0BSD OR MIT OR Apache-2.0. This fixture uses the
0BSD option, retaining its original copyright and license in
`upstream/LICENSE-0BSD`. Upstream files are used only by the **stock oracle**.

`recurrence.rs` replaces the original four-lane `U32X4` algorithm, chunking,
iterators, generic operator traits, struct state, and conversions with an ordinary
function, `&[u8]`, `.len()`, checked dynamic indexing, a `usize` while loop,
primitive casts and per-byte `% 65521`. It carries state in the packed `u32`
checksum between calls, rather than returning/mutating a struct. Modulo on every
byte preserves the mathematical recurrence while avoiding overflow. It does
**not** exercise upstream SIMD-like scheduling or performance. Original library
features, const fn, Hasher, Default, I/O, cfg and generic implementations are not
claimed supported by the native compiler.

Each generated driver binds concrete byte arrays, calls the adapted helper with
array-to-slice coercion, and compares its actual full 32-bit result with the
separately compiled original Adler2 oracle. Since the native language subset
supports only literal `println!`, it prints the exact `name:checksum` literal
**only after runtime equality**, otherwise `CHECKSUM_MISMATCH`. This is not a
truncated exit-code comparison. The harness requires exact stdout and success.
The oracle also independently agrees with Python stdlib zlib over concatenated
bytes, including incremental calls separated by empty chunks.

## Executable commands

From the repository root, run the stock-only gate now:

```sh
python3 experiments/adler2_adapted_acceptance.py \
  --output-dir "$HOME/.jcode/scratch/adler2-stock"
```

After the integrated native compiler is built, run the complete differential:

```sh
python3 experiments/adler2_adapted_acceptance.py \
  --native ./target/debug/jr-rust \
  --output-dir "$HOME/.jcode/scratch/adler2-native"
```

The runner compiles original Adler2 only with stock rustc. For each identical
adapted source it runs stock rustc, then public native CLI checks and builds at
workers **1 and 8**, then executes the native artifacts:

```sh
./target/debug/jr-rust "$HOME/.jcode/scratch/adler2-native/four.rs" --workers 1 --check
./target/debug/jr-rust "$HOME/.jcode/scratch/adler2-native/four.rs" --workers 8 -o "$HOME/.jcode/scratch/adler2-four"
"$HOME/.jcode/scratch/adler2-four"
```

Generated sources and `evidence.json` remain in the output directory. The exact
stock-validated source corpus is also committed under `cases/` (adapted `.rs`
and corresponding `.oracle.rs` drivers), with hashes in `stock-evidence.json`.
For example, `jr-rust fixtures/adler2_adapted/cases/four.rs --workers 8 -o /path/to/four`
can exercise one retained case directly. Generated drivers deliberately retain
their byte-exact generator formatting for hash reproducibility. Executables
use fresh temporary paths to prevent stale-artifact successes. A failed run
removes previous evidence rather than preserving a misleading old pass.
The report records rustc `-Vv`, exact source hashes, per-case checksum/output,
and, when supplied, native binary hash and workers 1/8 outcomes. The native
compiler is not rebuilt, installed, or invoked by the stock-only gate.

## Checked evidence and boundaries

`stock-evidence.json` records the stock run made October 17, 2026 UTC. All 16
cases pass original Adler2 versus stock-compiled adaptation versus zlib: empty,
singleton zero/255, mixed 4/16/257 bytes, all-255 at 5551/5552/5553,
22207/22208/22209/44417 bytes, and three incremental split-call cases including
empty calls. The lengths surround both the serial overflow discussion boundary
5552 and upstream four-lane chunk boundary 22208. A stock mutation check changed
the four-byte equality guard to compare against zero while retaining the success
literal: execution correctly emitted `CHECKSUM_MISMATCH`, not the success literal.

**Native status: not run, pending integration.** This stock evidence is not a
native success claim. The optional native gate fails hard on rejection or checksum
mismatch, with no stock fallback or expected-failure bypass. Wrong-lifetime,
unsupported-syntax, aliasing, and out-of-bounds rejection coverage belongs to the
separate shared-slice contract acceptance worker. This fixture is positive
real-recurrence coverage, not exhaustive slice soundness or full crate support.
No throughput or compile-speed claim is made.
