# Native instance home query, v66

## Status and ownership

Source candidate only, prepared October 31, 2026 against
`76994fa48fedc9057de6e8086c18e6c2ef1d8522` on `stable`.
Owned changes are limited to `toolchain/resident/crates/hir-ty-codegen/home.rs`,
that crate's `Cargo.toml`, and this new document.
No build, test, native compiler invocation, toolchain probe, lockfile edit,
commit, installation, measurement, or speed/default-promotion claim was made
by this lane. Root owns actual gates and measurements. Fixture assertions
below are authored requirements, **not observed passing results**.

## Switch and unchanged behavior

`RA_JIT_HOME_MEMO=1` enables the native query for instances with syntactically
nonempty stored generic arguments. Every other value, including absence,
`0`, `true`, whitespace, and `01`, leaves the original direct visitor selected.
The new experimental switch is latched at first use in each process using a
`LazyLock<bool>`, not forcibly at worker startup. Change it before launching a
new daemon/worker, not while reusing an existing process. This avoids a new
per-instance environment scan. The existing dynamic `RA_JIT_NO_HOME` check
and caller-bounds early return remain exactly where they were.

Even when opted in, syntactically empty arguments use the original direct
visitor. This avoids a tracked lookup for the common trivial empty-argument
function case without introducing a type-based or guessed timing threshold.
The direct path retains its original `FxHashSet`, without collecting a vector.
The opt-in path borrows the query's retained vector rather than cloning it on
every hit. No existing `Homes::sees` behavior was changed.

The query is `native_home_crates(db, instance.clone())`, a native
`#[salsa::tracked(returns(ref))]` function. Its key includes the entire
`Instance`: definition, stored generic arguments, and `checked`. It executes
the unchanged `Mentions::of` visitor and retains `Option<Vec<Crate>>`.
The vector is collected directly from the original `FxHashSet::into_iter`,
without sorting or constructing an intermediate set. `Homes::home` traverses
that exact order both to choose the first eligible crate and to check what it
sees. `mentioned_crates` uses the same wrapper, returning a set as before.
The query's vector equality also retains order sensitivity during Salsa
backdating, rather than treating different home-selection orders as equal.

`None` still means that the visitor encountered an opaque/unknown ownership
case. There is no fallback to an arbitrary crate, no changed closure-owner
handling, no changed reveal policy, and no change to `bodies_of`, `body_files`,
`written_in`, or the original visitor. In particular this query does not use
`Mentions::looking(..., true)` or cache body/layout results.

## Dependency model

All reads performed by `Mentions::of`, including definition/ADT module lookup,
closure/coroutine owner location, and trait ownership, execute inside the Salsa
provider. Salsa retains those query dependencies even when another tracked
caller finds an existing native memo. There is no mutable cross-query value
cache, text hash, source location key, approximate instance key, or separate
invalidation scheme.

Signature/inference/name-resolution queries producing resolved generic
arguments remain the caller's dependencies. A changed resolved type or changed
closure owner produces a changed full `Instance` key. Reusing an old `Instance`
does not magically re-resolve its arguments, nor did the original direct
visitor do that. A body-only edit with unchanged visited ownership need not
execute this provider. The fixtures distinguish changed argument/owner keys
from irrelevant revisions rather than demanding an execution after every edit.
The pre-existing `Homes::sees` map and its lifetime/invalidation contract are
not extended by this work.

## Real-database fixture source

All fixture tests are in `home.rs`, using `ide_db::RootDatabase` and
`test_fixture::WithFixture`. They discover actual `FunctionId`s through crate
def maps. Generic instances use resolved signature return types or actual
inferred closure types, not fake IDs or a mock database. The independent
oracle invokes the original `Mentions::of` visitor directly, separately from
the native query and wrapper.

The `native_home_` test prefix contains these authored checks:

- Exact opt-in parsing, without mutating the process environment.
- Cold nonempty real instance: one actual provider execution. Eight repeated
  wrapper calls in the same revision: zero further provider executions.
- Default-off calls and opted-in empty-argument calls: zero provider executions.
  Changing only `checked` creates a separate full-instance key.
- A dependency re-export switches from one crate's actual ADT to another
  without modifying the caller file. Resolved arguments, ownership output,
  and native key change. Two distinct tracked signature callers share a memo
  and independently validate the dependency change.
- A real return signature changes from the dependency ADT to `u32` while the
  function IDs remain stable. The new result equals the direct oracle and
  needs one new provider execution, followed by zero on repetition.
- A body-only edit retaining the `u32` signature preserves the key and direct
  equality, with zero new provider executions.
- An inferred closure moves from `owner_a` to `owner_b`. The full instance key
  changes, the provider executes once even though both owners mention the
  same crate, and ordered direct/native outputs remain equal. The repeated
  request executes no provider.
- A real return-position opaque `impl Sized`, with the miniature Sized lang
  item, retains `None` and exhibits cold-one/repeated-zero execution behavior.

Provider execution counts use a `cfg(test)` thread-local `Cell<usize>` that is
incremented only inside the actual tracked provider and reset by assertions.
There are no production provider counters, per-wrapper atomics, or inferred
hit counts. Native fixture compilation and execution are deliberately deferred
to root, so none of the counts above are evidence yet.

## Activation marker, not reuse telemetry

When the process-latched opt-in is true and the actual native provider first
executes, it attempts one fixed, bounded stderr line per process:

```text
ra_jit: native-home-query-v66 QUERY ENTERED (not workload cache-hit evidence)
```

The write's error is ignored. `Once` is checked on provider executions, not on
every wrapper visit, and no arbitrary instance/source content is printed.
This marker proves only that a query provider was entered in that process.
It does **not** prove workload reuse, provider avoidance, hit rate, speedup, or
comprehensive coverage. No shutdown hook or misleading per-Homes-drop report
was added. Actual workload reuse attribution would need a separately identified
diagnostic artifact and explicit reporting boundary. Diagnostic timings must
not be used as timing evidence.

## Exact lockfile requirement

`Cargo.toml` adds only these test dependencies:

```toml
ide-db.workspace = true
test-fixture.workspace = true
```

Both packages are already resolved in `toolchain/resident/Cargo.lock`, at
`0.0.0`, and both workspace paths are already declared. Root must add exactly
`"ide-db"` and `"test-fixture"` to the dependency list of the
`hir-ty-codegen` package entry in that lockfile before a locked fixture build.
No new package resolution or version change is requested. This lane has not
edited the lockfile or run a dependency resolver.

## Source freeze and release

Evidence root:
`<PRIVATE_EXPERIMENT_ROOT>/home-query-v66`.

`original/` retains the complete pre-edit owned sources. This document was
absent in the baseline, recorded by `original/docs/resident-home-query.md.absent`.
`candidate/` retains all three complete candidate files. `candidate.patch`
includes both tracked modifications and the new document. `SHA256SUMS` records
the original files, candidate files, patch, and baseline metadata.
`SOURCE-FREEZE.md` is the explicit handoff/release record.

Source-only checks are formatting of the owned Rust file with child traversal
disabled, formatter check mode, `git diff --check`, bytewise visitor preservation
inspection, and source/manifest inspection. They do not establish compilation,
fixture execution, runtime activation, actual cache reuse, or performance.
After the freeze record is written, this lane releases its source ownership to
root and makes no further changes unless root explicitly requests a new revision.
