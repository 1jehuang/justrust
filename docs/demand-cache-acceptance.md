# Demand-query-cache acceptance

`bench/reuse-demand-cache-accept.py` is a direct frozen-rustc correctness gate
for default-off, **tracked** `-Zjr-demand-query-cache=yes/no`. The candidate
skips end-of-session query-cache promotion, while demanded values remain
serialized normally. This harness is not a speed benchmark or default-promotion
gate. Root owns compiler builds, frozen compiler release, native execution,
and representative performance measurements. No native compiler or fixture
was executed during harness implementation. Root subsequently released the
frozen compiler and completed native acceptance on October 30, 2026, as
documented in the retained evidence section below.

## Execution boundary

Before root releases the new frozen compiler, run only mocks:

```sh
python3 -m unittest discover -s bench -p 'test_reuse_demand_cache_accept.py' -v
```

After release, root can run mandatory semantics alone:

```sh
python3 bench/reuse-demand-cache-accept.py \
  --rustc /absolute/path/to/frozen/rustc \
  --output /absolute/path/to/new-demand-acceptance
```

Root may independently request the optional native mechanism gate with
`--require-mechanism`. Its failure makes the overall verdict fail but preserves
a separately passed `semantic_verdict`. Without it, `mechanism_verdict` is
`not-requested`, never silently inferred from semantic success.

The output directory must not exist and its parent must exist. Use the actual
compiler executable and its sysroot exposing `lib/*rustc_driver*`, not a shell
wrapper. No installed compiler is replaced, and the harness does not set
`RUSTC_BOOTSTRAP`.

## Mandatory exact differential semantics

Every ordered revision runs nine separate direct compiler processes:

1. Fresh `no` in a revision-specific empty incremental directory.
2. Fresh `yes` in a different revision-specific empty incremental directory.
3. Persistent `no`.
4. Same-flag persistent `no` replay of the identical revision.
5. Persistent `yes`.
6. Same-flag persistent `yes` replay of the identical revision.
7. Transition `no` before.
8. Transition `yes`.
9. Transition `no` after.

The persistent `no` and `yes` paths are separate and remain unchanged across
**all revisions**, including errors and recoveries. Each has two same-flag
processes per revision. These paths, not flag transitions, are the crucial
missing-green-value regression gate. Because the option is tracked, no/yes/no
transitions may invalidate the old cache and cannot by themselves establish
safe reuse or expose absent promoted values. All three transition modes still
share one additional directory to check tracked-flag changes explicitly.

There are **32 revisions x 9 modes = 288 main compile observations**.
Dependency compiles are additional. The first 22 revisions adapt the existing
scalar-query acceptance cases: unchanged builds, ordinary body edits/reverts,
constness, async functions/closures, nested-owner addition/removal/reordering,
macro edits, lint/type errors and recoveries, dependency signature edits,
mismatches, and recovery. The last ten intentionally change demanded generic
specializations: both `u32` and `u64`, unchanged wide demand, narrow to `u32`,
unchanged narrow demand, unrelated narrow-body edit, return to wide demand,
unchanged wide demand, type error, recovery, and unchanged recovery. The generic
function and implementations remain unchanged across the wide/narrow/wide
sequence. Every revision also runs the same-flag replays above.

All successful modes must match independently specified runtime status/stdout/
stderr, not merely agree with each other. Expected error modes must exit `1`,
not crash, ICE, or falsely succeed. Main and dependency compiles request
`-Zincremental-verify-ich`. Source, output executable, dependency source and
artifact paths, crate names, and edition are identical across a revision's
modes. Only explicitly tested flags, incremental directories, and optional
profile directories vary. Old executable and dependency outputs are removed
before compiling, preventing stale execution after an error.

Every non-reference mode is compared against fresh `no` for exact compiler,
dependency compiler, and successful runtime status/stdout/stderr bytes. Nothing
is normalized: no source-path replacement, newline conversion, diagnostic
sorting, warning suppression, or span filtering. Raw subprocess bytes are
base64 encoded in the report. Artifact hashes are retained for investigation,
not required to match. The new harness imports the existing byte-exact
compile/identity helpers in a private module instance, with the demand flag,
and imports the scalar fixture matrix without editing either existing file.

## Optional mechanism evidence, distinct from semantic correctness

