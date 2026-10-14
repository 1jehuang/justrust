# Selected compiler launch coverage and query binding

Status: integrated opt-in native producers and strict client reconciliation.
The combined client check, 540 tests, and strict all-target lint passed. The
rebuilt descriptor-only native Cargo/compiler pair passed selected-authority
binding, warm executable identity, missing-evidence, failure, and fail-open
acceptance. A real proc macro observed identical complete argument vectors,
generated output, and executable SHA with capture off and on. The earlier
private-argument transport failed that regression and was replaced before
publication. See `docs/compiler-binding-native-results-20261014.md` for evidence.
No whole-pipeline completeness or speedup is claimed.

## Scope and trust boundaries

`src/execution_graph_compiler.rs` assesses one selected compiler-launch authority.
Its scope is `buildrunner-compiler-launch-coverage`. Every assessment retains
`full_pipeline_complete: false`, `cpu_work_ns: null`, and
`execution_span_ns: null`, including successful, structurally complete captures.
It does not claim command-entry, scheduling, codegen, linking, test execution,
resource, CPU-work, or whole-process-span coverage.

This consumer performs no filesystem operations or compiler invocations.
Its input is bounded typed evidence, plus two caller-validated projections:

* `SchedulerSummary { run_id, authority_root, capture_id, unit_ids,
  structural_complete }`: the exact scheduler receipt's unit set, capture ID,
  complete flag, and its parent owner identity. `authority_root` here is the
  Cargo owner that must equal the compiler root's `parent_authority_root`.
  Unit IDs are scheduler anonymous unit IDs, **not Cargo JobIds** and not a new
  consumer-generated sequence.
* `QuerySummary { schema, capture_id, structural_complete }`: constructed only
  after the existing query consumer strictly assesses the **original bytes**.
  The compiler consumer does not modify or reinterpret final query schema 3.
  Do not deserialize a small substitute object into this projection and mark it
  complete. A standalone schema-1 query receipt cannot satisfy a schema-3 launch.

These are trusted projections, not independently verified raw receipts. The
caller must not elevate unvalidated JSON, duplicate-erasing `serde_json::Value`
objects, missing files, or filename guesses into either projection.

## Descriptor-only launch transport

The native capability marker is exactly
`jr-query-admission-binding: reserved-fd-v2,query-schema=3`. The first supported
path is Linux, an unwrapped native compiler, and Cargo's native default executor.
Stock compilers, wrappers, primary overrides, and custom executors remain
unsupported evidence paths rather than silently satisfying required tickets.

Binding data travels through a bounded sealed memfd at child descriptor 198
and a validated private output-directory descriptor. It is not appended to
compiler arguments or inserted into the compiler environment. Capability
metadata is constant across capture-off and capture-on invocations.

The child mapping atomically reserves its slot without overwriting a foreign
or jobserver descriptor. Directory allocation cannot occupy the ticket slot.
Discovery first validates CLOEXEC duplicates, including seals, bounds, anonymous
IDs, directory ownership, and private mode. Rejected files, pipes, malformed
memfds, and invalid directories retain their original descriptors, flags, and
file offsets. Only the native process-main ownership boundary may consume a
fully validated inherited transport.

Collisions, unavailable descriptors, transport or publication failures, and
missing child evidence do not change ordinary compilation into a diagnostic
failure. They prevent a completeness claim. This is not a claim that arbitrary
proc macros cannot observe all compiler-internal resources or timing, nor a
claim of compatibility with cache wrappers or their early-reply process model.
Standalone user-requested query diagnostic flags are a separate path.

## Common encoding and limits

All listed fields are mandatory. Unknown fields and duplicate JSON fields are
rejected, including duplicates inside unit and launch snapshots. JSON integer
fields must fit their declared unsigned or signed Rust type. Numeric strings,
fractions, negative unsigned values, trailing JSON, and invalid enum values fail.

Every anonymous string ID is 1 through 128 bytes of `[A-Za-z0-9_-]`. No paths,
package names, whitespace, or non-ASCII characters belong in IDs. IDs within each
identity namespace are unique. Invocation, entry-capture, and query-capture IDs
are also disjoint across launches and cannot equal the compiler root ID.

Limits are 64 MiB per manifest, 16 KiB per journal or entry, 100,000 unit records,
100,000 launch records, 100,000 entries or query summaries, and 1,000,000 journals.
Independent expected counters are bounded even when their arrays are empty.
Spawn attempt counters are bounded by the journal limit. Reconciliation uses
ordered maps and per-unit launch indexes rather than a quadratic unit/launch
join. Parsing is directly into typed serde structs, never through `Value`.

