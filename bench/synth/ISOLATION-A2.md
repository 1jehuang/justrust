# A2 outcome and harness isolation investigation, 2026-10-15

## Preserved evidence

Read-only inputs: `bench/results/native-dispatch-synth40-A2-20261015.json`
and `bench/results/native-dispatch-synth40-A2-audit-20261015.json`.
Neither input was rewritten. The audit records baseline SHA256
`7b59851e5a660e9fdf18f2953184645ce230307f489ef8a122b228613ff601d3`.

Seven *points*, not seven individual tests, have unexpected test outcomes.
All have exit 101, `compile_failed=false`, and `expect_fail=false`.
There are 36 failed tests across these points:

| Point | Failed tests | Preserved diagnostic evidence |
|---|---:|---|
| 6 | 1 | bundled GitHub applet lacks `Review requested 1` |
| 11 | 5 | repository discovery failures, two tool-description token caps, feedback-hook output |
| 13 | 2 | test-harness detection with custom target directory, live provider catalog includes additional models |
| 15 | 10 | Git repository assumptions, ambient/onboarding state, cost model, reload/observe state, image clipboard text, UI behavior |
| 21 | 8 | repeated Git/ambient/onboarding/cost/reload/image issues, model-picker catalog |
| 28 | 1 | hint-chip horizontal position differs by 25px |
| 37 | 9 | repeated ambient/onboarding/Git/cost/reload issues, login/catalog and dispatch-state tests |

These are diagnostics, not established root causes. Repository discovery
messages support investigating snapshot Git metadata. Catalog, clipboard,
and reload messages support investigating process environment and persistent
application state. Token-cap failures may be ordinary pinned-source failures.
The test-harness detection failure needs examination of the pinned binary's
path/environment assumptions. None establishes compiler miscompilation or
proves serial test execution would fix the failure.

A2 is ineligible independently of this patch: 39/40 samples are noisy, only
point 34 is quiet, and seven points violate the existing outcome gate. The
five expected compile-error points remain 16, 17, 18, 23, 31. Identity audit
reports 86 invocations (40 pre, 40 run, 6 prev), no metadata hash mismatches,
and no missing exported identity run IDs. This is not matched A/B evidence.

## Safe changes implemented

`run.py` now acquires the existing mandatory synth lock before sampling and
before per-point restoration. Previously restoration could precede `apply`'s
mandatory lock, particularly with `BENCH_NO_LOCK=1` or direct `run_point` use.
This does not introduce parallel benchmark execution. The shared snapshot
and journal still require serialization.

`apply` now runs inside the cleanup region. If it raises after a partial
edit, the existing on-disk journal recovery runs immediately instead of
leaving the sources dirty until the next invocation. Normal measured-child
exceptions continue to restore completed edits.

Unexpected test failures now return a measurement error and retain the
measured log. Previously `run_point` accepted those failures and deleted the
log, although the report correctly blocked their aggregate. This aligns
collection with the existing report gate, not a change to expectations.
Expected compile failures remain accepted. Generator, weights, commands,
filters, test parallelism, and noise thresholds are unchanged.

## Validation and remaining gates

The initial safeguards phase launched no full benchmark, compiler build, Rust
build, or install. The follow-up below records targeted native Cargo builds and
acceptance tests, which were subsequently authorized. Run focused checks with:

```sh
python3 bench/synth/test_isolation.py
PYTHONPATH=bench python3 -m unittest bench/test_synth_reporting.py bench/test_synth_context.py
```

The final run passed 7 new isolation tests and 22 existing reporting/context
tests. `git diff --check` and `cargo fmt --check` passed, and the baseline
SHA256 still matches the value above. A disposable filesystem test exercises
actual journal restoration and generated-module cleanup. Mocked child
execution checks cleanup, lock ordering, expected errors,
and failure log retention. It does not demonstrate that the seven real
workload failures are fixed or validate actual multi-process lock contention.

Before an eligible new series:

1. Inspect pinned snapshot repository-discovery code and archived test
   environment without copying real credentials or mutating the snapshots.
2. Design an explicit, recorded application-state isolation policy. Do not
   blindly replace HOME: rustup, Cargo, toolchains, and justrust recording use
   it. Preserve declared toolchain/cache controls. Isolate application data
   and runtime paths only after identifying what the pinned code actually
   reads. Pre/run/prev must have a defined state lifecycle across stacks.
