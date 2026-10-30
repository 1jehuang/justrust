# Experimental resident native entry: jr-engine

`jr-engine` is explicitly **Blaze-derived**, using the imported Blaze native engine
at original commit `5d4f78f7577cf7170993b74f735099dd03f5c6ca` (import
`62a6ef3c3`). It is not a clean-room compiler, an upstream Blaze release, or a
shipping justrust default. Existing upstream attribution and licenses in
`toolchain/resident/` remain authoritative.

## Entry and identity

The `ra-jit` crate has two binary targets sharing `src/main.rs`. The compile-time
`CARGO_BIN_NAME` identifies `jr-engine`, independent of argv[0], executable renames,
and build-script `rustc` symlinks. There is no subprocess launcher around `ra-jit`.
The `ra-jit` target retains its previous dispatch and environment normalization.

After mandatory initialization, `jr-engine build|test|check|run [cargo flags]`
enters `cargo::command` directly. Existing internal dispatch remains available:
sandbox, as-rustc, doctest replay, build-script, rustc-wrapper, daemon, spare,
checker, worker, and source compilation requests. Initialization precedes even
sandbox dispatch and never starts a thread before its fork. Runtime wrappers
continue to take precedence over the public build commands.

`--version` identifies `jr-engine experimental`, the Blaze origin, and the owned
build ID. Help and version do not require a namespace. Every actual engine entry
requires `JR_ENGINE_BUILD_ID` to be compiled as `resident-v1-` followed by exactly
64 lowercase hexadecimal digits. An unidentified build is not operational.
The root-owned build provenance supplies this identity and separately salts the
source store keys. No performance claim follows from eliminating a process.

## Explicit private namespace

Before invocation create a dedicated directory, for example:

```sh
mkdir -m 700 /tmp/jre-$UID
export JR_ENGINE_HOME=/tmp/jre-$UID
jr-engine --version
jr-engine check
```

Use a short path because Unix daemon and worker socket paths have platform length
limits. The directory must already exist, be absolute and canonical, be owned by
the effective user, and deny all group/other permission bits. Root paths,
relative paths, symlinks, and non-private directories fail closed. Every engine
subdirectory is created with mode 0700 and checked similarly. Do not choose an
existing upstream cache or another application's directory. This is a private
single-user experiment, not a security boundary against malicious processes
running as that same user or an administrator replacing its directories.

Within `JR_ENGINE_HOME/resident-v1/`:

| Location | Boundary |
| --- | --- |
| `runtime/` | Daemon sockets, locks, logs, worker state, and Blaze-owned toolchain downloads |
| `store/resident-v1-<64hex>/` | Build artifacts and downloaded store objects, partitioned by full owned source identity |
| `functions/` | Experimental function-result cache, isolated from upstream defaults |

Both `BLAZE_HOME` and `RA_JIT_HOME` are replaced with the private runtime path.
Both cache spellings and both function-cache-directory spellings are replaced
before upstream alias normalization. Inherited `BLAZE_HOME`, `BLAZE_CACHE`,
`RA_JIT_CACHE`, `RA_JIT_HOME`, and function-cache-directory overrides cannot
redirect these engine stores. Automatic remote synchronization is forced off
under both variable spellings. The runtime path is not source-digest-lengthened
because that would exhaust socket path limits. Upstream daemon executable/version
checks still govern daemon replacement.

The real `HOME` and `USERPROFILE` are **not changed**. Cargo/rustup configuration,
registry downloads, project target directories, manifest settings, build-script
inputs, native tools, and normal user-home semantics remain visible. This entry
is not a hermetic Cargo environment and does not claim all writes are confined to
`JR_ENGINE_HOME`. Workspace target directories and user Cargo caches must not be
confused with the private resident artifact store. Custom explicit paths passed
to internal worker commands remain trusted internal inputs.

## Unsupported and validation boundaries

- Upstream `enable`, `disable`, `cache`, `closure`, and `std` management commands
  are rejected, including their `cargo`-prefixed form. No cargo configuration or
  shipping client defaults are changed by this entry.
- Non-Unix platforms fail closed until equivalent ownership/ACL validation is
  implemented. Long socket paths remain unsupported.
- `JR_ENGINE_HOME` is mandatory on every internal reentry too. Restricted child
  environments must explicitly forward it, including build-script as-rustc
  reentries. Native acceptance must check this propagation. If stripped, entry
  fails closed rather than selecting a shared fallback or trusting cache aliases.
- This is the existing experimental Blaze engine, with its existing supported
  Rust/platform behavior and diagnostics, not a claim of general Rust parity.
- Source/helper checks alone are not a native acceptance result. Native builds,
  worker/sandbox tests, real workloads, and benchmark comparisons belong to the
  coordinator's baseline and validation stage. No speedup is asserted here.

## Bounded entry gate result, 2026-10-30

The coordinator's fresh, separate-target native candidate build succeeded. Its
reported source identity was
`resident-v1-bc9689b69a6f40e47b93a11f679daa197eb5c1fbc10b9898a310e9cac3efdfc4`.
`bench/resident_entry_accept.py` passed all eight entry/namespace cases:

- Version without a home reports jr-engine, explicit Blaze origin, and build ID.
- Missing, relative, shared-permission, and symlinked homes are rejected.
- Private status succeeds despite hostile inherited upstream home/cache aliases.
- Management commands are rejected.
- A symlinked namespace subtree is rejected.

The receipt at
`~/.jcode/scratch/re-entry-20261030/acceptance.json` records unchanged executable
SHA-256 before/after:
`726edc785271a1907e5d40619fde5239da031f535fff035b62c6650a8b1aafef`.
Separately, all three actual-source helper tests passed using the exact
`nightly-2026-10-27` compiler and cached dependencies. That narrow helper harness
stubs only `crate::as_rustc::is_me` and is not full worker acceptance.

These results validate entry identity and fail-closed namespace handling only.
They do not establish Rust language correctness, successful real build/test
execution, restricted child-environment propagation, or a performance improvement.
The unseeded runtime comparison is a separate coordinator-owned gate.
