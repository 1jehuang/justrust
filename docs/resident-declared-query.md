# Rejected native declared graph query (v69)

**Decision: rejected for performance. The candidate implementation was restored
out of `main.rs`; no promotion or speed claim.** The coordinator's actual native
correctness gates passed, including 53 codegen tests, 367 resident tests and all
28 strict runtime rows. Two paired OFF/ON loops did not establish a useful
replicated scoped latency improvement. Noise remains unknown, so the results
are not a causal regression claim either.

The evidence bundle is `bench/results/resident-declared-query-20261031/`.
It preserves the full frozen candidate, original and live-before-restoration
Rust source, selected actual raw output and strict receipts, all named native
results, full inventories, original oracle sources, descriptive timing analysis,
and the exact-restoration receipt. The candidate is archived, not shipped.

## Hypothesis and implementation that was tested

Accepted source was `afa7d609519aa1ba05a6fa71770f12aa8ae76011`. The supplied
v68 OFF recovery worker log reported roots 4.0s, repeated eager scope/body
listing, and core's 6490-body listing twice. That observation motivated testing
whether Salsa memoization of the final complete declaration graph could avoid
repeated reconstruction. It did not establish a speedup in advance.

The candidate used exact `RA_JIT_DECLARED_QUERY=1`, default OFF, latched on the
first `declared_in` call in each process. Any other value stayed OFF, including
unset, empty, `0`, `true`, `01`, and `1 `. Changing the value required fresh
daemon and worker processes.

ON called native Salsa `crate_declarations` keyed by
`ide_db::base_db::Crate`, the actual Salsa input ID, not a HIR wrapper. It
returned `&Declared`; the existing caller cloned the full result. All vector
order, all fields, ordinary/provided traversal order, and the final false
`under_provided` traversal-state flag were preserved. Nothing was sorted,
deduplicated, decoded from storage, or omitted.

OFF called `declared_in_original`, whose algorithm body was byte-for-byte the
accepted `declared_in` body, including parallel `lower` and original listing
telemetry. The candidate introduced no persistent cache, v39 decoder cache,
v47 scope-only cache, global mutable graph map, or revision-key bypass.

The provider accepted `&dyn HirDatabase`, with no available RootDatabase
snapshot. ON therefore expanded the same graph sequentially through tracked
`body_declarations`, which reads `Body::of`. It did not prewarm on a different
snapshot or hide reads from Salsa. OFF retained parallel lowering. Sequential
cold traversal was a potential cost, not something assumed faster. The observed
latency comparisons do not isolate that cost as the cause.

## Dependency and cycle audit

The final provider explicitly read `crate_def_map`, `module_declarations`,
and the growing ordinary/provided worklists through `body_declarations`.
Module traversal retained child-module, definition-map and scope reads.
`Declared::in_scope` retained declaration order, unnamed constants, impl and
trait associated items, function containers and signatures through
`is_bodiless_intrinsic`. Body fragments retained `Body::of`, block definition
maps, module scopes, macro expansions, constants/statics and closure blocks.
The owned tracked `Body::of` read `Body::relative` or `Body::with_source_map`.
There was no untracked-read suppression or synthetic dependency.

The new binary-local final provider was called only by the wrapper and tests.
No lower-level library query called it, and no existing query was redirected
back to `declared_in`. The worklist traversal followed the original algorithm
rather than recursive final-query calls. Existing name-resolution/body cycle
handling was unchanged. This source audit is supplemented by the actual tests
and runtime rows below, not a proof of every possible program's behavior.

Body/signature changes can produce equal graph IDs and allow Salsa backdating.
Revalidation can still visit lower-level fragments even if the final provider
stays green. A final provider not rerunning does not prove zero lowering work.

## Actual correctness gates

The coordinator ran, with the pinned native toolchain and no lock changes:

```text
justrust check --release --locked -p ra-jit --tests -p hir-ty-codegen -j 6
justrust build --release --locked -p ra-jit --bin jr-engine -j 6
justrust test --release --locked -p hir-ty-codegen -j 6
justrust test --release --locked -p ra-jit --bin jr-engine -j 6
```

All exited 0. Full named results contain **53 codegen passes** and **367
resident passes**, with no failures or ignored tests. The resident gate included
all three actual `declared_query_` fixtures:

* `declared_query_flag_is_exact_and_default_off`
* `declared_query_real_database_revisions_match_both_original_walkers`
* `declared_query_tracks_dependency_macros_and_reuses_unrelated_crate`

The real RootDatabase fixtures compared complete `Declared` equality and order
against both the unchanged OFF algorithm, including parallel lowering, and a
direct independent scope/body walker that used neither declaration fragment
queries nor the final graph query. Actual-provider-only cfg(test) TLS counters
required cold entry 1 and unchanged repeat entry 0. Nonvacuous named fixtures
covered nested items, provided methods and associated constants, ordinary
constants/statics, closure blocks, cfg, macros, body/signature revisions,
recovery and dependency macro edits. An unrelated crate body stayed green.
Body/signature-only edits did not force provider execution when equal outputs
could be backdated. A dependent macro had to actually emit its new named item.