3. Reproduce the specific failed tests under unchanged parallelism and then
   controlled application state. Serial execution may be a diagnostic, not
   a replacement benchmark definition. Do not relax assertions or filters.
4. Resolve pinned-source correctness failures rather than relabeling them
   expected. Do not add Git metadata silently: build scripts may observe it.
5. Run a fresh complete matched baseline/candidate under quiet conditions,
   preserve all point plans, and apply unchanged outcome/noise gates. A2
   cannot be repaired by dropping noisy or failed points. No speedup or
   mapping/parallelism completion is established by this isolation patch.

## Follow-up: concrete no-build reproductions and native Cargo repair

The coordinator authorized native Cargo changes after the following evidence.
`reproduce_a2_isolation.py` and the new
`isolation-a2-reproduction-20261015.json` preserve source, metadata, and current
binary hashes plus the actual diagnostics. Reproduce without compiling:

```sh
python3 bench/synth/reproduce_a2_isolation.py "$JCODE_SCRATCH_DIR/dispatch-synth40-coherent/A2"
```

This uses recorded command cwd and `meta.env` over the current environment.
It is **not** an exact historical environment replay: only three environment
keys were recorded, and target artifacts may have been overwritten by later
A2 points. The receipt explicitly does not attest per-invocation historical
binary identity. Selected tests are run individually for diagnosis, not as a
replacement workload or a parallelism change. Disposable hardlinks and fake
configuration are removed afterward. Original snapshots, binaries, result
files, test expectations, and A2 logs remain unmodified.

### Proven harness-path cause

The recorded point-13 process executed:
`debug/build/jcode-base/aceefc8ad712df67/out/jcode_base-aceefc8ad712df67`.
Pinned `auth::running_in_test_harness` calls `is_test_harness_path`, which
requires `/deps/` (or immediate parent `deps` and a 16-hex hashed filename).
The new build-dir layout path fails both conditions. The preserved binary
reproduced the exact `test binary should be detected even with a custom Cargo
target directory` assertion at `auth/tests.rs:970` with exit 101.

| Same preserved binary, controlled execution path | Exit |
|---|---:|
| Original build/pkg/hash/out path | 101 |
| Same inode, original hashed filename under disposable deps/ | 0 |
| Same inode, original hashed filename under disposable out/ | 101 |
| Same inode, unhashed filename under disposable deps/ | 101 |
| Existing pure path-parser positive/negative unit test | 0 |

This establishes a path cause without compiler rebuilding, source changes,
credential overrides, test expectation changes, or timing assumptions.

Native owner: `toolchain/cargo/src/compiler/build_runner/compilation_files.rs`
excluded test outputs from `uplift_to`, while `build_runner/mod.rs` collected
`output.path` for execution rather than `output.bin_dst()`. The timing runner
merely forwarded this path, and was not responsible for relocation.

The repair publishes hashed test executables and their debug companions to
`<artifact-profile>/deps`, through Cargo's existing link/copy lifecycle, and
collects that public path for test execution. Compiler outputs stay in private
per-unit build storage. JSON artifact executable/filenames, `test --no-run`,
and actual execution now agree. Artifact layout prepares the deps directory
also when `build.build-dir` is separate. No runner shim was added, and pinned
app detection was not edited. Source identity is already covered by the owned
Cargo build fingerprint. No stock tool is given a private artifact format.

### Proven Gitless snapshot mismatch

Pinned `tool::bash::tests::repository_commands_export_a_logged_cargo_function`
uses `find_repo_in_ancestors(env!("CARGO_MANIFEST_DIR"))`, not runtime cwd.
`jcode-build-support/src/paths.rs::is_jcode_repo` requires both a Cargo.toml
containing `name = "jcode"` and an existing `.git` entry. The preserved A2
ancestor chain has one matching manifest and zero corresponding `.git`
entries. The isolated test fails at `tool/bash_tests.rs:84` with `test runs
inside the jcode repository`, exit 101. Changing cwd and `JCODE_REPO_DIR`
leaves that failure unchanged. A separate manual run from the live Git
repository with its override also failed identically.

