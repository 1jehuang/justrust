# Committed whole-program status, 2026-10-19

## Measurement and scope

The unchanged 20-program corpus still admits **5 programs** and explicitly
rejects **15** at committed snapshot
`0e14b69554df2a96575a4567386371ca4c392364`.
This is not a percentage of Rust compatibility and does not establish full Rust
support. It measures this corpus only.

Run: `20261019-150639831-750368`, through the installed stable justrust client on
hosted infrastructure, pinned `nightly-2026-10-08`. Command:

```text
justrust test -j1 --test full_programs -- --nocapture
```

The existing isolated export was refreshed to the committed snapshot. All 3,591
tracked blobs matched, with no additional source, test or library inputs.
Four units were freshly compiled. End-to-end command time was approximately
44.6 seconds; the native slim receipt records 41.131 seconds of build reporting.
These timings are provenance, not a before/after performance claim.

Both corpus ratchet tests passed. That means observed results matched the
manifest's five admissions and fifteen explicit rejections, not that all twenty
programs compiled. No crash, miscompiled successful result, or unexpected
admission was reported. The harness checks native CHECK, BUILD and program
output against recorded stock output. This run did **not** establish separate
worker-count 1 and 8 acceptance or freshly regenerate the stock oracle.

The privacy-preserving native export is retained at
`data/runs/rose-committed-corpus-20261019/2026-10.jsonl`. Its scratch repository
classification intentionally redacts names and arguments. The command and
snapshot above provide the interpretation of its two passing ratchet tests.

## Actual first rejection boundaries

| Program | Observed result or first boundary |
| --- | --- |
| 01 hello | Admitted |
| 02 fizzbuzz | Admitted |
| 03 collatz | Admitted |
| 04 primes sieve | Unresolved receiver type |
| 05 gcd/fib | Admitted |
| 06 matrix | Expected `i32`, got `usize` |
| 07 bank account | Admitted |
| 08 enum state machine | No applicable method / unresolved method generic |
| 09 Vec stack | No applicable method / unresolved method generic |
| 10 string processing | Closure parameter type inference, E0282 |
| 11 closures/iterators | `impl Trait` unsupported at the encountered position, E0562 |
| 12 linked list | Closure parameter type inference, E0282 |
| 13 trait objects | `dyn` currently supported only directly behind a reference |
| 14 generic container | Slice generic arguments unsupported |
| 15 error handling | Unqualified calls inside closures require capture-aware resolution |
| 16 sorting | Unqualified calls inside closures require capture-aware resolution |
| 17 HashMap | Unresolved or cyclic `crate::HashMap` import |
| 18 tokenizer | Unresolved receiver type |
| 19 operator overloading | Expected ASCII identifier, found `&` |
| 20 inventory | Unresolved or cyclic `crate::HashMap` import |

These are first failures, not complete inventories of missing behavior in each
program. Removing one boundary does not imply the rest of that program works.

## Implementation priorities

1. Connect typed nominal realization and the source solver's suspended recipe
   consumers to actual Iterator receivers. Use the unchanged sieve and matrix
   programs to expose the next integration failures.
2. Implement source-binding-aware calls inside closures. Do not simply remove
   the rejection: lexical shadowing, captures and macro hygiene must determine
   whether a callee is a local value or a source declaration.
3. Connect opaque physical bodies and generic helper memory effects to genuine
   ownership and code-generation consumers. Immutable proof drafts alone do not
   deliver executable support.
4. Advance generated item metadata and actual derive application against
   unchanged dependency sources, without rewriting cached crates or ignoring
   attributes.

Feature-specific acceptance can demonstrate real improvements while this
whole-program score remains unchanged. Re-measure the committed corpus after a
consumer-level unlock, rather than repeatedly running it for unrelated edits.
