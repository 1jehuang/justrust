# Private root-OR test expectation repair

Private source commits, in order, on preserved `4168f2186`:

- `9ef779cd6`: approved eight-file test-only correction of stale root-OR expectations.
- `4083794d1`: exact typed tuple-OR row assertions for the newly promoted fixture, plus a lint-only replacement of the single-element `@` negative loop with an equivalent block.

`refs/jcode/checkpoints/or-before-stale-test-repair` preserves the parent. No production source, graph gates, ceilings, kind IDs, compiler authority or stock fixture source bytes changed. No local build, full-suite run, push or installation was performed here.

## Exact changes

- `src/compiler/frontend/parser_tests.rs`: the former exact `fn main(){matches!(1,1|2)}` parse-negative now asserts one original Match, one integer scrutinee, ordered Int alternatives 1 and 2, and true/false arms. Its bool tail still fails main's unit return check. A separate unit-terminated source passes `good()`. All seven malformed arguments, the exact `@` negative and all three late invalid-type controls remain.
- `tests/matches_macro.rs`: unchanged scalar root-OR fixture becomes Live/check, partition `[18,1,2,0]`. Its source bytes, SHA256, fingerprint, stock streams and stock result are unchanged. Module shadow stays Unsupported.
- `tests/guarded_matches.rs`: only the expected diagnostic substring becomes `guards on or-patterns are unsupported`. The fixture remains Unsupported in parse, with unchanged source and category. Ordinary guards, `@`, non-Copy and invalid body controls remain.
- Aggregate, array, record-rest and pattern-decision tests: binding OR remains Unsupported, now asserted specifically in resolution after successful parsing, with its own counter rather than being called a checker rejection. All unrelated parser and checker negatives remain.
- `tests/pattern_usefulness.rs`: unchanged Copy tuple root-OR fixture becomes Runtime. Partition `(23,6,1,2)`, 128 workflow cells, modes `(46,46,12,12,2,2,4,4)`. Nonempty runtime-image reporting derives from executed cells. The exact checked type is `(bool,bool)` with two ordered rows, each containing exactly one Bool predicate at Field(0), respectively true and false. The preexisting Decision validation is unchanged. The positive inventory grows from 21 to 22; the new source contributes two independently checked rows.

All 186 frozen `source: r...` strings across the seven integration files were compared byte-for-byte against `4168f2186`. Existing stock hashes and receipts were not changed. Nominal/payload enum, non-arm while-let/let-else, nested OR, binder, guard, non-Copy, generated-origin and late invalid controls were not promoted.

## Validation, including preserved failures

All eleven owned runs below were individually checked with `justrust show`: **remote on hosted**. They use forced hosted, prebuild disabled, jobs 2 and serial libtest. `scoped.jsonl` contains their exact filtered `justrust export` records plus the original central full RED, and no unrelated records. All IDs have prefix `20261019-`.

| ID suffix | Result |
| --- | --- |
| `011323222-144488` | Original central full on `903bbe9e6`: 3634 passed, 17 failed, exit 101. Preserved, not rerun here. Post-lint did not execute. |
| `012159484-376275` | All-target check, exit 0. |
| `012202968-376425` | Complete library: 1523 passed, exit 0. This precedes the final equivalent `@` loop-to-block lint repair. |
| `012246095-378368` | Aggregate 5 + array 5 + **all 14 current graph_integrity tests** passed. Guarded tests all failed before oracle/native execution because hosted lacks hardcoded `/usr/bin/rustc`; 24 passed / 4 infrastructure failures. Following targets did not execute. Unchanged graph boundaries remain 1208 required / 1209 failing. |
| `012345584-384564` | All four matches_macro tests fail before oracle/native execution with the same `/usr/bin/rustc -vV` ENOENT. No source assertion result. |
| `012420514-386444` | Pattern-decision and usefulness: 9 passed / 1 failed. Newly promoted tuple OR reached a stale Decision-only positive inventory assertion. Repaired by validating its exact Alternatives rows, not excluding the fixture. |
| `012526727-390411` | Test-only compile RED: tuple assertion supplied Vec rather than Arc slice. No test verdict. |
| `012527327-390347` | Strict lint compile RED for that same test-only tuple storage mismatch. |
| `012549273-391213` | Remaining five targets complete: 20 passed (pattern_usefulness, pattern_decisions, record_rest_matches, root_or_patterns, root_or_authority), exit 0. Includes exact 128-cell usefulness native workflow and root-OR source authority controls. |
| `012614086-391138` | Strict lint RED: single-element `@` loop after removal of stale OR-negative row. No ignored lint or suppression; replaced with equivalent block. |
| `012634353-393269` | Final exact parser fixture after lint-only block repair: 1 passed, 1522 filtered, exit 0. |
| `012639406-393445` | Final strict all-target lint: actual exit 0. |

`cargo fmt --check` and `git diff --check` passed. No hosted fallback occurred. Existing oversized-crate telemetry is not a speed claim.

## Remaining acceptance blocker

**This is not an all-target green checkpoint.** `matches_macro` and `guarded_matches` hardcode `/usr/bin/rustc` for their stock-first probes; the hosted image used here does not contain that executable. Their frozen phase/CLI/graph/runtime tests therefore remain unvalidated after repair. No path workaround, source weakening, host modification, oracle skip or unauthorized local fallback was used. Coordinator must arrange that exact hosted oracle prerequisite or explicitly allocate a local-only validation slot. Central full acceptance and central integration remain coordinator-owned.
