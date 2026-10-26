# Cross-compilation and local execution rollout

Status: October 26, 2026. The intended split is **compile on shared Linux
workers, execute on the user's target machine**. Native test execution is not
required on the compile worker. It remains distinct from compilation success.
This is a staged implementation, not a declaration of hosted macOS or Windows
support.

## Implemented and verified

- Successful `build`, `check`, and `clippy` reports explicitly say tests were
  not run. Successful `test --no-run` says test binaries were built but not run.
  Existing test execution, failure precedence and command exit status remain
  unchanged. Both compact and detailed reports use this distinction, retaining
  the existing `justrust: ok` success prefix used by CI consumers.
- The opt-in helper in [cross-test-bundles.md](cross-test-bundles.md) packages
  only test executables named by successful Cargo JSON output. The immutable
  manifest records `compiled` / `not-tested`; the separate local execution
  result never mutates that build record. The runner verifies every artifact
  before starting any executable and rejects mismatched OS/CPU targets.
- A real x86_64 Windows GNU test executable was built on the local Linux
  machine using the already installed Rust 1.98.1 target libraries and MinGW
  linker. `file` identified the output as a PE32+ x86-64 Windows executable.
  No Windows test was executed. This is a GNU-target feasibility check, **not**
  validation of our existing MSVC release target or arbitrary native libraries.
- A real Linux test bundle exercised both passing and deliberately failing
  invocations. Cross-OS runtime testing is not claimed.
- The existing Linux, macOS and Windows CI jobs now run
  `ci/check-test-bundle.sh` to exercise native success, deliberate failure and
  immutable build status. The script passed locally on Linux. Adding these
  jobs is not evidence that their macOS/Windows executions have passed yet.

The installed client includes native `justrust test-bundle pack` and `run`
subcommands. The Python helper remains available for interoperable transfer
and execution on machines without the native client.

A real hosted Linux compile-only build returned Cargo-selected test
executables with paths rewritten to downloaded local files. The native pack
command consumed that JSON directly. Two tests passed locally, a deliberate
failure returned CLI status 1 with underlying libtest status 101, and the
original manifest remained `not-tested`. The hosted run was
`20261026-015620295-319789` (pinned Rust 1.90.0).

## Validation evidence

The run summaries are exported under `data/runs/`:

- `20261025-224638736-4157664`: hosted compile check for the reporting change.
- `20261025-224642934-4157662`: both compile-only reporting regression tests passed.
- `20261025-224700102-4160286`: full hosted library run had 716 passing tests,
  three failures and three ignored tests. The failures were existing
  self-reexecuting toolchain tests whose executable path was unavailable under
  the pooled runner, not reporting assertion failures.
- `20261025-224813602-4170550`: the full hosted library run with
  `JUSTRUST_TEST_POOL=0` passed 719 tests with three ignored tests.
- `20261025-224755014-4167853`: hosted clippy with `-D warnings` passed.
- `20261025-224737351-4164888`: real local Windows GNU `test --no-run` build.
- `20261025-225111827-4192104`: installed committed client on that warm Windows
  GNU build emitted `test binaries built, tests not run (--no-run)` and reused
  its artifacts without compilation.
- `20261025-225307728-11734`: Linux native CI fixture compiled without running
  tests, then its bundle passed one selected test and preserved an intentional
  failing test's nonzero exit in a separate result.
- `20261025-225308219-11879` and `20261025-225318517-11548`: the success-prefix
  regression tests and hosted clippy passed after the CI compatibility update.

These are correctness/feasibility checks, not before/after performance evidence.
No speedup or general cross-platform compatibility claim is made.

## Remaining platform gates

The native CLI and hosted artifact handoff are implemented. Windows GNU is an
explicit additional target, with managed account-local tools and PE x86-64
validation. It does not replace MSVC release artifacts or enable macOS.

Verified hosted runs:

- `20261026-020244906-366218`: Windows GNU compile-only test build with exact
  Rust 1.90.0, two selected PE x86-64 executables downloaded. Native packaging
  succeeded; Linux refused execution with status 2 and no results file.
- `20261026-020342575-373348`: Windows GNU release executable built with a C
  dependency using Windows headers and the managed MinGW tools, downloaded
  and identified as PE x86-64. Not executed.
- `20261026-020310924-369516`: initial release fixture failed because its binary
  called a C symbol without referencing the library carrying that native
  dependency. Correcting the fixture's library linkage produced the successful
  run above. The compiler/linker error was shown and exit 101 preserved.
- `20261026-020542447-387710` and `20261026-020546307-387709`: final installed
  hardened client repeated the Windows test download (two delta artifacts)
  and warm release build successfully. Native packaging preserved `not-tested`.
- `20261026-020408891-376375`: final Windows-aware library regression passed
  733 tests, three ignored, using a single test thread to avoid existing
  lock-test interference.
- `20261026-020436454-376373`: final native bundle, remote artifact and required
  routing integration tests passed all 13 tests.
- `20261026-020038375-350457`: PE-enabled all-target clippy passed with warnings
  denied.

For commands and prerequisites, see [managed-windows-gnu.md](managed-windows-gnu.md).
Remaining work for other platform support:

1. **Provision versioned target toolchains on the Linux worker.** Rust target
   libraries must match the selected compiler. Windows MSVC needs a reviewed
   SDK/CRT and compatible C/C++/assembler/linker setup. macOS needs its SDK,
   compatible linker and framework/native build support. A read-only inspection
   of the active hosted machine found clang and lld-link, but not MinGW or SDKs
   at the common locations checked. The new Windows GNU path provisions its
   pinned MinGW tools without root. MSVC/macOS SDK acquisition is still separate
   and must respect applicable licensing.
2. **Extend the native hosted build path.** Retain explicit target admission,
   tenant isolation, exact toolchain selection, target-aware artifact
   validation and cache separation. Do not remove the existing unsupported
   target guard simply because the local compiler can emit a binary.
3. **Return the selected test artifacts.** Implemented for hosted native Linux
   `test --no-run`: Cargo-selected executables are downloaded and JSON paths
   are rewritten to local files. Additional targets require target-aware
   admission and binary-format checks, not just a compiler flag.
4. **Provide a first-class local runner.** Implemented in the installed client.
   Resource fixtures, dynamic native libraries and working-directory
   requirements remain explicitly caller-managed, not implicitly bundled.
5. **Validate representative real projects before promotion.** Check native
   dependencies, build scripts, proc macros, target ABIs, incremental rebuilds,
   corruption/mismatched-target rejection and local failure reporting. Keep
   signing/publishing credentials away from compile workers. Preserve the
   existing MSVC release contract unless a change is explicitly approved.

User projects may choose local runtime testing. justrust's own release pipeline
should still retain native launch/smoke validation before publication. Nothing
in this rollout silently disables tests or changes the meaning of `justrust
test`.
