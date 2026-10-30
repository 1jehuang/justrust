# Matched Blaze runtime results, 2026-10-30

## Follow-up: seeded LLVM also passes the runtime contract

The prospective `br35` run adds an explicit `BLAZE_BACKEND=llvm`, `BLAZE_OPT=0`
stack alongside explicit Cranelift and the same four owned configurations.
Both Blaze stacks receive byte-identical copies of the same retained seed.
All six stacks pass compile-error/recovery and test-failure/recovery gates,
then five identical executed edits each. This is 60 command records and 30
matched edits. It removes the *backend family* difference for the LLVM pair,
not the different frontend, LLVM version, compiler implementation or coverage.

| br35 stack | Median wall seconds, 5 edits |
|---|---:|
| Owned, earlier experiments off | 3.503743 |
| Owned, earlier three on | 3.340807 |
| Blaze Cranelift | 0.307640 |
| Blaze LLVM | 0.401728 |
| Owned, all four on | 3.633667 |
| Owned, all four plus hotpatch | 3.862631 |

**This was heavily contended:** load averages during the first timed edits
were about 42.9 on this 16-logical-CPU machine. Do not compare these absolute
times to `br34`, claim controlled ratios, or conceal the hotpatch configuration's
slower median here. All five hotpatch edits actually patched, but that does not
guarantee a wall-time improvement. The observed gap persists with Blaze's LLVM
backend in this scoped workflow, so choosing Cranelift alone is not a sufficient
explanation. This does not isolate the causal share of frontend retention.

The earlier fresh-store LLVM failures remain failures. The successful seeded
LLVM gate does not retroactively establish cold-store reliability.
Raw logs, copied-store manifests, backend environment pins and identities are
in `bench/results/blaze-runtime-comparison-20261030/br35/`; forty owned calls
are exported in `data/runs/xps14-runtime-br35-*-20261030/`. Replay with:

```text
python3 bench/results/blaze-runtime-comparison-20261030/validate-complete.py --case br35
```

The portable audit passes six gate stacks, 30 matching edits, exact raw-output
hashes, fresh runtime markers, five actual patch receipts and all forty exports.
No default was changed. The earlier `br34` observations below remain a separate
five-stack run, not samples pooled with this follow-up.

## Outcome and scope

The final `br34` comparison completed all five stacks, all correctness gates,
and 25 timed edits. It measures a **seeded warm-store edit, compile, link and
execute workflow**, not cold support, a universal compiler comparison, or a
statistically established speedup. No experimental compiler flag was promoted.

| Stack | Median wall seconds, 5 edits |
|---|---:|
| Owned compiler, three earlier experiments off | 1.318050 |
| Owned compiler, three earlier experiments on | 1.263141 |
| Blaze, Cranelift, opt 0 | 0.143916 |
| Owned compiler, all four experiments on | 1.284972 |
| Owned compiler, all four on, existing hotpatch enabled | 1.073436 |

Raw evidence: `bench/results/blaze-runtime-comparison-20261030/br34/`.
Run `python3 bench/results/blaze-runtime-comparison-20261030/validate-complete.py`
from any checkout location to replay acceptance and verify owned run exports.

## What was matched

All stacks use the same pinned `f97fcc2` source bytes and the same four
`summary::tests::` tests, one thread, nocapture, opt 0, debug 0, incremental
builds, and five identical executed test-body edits in rotated stack order.
Each timed edit must print its exact fresh runtime marker once. Each stack
first passes deliberate compile-error/recovery and test-failure/recovery gates.
Expected successful counts are four passed, zero failed, 144 filtered.

Targets, homes and stores are private. The owned compiler, its shared driver,
Cargo, client and copied wild linker are hash-attested. All 63 source files
are restored. Owned output is replayed from the uniquely fresh saved raw
journal, not inferred from condensed UI. Forty owned calls are exported under
`data/runs/xps14-runtime-br34-*-20261030/`. All five timed hotpatch calls have
actual `patched` receipts and pass the changed-code runtime assertion.

## What was not matched or established

* Blaze uses its own frontend and Cranelift, while the owned compiler uses LLVM.
  Matching observable tests does not establish identical language coverage.
* Blaze was seeded from the previously successful private `br30` disk store.
  The seed and copied files are identified and unchanged. Repeated fresh-store
  attempts failed on writable statics, including explicit Cranelift. Those
  failures remain retained and excluded, not described as LLVM-only failures.