This particular failure is an incomplete snapshot environment, not a compiler
error and not fixed by changing cwd. `suite.prepare` uses `git archive`, which
deliberately excludes repository metadata. A future cohort needs an explicit,
matched and recorded Git-metadata policy, preserving revision and dirty-state
semantics seen by build scripts and tests. Neither an empty `.git` marker nor
silently initializing the existing A2 snapshot is an acceptable repair.
Other Git/onboarding failures need their own acceptance checks before claiming
that a complete repository fixture resolves them all.

### Proven pinned-source token-cap defects

Two existing tests fail individually even under an empty disposable HOME,
JCODE_HOME, and XDG directories with a minimal environment:

| Assertion | Pinned input | UTF-8 bytes | Existing estimate (bytes / 4) | Cap |
|---|---|---:|---:|---:|
| `tool/tests.rs:715`, descriptions over cap | compile_remote DESCRIPTION | 405 | 101 | 20 |
| `tool/tests.rs:786`, parameter descriptions over cap | gmail $.properties.bcc | 104 | 26 | 25 |

The same counts follow directly from the pinned literal strings and the
pinned `s.len() / 4` estimator, independently of execution or compiler
behavior. Both tests exit 101. These are inherent pinned workload defects,
not evidence of compiler miscompilation. No cap or string was changed.
The normal current environment additionally enables codemode (~395 estimated
tokens), unlike A2's saved assertion context. That extra item disappears in
the clean-home control, and is not attributed to the historical A2 failure.

### Catalog evidence remains bounded

The point-13 Anthropic catalog test passes by itself in the clean environment.
Adding only a synthetic API-key config file also passes, as does removing it.
That rejects a simplistic "any key file is the cause" claim. Pinned
`known_anthropic_model_ids` unions API and OAuth catalogs and may add static
fallbacks for configured routes, while this test clears only the process API
key and does not use the broader clean-provider helper. This is a concrete
isolation lead, not proof of the complete historical cause or of all catalog
failures. No real credentials were read or copied for the clean controls.

### Native Cargo acceptance, lint limitation, and remaining gates

Hosted commands, each confirmed `where remote on hosted`:

- Check: `20261015-052129578-780952`, exit 0.
  `justrust check --manifest-path toolchain/cargo/Cargo.toml -p cargo --test jr_test_harness_paths --locked`
- Targeted real CLI acceptance: `20261015-052228514-792978`, 2 tests passed.
  `justrust test --manifest-path toolchain/cargo/Cargo.toml -p cargo --test jr_test_harness_paths --locked -- --nocapture`
- Strict lint attempt: `20261015-052452170-815969`, exit 101, blocked by ten
  unchanged errors outside the repair.
  `justrust clippy --manifest-path toolchain/cargo/Cargo.toml -p cargo --test jr_test_harness_paths --locked -- -D warnings`

The new `toolchain/cargo/tests/jr_test_harness_paths.rs` runs the newly built
Cargo CLI, not copied functions or mocks. It covers lib, integration, and
harness=false tests, debug/release, split artifact/build directories, JSON
`executable`/`filenames`, `--no-run` followed by direct executable invocation,
missing-public-link recovery with all artifacts still `fresh`, and Cargo's
own execution collection. An ordinary non-test binary is a negative control
and stays at the profile root.

The strict lint blockers are exactly: `jr_cutoff.rs:54` disallowed
`std::env::var`, `jr_cutoff.rs:289` disallowed `std::env::var_os`,
`fingerprint/mod.rs:690` disallowed `std::collections::HashSet`, and seven
`eprintln!` sites in `jr_daemon.rs:207,450,458,468,473,528,587`. No inherited
lint rule was relaxed and no unrelated code was edited. The full strict lint
gate remains blocked, not passed. The three owned redacted run records are
in the new shard `data/runs/isolation-a2-cargo/2026-10.jsonl`.

Cargo formatting and `git diff --check` pass. The full native Cargo suite,
platform-specific debug sidecars, cross-target execution, and a matched
performance series are not validated here. This is a correctness fix, not a
speedup claim or default toolchain promotion. The patched Cargo was built and
tested on hosted, not registered over any frozen A2 toolchain. No compiler
rebuild or full benchmark was launched. A2 remains ineligible, all seven
point outcomes are not yet resolved, and quiet thresholds remain unchanged.
