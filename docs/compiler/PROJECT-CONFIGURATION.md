# Project configuration admission

Status: bounded native Cargo-manifest configuration support, not full Cargo or
multi-edition Rust compatibility. This slice addresses the edition and ignored
profile findings in `RUST-SUPPORT-AUDIT.md`. Linux x86-64 is the native target.

## Frozen contract

* Every package admitted to native compilation must use edition 2024, directly
  or through `workspace.package.edition`. Unknown editions (for example 2099)
  are invalid in every loaded manifest, including inactive dependencies.
  Earlier valid editions 2015, 2018 and 2021 are explicitly unsupported when
  selected for compilation, including Cargo's implicit 2015 default when the
  edition is absent. They are never silently reinterpreted as 2024. Inactive
  dependency metadata may retain a valid older edition without compiling it.
* Built-in `dev`, `release`, `test` and `bench` profiles are resolved explicitly.
  Test inherits dev, bench inherits release, and custom profiles require an
  explicit `inherits` chain ending in dev or release. Undefined parents and
  inheritance cycles reject. Custom inheritance from test/bench is unsupported.
* `debug-assertions` controls the compilation cfg. `overflow-checks` is passed
  separately through `SourceOptions` to native code generation, not inferred
  from cfg or optimization. Dev/test default both settings on, release/bench
  default both off. Explicit values override inherited values independently.
* `test` selects the test profile unless a profile or release was requested.
  Dev/test artifacts use the debug directory, bench uses release, and custom
  profile artifact directories use their validated profile names.
* Explicit `opt-level = 0` is admitted. Other explicit optimization levels are
  unsupported until optimizer selection is implemented. Implicit release still
  uses native unoptimized code generation, and its status line says so. Neither
  optimization nor debug information is falsely advertised.
* All other profile keys, including panic strategy, debug information, package
  overrides and build overrides, reject explicitly rather than being ignored.
  Non-table profiles, invalid/reserved names, bad value types and conflicting
  `--release`/`--profile` selection also reject before compilation.

Profile validation is conservative across all root profile definitions, even
when a particular profile is not selected. Dependency/member profile tables
remain non-authoritative, following Cargo's workspace-root profile ownership.

## Compiler/graph boundary

Direct source compilation accepts `--edition YEAR` and `--edition=YEAR`, once,
with 2024 as its default. `SourceOptions::edition` carries a typed edition input
and rejects earlier editions before source IO or parsing, including in-memory
source preparation. Project and direct-source admission share the same supported
edition check. This makes the edition explicit; it does not add 2021 semantics.
The Adler2 probe passes its pinned 2021 edition to direct-source diagnostics as
well as retaining that edition in the unchanged project manifest.

Manifest resolution produces immutable profile values. The driver passes the
resolved cfg and overflow policy into `SourceOptions`; no mutable compiler
side slot or graph bypass is introduced. The overflow backend implementation
and its arithmetic/authority coverage belong to the overflow-policy slice.
Edition rejection precedes native parsing rather than claiming edition-sensitive
parsing that does not exist.

## Acceptance gates

`tests/project_configuration.rs` invokes the actual `jr-rust` binary and stock
Cargo, requiring the stock toolchain rather than silently skipping comparisons.
It covers invalid and inherited editions, explicit earlier-edition rejection,
unchecked overflow printing `0`, default/checked release and custom inheritance,
independent debug assertion settings, test-profile selection, and malformed or
unsupported profile settings. Native successful and rejection paths run with
workers 1 and 8. Stock-valid unsupported settings are not claimed as supported.

Baseline reproduction on 2026-10-20: the pre-change local `target/debug/jr-rust`
accepted edition 2099 with exit 0. Hosted run
`20261020-025535825-3560876` reproduced stock output `0` versus native release
abort 134 for explicit `overflow-checks = false`. That run intentionally failed
the new differential assertion before the backend/profile fix was available.

Required validation: `justrust check`, `justrust test --test project_configuration`,
existing `projects` tests and `graph_integrity`. The unchanged authenticated
`utf8-width` registry fixture is edition 2021 and remains explicitly unsupported:
its test records that boundary, not positive registry execution compatibility.
These gates establish only this configuration slice, not full Rust support or a
performance improvement.

## Validation evidence (2026-10-20)

* Hosted `justrust check`: pass, `20261020-030544151-3590715`.
* Hosted `justrust test --test project_configuration`: all 8 pass,
  `20261020-030553857-3591025`, including deferred active-edition admission.
* Updated invalid-project regression: pass, `20261020-030739174-3595104`.
* Unchanged registry earlier-edition rejection regression: pass,
  `20261020-030800677-3590713`. This is not successful registry compilation.
* Graph integrity had no failing cases in the earlier combined run
  `20261020-030100024-3578537`. That combined command failed overall on initial
  configuration fixtures using unsupported `cfg!` and stale project-edition
  expectations; those cases were corrected and rerun above. A final integrated
  graph run passed: `20261020-031347420-3616208`, all 36 tests across
  target dependencies, project configuration, projects and graph integrity.
  Strict all-target Clippy also passed: `20261020-031644716-3623631`.

## Honest test selection (2026-10-20)

Default library test selection requests doctests. Until native doctest discovery
and execution exist, this request fails explicitly before any successful test
report. A scan of documentation fences cannot establish that there are no tests.
Explicit `--lib`, `--tests`, binary, integration-test and `--all-targets`
selections, and `--no-run`, do not request doctest execution. The manifest's
`[lib] doctest = false` disables it, while nonboolean values reject. `test = false`
only disables the unit harness and must not silently disable doctests.

The stock/native selection matrix in `tests/projects.rs` retains a failing
doctest alongside passing unit and integration tests, checks target selection
at workers 1/8, and verifies that unsupported requests print no successful
test result. Independent hosted release-bootstrap validation passes in
`20261020-043622238-3950503`. This closes a false-success path, not full doctest
or Cargo compatibility.
