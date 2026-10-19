# Source trait prerequisite and receiver validation

These five rows are unchanged `justrust export` path-redacted receipts,
selected from a scratch export so unrelated machine history is not committed.
They validate implementation through `c470f5f94`, including the fail-closed
nominal normalization seam in `952c22c20` and ordinary projected receiver
support in `1d3f991ce`.

| Run | Validation | Result |
| --- | --- | --- |
| `20261018-201336523-2200439` | `justrust test --remote --test prelude_traits --test graph_integrity` | 13 passed |
| `20261018-201343322-2204155` | `justrust test --remote --lib nominal_normalization` | 3 passed |
| `20261018-201351334-2199610` | `justrust clippy --remote --test prelude_traits -- -D warnings` | passed, no warnings |
| `20261018-201524606-2282115` | `justrust test --remote --lib` | 1366 passed |
| `20261018-201528818-2284888` | Composed source-library targets below | 56 passed |

The composed command selects `prelude`, `prelude_methods`, `prelude_traits`,
`canonical_copy_graph`, `core_identity_graph`, `source_configuration_graph`,
`generics_graph`, `modules`, and `tuples`, without test-name filtering.
All five runs completed on hosted. An earlier plain-command run used the
stable client's recent-remote-failure local fallback. Explicit `--remote`
retries succeeded and provide the receipts here.

## What is established

- Exact ordinary source trait fixtures are stock-compiled before their native
  acceptance or explicit unsupported boundary is checked, in check/build
  modes with one and eight workers.
- Shared and mutable projection-free borrowed receivers with associated value
  results execute with stock parity, including mutable reborrows. Full source
  schema, universal body/admission, library SOURCE, and per-object proof
  ordering assertions remain in their acceptance harness.
- Stock-invalid mutable aliasing (`E0499`) and returning a local loan (`E0515`)
  after projected receiver calls produce genuine native validation failures,
  not unsupported-syntax substitutes, and never publish a link result.
- The canonical reapplication validator is tested against real source-created
  applications with changed source declaration, nominal kind, actuals, foreign
  snapshots, forged source ownership, and rewritten payloads retaining an
  application ID. This does not demonstrate a working remint adapter.
- The world normalization seam defaults to Unsupported until an authoritative
  source-schema remint adapter is supplied. It never rewrites nominal fields
  or reinterprets an existing application ID with changed actuals.

## What is not established

No global source `Iterator`, `IntoIterator`, `Deref`, or `DerefMut` declaration
has been attached. Exact `Option<Self::Item>` still encounters the native raw
projection boundary pending the coordinated immutable nominal authority and
AST-authorized source application adapter. Exact `?Sized` and source bounds
with associated equalities on associated declarations also remain unsupported.
Reference-bearing projected results remain outside the admitted receiver-only
scope. These receipts make no full Iterator, full-suite, full Rust/std, or
compile-time speed claim. No graph ratchet was loosened for this work.
