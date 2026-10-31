# v68: empty-argument function home specialization

Status: source-only candidate, default OFF. No runtime result or speedup is claimed.
Root owns compilation, strict runtime correctness, counterordered loop measurements,
and acceptance or rejection. Source worker must not build, test, probe, change locks,
commit, install, or retain ownership after explicit release.

## Exact branch

`RA_JIT_HOME_EMPTY_FAST=1` is the only enabled value. A process-wide `OnceLock<bool>`
latches that decision on its first eligible call. The original dynamic
`RA_JIT_NO_HOME` presence check and nonempty caller-bounds check execute first,
unchanged. Only an empty `StoredGenericArgs` and `InstanceDef::Fn` specialize.

The original `Mentions::of` for that case records exactly the function's crate,
allocating a singleton hash set and a one-element bodies vector. Its opaque flag
remains false. The candidate gets the same `func.krate(db)`, uses the identical
`self.sees.entry(krate).or_insert_with(|| loaded_crates(db, krate).iter().copied().collect())`,
and checks `sees.contains(&krate)`. It does not assume that loaded crates contains
self. Success constructs the same `ParamEnvAndCrate { krate, ..asked.as_ref(db) }.store()`.
Failure returns `asked.clone()`. No new query or query cache is introduced.
The original algorithm is retained byte-for-byte after the branch, including
all generic, closure, coroutine, opaque, and shim handling.

The fixed marker `ra-jit: home-empty-fast-v68 branch` is written to stderr only
on actually entering the enabled specialization, once per process. Write errors
are ignored. A thread-local initializer invokes a process-wide `Once`, avoiding
marker-related atomic checks after that thread's first branch entry. The flag's
OnceLock access remains a per-call synchronization check. The marker is not a
speed measurement and does not establish benefit.

## Differential fixtures for root gates

`home::empty_fast_tests` uses a real `ide_db::RootDatabase` with `test_fixture`,
not a fake `HirDatabase`. Its independent frozen pre-v68 `OriginalHomes::home`
retains the original algorithm and owns a separate `sees` cache. Every comparison
asserts equality of the entire `StoredParamEnvAndCrate`, the unpacked environment,
and the complete cache contents. It reuses unchanged `Mentions`, which this
candidate does not modify.

Coverage includes same-crate and cross-crate requests, repeated cache hits,
two cfg graphs with an actually cfg-selected function, signature-only dependency
references, live source revisions removing and restoring that reference,
retained and fresh caches after those revisions, nonempty generic arguments,
an inferred real closure, opaque return-type arguments, all non-coroutine shim
variants, nonempty real caller bounds, and a separately invoked NO_HOME guard.
An explicit equal-cache seed without self tests the false membership outcome.
That corner case does not claim `loaded_crates` naturally omits self.
Cfg variants use separate fixture databases. These tests do not claim to cover
in-place crate-graph cfg mutation, coroutine inference, or every opaque shape.

Root should run ordinary `v68_real_database_` tests in separate processes with
flag unset, `0`, and `1`, without NO_HOME. Only the enabled run should emit the
branch marker. Run the ignored `v68_real_database_no_home_guard` alone in a fresh
process with both flags set. It must preserve asked and leave the cache empty,
without a branch marker. Exact flag value and process-latching runtime probes
belong to root. All fixtures and assertions are unexecuted by the source worker.

The only manifest additions are `ide-db.workspace = true` and
`test-fixture.workspace = true` under hir-ty-codegen dev-dependencies. Both
packages already exist in the lock. Root must add exactly these two dependency
edges to hir-ty-codegen in `toolchain/resident/Cargo.lock` before locked gates.
The source worker leaves that lock untouched.

## Preservation and release

Private preservation root:
`<PRIVATE_EXPERIMENT_ROOT>/home-empty-fast-v68`.
`baseline/` contains the complete pre-edit home.rs and Cargo.toml plus an explicit
doc-absence record. `candidate/` contains all three owned final files.
`candidate.patch` is the complete baseline-to-candidate patch.
`SHA256SUMS`, `sourcechecks.txt`, and `RELEASE.txt` record integrity, source-only
checks, and explicit release. Root may then change, build, measure, or reject the
candidate without further source-worker ownership.
