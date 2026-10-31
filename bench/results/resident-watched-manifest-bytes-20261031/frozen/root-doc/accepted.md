# Watched checkout manifest bytes, v76

Bounded default-OFF candidate on accepted v74 / v58 script witnesses. Root-reported
scoped acceptance and descriptive measurements are recorded below, not a general
speedup or default-promotion claim.

## Selection and scope

`RA_JIT_WATCHED_MANIFEST_BYTES=1` is exact, default off, latched on first
`Inputs::watch` with `OnceLock`. Other values, unset, or non-Unicode values do not
enable it. `Watched::manifest(path, enabled)` is the testable selector, so fixtures
never change process environment. Off uses the original stamp function with no
content reads, canonicalization, byte buffers, or proof allocations. The optional
proof pointer increases the Watched layout but allocates nothing when disabled.

Only the root Cargo.toml and Cargo.lock and local package manifest roles installed
by Inputs::watch opt in. Cargo config, both rust-toolchain names, discovered
build.rs, directories, and all Script files retain the original metadata path.
The original lossless PathBuf stays on Watched. Linux successful regular reads
add a lossless canonical path, optional raw symlink target, and complete bytes.
Other platforms retain metadata behavior rather than using a weak proof.

## Proof and lifecycle

Every check of a known eligible file reads fresh bytes, even with identical size
and mtime. Exact bytes and canonical/link identities must match the loaded
witness. The read checks device/inode/mode/length/mtime/ctime before and after,
including nanoseconds, both through the descriptor and through the watched path.
Nonblocking Linux open avoids FIFO replacement hangs. Read failures, races,
nonregular replacements, disappearance, and canonicalization failures of known
files return changed. Initially unknown/missing/nonregular/unreadable paths get
no content certificate and retain the original stamp semantics, including its
limitations. There is no inferred known absence and no metadata-only certificate.
As with normal filesystem checks this is an observation, not a lock against
writes after the observation. Descriptor/path metadata checks reject detectable
races, not malicious filesystem timestamp forgery.

No cached bytes or checkout stamps are updated by stale. New proofs are installed
only by the existing successful-load watch lifecycle. Returning to the actually
loaded bytes after a failed reload is valid, and changed bytes keep requesting
reload until a new watch is installed. Equal bytes suppress only this individual
checkout file's changed result. They never return early from Inputs::stale:
all remaining checkout files, flags, and Scripts::changed remain in the same order.
The v58 Scripts implementation and existing assertions are unchanged. Script
manifest watches retain real raw-stdout plus generated-digest rerun proof. Failed
environment reloads do not consume A/B/A witnesses. Native directives are not
replaced by WorkspaceBuildScripts projection equality.

## Authored fixtures, not executed by this worker

Real temporary-filesystem and Inputs/Scripts fixtures cover both selectors,
manifest/lock retouches, byte change/restoration, size/mtime-preserving changes,
missing/present and directory/error states, symlink target replacement, unchanged
bytes continuing flags/env A/B/A checks, explicit script manifest watches without
a valid rerun proof, and metadata-only configuration/toolchain/build.rs/directory
watches. The original Watched::new path supplies an independent baseline where
valid: a retouch must change it and off must agree. That baseline deliberately
misses same-size/restored-mtime changes. Permission-denied and concurrent-race
injection are not claimed as executed fixture coverage. Existing native tests
remain untouched, including their real compiler/archive acceptance behavior.

Only rustfmt and Python source checks are worker validation. Root owns compilation,
actual53/full resident tests, both-flag environment/file A/B/A, manifest read-error
acceptance and pinned retouch measurements with v74 whole-generation and v76 flags.
No builds, tests, native probes, lock/install operations, commits, or speed claims
are performed by this worker. ROOT/source was observed to be older than the
checkout v58 implementation, so originals are from the authoritative owned
checkout source, not ROOT/source.

## Root-reported scoped acceptance, October 31, 2026

The coordinator reported these gates PASS for the immutable released Rust candidate:

- Native gates: native53 and 373 full resident tests.
- Configuration gates: cfg9.
- Native environment/file A/B/A gates: 6.
- Strict gates: 28.
- Python regression: 761 tests PASS, as subsequently reported by root.

These are root-executed results, not tests executed by this source-only worker.
The initial native gate job was `task594819rrit`. Runtime acceptance is scoped to
the default-OFF candidate and those gates. Activation must be established from
actual behavior and a same-worker-PID oracle, not merely from setting the flag:
the candidate has no explicit activation marker. No coverage beyond the reported
gates, such as a new exhaustive race-injection campaign, is implied.

### Pinned retouch experiment

The coordinator reported fresh pinned `wm76b` complete in `task662968rcy9`, with
54 calls and 5 stops. Its suite comparison reported OFF 2.457933 seconds versus
ON 0.054515 seconds, a descriptive reduction of 97.78%. First OFF observations
were 12.057 and 12.326 seconds, while all ON observations were 0.048 to 0.061
seconds. These are reported comparison values, not a newly calculated estimator
or an assertion that the first observations equal the suite aggregate.

The workload is the bounded watched-manifest/lock retouch hypothesis with v74
whole-generation reuse and the v76 selector under the root's pinned setup. It is
not the comprehensive session-weighted workload and does not establish broad
speedup, default promotion, or a noise-adjusted causal estimate. Noise remains
unknown. Root owns the raw harness/evidence identities and final regression gate.

Earlier `wm76a` was aborted at 52/54 calls by its outer 600-second limit. It is
retained separately, with the pin restored and stop status 0 reported, but is
**not accepted** and is not merged into the fresh completed comparison.

## License and source lineage

This changes the owned, licensed Blaze-derived resident `ra-jit` implementation,
retaining its existing
rust-analyzer source lineage and the resident tree's dual MIT/Apache-2.0 license
files (`toolchain/resident/LICENSE-MIT` and `LICENSE-APACHE`). It is not a
clean-room replacement or a new independently licensed compiler. Existing
attribution and license files are unchanged. The original snapshot is the
checkout's accepted v58 script-witness implementation on the coordinator-supplied
v74 base `de8533b93`, not the older `ROOT/source` copy. This final documentation
addendum does not alter Rust or the original v76 freeze artifacts.
