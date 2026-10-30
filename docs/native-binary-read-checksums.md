# Native builtin binary-read checksum binding

Validated 2026-10-30 UTC on x86_64 Linux. This is a bounded compiler
correctness prerequisite, not command-result-cache authority or a speed claim.

## Change

`include_str!` and `include_bytes!` obtain `(Arc<[u8]>, Span)` from
`source_util::load_binary_file`. Previously dep-info reopened their
`file_depinfo` paths later, after expansion, to compute checksums. A file
changed between these operations could therefore describe new bytes in dep-info
while the artifact embedded old bytes.

The Session now has a separate `tracked_binary_read_checksums` map. Builtin
binary includes record the length and `SourceFileHash::new_in_memory` hash of
the actual returned bytes. Recording is gated on
`-Zchecksum-hash-algorithm`, before hashing or inserting anything. Dep-info uses
the recorded value instead of reopening those paths:

- First read records `(length, hash)`.
- Repeated identical reads preserve the value.
- Unequal reads replace the value with a sticky ambiguous `None`. Subsequent
  reads cannot repair it. Dep-info retains the dependency but omits its checksum.
- An absent map entry is distinct from ambiguity. Untracked paths, including
  proc-macro tracked paths, retain their existing reopen behavior. A failed
  reopen still omits the checksum rather than failing compilation.
- With checksums disabled the map remains empty and existing dep-info behavior
  is unchanged.

This is not a complete proof of compiler inputs. Reopened proc-macro paths,
other input categories, multiple aliases for a file, overlaps with other input
categories, and external changes not observed by this read path remain outside
this guarantee. No cache admission, receipt schema, Cargo behavior, client,
or installed toolchain was changed.

## Deterministic acceptance fixture

`toolchain/rustc/tests/justrust/binary-read-checksums/driver.rs` uses the real
`rustc_driver::Callbacks::after_expansion` callback. Its input includes the same
file with `include_str!` and twice with `include_bytes!`. The callback mutates
or deletes that file before dep-info emission. The test runs the actual linked
executable and independently checks SHA-256 dep-info using Python's `hashlib`.
There are no sleeps or timing-dependent races.

The different-read case performs an additional actual filesystem read in the
callback and passes its bytes through the same Session recorder, checking that
ambiguity is sticky even after recording the old bytes again. The identical
case likewise exercises the recorder explicitly, in addition to the three real
builtin includes. These callback reads test recorder semantics, not expansion
of a second include after the callback.

Each case then writes new bytes and compiles twice with ordinary rustc, sharing
the incremental directory with the callback-driven compile. Both ordinary
executables must print the new bytes and, when enabled, dep-info must checksum
those same bytes. This covers the post-mutation and unchanged incremental paths.

## Reproduction

From this repository root:

```sh
(cd toolchain/rustc && ./x build --stage 1 --keep-stage-std 0 library compiler/rustc)
python3 toolchain/rustc/tests/justrust/binary-read-checksums/test.py \
  --rustc toolchain/rustc/build/host/stage1/bin/rustc \
  --output docs/results/binary-read-checksums-2026-10-30-after.json
(cd toolchain/rustc && ./x test --stage 1 --keep-stage 0 --keep-stage-std 1 \
  tests/run-make/checksum-freshness tests/run-make/dep-info \
  tests/run-make/track-path-dep-info tests/run-make/track-pgo-dep-info)
```

Use `--keep-stage-std 0` for the compiler rebuild, not `--keep-stage 0`, which
can skip the changed compiler. The latter is appropriate only for the following
test invocation after the rebuild succeeds. This is intentionally a local
owned-toolchain test, not a hosted build of the shipping client.

The callback executable must be compiled by the stage0 compiler against the
matching stage0-sysroot. The stage1 driver was itself compiled by stage0 and its
private metadata is not compatible with using the stage1 compiler to build this
fixture. The runner defaults to that pairing, verifies full driver metadata
exists, and verifies the paired sysroot dylib is byte-identical to the stage1
dylib under test. It records the dylib SHA-256 in the result. Bootstrap splits
full `.rmeta` metadata from dylib stubs, so both are resolved through the paired
sysroot instead of binding `--extern` to one partial artifact.

## Exact evidence

The stage1 build completed successfully in 4m13s. The compiler identified as
`1.101.0-nightly (1e7f71794 2026-10-14)`, LLVM 23.1.3. The tested
`librustc_driver-e2bd696299265a4a.so` SHA-256 was
`c684752b72cdf8bad2dda3235e225bb48ba7ad509a526db008a3b8bf50a170b5`.

The committed `docs/results/binary-read-checksums-2026-10-30-after.json` records
all seven passing cases and 21 compilations:

| Case | Boundary executable | Boundary payload checksum |
| --- | --- | --- |
| mutate after expansion | `old\|old\|old` | SHA-256 of `old`, length 3 |
| delete after expansion | `old\|old\|old` | SHA-256 of `old`, length 3 |
| identical repeated read | `old\|old\|old` | SHA-256 of `old`, length 3 |
| different repeated read | `old\|old\|old` | omitted, dependency retained |
| checksums default-off | `old\|old\|old` | omitted, map remains empty |
| untracked and missing paths | `old\|old\|old` | builtin old hash, untracked reopened hash, missing omitted |
| unchanged | `old\|old\|old` | SHA-256 of `old`, length 3 |

The old-byte hash was
`cba06b5736faf67e54b07b561eae94395e774c517a7d910a54369e1263ccfbd4`.
All 14 subsequent ordinary compiles printed
`new content|new content|new content` and matched the new-byte checksums when
checksums were enabled. Existing run-make selection passed 9 tests with zero
failures (the `dep-info` filter also selects related tests), in 1.04s test time
and 23s including harness preparation. This includes the existing BLAKE3
checksum fixture with non-UTF-8 binary bytes and proc-macro tracked paths.

### Baseline attempt and diagnostic isolation

The saved `build/jr/dev` compiler was not a usable before-baseline for this
driver fixture because its matching full private-crate metadata was not present
in the active bootstrap sysroot. Compiling the fixture with that stage1
compiler produced E0514/E0463. A diagnostic-only attempt to inspect the saved
`.so` with stage0 `rustc -Zls=all` panicked with `MemDecoder exhausted` because
the `.so` is only a metadata stub, not the required full `.rmeta`. The original
report is preserved at
`docs/results/binary-read-checksums-2026-10-30-metadata-inspection-ice.txt`.
Pointing `--extern` only at a stub or only at full metadata also failed, so the
final runner uses and validates the complete paired sysroot instead.

No incompatible incremental state was reused from these failed fixture setup
attempts. Each matrix run has a fresh temporary directory. The optional
`--baseline` mode can capture the pre-fix mutation bug only when supplied with a
matching baseline driver compiler/sysroot pair. No pre-fix runtime result or
before/after performance improvement is claimed here.
