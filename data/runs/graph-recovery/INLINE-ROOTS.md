# Inline owned-root recovery checkpoint

Recovered from the preserved `borrow.rs` and `ownership.rs` WIP on
`e17aa8213`, with all nine existing stashes retained. The pre-change WIP and
stash inventory also remain in the external recovery directory. Main was not
edited or built. No install or push was performed.

`ResolvedTargets::Inline` holds precisely one owned local root with empty
projection and authority vectors. The fast path applies only to an existing
local with no projection. It spends the same one unit of traversal work as
`resolve_cells`, but does not allocate or admit nonexistent heap storage.
Projected, indirect and raw-dereference resolution continues through the
existing resolver. Ownership access, initialization and active-loan checks
still consume the resolved target normally.

## Exact validation on the recovered composed tree

All following Rust commands used the installed stable justrust client with
explicit `--remote`. The client reported hosted execution, not cooldown local
fallback. Host-side compilation-unit labels saying `local` are not local-machine
execution; `justrust show 20261018-210116813-3366477` confirms
`where remote on hosted`.

- `20261018-210116813-3366477`: check passed.
- `20261018-210132998-3377317`: `--lib inline_`, seven tests passed. Includes
  exact full-resolver equivalence, 1,024 root resolutions under zero byte
  admission, work exhaustion, projected provenance rejection, raw pointer
  behavior, and move/uninitialized/live-loan rejection.
- `20261018-210144300-3386459`: graph integrity, six passed and one failed.
  The unchanged 1,165-function gate exhausts the 32,000,000 work limit in
  `SchemaAdapter::declaration_info` at frontend.rs:17362, requested one unit,
  used 32,000,000. Failing resource node is kind 109 (`ANALYZE`), params
  `[1166]`, task 1178. The diagnostic reports associated normalization's
  poisoned ledger. This is a source resource failure, not a hosted worker
  failure. The inline change is not claimed to recover the composed gate.
- `20261018-210224187-3418395`: three immutable retained-receipt tests passed.
- `20261018-210225001-3418988`: compact runtime lease layout test passed,
  requiring 24 bytes on x86_64.
- `20261018-210225908-3419663`: exact ownership proof publication accounting
  and overflow test passed.
- `20261018-210316354-3435597`: strict all-target clippy passed.
- `cargo fmt --check` and `git diff --check` passed.

No budget, graph gate or debt baseline was increased. This is an independently
preserved allocation-representation checkpoint, not full graph acceptance,
complete immutable-budget migration, or a speed claim. The instance receipt
migration stays separate in the preserved stash until its own composition and
acceptance can be established.
