# Real native dead-code item retention, 2026-10-30

Read `docs/dead-code-real-retention-20261030.md` for the scope, identities,
selected count tables and explicit unknown-label limitations.

This directory is a count/correctness mechanism proof, not a performance
comparison. All eight real checks and exports succeeded using the same frozen
compiler with the option off/on and incremental stable-hash verification.
The body-edit and restoration on captures each have 3,211 ordinary and 180
allow/expect named item-query disk loads, and five ordinary same-value provider
reexecutions. No selected named records are observed for forced unchanged,
and none are inferred from unknown load totals. Off uses the original direct visitor and
therefore has no item-query records by design.

- `*-receipt.json`: full binary identities, local flags/environment, source
  and raw-profile hashes, check/export statuses, analyzer commands/statuses,
  immediate whole-query-cache logical/allocated snapshots.
- `count-comparison.json`: per-capture named selected query kinds and native
  marker arguments, whole-profile totals, explicit unknown counts and strict
  replay errors. No cross-invocation numeric-ID joins.
- `*-query_profile-unknown.json`: explicit-unknown raw profile count summaries.
- `*-{query_profile,reuse_ceiling,provider_cost}-{strict,unknown}.{stdout,stderr}`:
  original analyzer output, including all 24 strict failures and all 24
  explicit-unknown successes. Provider elapsed fields are raw scope evidence,
  not summed or compared to claim saved time.
- `*-strict-full-traceback.txt`: independent strict decoder failure traceback.
- `*-raw-{output.jsonl,meta.json,summary.json}`: unmodified full recorded
  Cargo/compiler output and run records, indexed and hashed in
  `raw-diagnostics-index.json`. All eight have zero warnings/errors.
- `exported-runs/`: four isolated justrust run summaries per mode.
- `source-manifest.json`, `identity.json`: all restored tracked source object
  identities/SHA-256 and frozen sysroot/analyzer/runner identities.
- `runners/`: snapshots of private capture, decode, preserve and validate
  scripts, retained as experimental provenance rather than reusable CLI APIs.
- `validation.json`, `validation.log`: verified exits, identities, source
  restoration, actual rustc flags, selected observations and evidence hashes.

Raw `.mm_profdata` and query-cache files remain at the private paths recorded
in receipts under `/home/jeremy/.jcode/scratch/dead-code-real-retention-20261030/`.
They are not copied into Git. The compiler was provided frozen by the
coordinator, with driver SHA-256
`3527393f3db7e09eeef270a83f51c5aeedb787c77f845ab76a76618f76ae137e`.
No compiler/client source, flag default or old baseline artifact was changed.
The separate native fixture gate is preserved by the coordinator.

The private capture and analysis scripts follow the repository's existing
real-associated-retention protocol and use its primary decoder/analysis APIs.
Compiler query semantics and instrumentation remain the existing owned Rust
sources with their original attribution and licenses.
