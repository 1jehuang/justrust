# Dependency-mapping acceptance trace, 2026-10-15

## What is actually better

The improvement is **additional trustworthy diagnostic information**, not a
proven compile-time improvement or a completed execution DAG. The original cold
semantic read stream remains `Disabled`. On the accepted four-thread cold build,
the new public dispatch assessment nevertheless exposes **18,297 actual returned
indices and 17,669 certified request-to-prior-publication relations**. Those
relations were unavailable through the prior cache-only assessment. The same
capture independently exposes 153,723 cache-hit relations. They remain separate
assessments and neither is relabeled an execution dependency.

This is an observed information gain from the actual Rust compilation path.
It does not establish that the compiler runs faster, that every dependency is
known, or that 96 threads would be useful.

## Acceptance paths, not just test counts

1. **Compiler-to-executable acceptance already executed:** the six-test native
   acceptance compiled untouched, locked `itoa 1.0.18` through the owned compiler
   at one and four threads, cold and incremental. It linked an actual consumer,
   executed that consumer, checked compiler argv/env/output neutrality, and
   independently checked every certified dispatch row against original native
   request/publication/admission records. This is the real Rust source-to-result
   path, not a mocked provider or a synthetic replacement source. Retained
   artifacts and observations are described in
   `dispatch-result-provenance-results-20261015.md` and
   `../bench/results/dispatch-result-provenance-20261015/README.md`.
2. **Installed public interface independently re-exercised:**
   `bench/verify_dispatch_public.py` invoked the installed `justrust graph`
   executable on those original real-build captures. Its final run,
   task `153518z1dm`, passed all eight cases in 17.22 seconds. Ten original JSON
   inputs and the consumer binary were hashed before/after and were unchanged.
   The path-redacted observed outputs are committed in
   `../bench/results/dispatch-result-provenance-20261015/public-cli-acceptance.json`.
   This replay validates the installed consumer, not a new compilation run.
3. **Adversarial cases supplement, not replace, that path:** native and consumer
   tests cover malformed/ambiguous/lost records, clock failure, nested dispatch
   and publication races that a natural `itoa` schedule cannot guarantee.

The installed consumer SHA-256 was
`e346c429d0373a4341314e375cd403611755a8c1f43cee61ad3f674ffa1d9666`.
Full replay stdout/stderr is retained privately under
`$JCODE_SCRATCH_DIR/dispatch-public-recheck-20261015-final/`. The compact summary
records exact output lengths and hashes. An initial verifier attempt used the
analyzer's `read_coverage` name instead of the public graph's `query_reads` name
and failed with `KeyError` after the CLI had succeeded. That failed attempt is
retained separately as `dispatch-public-recheck-20261015-v1`; no compiler or
consumer fix was needed. A corrected intermediate pass and the stricter final
pass are not presented as independent compiler benchmarks.

## Requirement-to-check-to-observation matrix

