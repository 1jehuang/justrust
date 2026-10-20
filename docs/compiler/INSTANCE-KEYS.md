# Canonical instance graph coordinates

Status: shared codec prerequisite. This is not new Rust language support or
permission to execute a decoded target.

The process-local format identifies a resolved callable declaration in its
immutable snapshot and ordered, tagged, length-delimited closed actual types.
Nominals carry their source declaration and ordered actuals, never allocation
IDs or display names. Persistent native symbol encoding remains separate.

Decoding is bounded by the supplied input and existing task budget, checks
current-snapshot declarations, reconstructs nominal metadata through PlanCtx,
replays canonical instance facts, and requires byte-for-byte word re-encoding.
Truncation, invalid frames, wrong arity, foreign declarations and trailing words
are rejected. A caller still needs the appropriate sealed source-use authority.

Closure actuals and opaque tag 16 are explicitly unsupported pending their
owner-metadata integration. Rich trait objects with nonempty principal actuals
or associated assignments are rejected rather than conflated with plain dyn.

Acceptance: codec round trips, nominal alias equivalence, ordered/tagged key
separation, malformed and foreign key rejection, plus graph integrity. The codec
alone does not activate BODY aliases, associated constants or opaque execution.