`--require-mechanism` adds ten successful direct compiler processes: cold,
unchanged, demand-narrow, unchanged demand-narrow, and demand-wide-return,
each with an off control and on candidate. Independent `mechanism/no` and
`mechanism/yes` caches persist across this entire five-revision sequence.
They never share caches with the semantic matrix. Each process has a new
profile directory under `mechanism-profiles/` and requests
`-Zself-profile-events=default,function-args`, including generic activities.
Every compile and runtime still passes independent expectations and exact
no/yes comparison for that revision.

The compiler owner confirmed these exact no-argument `GenericActivity`
interval labels, entered on successful incremental persistence even for cold
sessions:

- `incr_comp_query_cache_promotion`: off must be positive, on must be zero.
- `jr_demand_query_cache_promotion_skipped`: on must be positive, off must be zero.

Counts, not positive elapsed durations, establish the branch evidence. Missing
markers, wrong event kind/type, malformed profiles, incomplete sequences,
absent off controls, or both branches observed fail the requested mechanism
gate. There must be exactly one `.mm_profdata` per profile directory. Raw
profile hash, size, metadata, marker counts, and unresolved record count are
retained. Unresolved virtual query labels remain unknown and never guessed.

`jr_incremental_green_recompute` is also counted. Its count at on-demand-wide
return is **observational only**: zero is not a semantic or mechanism failure,
and a positive count does not by itself identify a particular missing green
value as its cause. The changing-demand fixture attempts to exercise fallback,
but no claim of causal missing-value fallback proof is made without further
query-level attribution. Skipped-promotion evidence proves a native branch
was taken, not that a cache hit happened or that execution became faster.

## Failures and immutable identity

Before and after the run, record executable SHA-256, raw `-vV`, sysroot query,
and resolved compiler-driver library SHA-256 values. Any identity change
invalidates the run. Probe both explicit flag values with verify-ICH before
fixtures. Any rejection or crash fails with raw diagnostics, never skips a
required case or silently substitutes a compiler.

`acceptance.json` is written even on failure and retains the planned/completed
case counts, every attempted mode result, full fixture sources/expectations,
commands, raw diagnostics, semantic/mechanism verdicts, and available profile
evidence. A failed optional mechanism gate preserves a passed semantic verdict.
Mock tests establish only this harness contract, not compiler correctness or
performance. Do not lower expected counts or discard failures to promote the
candidate.

## Retained native evidence, October 30, 2026

Root's released native run passed all **32 revisions x nine modes = 288 main
compile observations**, **198 dependency compiles**, and the separate ten-process
mechanism gate. Both `semantic_verdict` and `mechanism_verdict` passed. The
full unmodified report and all ten raw profiles are retained in
`bench/results/demand-query-cache-20261030/native-evidence.tar.gz`, along with
the compiler build/status, version, before/after identity logs, acceptance
log/status, and root's successful 630-test Python suite log. The archive
preserves original physical absolute paths and byte-exact diagnostics, not
normalized substitutes. It excludes native executables and incremental caches.

The five off controls each recorded one normal promotion and zero skipped
promotions. The five on runs each recorded one skipped promotion and zero normal
promotions, including cold sessions. Wide-demand return observed **1,376** green
recomputations on and **1,242** off. These counts do not establish which missing
value caused any fallback, a cache hit, or a speedup.

Frozen executable SHA-256:
`570faeb27338cea8625c4a4a7eaeef71fc75470508657ff489c3da96ce5f9144`.
Frozen driver SHA-256:
`ed21691a9db267ffb71cc3b8975a8cd89768397a7a7f82c9c839083485424efd`.
Both before/after identities match.

Offline validation requires no original scratch directories, compiler, or
fixture binary execution:

```sh
python3 bench/results/demand-query-cache-20261030/audit.py
python3 bench/results/demand-query-cache-20261030/test_audit.py -v
```

The portable audit repeats every exact semantic comparison and independent
runtime/error expectation, checks identities/version, explicit mode flags,
verify-ICH, sources/output paths, edition, and persistent cache continuity
across revisions. It reparses all ten raw profiles, verifying hashes, byte
counts, metadata, native marker counts, and unresolved records before validating
the mechanism proof. `audit.json` records the successful replay and archive
hash. Eight mutation tests check that diagnostic changes, missing replays,
cache resets, identity drift, absent markers, conflated mechanism caches, and
corrupted raw profiles are rejected. This evidence does not substitute for
the separate representative performance or default-promotion gates.
