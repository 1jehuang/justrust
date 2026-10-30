# Native declarative cache: audited lexical `tt` subset

Status: native correctness and independent literal/TT hit gates passed on
2026-10-30. Default remains **off** under the existing bounded
`-Zjr-cache-declarative-macros` flag. This is not a latency or default-promotion
claim. The coordinator built and froze the driver and ran native acceptance
before authorizing this widening commit.

## Exact admission contract

The entire rule set must consist of function-like rules. Every matcher fragment
in every arm must be `tt`. Literal matcher tokens, delimiter traversal and the
existing matcher repetition machinery are allowed. Any `expr`, `ident`,
`lifetime`, `item`, `ty`, `pat`, `block`, `stmt`, `meta`, `path`, `literal`, `vis`
or `guard` fragment anywhere rejects the entire definition, including an earlier
arm that would otherwise not be selected.

A transcriber may contain literal tokens and ordinary groups recursively, plus
only repetitions with all of these properties:

* The repetition body is exactly one metavariable reference.
* That reference resolves by `MacroRulesNormalizedIdent` to a `tt` declaration
  in the same arm.
* There is no separator.
* The repetition operator is `*` or `+`, never `?`.

For example:

```rust
macro_rules! pass {
    ($($t:tt)*) => { $($t)* };
}
pass! { fn answer() -> u32 { 42 } }

macro_rules! wrap {
    ($($t:tt)+) => { fn answer() -> u32 { $($t)+ } };
}
wrap! { 40 + 2 }
```

Standalone `$t`, `$($t),*`, `$($t)?`, `$(prefix $t)*`, nested RHS repetitions,
unknown references and metavariable expressions remain on the original path.
Matcher repetition shape is not assumed to guarantee successful transcription.
The original transcriber still checks repetition depth, cardinality, zero-length
`+` and lockstep constraints. A constructed transcription diagnostic is canceled
in speculative computation and the original expansion path owns error emission.
Failed matching, ambiguity, or rejected output likewise falls back.

## Parser audit

The exact call chain is `TtParser::match_one` in
`compiler/rustc_expand/src/mbe/macro_parser.rs`, then
`Parser::parse_nonterminal(NonterminalKind::TT)` in
`compiler/rustc_parse/src/parser/nonterminal.rs`, then `parse_token_tree` in
`compiler/rustc_parse/src/parser/mod.rs`.

The TT branch directly returns `ParseNtResult::Tt(self.parse_token_tree())`.
It does not collect an AST, parse an expression, gate a feature, register an
attribute, or emit a parser diagnostic. `parse_token_tree` clones a lexical group
and skips its cursor to the end, or consumes one lexical token. `Parser::bump`
and `TokenCursor::inlined_next_and_bump` only update parser/cursor-local state
and perform deterministic dummy-span repair. `Parser::new` begins with token
capture disabled. `parser_from_cx` desugars documentation comments, but the
recursive lexical filter rejects those before entry.

`try_match_macro` temporarily removes the pre-existing gated-span map while
matching arms. In particular, its ambiguity return precedes snapshot restoration.
Speculative cache computation therefore separately takes and restores the
caller's original gated-span map around matching, including failed and ambiguous
results. Audited TT matching cannot contribute new gates. The ordinary fallback
is unchanged and begins with the original gate state.

## Why general TT transcription is not admitted

`transcribe_metavar` dispatches `ParseNtResult::Tt` to
`transcribe::maybe_use_metavar_location`. Ordinarily that routine inserts original
and metavariable spans into the global `with_metavar_spans` side table. Collisions
can also change output span locations. Neither skipping those writes on a hit nor
replaying only the returned token stream is correct.

There is one explicit side-effect-free branch: when the top transcription frame
is a separator-free `*` or `+` sequence containing exactly one tree,
`maybe_use_metavar_location` returns `orig_tt.clone()` **before** marking the
metavariable span, inspecting source-map import status or touching the global
span table. Admission requires that one tree to be a known TT metavariable.
Thus every actual TT substitution necessarily takes that early return.

Literal RHS tokens/groups still use the existing `Marker` operation with the
current expansion ID and transparency. Captured TT tokens are raw clones, so
caller hygiene, lexical spacing and group spans are preserved, with no invisible
AST fragment wrapper. Repetition sequencing only maintains local frame/count
state. Its errors are returned as un-emitted diagnostics and canceled on fallback.

## Key, codec and outside-query effects

The existing query boundary and key are unchanged: full original definition and
input token streams, definition span, expansion identity/hygiene, edition and
transparency. Incremental compilation is required and ignore-spans mode remains
ineligible. The recursive lexical guard runs on definition, input and successful
output. It rejects invisible groups, interpolated ident/lifetime tokens,
invisible delimiter token variants, documentation comments and EOF. No AST codec
or new fragment encoding is introduced. Ordinary token/group spans and spacing
continue through the existing token-stream codec.

Trace output, selected-rule usage recording and `ParserAnyMacro` construction,
including parsing the resulting stream and its diagnostics, remain outside the
query, so cache hits cannot suppress them.

## Source tests and required native evidence

`declarative_cache.rs::tests::lexical_scope_rejects_interpolation_recursively`
checks the recursive lexical filter. `macro_rules.rs::declarative_cache_tests`
checks TT-only all-arm fragment admission and the exact accepted/rejected RHS
lexical-tree shapes. These source unit tests are not a replacement for native
compiler acceptance. This lane only formats and inspects source.

## Native correctness outcome, 2026-10-30

The coordinator's frozen driver
`ac116b9398e1f5c677e73952b8719be9d9e47190fe61fbaeba9b7cf270739595`
passed all 51 semantic cases in seven modes, 357 semantic compiles, with exact
unnormalized diagnostic bytes, exit statuses, runtime results and
`-Zincremental-verify-ich`. Independent five-compile literal and five-compile TT
positive-hit checks both passed. The literal-only older compiler failed the
TT-positive requirement as expected, so literal hits cannot satisfy that gate.
The coordinator also reported all 610 Python tests passing.

Native evidence is in the coordinator scratch directory
`~/.jcode/scratch/declarative-cache-20261030/`: `acceptance-tt/acceptance.json`,
`identity-tt-before.sha256` and `identity-tt-after.log`. The acceptance JSON records
`semantic_verdict`, `native_hit_verdict`, `tt_hit_verdict` and overall `verdict`
as `passed`, with all 51 cases completed. This implementation lane inspected that
result before committing and ran no additional builds or native workloads.
The Rust unit tests added here are source-level regression coverage, not claimed
as independently executed by the acceptance harness. Real eligibility and
performance measurements remain separate coordinator work.

Future changes must preserve the native correctness matrix with flag off/on,
positive disk-hit evidence for the exact `pass` or `wrap` shape, repetition-error
fallback diagnostics, imported/local hygiene and incremental edits. General TT
identity and separated repetition remain fallback semantic cases, not positive
hit requirements. Real-workload eligibility and before/after timing must be
measured separately. The old literal-only result (106 eligible versus 1858
rejected; 1.58 s enabled versus 1.47 s off) is not evidence of a speedup or of this
new subset's performance.
