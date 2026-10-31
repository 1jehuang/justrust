# Rejected resident entry key prefix experiment, v44/v45

Experimental licensed Blaze-derived resident, not clean-room work. Base accepted
`cb9d8a01000e2a1efb44e87e74803b4317cdf997`. This is a default-off hypothesis,
not a measured speedup or a promotion decision.

## Decision: reject and restore

v45 did not demonstrate a repeatable useful win. The prototype was rejected,
and `cached.rs` was restored from the full accepted original backup on
2026-10-31 at 03:59:19 UTC. Both `cmp` and `git diff --exit-code HEAD` passed.
The restored source SHA256 is
`11543e4a6a48afec1c23819a999cf6835ed2c1d7990919aea70cabe134c2906f`.
Neither the prefix nor its activation marker remains in production source.
The design and fixture descriptions below are historical candidate descriptions.

All 28 v45 strict rows passed, with one successful-preparation marker in each
cumulative ON worker-log snapshot and zero in each OFF snapshot. These are
cumulative snapshots of one prepared context, not seven independent activations.
Both v44 and v45 native suites passed 57 codegen tests and 360 engine tests.
The original v44 positive-activation admission failed and remains unadmitted.

Actual v45 ON/OFF wall-time differences, `(ON / OFF - 1) * 100`, in percent:

| State | First pair | Repeat pair |
| --- | ---: | ---: |
| Cold prepare | +36.944 | -2.252 |
| Repeated noop | -13.835 | +36.545 |
| Manifest feature add | +6.030 | -5.411 |
| Type error | +1.465 | +0.055 |
| Type recovery | +5.679 | +1.426 |
| Assertion failure | +5.663 | +7.828 |
| Assertion recovery | +2.552 | -0.928 |

Noise remains **unknown**. Resident daemon CPU is not waited child CPU, and these
scoped pairs do not establish causality, a quiet machine, or general performance.
No speedup, default promotion, or regression attribution is claimed.

The self-contained evidence bundle is
`bench/results/resident-entry-key-prefix-20261031/`. It retains full frozen
sources, marker-only repair, native journals and complete captured native output,
all 56 raw state rows, source/tool/toolchain identities, separate v44 activation
failure and v45 positive/negative controls, restoration receipt, and operational
postmeasurement cleanup receipts. Replay is offline and uses preserved copies of
the original semantic oracle. Run:

```sh
python3 -B bench/results/resident-entry-key-prefix-20261031/replay.py
python3 -B bench/results/resident-entry-key-prefix-20261031/test_replay.py
```

No native jobs were run by this evidence lane. Cleanup is operational provenance
only and does not establish historical quietness or timing validity. Stores were
not deleted. The original v44/v45 scratch journals remain preserved.

## v45 activation evidence repair

The coordinator reported v44 full native suites passing (57 and 360 tests) and
all 28 strict seven-state cases passing. Positive-activation admission nevertheless
failed: v44 emitted no successful-preparation marker, and raw ON worker logs had
none. Those original measurements and the admission failure remain preserved in
the v44 evidence. They are not admitted proof of runtime prefix activation or
speed acceptance.

v45 adds only a constructor diagnostic to the implementation: after successful
preparation, exactly when `key_prefix.is_some()`, it attempts to write the fixed
line `ra-jit entry-key-prefix v1 prepared` to locked stderr. The write result is
ignored. This is one fixed-length line per successfully prepared Entries context,
not per lookup, with no names, program bytes, keys, or other secrets. OFF and
unavailable context produce no marker. There are no key, query, Store, writer,
or oracle changes relative to v44. The coordinator must rerun the full gate and
strict seven-state pairs and establish activation from the raw worker logs.
This lane performed source checks only, not native runs, for v45.

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

## Historical frozen handoff

Exact full v44 originals and v45 candidate source/documentation, candidate patch,
SHA256SUMS, BASE_HEAD, and
FROZEN_READY.txt are preserved in
`/home/jeremy/.jcode/scratch/blaze-native-engine-20261030/entry-key-prefix-v45`.
The distinct v44 freeze and its failed activation admission remain unchanged.
Only the two assigned repository paths are candidate inputs. Once explicitly
released, this lane will not edit them while the coordinator validates hashes,
builds, and runs the candidate.
