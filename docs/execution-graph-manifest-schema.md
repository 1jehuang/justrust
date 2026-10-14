# BuildRunner scheduler manifest protocol v1

Status: independent consumer and synthetic fixtures. Native producer and real
acceptance must be tested separately. This is not whole-run coverage.

## Scope and authority

`profile = "buildrunner-scheduler-entry-v1"` attests only supplied native
`BuildRunner::compile` entries. Each owner has exactly one policy-required
scheduler ticket declared before queue construction. It does not attest that all
BuildRunner entries of an outer Cargo invocation were inventoried. In the initial
single-owner transport `run_id = authority_root = owner.invocation_id`.

Output is `coverage = "buildrunner-scheduler-entry-coverage"`, with
`scheduler_entry_complete` and **always** `full_pipeline_complete = false`.
Outer Cargo launch, setup/resolution, compiler queries, build scripts, proc
macros, native tools, linking, tests, fingerprints, waits, client cache, and
remote-service boundaries remain opaque. No compiler invocation is inferred
from a scheduler unit, including a fresh unit or a restored artifact.

## Minimal wire objects

All objects reject unknown JSON fields. Enums use the exact snake-case strings
below. JSON duplicate keys are rejected by serde struct deserialization. All
non-optional fields are required. Optional identities serialize as null when
absent (serde also accepts an omitted optional field). No path is an identity.

```json
{
  "schema": 1,
  "profile": "buildrunner-scheduler-entry-v1",
  "run_id": "owner-1",
  "authority_root": "owner-1",
  "owners": [{
    "invocation_id": "owner-1",
    "parent_invocation_id": null,
    "parent_task_id": null,
    "sealed": true,
    "interrupted": false,
    "lost_events": 0,
    "outcome": "success",
    "tickets": [{
      "invocation_id": "scheduler-1",
      "parent_task_id": "scheduler-0",
      "policy": "required",
      "lifecycle": "succeeded",
      "execute_entered": true,
      "capture_id": "capture-1"
    }]
  }]
}
```

Owner `outcome`: `pending`, `success`, `failed`, `cancelled`, `interrupted`.
`pending` is valid only for an unsealed owner with `interrupted = false` and
always yields incomplete coverage. Declaration and execute snapshots use this
state until the owner observes its final outcome. A sealed `pending` owner is
rejected. Actual interrupted guard drops use `interrupted = true` and
`outcome = interrupted`, never a synthetic success.
Ticket `policy`: `required`, `opaque`, but this profile rejects `opaque`.
Ticket `lifecycle`: `declared`, `not_started`, `execute_entered`, `succeeded`,
`failed`, `cancelled`, `opaque`. The required boolean `execute_entered` retains
observed start evidence: false for `declared`/`not_started`, true for
`execute_entered`/`succeeded`/`failed`/`cancelled`. Sealed owners permit terminal
execution outcomes or explicitly `not_started`. `opaque` is rejected rather
than waiving a required slot. An unsealed declaration may use `declared`.
A pre-execute failure uses `not_started`, `execute_entered = false`, owner
`outcome = failed`, with its dropped scheduler receipt, which must remain
structurally incomplete. `not_started` never waives this profile's required
receipt. A structurally complete receipt without execute-enter is rejected.
A success owner must have a succeeded scheduler ticket. A failed owner may have
a succeeded ticket if later result collection failed. Interrupted owners are
incomplete even with a valid receipt. Nonzero `lost_events` is incomplete.

Schema-1 scheduler receipts have no authoritative parent identities. They
require this explicit versioned `ReceiptEnvelope`, never a filename guess:

```json
{
  "schema": 1,
  "run_id": "owner-1",
  "invocation_id": "scheduler-1",
  "parent_invocation_id": "owner-1",
  "parent_task_id": "scheduler-0",
  "capture_id": "capture-1",
  "receipt_schema": 1
}
```

After independently validating the complete existing scheduler receipt, the
caller supplies `ReceiptSummary` (not a wire replacement for the receipt):