Mandatory nullable fields are `Launch.wait_status`, `Journal.unit`,
`Journal.launch`, `Entry.cpu_work_ns`, and `Entry.execution_span_ns`. Missing is
not equivalent to explicit `null`. In particular, `parent_authority_root` and
all launch binding IDs are **non-null strings**, as agreed with the native Cargo
registry owner.

## Root manifest, schema 1

Profile: `buildrunner-compiler-launch-v1`.

```text
Manifest {
  schema: 1,
  profile: "buildrunner-compiler-launch-v1",
  run_id: String,
  authority_root: String,
  parent_authority_root: String,
  scheduler_capture_id: String,
  expected_units: u64,
  expected_launches: u64,
  expected_journals: u64,
  sealed: bool,
  interrupted: bool,
  lost_events: u64,
  outcome: "pending" | "success" | "failure",
  units: [Unit],
  launches: [Launch]
}
```

The independent counters must equal observed record counts. Launch ordinals
cover exactly `0..expected_launches`, without duplicates. The compiler unit set
must equal the exact caller-validated scheduler unit set. The run, scheduler
capture, and parent owner IDs must all match that scheduler context.

The initial registry file is
`compiler-owner-<authority_root>.declaration.json`. It has the same root shape,
all counters zero, empty arrays, `outcome: "pending"`, `sealed: false`,
`interrupted: false`, and `lost_events: 0`.

The final file is `compiler-owner-<authority_root>.manifest.json`, with
`sealed: true` and an explicit `success` or `failure` outcome. An interrupted
`compiler-owner-<authority_root>.interrupted.manifest.json` is diagnostic evidence,
not an acceptable final seal. A missing initial declaration or final manifest
must fail discovery before calling `reconcile`; never synthesize either from
journals or children. Initial and final root identities must match exactly.

An unsealed, interrupted, lossy, pending, or nonterminal authority cannot be
structurally complete. Structural completeness and build success are independent:
a fully observed failed build may have complete coverage and `outcome: "failure"`.
A successful root cannot contain failed, cancelled, or not-started units or
failed compiler launches.

## Unit records

```text
Unit {
  unit_id: u64,
  kind: "rustc" | "rustdoc" | "build-script" | "doc-test",
  disposition: "pending" | "fresh" | "cutoff" | "required" | "unsupported",
  outcome: "pending" | "success" | "failure" | "not_started" | "cancelled",
  terminal: bool
}
```

Declared units start pending/nonterminal. Resolution moves pending to a concrete
disposition. A second resolution may move `required` to `unsupported`, but only
before any launch declaration, for the producer's final capability/executor
check. Kind and unit identity never change. Terminal outcomes are non-pending.
An unresolved terminal unit must be `not_started`. Only `rustc` units may claim
supported `required` coverage in this profile.

Fresh, cutoff, non-required, and not-started units have no compiler child.
A required unit with observed prelaunch failure or cancellation can legitimately
have no launch. A successful required unit must have a launch declaration.
Unsupported units always make this assessment incomplete, even if Cargo
successfully executes them through another path. No unsupported compiler-unit
coverage is claimed.

## Launch records and parent process truth

```text
Launch {
  invocation_id: String,
  launch_ordinal: u64,
  unit_id: u64,
  parent_task_id: "cargo-unit-<unit_id>",
  entry_capture_id: String,
  capture_id: String,
  spawn_attempts: u64,
  spawned: bool,
  terminal: bool,
  outcome: "pending" | "success" | "failure" | "spawn_failed",
  wait_status: null | i32
}
```

Launch declaration precedes any attempt and predetermines all child IDs. It
starts with attempts zero, not spawned, nonterminal, pending, and null wait
status. Declaration belongs to a previously declared active required unit.

Each `spawn-attempt` increments only the attempt counter by one. Each attempt
requires a subsequent `spawn-result` before another attempt. A retryable failure
remains pending, unspawned, nonterminal, and has no wait status. A final spawn
failure is terminal `spawn_failed` with at least one observed attempt and an
observed failed spawn result. It has no compiler child and no wait status.

A successful spawn result sets `spawned: true`; only a subsequent parent
`wait-result` can make it terminal. Terminal spawned launches require a
nonnegative raw Unix `i32` wait status: raw zero means launch success and raw
nonzero means launch failure. This is the **raw wait status**, not an extracted
exit code, query-interface outcome, or guessed process result.

