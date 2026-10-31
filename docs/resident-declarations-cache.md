# Resident exact declarations cache: rejected v41 experiment

**Decision: REJECT, no promotion.** The coordinator restored live `main.rs` and
`frozen.rs` to their accepted originals. The implementation described below is
historical frozen source, not an installed feature. Evidence and an offline
semantic validator are retained in
`bench/results/resident-declarations-cache-20261031/`.

Actual v41 check/build, 48 codegen tests and 369 resident tests passed. Four
seven-transition sessions passed all 28 strict runtime oracles and identity
checks. Nevertheless the paired elapsed-time changes were:

| Transition | First on/off | Reverse-order on/off |
|---|---:|---:|
| Cold | +2.26% | +2.18% |
| Manifest feature add | -0.96% | -3.14% |
| Type recovery | -2.12% | +0.03% |
| Assertion failure | +1.72% | +4.95% |
| Assertion recovery | +22.29% | +10.03% |

Positive means slower. This is a focused diagnostic with unknown/uncontrolled
noise, not the comprehensive session-weighted benchmark. There is no robust
useful gain above the floor and there are important recovery regressions.
Correctness acceptance and positive cache hits do not establish a speedup.

Each v41 on-session cold snapshot has 21 positive native hit markers. Later
snapshots copy the same cumulative worker history: they do **not** show 21 new
hits per transition. Raw v40-on logs actually contain 21 markers in cold/noop
and zero after manifest reload, correcting the earlier zero-throughout report.
The v40 full-process-environment key overinvalidates after the nonce changes.

The bundle retains v39 four-error compiler failure, v40 repair with 48/368
passing tests and 14 runtime rows, v41 48/369 and 28 rows, exact source patches,
3757-source gate identities, 4020-file runtime toolchain identities, frozen
runners, complete raw journals and slim exports. Its portable controls replay
existing evidence only, never run native jobs, and reject coherently re-signed
semantic corruption. Historical sources remain licensed Blaze-derived, not
clean-room.

## Historical implementation design

## v40 evidence and bounded v41 followup

Coordinator reports v40 actual check/build, 48 codegen tests, 368 resident tests,
and all seven strict runtime transitions passed. The fresh-j1 runtime sequence
was 42.5 s under `ROOT/declarations-cache-v40-on`. The initial report of zero
hit markers was incomplete: retained raw logs show 21 in cold/noop and none
after manifest reload. This is correctness evidence, not a performance gain.
The v41 source below subsequently passed native gates but was rejected above.
Only `frozen.rs` and this document changed in the followup. Exact v40 originals,
v41 full candidates, incremental patch and hashes live under
`ROOT/declarations-cache-v41`.


This is licensed Blaze-derived resident source, not the clean-room compiler.

## Measured owner and scope

The coordinator's v36 baseline is approximately 16 s cold, 7.9 s manifest
change, and 7.3 s recovery. The measured `listing: name resolution ...,
scopes ..., ... bodies ...` message belongs to
`ra-jit/src/main.rs::declared_in`, not `hir-ty-codegen/stable.rs::declared`.
`stable.rs` is deliberately unchanged. No performance improvement is claimed
before the coordinator's same-binary acceptance measurements.

`declared_in` now checks exactly `RA_JIT_DECLARATIONS_CACHE=1`. Other values,
including an absent flag, use the unchanged traversal renamed
`declared_in_original`. The disabled branch starts no new clocks, queries no
certificate, and accesses no store. This is an experiment, not a default
promotion. Existing `RA_JIT_FROZEN=off/0/check/verify` modes always take the
original declaration traversal and retain their historical behavior.

The native implementation and fixtures are in `ra-jit/src/frozen.rs`. No
hir-def codec API expansion was necessary. All normal listing constructors,
root/export/static classifications, and subsequent code generation remain on
their existing paths.

## Why the existing frozen key is not reused

Historical `frozen::key_of` hashes an immutable-path suffix, edition, target
kind, attributes, cfgs, normalized environment, overlays, loaded OUT_DIR file
hashes, and dependency keys. It does not hash ordinary package source contents.
It also excludes `CARGO` and normalizes package and OUT_DIR environment values.
That contract is insufficient for this experiment's exact changed-input gate.

Likewise the checked portable codec hashes referenced file contents only when
their paths begin with `../` or the OUT_DIR marker. Other file headers validate
loaded paths, not current in-package contents. Checked portable decoding is
necessary but is not itself a complete discovery-input certificate.

## Additional exact certificate

The separate `ra-jit exact declarations 41.1` namespace certifies the current
native database inputs before any lookup:

- Breadth-first dependency closure, preserving every dependency's order,
  name, extern-prelude/sysroot flags, and portable graph index. No Salsa IDs
  enter this key or the disk payload.
- Complete sorted path/content inventory of every source root used by that
  closure, not merely previously discovered declarations or referenced files.
  Thus loaded source-file additions/removals, path changes, and source edits
  invalidate the key, including files that were previously unreferenced.
- Exact crate root path, edition, origin, attributes, target kind, proc-macro
  working directory, workspace/target/toolchain metadata, extra crate metadata,
  sorted cfg, and full unnormalized crate environment including OUT_DIR.
- Process inputs are certified by a complete **actual observed-read witness**
  in the record, validated before any portable declaration definitions decode.
  For every native `env_read` in the closure, store its portable crate index,
  variable name and SHA-256 of the effective Option(value), respecting crate
  environment precedence and distinguishing missing from empty. Raw process
  values are neither stored in this witness nor printed. Unlike v40, an unused
  nonce variable changing does not invalidate the source/configuration key.

