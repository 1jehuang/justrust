# Watched checkout manifest bytes, v76

Source-only bounded hypothesis on accepted v74 / v58 script witnesses. No speed claim.

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
