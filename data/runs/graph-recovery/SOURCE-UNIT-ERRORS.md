# Source-unit error publication: frozen private evidence

## Checkpoints and scope

- Base: `b85a0b6dd222dd39ec2bd8cd4ddcff29d362320e`.
- Frozen source: `2f626390a816b22d688c6cff4ffb51aafb86dbd0`.
- Private worktree: `cr-source-unit`; no main import, push, install, local/full
  suite, cleanup, agent spawning or existing stash mutation.
- Source changes are limited to `src/compiler/source.rs`, minimal existing
  `SOURCE`/`PARSE` adapters in `src/compiler/mod.rs`, and dedicated tests in
  `src/compiler/source_unit_tests.rs`. The other source-commit files are the
  feature contract, implementation/integrity documentation and precisely two
  observed reachable-fact debt deletions.
- General `SourceError`, `ByteLease`, expansion/invocation budgets, frontend,
  IR and CFG implementations are unchanged. No budget or ceiling is raised.

Final file SHA-256:

| File | Hash |
|---|---|
| `src/compiler/source.rs` | `a29eaa9af969be5b693f28d80373adedd2fd2dfd4853592e8d3917271827cb99` |
| `src/compiler/mod.rs` | `30c1fc9e5e4fae4de74dfdfa938cc55448bc15acb40252cf405e5ad13aa4f676` |
| `src/compiler/source_unit_tests.rs` | `3bc52ac0fe1bd468d7f51f385bf2a0d64d5a1ffa0fda74c31d9b19f03e9ec052` |

The initial six-test and unchanged-scanner runs used source.rs hash
`7359ccca57ea999fc5dc66973a1a5f105fe0350ae5114a1a52fc974b4d17ae9c`.
After those runs, only its `cfg(test)` independent-file assembly helper changed:
it now stores published `UnitResult` rather than `PendingUnitResult`, calls
`publish_unit` around root/child preparation, uses `prepare_published_child_unit`,
and consumes facts with `provide_published`. Production implementation, mod.rs
and dedicated test hashes did not change. All final green acceptance commands
below exercised the final source hash.

## Commands, exact selectors and placement

All Rust invocations below used the installed stable client with
`JUSTRUST_REMOTE=force`. Both saved metadata (`place: hosted`) and the client
terminal report identify hosted execution. No fallback was used. Compiler unit
`local` in a slim export means a workspace crate, not local execution placement.

| Run ID | Command after `justrust` | Observed result |
|---|---|---|
| `20261019-010439305-106242` | `check` | pass, initial production implementation |
| `20261019-010547002-111650` | `test --lib compiler::source::publication_tests` | 6/6 pass |
| `20261019-010607632-111472` | `test --test graph_integrity` | expected red: 13/14, only exact two-entry shrink; existing 1208 program gate passed |
| `20261019-010817482-122265` | `test --lib compiler::source::` | 95/95 pass, includes six new tests and published independent-file equivalence |
| `20261019-010823306-122606` | `test --lib expanded_adapter::tests` | 20/20 pass, includes actual-capacity final diagnostic transfer/refund and exhaustion cleanup |
| `20261019-010823725-122650` | `test --test source_error_order --test source_configuration_graph --test prelude` | 19/19 pass in three binaries; source ordering exercises workers 1/8 and CHECK/BUILD |
| `20261019-010826888-122754` | `test --test graph_integrity` | 14/14 pass, including unchanged positive 1208 program gate |
| `20261019-010833347-122150` | `clippy --all-targets -- -D warnings` | pass, warning-free |
| `20261019-010958542-129367` | `check` | pass on frozen source |

`cargo fmt --check` and `git diff --check` passed before the source commit.
The first attempted command `justrust --remote check` failed argument parsing
(exit 2: unexpected argument `--remote`), before any Rust build. It produced no
run receipt. It was corrected to the stable client's supported environment
interface, not worked around with local execution.

`source-unit-errors.jsonl` contains exactly these nine owned records selected
without modification from an official `justrust export --out <scratch-export>
--machine graph-recovery` output. `source-unit-errors-placement.jsonl` records
only ID, exact argument selectors, subcommand, hosted placement, session,
toolchain and CPU count from each corresponding run metadata file. No unrelated
exported runs, private source paths or account credentials were copied.

## Original unchanged-scanner red

Before any ratchet adjustment, the failing test reported:

```text
reachable-fact mutable state shrank. Lock in the progress with
`GRAPH_INTEGRITY_SHRINK=1 justrust test --test graph_integrity`
and commit tests/graph_integrity/fact_reachable_state.debt:
  UnitResult ExpansionBudget Mutex
  UnitResult InvocationBudget Mutex
```

All other 13 graph checks passed. Coordinator approved recording only those
observed deletions. The gate implementation itself is unchanged. `UnitResult`
is still read as an explicitly typed graph fact in the existing producer and
assembler paths, so this is not a disappeared-root coverage workaround. The
inventory shrinks from 62 to 60 entries, with all six broader mutable carriers
still present elsewhere. The existing same-name `State` scanner approximation
is not exploited or rewritten.

## Ownership and negative controls

The dedicated tests check all eight diagnostic kinds, primary spans, Unicode
message/path/location and exact message/path allocation identity across the
unleased consuming conversion. Retained and temporary diagnostic leases are
rejected without a counter, retention, pointer or message change, then refund
only when the returned original error is dropped. Both terminal and outer
failure positions reject a lease. Anchored flags and unique/shared assembly
produce matching diagnostics; terminal provenance matches direct text loading.

The existing generated diagnostic test owns actual allocated capacity through
its final SourceError transfer and observes release after its final drop. Its
exhaustion counterpart observes partial-message cleanup. They remain runtime
refundable errors, not published permanent receipts. No general diagnostic
retention semantics were changed. Source diagnostic allocation metering is not
claimed repaired, and the conversion is not semantic admission evidence.

No stock oracle, full Rust support, speedup, RSS reduction or full debt-removal
claim is made. Independent review and coordinator integration remain pending.