| Requirement or changed public output | Concrete check | Observed behavior | Scope / remaining limit |
| --- | --- | --- | --- |
| New root-6/read-2 data requires explicit dispatch analysis | Installed CLI, `root6-default-refusal` | Exit 1, `unsupported query receipt schema` | Default analysis was not silently broadened |
| Large captures require explicit bounded opt-in | Installed CLI, `cold-strict-budget-refusal`, then `cold-both` | Strict mode exits 1 with byte-budget error; explicit large mode succeeds on the unchanged 241,266,883-byte query receipt | This does not change strict defaults or claim unbounded capture |
| Dispatch results belong to the exact lexical successful request | Actual native cold one/four-thread builds, every-row original-stream oracle, then installed `cold-both` | Cold t1: 18,199 observed / 17,569 certified. Cold t4: 18,297 / 17,669, with original request, context, returned index and prior publication checked | Availability only, not fresh-provider/wait/cycle classification |
| Returned-index uncertainty and missing authority remain unresolved | `dispatch_result_nullable_fields_are_mandatory_and_wire_is_strict`, `dispatch_result_forbidden_metadata_routes_are_errors_even_with_loss`, and `dispatch_result_provenance_loss_namespace_and_clock_authority_stay_separate`; native exact-context/normal-terminal tests | Invalid association does not yield a certified row; source/admission loss clears certification, and null/unknown and loss counters remain explicit | These malformed/loss controls are synthetic adversarial fixtures, not naturally observed real-build loss |
| First-cache and dispatch metrics remain independent | Installed `warm-dispatch-only` versus `warm-both` | Removing only `cache_call_provenance` from the latter exactly reproduces the complete former JSON; cache count 3,580 | No merging into semantic reads or execution edges |
| Incremental dispatch is not falsely attributed by the nonincremental-only feature | Actual warm t1/t4 native builds and installed `warm-both` | Both observed and certified dispatch counts are zero; cache results remain present | Zero means not observed by this feature, not zero query work |
| Existing default root-4/root-5 behavior remains unchanged | Installed `legacy-root4` and `legacy-root5` compared to frozen real acceptance stdout | Exact stdout bytes match, 3,061 and 3,857 bytes respectively | Original native recorder byte-projection tests additionally cover default serialization |
| Timing and dispatch options compose without inventing whole-frontend work | Actual mixed native build and installed `mixed-timing` | Reciprocal clock aligned, retained CPU records complete, 17,603 dispatch relations; original mixed capture has 379 exclusive segments on three threads | Full `cpu_work_ns` / `execution_span_ns` remain null |
| CPU measurements are exclusive and failures cannot become zero-work claims | Actual native busy/sleep, nesting and unobserved-registry dispatch tests; raw real mixed segment checks | Segment arithmetic, ownership and per-thread/task interval checks passed; clock/regression/TLS uncertainty suppresses aggregate claims | Retained scoped CPU is not complete frontend CPU or conservation |
| New query schema must not accidentally authorize resource edges | `dispatch_result_provenance_graph_does_not_promote_resource_authority` exercises the real filesystem scanner on synthetic authority documents; installed `cold-both` independently checks its real capture | Resource state stays unvalidated/unknown; the synthetic control transitions from legacy validated authority to explicit missing reciprocal-authority diagnostics | The real capture has no resource documents, so it alone does not prove every rejection boundary |
| Observation does not change user artifacts or diagnostics | Actual compiler OFF/ON/OFF plus OFF/OFF controls, actual link and execution | Cold whole rlib/object/consumer identity passed; warm rmeta/object/consumer identity passed; argv/env/stdout/stderr unchanged | Warm whole-archive identity is not claimed because OFF/OFF also differs in randomized link metadata |
| Public analysis does not overstate semantic graph, critical path or useful threads | Every successful installed CLI case and committed compact reports | `full_pipeline_complete=false`, total CPU work/span null; semantic coverage remains false | Complete execution DAG, weighted critical path and thread ceiling are still open |
| Default-disabled performance needs a representative matched comparison | Actual attested 40-point A2 baseline and official suite eligibility audit | 40 points observed, 86 identity invocations verified, 39 noisy points and seven unexpected failing-test outcomes; zero comprehensive eligible workload | Performance acceptance is blocked. B2 was withheld; no speed or nonregression claim |

The requirement matrix deliberately distinguishes a passing scoped behavior
from the broader unmet goal. Aggregate test counts alone are not the acceptance
argument. Named consumer checks above are in `src/execution_graph_reads.rs`,
`src/execution_graph_query.rs` and `src/execution_graph.rs`. The actual-source
all-row oracle is `tests/test_frontend_mapping_native.py::assert_dispatch_receipts`;
`dispatch_results_interleaved_terminals_bind_their_own_exact_index` is a separate
adversarial consumer check, not a claim that natural scheduling exercised every
interleaving. The named checks were included in the recorded hosted/native
correctness gates described in the results document.

## How to reproduce the public replay

With the retained real acceptance cohort available:

```sh
python3 bench/verify_dispatch_public.py \
  --justrust "$HOME/.cargo/bin/justrust" \
  --accepted "$JCODE_SCRATCH_DIR/dispatch-result-real-final-20261015/frontend-real-itoa-1ov3ms2y" \
  --evidence "$JCODE_SCRATCH_DIR/dispatch-public-recheck-NEW"
```

The evidence directory must not already exist. Full stdout is private and
exclusive-created. This verifier invokes the actual installed executable. It
does not substitute an implementation, rewrite input receipts or run a compiler
benchmark. Its summary's original input hashes allow checking the exact retained
cohort rather than assuming any similarly named capture is equivalent.

## What is and is not closed

**Closed for this scoped change:** actual source-to-artifact behavior neutrality,
strict opt-in public analysis, added exact request/result availability evidence,
legacy output preservation, independent cache/dispatch/timing composition, and
conservative unknowns. These have direct observations from the compiler path
and installed public CLI, with per-requirement evidence above.

**Not closed:** complete semantic/control/resource/nonquery/backend coverage,
full frontend work/span, theoretical architectural parallelism, useful-thread
ceiling, and default-disabled performance. The complete A2 result and official
46-reason audit are preserved in `bench/results/native-dispatch-synth40-A2-20261015.json`
and `bench/results/native-dispatch-synth40-A2-audit-20261015.json`. Fixing workload
validity and obtaining a controlled matched measurement is required before a
performance conclusion. Dropping the noisy or failing points would not close
that loop.
