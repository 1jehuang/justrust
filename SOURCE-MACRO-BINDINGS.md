# Source-owned macro binding repair

Status: implemented and acceptance-validated for the bounded original-source
macro client. Repository-wide lint and broad macro-unit baseline failures below
remain separate blockers. This is not full Rust or macro import/export support.

## Frozen contract

The same source compiled by stock Rust and the native compiler must select the
same admitted unqualified original-source macro. User textual macros may shadow
builtins in user source, including `panic`, without changing library bodies.
Inline modules and external module files inherit the enclosing crate's exact
textual frontier. Source-order redefinition, block shadow/restore and selected
rule commitment remain unchanged. Imported functions keep the macro expansion
of their defining source. Mounted dependency and native prelude source are
separate crate environments, even though their items use one module arena.

No builtin-name exception, removal of user source, printed expansion or fallback
to rustc is involved. User module names or lexical filesystem paths are not
crate authority. Arbitrary same-named macros use the same routing policy.

## Representation and source authority

Source assembly records an immutable crate-root file identity for every input
file using the requested graph ordinal and attachment ancestry. A dependency or
prelude attachment starts a new owner. Descendant module files inherit that
owner. Synthetic dependency delimiters anchor to that dependency's source,
not to the user file. Raw single-crate source maps retain their one-crate model.

The prepared lexical snapshot copies and validates the complete ownership table
under its existing retained byte lease. Partial, forward and non-root mappings
reject rather than falling back to user ownership. Snapshot-qualified file
handles prevent foreign-session reuse. DefinitionSite now names the owning
crate root, not whichever module file happens to contain the macro declaration.

`source_macro_binding` queries only checked source/expansion handles. Generated
macro-name lookup follows the actual expansion call chain rather than the
transcriber token's original file. Module macro probes, item selection and
function-local imports share one source-domain plus textual-frontier lookup.
Selection freezes the definition and owner. Expansion rejects a selected handle
transplanted to a different source crate. Local definitions validate their
DefinitionSite owner before rule compilation.

The change adds no IR or CFG instruction, graph node kind, whole-program read,
mutable published carrier, lock or budget. Ownership is immutable metadata of
the existing per-file SOURCE facts and prepared snapshot. Existing parsing debt
is not expanded or reclassified as a new graph milestone.

## Acceptance gates

These are slice obligations, not completion of the full program gates:

| Program gate | This repair's obligation |
|---|---|
| G0 | Frozen admitted and unsupported contract above |
| G1 | Original-file provenance and definition-crate identity |
| G2-G4 | Existing typing, ownership and trait checks remain mandatory |
| G5 | Same-source lexical selection and independent crate isolation |
| G6 | BUILD ELF execution with exact observable result |
| G7 | Prelude source cannot inherit user macros |
| G8 | File modules and mounted dependencies, no macro import claim |
| G9 | Immutable source metadata and unchanged integrity ratchets |
| G10 | Stock differential, invalid and private forged-owner controls |
| G11 | No performance claim or representative final audit |
| G12 | CHECK does not demand object or optimizer output |

- `tests/source_macro_bindings.rs`: stock-first exact original source, workers
  1/8 CHECK with no object/codegen and BUILD with ELF execution, exact stdout,
  stderr and status. Cases cover the reported panic witness, other builtin
  names, prelude imports, inline/file modules, inherited/child/local shadowing,
  nested calls, function imports and separate mounted dependency sources.
- `tests/source_macro_bindings_authority.rs`: stock/native rejection of
  before-definition use, scope escape, selected-rule mismatch, false-cfg
  definitions, unused unresolved/type-invalid expansions and dependency lookup
  of a user-private macro.
- Private module tests: wrong definition crate, non-root module-file crate,
  foreign snapshot handle, malformed owner tables and generated call-site
  versus definition-source routing. Existing driver tests retain commitment
  and foreign selected-handle controls.
- `tests/source_macro_bindings_graph.rs`: identical per-file source keys for
  shadowed and plain controls, single evaluations, independent dependency
  source key and CHECK without object/build/optimizer work.
- Unweakened `tests/graph_integrity.rs`, focused prior macro regressions,
  check --tests, formatting and strict focused lint remain required.

## Validation receipt (2026-10-19)

- Hosted bounded `justrust check --remote -j1 --tests` passed at the owned
  export's initial exact parent, run `20261019-095541913-2730831`.
- The first focused hosted run, `20261019-095731931-2747948`, passed 20 tests
  and exposed the legacy `println` spelling bypass. It was repaired through
  the same checked builtin routing, not a fixture change or name exception.
- The repaired differential, authority, graph and unchanged graph-integrity
  suites passed all 21 tests in `20261019-100540363-2861159`. Hosted storage
  returned ENOSPC, so this receipt is an observed local fallback, not a hosted
  success or performance measurement. Workers 1 and 8 both CHECK and BUILD/run.
- An immutable copy of the explicitly built native CLI, SHA-256
  `003c53e84a2a0785d565b39631aaa7bb288180e731e093a8dae1acf67fe88268`,
  separately accepted the exact reported panic witness at workers 1 and 8:
  CHECK had zero executable bytes, BUILD produced an 8391-byte ELF, and both
  programs exited zero with empty output. PATH and RUSTC were nonexistent.
- Five focused private owner/provenance and selected-handle tests passed.
  The wider `--lib macro` filter passed 50 tests but failed the independent
  existing `$x:expr` unsupported expectation in `macro_rules.rs`, receipt
  `20261019-100210698-2814935`. That assertion was not weakened in this repair.
- Strict affected-target lint was attempted, receipt
  `20261019-101440549-2949337`, and failed on 27 existing diagnostics outside
  this repair, including `project/cli.rs` unused mut, formatting lowering,
  iterator and literal inference helpers. No lint allowance was added.
- At exact parent `919ecc03`, the refreshed owned patch passed
  `justrust check --local -j1 --tests` with 258 metadata units, receipt
  `20261019-102046623-3026423`. The expanded authority suite passed both tests,
  including additional shadowed `panic` and `println` rule-mismatch controls,
  receipt `20261019-102242724-3046465`. Neither may escape to a builtin.
- After the nonoverlapping break-prefix parent advance, the exact
  `a9351abba` parent plus frozen owned patch passed affected metadata-only
  `check --local -j1 --lib --test source_macro_bindings_authority`, receipt
  `20261019-102530689-3092063`. No whole graph/link replay was substituted.
- Path-redacted build receipts are retained under `data/runs/macro-bindings/`.
  Final exact-parent metadata and added invalid-control checks are recorded in
  the commit validation report. No full repository or full Rust claim follows.

Macro imports/exports, `$crate`, generated definitions, cross-crate exported
macro expansion and general stacked hygiene remain open under
MACRO-EXPANSION.md. This repair does not turn those unsupported constructs into
approximate translations and makes no speed claim.
