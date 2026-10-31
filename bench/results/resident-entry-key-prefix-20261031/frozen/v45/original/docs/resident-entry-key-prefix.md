# Resident entry key prefix experiment, v44

Experimental licensed Blaze-derived resident, not clean-room work. Base accepted
`cb9d8a01000e2a1efb44e87e74803b4317cdf997`. This is a default-off hypothesis,
not a measured speedup or a promotion decision.

## Scope and switch

Only `RA_JIT_ENTRY_KEY_PREFIX=1` enables preparation. Unset, empty, `0`, `true`,
`01`, and whitespace variants stay on the untouched `Entry::key` direct path.
The switch is read for each `Entries::new`, not memoized globally. No environment
mutation is used by the fixtures. `get` and `prefetch` use the selected path.
Writers, Store, cache namespace, schema number, and serialization are unchanged.

One SHA-256 state is owned by each Entries compilation. Its storage is fixed
size, with no per-name map, retained formatted strings, or global hasher cache.
Each lookup clones that state and appends the remaining fields. Names are still
hashed in full, including NULs and multibyte UTF-8. The original `Entry::key`
implementation remains an independent native oracle and disabled control.

## Exact bytes and lifetime constraints

Each field retains its eight-byte little-endian u64 byte length, followed by
its complete bytes, in this order:

1. `ra-jit entry 12`
2. empty bytes for no program, otherwise all 32 program bytes
3. backend Display bytes
4. only for a nonzero MIR optimization level, `[b'M', level]`
5. selected target triple bytes
6. only when enabled, `frame pointers`
7. only when enabled, `link dead code`
8. one byte indicating Told is not Nothing
9. only for Told::Lines, `lines alone`
10. name bytes

The prepared state stops after field 5. Backend selection is cached by the
existing `Backend::from_env` OnceLock in `backend.rs`. MIR optimization is the
existing `mir_opt_level` LazyLock in `mod.rs`. `Target::select` in `target.rs`
uses a OnceLock and rejects a different subsequent triple. The existing target
contract requires selection before compilation. Program bytes are copied into
Entries at construction and are not mutated by production methods. Debug mode
is likewise owned by Entries, but remains in the suffix to preserve ordering.

**Frame-pointer and link-dead-code settings cannot safely be snapshotted here.**
`backend::keep_frame_pointers` and `backend::link_dead_code` write process-global
AtomicBools. The former is called at `CodegenUnit::compile_or_hold` entry, and
the latter through `rustc_links_dead_code`. There is no type-level exclusion
preventing another compilation from writing them during this Entries lifetime.
Both are therefore read on every prefixed key, in the same order as the direct
path. The experiment removes repeated backend/target/MIR reads and backend
formatting, plus prefix hashing, but deliberately does not remove these two
mutable config reads. This is not a claim to fix existing concurrent compilation
configuration semantics.

A failed backend/target preparation yields no prepared context. Subsequent
lookups retry through the original direct function. No unavailable-context key
is fabricated and preparation failure is not itself cached as an entry miss.
Enabled preparation initializes the existing immutable selectors at Entries
construction rather than the first uncached name lookup. Disabled preparation
does not consult them at all.

## Native fixtures and validation boundary

New cfg(test) fixtures are in the actual `cached.rs` test module:

- `entry_key_prefix_gate_is_exact`: exact switch parsing, no preparation on the
  off path, and enabled unavailable context, using injected closures, not env writes.
- `entry_key_prefix_matches_native_original`: actual untouched Entry::key versus
  prepared output for all Told modes, None and three Some program patterns,
  empty/ASCII/NUL/Unicode names and a 20,485-byte UTF-8 name.
- `entry_key_prefix_schema12_optional_fields_and_boundaries`: independent explicit
  field-byte assembly for optimization 0/1/2/255, every frame/dead flag combination,
  all Told modes, program None/Some, and adversarial field boundaries. This
  supplements, not replaces, the actual native oracle comparison.
- `entry_key_prefix_native_store_get_prefetch_and_unavailable_context`: real
  Store insertion and sealing, direct-key publication, get, transitive prefetch,
  missing entries, and on/off/unavailable-preparation modes. It preserves the
  accepted native fixture seal boundary and never changes writer semantics.

Only source rustfmt and diff whitespace checks were run by this lane. No native
build, test, workload, lock change, or commit was performed. The coordinator must
run native suites and actual strict varied workflows on/off, inspect failures and
regressions, and reject this experiment if it is not useful. Whole build.rs
source fingerprinting applies. No auxiliary repository source helper was added.

## Motivation, not performance evidence

The actual v43 manifest observations were 45,508 missing-read attempts and
187,729 get-positive returns. Recovery observed 34,900 missing reads. These
counts motivate investigating repeated key preparation only. Get-positive calls
can return already settled cells without deriving a key, so their count is not
a key-derivation count or estimated savings. No speed claim follows from them.

## Frozen handoff

Exact original and candidate source, the new documentation, candidate patch,
SHA256SUMS, BASE_HEAD, original-absence receipt for this new document, and
FROZEN_READY.txt are preserved in
`/home/jeremy/.jcode/scratch/blaze-native-engine-20261030/entry-key-prefix-v44`.
Only the two assigned repository paths are candidate inputs. Once explicitly
released, this lane will not edit them while the coordinator validates hashes,
builds, and runs the candidate.