Each field and variable-length inventory is length-framed before SHA-256.
Whole-root hashing is a native Salsa query keyed by `InternedSourceRootId`,
so crates sharing a root reuse the hash while source inputs still invalidate
it. Missing file text is a refusal, not an empty content hash. The ordinary
`Store::global/get/insert` compiler/toolchain namespace and checksum protection
remain in force. No lasting/by-content store API is used.

Initial eligibility is intentionally restricted: an actual language-library
origin, non-local/non-proc-macro dependency closure, library source roots,
and real source paths all below the same toolchain source directory. Paths
only decide eligibility. The entire content/configuration/dependency inventory
decides identity. Local, registry, git, mixed-root, virtual, binary/no-text,
and unsupported input cases retain the original walk. Absolute paths and full
environment values sacrifice cross-checkout reuse for exactness. Observed
process-value changes reject the old record even though its source key matches.
An old record is not overwritten by `Store::insert`, so a new observed process
value may continue falling through until its former value is restored. This
bounded prototype prioritizes safe reuse over a multivariant cache index.

This inventory describes the current loaded DB snapshot, as the uncached walk
does. Normal VFS loading and watching remain responsible for reflecting disk
changes into that snapshot. This experiment does not assume a path name or
crate display name certifies its contents.

## Actual discovered macro inputs and diagnostics

Native `db.env_read()` and `db.env_unset()` sets for the complete dependency
closure are retained in the portable record. The inner declaration payload is
encoded first, allowing codec block-owner queries to discover more reads.
An outer checked Enc/Dec envelope then captures the complete actual read witness
and embeds the checked inner payload. All witness values must validate before
decoding the inner definitions. With unchanged exact source/configuration and
only deterministic native builtin macros, a discovery branch can change only
when an already observed read changes. Proc macros/external reads remain refused.
On a complete successful decode,
`SourceDatabase::env_is_read` and `env_is_unset` replay those actual effects,
preserving process-input watching and unset-environment diagnostics. No text
scan or function-name heuristic is involved.

External `include_bytes!`/`include_str!` reads recorded by `db.included_by`
are currently unsupported and fall through. So do any pending/native
`db.included_sources` requests. Source includes already present in the root
are covered by the full inventory and the original codec's checked file and
macro remapping. Entire proc-macro dependency closures are refused rather
than assuming their environmental or filesystem inputs are pure.

Read support and certificate identity are rechecked after the original walk
and again after encoding, since codec queries can themselves discover inputs.
A checked decode that discovers unsupported reads is not counted or returned
as a hit. All miss/refusal/read/corruption paths execute the original walk.

## Portable result and positive evidence

The payload uses existing `hir_def::nameres::frozen::Enc/Dec`. It retains every
`Declared` field: ordered `functions`, `statics`, `in_provided`, `bodies`, and
`provided` vectors, including duplicate entries, plus `under_provided`.
Functions/statics use `def`, body queues use `body_owner`, and decoded enum
variants are checked. `Dec::new_checked` and `finish` remain mandatory. Actual
interned locations are remapped into the new database. Names and raw Salsa IDs
are not stored. Every actual crate is cached separately, without merging
crates by stable name or display name.

Only a completely decoded supported record increments `DECLARATIONS_HITS`.
With `RA_JIT_STATS`, the positive signal is:

```text
declarations-cache: native_hit=1 native_hits=N
```

Fallback attribution now includes actual crate name/origin, exact eligibility
reason, store miss, decode/encode error category plus SHA-256 fingerprint,
walk/encode certificate changes, unsupported read discovery, external-read and
pending-source counts, and write attempts. Codec errors are not echoed verbatim,
so substituted environment values cannot leak through error text.
Absence of traversal output
is not hit evidence. Store availability or successful writes alone are not
hit evidence. An unreadable existing record may remain in the store, but it
always falls through safely rather than changing build results.

## Native fixture and release status

Nine source fixtures under `frozen::declaration_tests` cover:

1. Original walk versus portable decode in fresh `RootDatabase` instances,
   intentionally resolving unrelated functions first to force ID remapping.
   The fixture includes nested blocks, macros, statics, impl methods, and
   declarations beneath trait-provided methods.
2. Exact vector order/duplicates, both body queues, and traversal state.
3. Every truncated prefix, extra trailing bytes, corruption, and wrong input
   certificate rejection.
4. Source, cfg, source path/inventory, crate OUT_DIR environment, and process
   environment certificate changes, including same-DB tracked invalidation.
5. Actual builtin `env!` expansion read/unset diagnostic replay after decoding.
6. Local-origin refusal and actual real-path include-source request refusal.
7. Dependency content and graph-edge changes.
8. Actual native external-byte input registration and conservative refusal.
9. Fresh native databases with actual builtin env reads: unused nonce changes
   permit checked decode, observed-value changes reject it, and absent versus
   empty values have distinct witness digests.

**Source-ready, not native-validated.** This owner ran rustfmt and
`git diff --check` only, no builds, tests, native workloads, or commits.
The coordinator owns compiler/check/build/native48/full tests and same-binary
off/on cold/manifest/error/recovery acceptance after all source owners freeze.
Positive native hits and useful correctness/performance results are release
gates, not yet claimed outcomes.

HEAD originals, original working copies, full candidates, source/doc patch,
and SHA-256 hashes are preserved under `ROOT/declarations-cache-v39`, with
root-validated v40 originals and the incremental v41 followup separately under
`ROOT/declarations-cache-v41`.
The unchanged `stable.rs` SHA-256 is
`9f2299fe8a5a17f55dee3f5ab48c09f4166029b7b520f370131da6d3ee3df8b9`.
