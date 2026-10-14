# Native execution graph acceptance, October 14, 2026

Contract: [execution-graph-completeness.md](execution-graph-completeness.md).
This is an observability improvement, not a speedup or default-toolchain
promotion. The source-to-test-result graph is still incomplete.

## Verified checks

| Check | Observed result |
|---|---|
| Hosted client check | Pass, `20261014-075549378-278694` |
| Full hosted client tests | 449 passed, 0 failed, 3 ignored, `20261014-075616983-282295` |
| Strict client clippy | Pass with `-D warnings`, `20261014-075636785-278367` |
| Hosted native Cargo graph/fingerprint tests | 12 passed, 0 failed, `20261014-080048729-333334` |
| Strict native Cargo lib clippy | Blocked by 10 existing findings in cutoff, fingerprint HashSet and daemon diagnostics, `20261014-080057686-333304`. No capture-module lint findings remain. |
| Installable native diagnostic Cargo | Local build reconciled successfully after server reload, 0 compiled / 316 fresh, `20261014-080327843-350215` |
| Client installation | `cargo install --path . --locked --offline` completed, installed CLI independently exercised |
| Real repository library build | Pass, `20261014-080423984-376339`, 31.832 seconds under contention |
| Native-to-client graph validation | Primary real build has 86 declared nodes / 126 prerequisite edges, no issues; scoped structural completeness true, full pipeline false |

The real repository build restored 73 dependency units from cache and compiled
its library. Primary raw receipt SHA256:
`b9c1fac67b671a1d02915974af4551ea10cbef2d59b467377992bb5c3bdd3111`.
Raw data stays in private scratch storage. The compact CLI assessment is retained
in `bench/results/execution-graph-completeness-20261014/real-build-assessment.json`.
Each receipt is assessed independently. A directory of receipts is not a
cross-process causal graph or an attestation that every producer was present.

## Actual build neutrality and failure acceptance

`tests/test_execution_graph_native.py` runs an actual rebuilt jr-cargo through
justrust, not a copied implementation. Its fixture contains two packages, a
library-to-library metadata edge, a binary and a real build script. Both the
binary's compile-time environment and build-script runtime assert that the
capture sink is absent. The script also tracks the sink with
`rerun-if-env-changed`, deliberately exercising fingerprint neutrality.

The final public test invocation passed:

```sh
JR_GRAPH_CARGO="$JCODE_SCRATCH_DIR/jr-graph-native-target/debug/cargo" \
JR_GRAPH_RUSTC="$HOME/.justrust/toolchains/nightly-2026-10-08/bin/rustc" \
python3 -m unittest discover -s tests -p test_execution_graph_native.py -v
```

Recorded acceptance sequence:

- Baseline fixture build: `20261014-080345438-367494`, 4 compiled.
- Capture-on with unchanged inputs: `20261014-080345707-367635`, **0 compiled**.
  Every native scheduler node is fresh and the executable SHA256 is identical.
- Capture-off again: `20261014-080345872-367661`, **0 compiled**.
- Separate fresh target with capture: `20261014-080345970-367689`, 4 compiled,
  successful task lifecycles and an actual `rmeta` prerequisite edge validated.
- Intentional compiler failure: `20261014-080346254-367821`, exit 101 with the
  expected diagnostic, failing node and unstarted dependents fully accounted.
- A sink path occupied by an existing ordinary file:
  `20261014-080346451-367921`, build succeeds and existing contents are unchanged.

This establishes correctness and artifact reuse for the exercised cases. It is
not a comprehensive performance benchmark or proof about arbitrary build
scripts, remote execution, or compiler internals.

## Findings corrected before publication

The first real acceptance attempt, `20261014-074800372-188303`, showed that
removing the sink from child environments alone was insufficient: Cargo still
compared rustc's recorded environment against its own capture-enabled
configuration, producing a Dirty unit. Native fingerprint comparisons now use
the actual child-visible absence of this reserved observation variable. The
final on/off sequence above proves no rebuild in the targeted regression.

Independent review also caught two false-completeness cases in the consumer:

1. An invented `all` artifact-ready event could incorrectly allow a failed
   producer to unblock a dependent. Only successful/fresh terminal events now
   make `all` available. Explicit readiness events are metadata only.
2. Equal timestamps could hide reversed task lifecycle or handoff ordering.
   The validator now checks the native event sequence as well as timestamps.

Both have regression tests. Successful metadata readiness followed by later
codegen failure remains valid, preserving actual pipelining behavior.

Native writer tests cover the exact serialization budget, budget-minus-one
failure, partial cleanup, existing partial/final collision preservation, and
private permissions. Final publication is atomic and non-replacing, with a
64 MiB streaming cap.

## Evidence and remaining work

Only the 14 cited runs were selected through a scratch export home and exported
from the repository root using `justrust export`. Their path-redacted summaries
are in `data/runs/execution-graph-20261014/2026-10.jsonl`. Existing unrelated
staged FINDINGS and run-history work was preserved, so these observations are
recorded here rather than mixed into that staged change.

The installable Cargo is a diagnostic build in an isolated artifact target,
not a promoted fleet or default binary. Client graph inspection is installed.
The initial producer is opt-in and local. Compiler query/spawn/join internals,
jobserver/resource waits, linker internals, tests, cache/service boundaries and
hosted receipt transport are still opaque. Full completeness, CPU work,
execution span and a 96-core scaling ceiling remain unknown.