```json
{
  "schema": 1,
  "capture_id": "capture-1",
  "structural_complete": true,
  "full_pipeline_complete": false
}
```

## Public Rust API

`parse_manifest(&[u8]) -> anyhow::Result<Manifest>` and
`parse_envelope(&[u8]) -> anyhow::Result<ReceiptEnvelope>` parse bounded JSON.
`reconcile(&Manifest, &[ReceiptEnvelope], &[ReceiptSummary]) ->
anyhow::Result<CoverageAssessment>` is pure and has no filesystem access.
All wire fields above are public struct fields with corresponding Rust names.
`OwnerOutcome`, `TicketPolicy`, and `TicketLifecycle` are public enums.

`CoverageAssessment` public fields: `schema: u32`, `coverage: String`,
`profile: String`, `run_id: String`, `authority_root: String`,
`excluded_authority_roots: Vec<String>`, `outcome_agreement_verified: bool`,
`scheduler_entry_complete: bool`,
`full_pipeline_complete: bool`, `expected_receipts: usize`,
`accounted_receipts: usize`, `issues: Vec<String>`,
`opaque_boundaries: Vec<String>`.

Malformed identities, duplicates, foreign or orphan envelopes/receipts,
conflicting parents, owner cycles, unsupported schema/profile, invalid sealed
lifecycles, and required-to-opaque bypass return `Err`. Supported unsealed,
interrupted, lost-event, missing-receipt, or invalid-receipt inventories yield
an explicit incomplete assessment. An `Err` must never be treated as complete.
A valid extra receipt cannot repair a missing ticket. Every expected ticket is
checked independently. Successful receipt validation does not seal its owner.
Failed execution can be structurally accounted with a valid receipt.

The selected `authority_root` is copied explicitly into the assessment. This
is coverage of that selected authority and its declared descendants only, never
of a directory or an entire run. `excluded_authority_roots` starts empty in pure
reconciliation. The IO caller enumerates independent roots excluded from its
selected assessment and fills this list, without merging their owner snapshots.
The CLI additionally validates selected-owner declaration and execute journals
against the final snapshot. Pure reconciliation of supplied snapshots alone is
not proof those native journals existed or that the expected set was persisted
before scheduling. A missing journal must prevent the CLI's completeness claim.

`outcome_agreement_verified` is always false. Structural completeness does not
prove that the child's semantic success/failure agrees with the owner's stated
outcome. Schema-1 summaries cannot independently check that agreement. An owner
claim of success is checked only for snapshot lifecycle consistency, not against
a semantic child outcome. No success-agreement claim is made.

## Bounds and transport

`MAX_BYTES = 1,048,576` per parsed JSON object,
`MAX_ENTRIES = 256` owners, envelopes, and summaries, exactly one ticket per
owner, and `MAX_ID_BYTES = 128` per identity. IDs must be nonempty ASCII
alphanumeric or `.`, `_`, `:`, `-`. Profile/schema strings are matched exactly.
Reconciliation enforces semantic/count/identity bounds even for structs created
without the parser. Parser byte bounds cap allocation before deserialization.
No records are silently truncated.

Native transport uses independent no-replace publications:
`owner-<id>.declaration.json` before queue construction,
`owner-<id>.manifest.json` for final sealed normal return,
`owner-<id>.interrupted.manifest.json` for unsealed guard drop, and
`<capture_id>.envelope.json` beside unchanged `<capture_id>.json` receipts.
These names select transport objects only, never identity. The caller selects
the final snapshot, rather than treating declaration and seal as separate
owners or merging their ticket sets. An absent final seal is unknown/incomplete,
not evidence that the declaration was superseded successfully. Directory file
counts and lexical order establish nothing. Build recording remains fail-open
for compilation and fail-closed for coverage claims.

The module's fixtures are synthetic until native producer integration and real
capture-on/off, preparation-failure, interruption, and missing-receipt acceptance
checks pass. They establish neither end-to-end acceptance nor a speedup.