The build's complete 3757-source inventory and root Cargo.lock were checked
unchanged after native gates. Four actual seven-phase runtime cases retained
63 workload-source identities and 4020 pinned-toolchain file identities. Each
row used fresh runtime/type/assertion nonces and the original strict named-test
oracle, including expected error status 101 for type/assertion failures and
successful recovery. All **28 actual raw/strict rows passed**.

## Scoped descriptive timings

Seconds are elapsed runtime command time, not isolated query time. Each pair
compares the same candidate binary with default-OFF selection versus enabled
selection. The second pair is an independent seven-phase repeat.

| Phase | OFF 1 s | ON 1 s | ON vs OFF 1 | OFF 2 s | ON 2 s | ON vs OFF 2 |
|---|---:|---:|---:|---:|---:|---:|
| Cold prepare | 26.673 | 28.540 | +7.00% | 28.706 | 32.439 | +13.01% |
| Repeated no-op | 0.056 | 0.058 | +4.44% | 0.060 | 0.059 | -1.02% |
| Manifest feature add | 12.645 | 15.028 | +18.84% | 13.308 | 15.162 | +13.93% |
| Type error | 12.944 | 16.170 | +24.92% | 13.827 | 15.484 | +11.98% |
| Type recovery | 12.034 | 14.320 | +19.00% | 12.860 | 14.438 | +12.27% |
| Assertion failure | 3.008 | 3.050 | +1.41% | 3.070 | 2.964 | -3.46% |
| Assertion recovery | 0.613 | 0.633 | +3.37% | 0.684 | 0.676 | -1.13% |

These are scoped observations only. Unknown contamination prevents attributing
the differences causally to sequential lowering, Salsa bookkeeping, or another
component. The no-op/assertion deltas also do not establish a useful repeatable
win. This was not the comprehensive session-weighted synthetic gate and makes
no general stock-versus-justrust or default-promotion claim.

## Marker interpretation

`RA_JIT_DECLARED_QUERY_MARKER` appended
`declared-query-provider-entry pid=<pid>` once per process, inside the actual
provider. Open/write errors were ignored. The first provider entry consumed the
attempt even if no path was supplied or writing failed.

Both OFF cases had cumulative counts `[0, 0, 0, 0, 0, 0, 0]`.
Both ON cases had cumulative counts `[1, 1, 2, 3, 4, 4, 4]`.
These snapshots prove actual provider entry in enabled processes only.
They are **not cache-hit proof**, successful-completion evidence, per-phase
query counts, avoided-work counts, or latency attribution. Repeated snapshots
retain earlier lines and worker replacement can add a new process entry.

## Exact restoration and evidence-only replay

Before restoration, live main and proposal document each matched their frozen
candidate byte-for-byte. Both were saved under `live-before-restoration`.
Only `main.rs` was then restored with the write tool from `main.original.rs`.
At `2026-10-31T12:50:57Z`, exact original comparison and scoped `git diff HEAD`
were empty, with main SHA256
`0246385937d776e28fc87a0484db8830078f6bc629e930c988f5acb82b63d176`.
The candidate SHA256 was
`1b697f35ac35831b6e77e36980bda0dddeca026d034d624e4a0b0fa8f23bc34a`.
Source was immediately released to the coordinator. This is a point-in-time
restoration receipt, not a claim that later, separately owned experiments leave
live main identical. Frozen originals were not modified. Root separately stopped
four exact completed private namespaces, preserving their stores.

The worker performed source formatting/checks before release, then restoration
and evidence-only offline Python work. It ran no native jobs, probes, installs,
lock edits, or commits for this lane. Root performed the recorded native gates.

Portable replay, including after relocation, requires only Python's standard
library and packaged files:

```sh
python3 -B bench/results/resident-declared-query-20261031/validate.py --manifest
python3 -B bench/results/resident-declared-query-20261031/test_offline.py
```

The verifier extracts hash-bound original pure parser/oracle function bodies
without importing execution drivers. It freshly reparses selected actual raw
text into the original tuple-valued results, rather than weakening the original
oracle to accept JSON lists. Fixed historical observation/native-result digests
protect against coherent rewrites even if mutable manifest hashes are replaced.
These are integrity commitments, not external cryptographic authentication.

Privacy export uses fixed fields, eight explicit non-path environment values,
five explicit controls, and full-line stdout/diagnostic/PID-marker allowlists.
Twelve path-bearing environment entries per row are omitted, not republished.
Full relative inventories are retained. Private state, ambient environment,
worker/daemon logs, credentials, telemetry, binaries and targets are excluded.
Only selected source/document/script derivatives replace private root constants
with labeled placeholders. Full Rust source retains existing synthetic fixture
paths such as `/home/one/.cargo`, not the user's actual home.
