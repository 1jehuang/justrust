# Native owner coverage acceptance, October 14, 2026

This gate detects absent scheduler captures against an independent native
BuildRunner declaration. It is not whole-build, directory, compiler-internal,
or hosted coverage. No speedup or thread ceiling is claimed.

## Final gates

- Hosted client check: `20261014-083226907-825025`, passed.
- Hosted complete client tests: `20261014-083229732-825527`, 457 passed,
  zero failed, three ignored.
- Hosted client strict clippy: `20261014-083245294-829731`, passed.
- Hosted final native capture/owner tests: `20261014-083217673-822254`,
  all 19 passed. Eight owner tests supplement 11 original capture tests.
- Native strict clippy remains blocked by 10 pre-existing cutoff, fingerprint
  and daemon findings. Baseline failure: `20261014-082825487-755867`.
  Baseline-excluded lint passed: `20261014-082943253-778873`.

The installed client validator uses the final pending-state schema. The native
Cargo diagnostic binary was rebuilt locally in an isolated scratch target.
Local execution is required to exercise the newly built producer, not a fleet
binary. The native binary was not promoted to a default toolchain or hosted fleet.

## Actual rebuilt native acceptance

Run the opt-in acceptance:

```sh
JR_GRAPH_CARGO="$JCODE_SCRATCH_DIR/jr-graph-native-target/debug/cargo" \
JR_GRAPH_RUSTC="$HOME/.justrust/toolchains/nightly-2026-10-08/bin/rustc" \
python3 -m unittest discover -s tests -p test_execution_graph_native.py -v
```

The final test passed against native-produced records. It verifies unchanged
binary hashes and capture off/on/off reuse, compiled metadata edges, intentional
compiler failure accounting, and an unwritable sink preserving private contents.
It also deletes an expected envelope and receipt, deletes the initial owner
journal, contradicts observed execute-enter with a forged not-started final
manifest, and injects an independent pending authority. Missing and conflicting
selected-authority evidence is rejected or incomplete. The independent authority
is explicitly listed as excluded, never included in run coverage.

Final fixture builds:

| Run | Observation |
| --- | --- |
| `20261014-083632675-893606` | Initial build, four compiled units |
| `20261014-083633187-893812` | Capture on, zero compiled units |
| `20261014-083633383-893845` | Capture off, zero compiled units |
| `20261014-083633538-893877` | Separate target, four compiled units |
| `20261014-083633829-894010` | Intentional compiler failure, exit 101 |
| `20261014-083633954-894053` | Unwritable sink, successful build |

## Real repository build

`20261014-083519591-872352` built this library using the rebuilt native producer:
33.741 seconds, 74 recorded compiled invocations, 11 dependency-cache hits and
62 misses. Changed configuration caused dependency rebuilds, and unrelated
processes averaged about nine cores of concurrent use. These are diagnostic
conditions, not a before/after performance comparison.

The independent receipt has 86 nodes and 126 edges. The owner declared one
required scheduler capture and reconciled one valid capture, with no issues.
The committed compact assessment identifies only that authority. Full pipeline
coverage and outcome agreement remain false, and CPU work/span remain unknown.

Raw private journals remain in scratch. Selected cited build records are exported
under `data/runs/execution-graph-owner-20261014/`. Ancestor-directory trust,
bounded observation I/O latency, outer command ownership, compiler/query
instrumentation and hosted transport remain separate work.
