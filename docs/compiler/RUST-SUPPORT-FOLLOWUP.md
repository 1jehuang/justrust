# Rust support audit follow-up

Date: 2026-10-20. Baseline: `c5d75a98e` (audit delivery verification). This records remediation, not completion of full Rust support.

## Implemented changes

- **Project configuration:** invalid editions are rejected. Valid older editions remain parseable as metadata but are explicitly unsupported if selected for native compilation, including transitive mounted dependencies. Profile inheritance and runtime overflow settings are validated and applied. Native output no longer falsely labels unoptimized release builds as optimized. See `PROJECT-CONFIGURATION.md`.
- **Overflow semantics:** source-owned checked/unchecked policy reaches normal native, physical opaque and SSA object lowering. Unchecked arithmetic wraps at the declared integer width, and dynamic shifts mask their count. Division/remainder traps, bounds checks, CTFE and default static overflow diagnostics remain separate. See `OVERFLOW-PROFILES.md`.
- **Nominal indexing:** expression indexing uses Index/IndexMut obligations rather than passing a ConstantIndex to a field-only nominal recipe. Explicit dereference preserves and consumes its reference layer instead of prematurely stripping it. No recipe validator was weakened. Unchanged mutable-index and noncopy-formatting Vec regressions now pass against stock at workers 1/8. See `NOMINAL-RECIPE-SELECTORS.md`.
- **Target dependencies:** native target conditions choose dependency edges and do not let inactive declarations mask active feature requests. Inactive valid older-edition path packages are metadata-only, while malformed editions remain errors. See `TARGET-DEPENDENCIES.md`.
- **Strict lint:** equivalent expression simplifications, named argument bundles, small enum indirections, obsolete private helper removal and test-module relocation. PathKey lint expectations are statement-scoped and document that mutable resource accounting is excluded from immutable semantic equality/hash identity. Graph-integrity ceilings are unchanged.
- **Stale trait unit tests:** the unchanged baseline had 12 failures asserting obsolete unsupported boundaries. Updated tests assert exact current normalization/carrier facts and retain invalid identity, unproved obligation and foreign-context rejection controls. This is not new runtime trait support.

## Observed evidence so far

| Check | Result | Hosted run |
|---|---|---|
| Target dependencies, initial four tests | Pass, including stock comparison and native workers 1/8 | `20261020-025956500-3578205` |
| Nominal selectors plus authority and graph-integrity | 22 pass | `20261020-025823428-3575045` |
| Forged nominal selector unit gate | Pass | `20261020-030530712-3590263` |
| Nominal selector graph gate | Pass | `20261020-030542342-3590607` |
| Original-tree trait tests | 106 pass, 12 fail | `20261020-030238454-3583981` |
| Updated trait tests | 118 pass | `20261020-030458673-3589178` |
| Borrow unit tests | 9 pass | `20261020-030529109-3590155` |
| Semantic core unit tests | 18 pass, 1 pre-existing ignored | `20261020-030646247-3592696` |
| Graph integrity after IR/ownership changes | 14 pass | `20261020-030647822-3592694` |
| Integrated snapshot CHECK | Pass | `20261020-030608075-3591464` |
| Strict all-target Clippy | Pass, `-D warnings` | `20261020-030943038-3600503` |
| Formatting regression with live stock oracle and graph integrity | 18 pass, workers 1/8 | `20261020-031050478-3603331` |
| Project configuration | 8 pass | `20261020-030553857-3591025` |
| Final target dependencies, configuration, projects and graph integrity | 36 pass | `20261020-031347420-3616208` |
| Final strict all-target Clippy | Pass, `-D warnings` | `20261020-031644716-3623631` |
| Final graph integrity after call-receipt and diagnostic fixes | 14 pass | `20261020-031653666-3623531` |
| Call-conversion mutation and omission authority | Pass | `20261020-031211231-3608364` |
| Corrected unused-body / CTFE overflow rejection oracle | Pass | `20261020-031415634-3617097` |
| Invalid generic bound E0277 regression | Pass | `20261020-031655224-3624404` |
| Unchanged mutable Vec indexing and noncopy formatting | Pass, workers 1/8 | `20261020-031257735-3614899`, `20261020-031355043-3614898` |
| Shared-index mutation rejected | Pass | `20261020-031505885-3618500` |
| Final selector graph and graph integrity | 15 pass | `20261020-031608446-3621575` |

The final project and graph suite and strict Clippy pass. The later integrated reconciliation run `20261020-035159955-3782949` passed all **67 tests across 11 binaries**, including the complete overflow arithmetic matrix, source-policy authority, nominal selectors, formatting, project configuration, target dependencies, projects and all graph-integrity gates. Compact exported summaries are retained in `data/runs/rust-support-followup-20261020/reconciliation.jsonl`.

An isolated source export of unchanged `c5d75a98e` had 31 failures in the 109-test backend slice (`20261020-035113221-3780561`); this reconciliation snapshot had 15 failures in the same slice (`20261020-035033537-3777547`). All 15 remaining failing test names were already failing before these changes. This demonstrates useful regression repair, not a green full suite or full Rust support. The earlier server reload interrupted a combined run. Its partial output identified an incorrect test oracle: stock metadata-only CHECK does not issue the unused-function arithmetic-overflow lint that stock BUILD issues. The test compares the proper stock operation, without weakening CTFE or native invalid-body checks. Earlier intermediate build failures from partially edited caller signatures are not final validation results.

## Remaining evidence-backed failures

The last completed unchanged-corpus replay has **5/20** passing programs. The three former internal metadata failures now reach explicit unresolved nominal-inference boundaries. This improves failure classification, not verified corpus admission. Later formatting and explicit-dereference corrections have separate focused acceptance results and do not establish a new full-corpus score.

The unchanged baseline independently reproduces a stack overflow in `compiler::cfg::tests::casts_validate_all_integer_pairs_dead_operands_and_evaluate_source_once` (`20261020-030710349-3593527`). Increasing a test-thread stack is diagnostic only and does not make the default gate pass. A separate baseline CFG slice-ownership run reports 10 pass and 5 nominal-inference failures (`20261020-030733652-3594941`), including a fifth test not reached before the current combined CFG process aborted.

Native associated-declaration-bound tests additionally report two positive BUILD failures with `associated RHS structural substitution changed its source type`. The focused native contract batch (`20261020-030723799-3589174`) finished with 4 passes and 4 failures: all three iterator-universal tests passed, while reference-actual and reference-aggregate suites still encountered resource limits, unresolved nominal constraints or invalid reference-flow authority. These are substantive integration blockers, not successful support inferred from metadata-unit tests. The missing E0277 diagnostic prefix on an already-rejected invalid trait bound has been corrected; its focused acceptance is tracked separately.

The unchanged formatting integration fixture previously failed BUILD with `conflicting nominal substitution` even though CHECK succeeded. Keeping validated call conversion inventories in their closed receipts, rather than merging them into the enclosing body's substitution map, fixes the fixture. Live stock differential tests at workers 1/8 and all 14 graph gates pass. True body-map conflicts still reject. Other previously unrun focused suites surfaced stale closure expectations and semantic-scope failures. These are not silently counted as green.

## Still not done

Production integration of the general semantic checker through per-item graph facts; broad inference/iterator pipelines; complete ownership, region and Drop behavior; recursive layouts; general trait objects; async/Future/Pin; independently built core/alloc/std and collections; public ABI/runtime/unwinding; build scripts, procedural macros and full Cargo workflows; persistent graph invalidation and measured performance gates. `IMPLEMENTATION.md` G0-G12 remain open/partial.
