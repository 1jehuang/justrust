# Required-producer reconciliation design

Status: design only. No manifest producer, reconciliation implementation, or
full-pipeline guarantee is shipped by this document.

## Why receipt validation is insufficient

`justrust graph` validates receipts it finds. A valid receipt does not prove that
a different invocation, subprocess, or execution boundary was captured. A
capture directory is not an inventory of expected work. Missing producers must
be identified from a separate, positively established parent lifecycle.

## Proposed next gate

Introduce a versioned run manifest separate from schema-1 scheduler receipts.
Do not infer this manifest by enumerating files after the build. The native owner
of an invocation declares the expected child before scheduling or spawning it.
The run owner declares its own supported scope and explicitly lists opaque
boundaries. Declarations and receipts are observational, never cache inputs.

Required identities are run ID, producer invocation ID, parent invocation ID,
and parent task ID where applicable. Process IDs alone are not identities.
Repeated scheduler invocations have distinct identities. Existing capture IDs
are receipt identities, not proof of a parent-child relationship. Schema-1
receipts cannot satisfy a new expected ticket without a versioned envelope or
schema-2 identity extension. The manifest names an authority root and a versioned
coverage profile. Policy-required producer slots cannot be reclassified as
opaque to avoid a missing-receipt failure.

The manifest records declarations, observed starts, terminal outcomes, and a
sealed expectation set. A child has one of: accounted receipt, explicitly
not-started, explicitly opaque, or missing. Opaque is not complete. A child that
started and was killed without a receipt remains missing, even if the parent
reports a known exit status. A failed child can be structurally accounted if its
receipt is present and valid. Failure does not imply complete instrumentation.

Reconciliation rejects duplicate invocation IDs, foreign run IDs, orphan
receipts, unexpected producers, conflicting parents, cycles, unsupported scope
versions, unsealed manifests, lost events, and missing expected receipts.
Directory filename order and counts cannot substitute for these checks. Keep
existing per-receipt assessments and add a separate run coverage assessment.
Never promote per-receipt structural completeness to run completeness.

## Scope and unsupported execution

The first implemented manifest may require only the Cargo scheduler producer.
It must call its result `cargo-scheduler-coverage`, not full pipeline coverage.
Compiler queries, build scripts, proc macros, native tools, linking, tests,
fingerprint work, waits, client cache work, and remote service boundaries remain
opaque until each has a native identity and tested lifecycle producer.

Fresh units have no expected compiler invocation unless an actual invocation is
created. Cache restoration is a distinct operation, not fabricated compiler
work. A scheduler declaration must not imply a rustc subprocess necessarily ran.
Hosted and stock Cargo paths without the protocol are unsupported or unknown,
not silently accepted. Cross-machine clocks must not be compared directly.
Explicit causal handoffs are required for later cross-process span analysis.

## Correctness constraints

Build results, diagnostics, exit status, jobserver behavior and pipelining must
remain unchanged. Reserved sink and identity environment variables must be
removed from user subprocesses and excluded from fingerprint comparisons, as
with the existing scheduler sink. Compiler-owned instrumentation will need an
explicit native protocol, not accidental build-script inheritance.

Publication uses bounded private files and no-replace atomic finalization.
Missing or unwritable transport fails open for the build and closed for coverage
claims. If the run owner itself is killed, no sealed complete run can be inferred.
A successful build without a final manifest is unknown coverage.

## Inspected native integration points

`BuildRunner::compile` in
`toolchain/cargo/src/compiler/build_runner/mod.rs` creates the queue before LTO,
preparation, collision checking and recursive unit scheduling, then consumes it
with `queue.execute`. The run-owner guard must begin before these fallible
operations, not only at `execute`. It must survive consumption of the queue and
account for errors during post-execution result collection. Otherwise a valid
scheduler receipt can coexist with an unaccounted failed owner lifecycle.

The existing `JobQueue::new` capture identity must be bound to the owner's
expectation before any enqueue work. This calls for a shared native context,
not directory discovery. At the next larger boundary, an outer Cargo invocation
owner is needed to attest how many `BuildRunner::compile` calls were expected.
A BuildRunner manifest alone cannot prove coverage of command setup,
resolution, or invocations that never reached BuildRunner.

For the conservative first slice, declare and persist a scheduler ticket before
queue construction. Bind it into the child's capture, require its receipt even
when preparation fails, record execute-enter before `queue.execute`, and observe
its result before propagating the existing error. A parent cannot invent a
not-started outcome after execute-enter. A pre-execute dropped capture stays
unsealed/incomplete. An independent bounded declaration journal must survive
child-writer failure, while absence of a final owner seal still means unknown or
incomplete. This first profile covers BuildRunner scheduler entries only. A
separate launch-owner expectation in `src/record.rs` plus native invocation
registration is required to extend coverage to the whole build command.

Independent read-only review confirmed these boundaries. Neither this design
review nor the journal proposal establishes a shipped runtime guarantee.

## Acceptance matrix before implementation promotion

- Two valid expected invocations reconcile only when both receipts exist.
- Removing either receipt makes run coverage incomplete.
- A valid extra or foreign receipt cannot repair a missing expected receipt.
- Duplicate identities and conflicting parent relationships are rejected.
- Not-started children require explicit parent accounting and no start event.
- Failure, cancellation, abrupt kill and owner interruption stay distinguishable.
- Capture on/off preserves fresh reuse, output hashes, diagnostics and exit codes.
- Unwritable sinks preserve results and never produce a complete assessment.
- Stock and hosted paths without native declarations report unknown coverage.
- Limits and sequence loss prevent completeness rather than truncate silently.

Native lifecycle integration points must be reviewed before selecting a wire
schema. The independent manifest consumer can be developed first, but fixture
validation alone is not end-to-end acceptance. Full-pipeline coverage and a
thread ceiling remain unimplemented.