* Machine quietness is unknown. These small, ordered sets provide descriptive
  observations, not significance, a causal attribution for small differences,
  or a general speed claim. Preparation and correctness gates are untimed.
* This is one executed test edit in one pinned project, not the comprehensive
  session-weighted synthetic gate. That gate remains incomplete from earlier
  pinned environment/test failures. No default promotion is justified.

`br30` through `br33` remain retained. The original `validate.py` and
`validation.json` describe only the first three historical attempts. The new
`validation-complete.json` describes the accepted final run. In `br33`, the
hotpatch stack was excluded because explicit compiler selection did not
establish nightly pre-link argument routing. The final harness verifies
`rustc -vV` before timing and sets the existing nightly capability flag only
for a hash-attested nightly compiler. It does not force that flag for stable.

## Transferable lessons and implementation

Blaze's observed warm behavior includes retained frontend state, function-level
code generation and executable patching. It is not merely rustc's on-disk cache
moved into RAM. Logs from the accepted initial warm store show almost all
functions reused and only a handful patched. That is a finer-grained pipeline
than rerunning substantial crate frontend work.

We implemented native per-item liveness query retention in the owned compiler,
behind tracked, default-off `-Zjr-cache-dead-code-items`. The original scanner
and global reachability fixed point remain authoritative. Two query contexts
preserve allow/expect behavior and diagnostic replay. The differential native
gate passed 32 revisions across seven modes, 224 compiler invocations, with
byte-exact diagnostics/status, runtime checks, and incremental hash validation.

On actual pinned body edits, the experiment performs 3,211 named ordinary and
180 named allow/expect disk loads, with five ordinary same-value provider
executions. This proves finer-grained reuse, **not saved wall time**. Forced
unchanged captures have unresolved labels and are not assigned to these query
kinds. See `docs/dead-code-real-retention-20261030.md` for full limitations.

We also activated and measured the existing opt-in hotpatch path correctly.
All five final edited executables were patched and executed successfully.
This is not a claim that hotpatching was newly implemented in this work.

## Independent supported benchmark gates

Both pairs use `bench/suite.py run justrust-body-test -n 3 --warmup 1
--quiet-wait 0 --toolchain jr-b7b7efc9982d` with coherent, attested rustc,
shared driver, Cargo, rustdoc and clippy artifacts. Full tests pass: 157 per
measured sample, zero unexpected failures. The same `jcode-isolated-v1`
runtime policy applies to both sides. Private depcache is enabled equally
because this pinned project's build-script fixture requires it. That policy
is different from the four-test runtime harness, so these numbers are not
interchangeable with its table.

Exact `bench/suite.py compare` conclusions:

```text
Dead-code retention: 2.57 -> 2.60s, +0.03s, +1%, uncontrolled (noisy or unknown samples)
Hotpatch only:       2.78 -> 2.73s, -0.05s, -2%, uncontrolled (noisy or unknown samples)
```

The first pair changes only dead-code item retention with the earlier three
flags enabled. The second changes only hotpatch with all four flags enabled
and identical pinned wild on both sides. Each of the three measured hotpatch
runs reports three patched test executables. There is **no demonstrated
retention speed win and no statistically supported hotpatch improvement** in
these full-test samples.

Both full result pairs, runner snapshots, artifact identities and comparison
output are retained in `bench/results/{dead-code,hotpatch}-suite-20261030/`.
All sixteen calls, including warmups, are exported under
`data/runs/xps14-native-suite-20261030/`. Example measured run IDs:
`20261030-025226633-1068258516` (retention off),
`20261030-025255895-1077024379` (retention on),
`20261030-025802164-1135394810` (hotpatch off),
`20261030-025813885-1141148463` (hotpatch on).

## Remaining frontier

The measured gap is still primarily a reason to investigate retained frontend
state and function-granular compilation, not to add every cache indiscriminately.
Resolver/HIR and crate-wide frontend work remain targets. Prior inclusive
profile scopes overlap and cannot be added as potential savings. Neither the
maximum reusable fraction nor an optimal end-to-end architecture is established.
The new item cache stays off because mechanism reuse alone did not translate
into a measured full-test win. Cold Blaze reliability and the comprehensive
session-weighted gate remain open limitations.

Validation: all 570 benchmark Python tests passed after the final harness
changes. The portable final audit verifies 50 command records, 25 matching
edits, five gate stacks, five actual patches, 40 runtime exports, and 16
supported-suite exports. No shipping default or global toolchain was changed.