Unit success is incompatible with any failed or spawn-failed child and cannot
precede the observed terminal parent wait/spawn result of a declared launch. A successful
child may accompany a later unit failure, for example parent-side output
processing. A schema-3 query capture's interface completion never supplies or
overrides the parent process outcome.

## Per-event journals

Each journal is
`compiler-owner-<authority_root>.event-<sequence>.json`, with sequences starting
at 1. The exact independently expected journal range is
`1..=expected_journals`. Input ordering does not matter, but duplicate sequences,
foreign roots, gaps, extras, and reused identities cannot satisfy coverage.

```text
Journal {
  schema: 1,
  profile: "buildrunner-compiler-launch-v1",
  authority_root: String,
  sequence: u64,
  event: "unit-declared" | "unit-resolution" | "unit-terminal"
       | "launch-declared" | "spawn-attempt" | "spawn-result" | "wait-result",
  unit: null | Unit,
  launch: null | Launch
}
```

Unit events have a full post-event unit snapshot and `launch: null`. Launch
events have a full post-event launch snapshot and `unit: null`. Journal replay
checks declarations, immutable identity, legal lifecycle transitions, independent
launch ordinal allocation, exact attempt-counter increments, and result/wait
ordering. The replayed final unit and launch maps must equal the final root and
its independent declaration counters. A journal gap is incomplete evidence,
not permission to infer an omitted attempt, launch, or terminal transition from
later snapshots.

## Independent compiler entry

```text
Entry {
  schema: 1,
  producer: "rustc-query-admission-entry",
  scope: "compiler-entry-to-query-capture-binding",
  capture_id: entry_capture_id,
  run_id: String,
  invocation_id: String,
  parent_invocation_id: compiler_authority_root,
  parent_task_id: "cargo-unit-<unit_id>",
  expected_query_captures: 1,
  query_capture_ids: [capture_id],
  query_receipt_schema: 3,
  full_pipeline_complete: false,
  cpu_work_ns: null,
  execution_span_ns: null
}
```

Every spawned launch requires **both** this independently produced entry and
the final caller-validated schema-3 query receipt, including failed compiler
processes. The entry's run, invocation, parent invocation, task, entry-capture,
and predetermined query-capture IDs must all echo the root/launch binding.
Missing entry, missing final query, an incomplete query capture, schema-1/2
substitution, duplicates, and foreign/unbound children cannot count. An entry
alone does not prove a final query was emitted. A query alone does not prove the
compiler independently admitted the binding. Query counts require an exactly
bound entry as well as the exact final query ID.

## Public API and assessment

```rust,ignore
pub fn parse_manifest(bytes: &[u8]) -> anyhow::Result<Manifest>;
pub fn parse_journal(bytes: &[u8]) -> anyhow::Result<Journal>;
pub fn parse_entry(bytes: &[u8]) -> anyhow::Result<Entry>;
pub fn reconcile(
    manifest: &Manifest,
    initial: &Manifest,
    journals: &[Journal],
    scheduler: &SchedulerSummary,
    entries: &[Entry],
    queries: &[QuerySummary],
) -> anyhow::Result<CoverageAssessment>;
```

The assessment exposes scope, authority root, run ID, expected/observed unit,
launch and journal counts, bound entry/query counts, issues,
`structural_complete`, explicit root outcome, and the always-false/null full
pipeline/timing claims. Contradictory identities, malformed wire, impossible
lifecycles, and replay/final-root contradictions return errors. Missing evidence
or unsupported scope yields an incomplete assessment. A caller must treat
**either** an error **or** `structural_complete: false` as not covered.

Directory-level authority discovery, excluded roots, missing initial/final file
handling, CLI output, and raw scheduler/query validation belong to main's
integration. The pure API cannot prove file presence or distinguish a fabricated
in-memory initial declaration from one loaded from disk. Discovery must retain
independent owner expectations so deleting an entire compiler root and all its
children cannot become an empty successful selection.

## Validation status

The module includes 28 synthetic regression tests for success, missing entry/query/
journal/unit/context mapping, required-launch deletion, independent counters,
schema-1/2 substitution, unsupported/fresh/cutoff/non-started/prelaunch failure,
spawn failure and retries, failed query capture, failed compiler with complete
query, successful child with parent failure, impossible waits/outcomes,
duplicate identities/JSON fields, unknown/missing mandatory fields, bounds,
unsealed/interrupted/lossy roots, and binding mismatches.

Per task constraints, no builds or test executions were performed by this
worker. Formatting and source inspection are not acceptance validation. Main
must wire the isolated module and run its tests only after producer/shared
schema ownership is frozen, then verify a real native Cargo/compiler workload.
