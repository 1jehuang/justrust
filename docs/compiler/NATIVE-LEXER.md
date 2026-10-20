# Native lexical transport and parser cutover

This checkpoint replaces the production character scanner with native lexical
atoms backed by checked source snapshots and token sessions. It does not
implement macro expansion, hygienic semantic identifier resolution, general
literal semantics, or full Rust compatibility. Acceptance results are recorded
below only after the final frozen gates run.

## Source and ownership

`PreparedLexInput` borrows the configured composite source text and its origin
segments. Its sealed snapshot shares original content Arcs but copies file,
path and line-table metadata through checked, fallible, budgeted allocations.
The metadata lease remains live until the last snapshot/session owner drops.
The raw syntax-only entry point instead builds a budgeted owned text snapshot,
without recursively preparing source, applying configuration, or performing I/O.

Original source limits remain 8 MiB and 1024 files. These bounds do not excuse
unaccounted public SourceMap metadata: paths, line tables and copy work are
checked independently before retention. Snapshot identity is explicit. A raw
FileId or SourceSpan is not accepted as an owned token provenance capability.

The bridge checks ranges, UTF-8 boundaries, coverage, original byte equality
and synthetic transport anchors. Canonical lexical tokens must project as
whole tokens before parser-only views are derived. Logical operators are also
checked as whole compounds, rather than accepting individually valid atoms
that cross an origin boundary. A contrived prepared map that splits a canonical
float token is rejected as a transport contract error even if its later tuple
field views could individually map. Normal tuple field source remains a
required positive regression.

## One lexical implementation

`lex_prepared(input, edition, session)` validates the complete lexical input
before projecting spans or interning token spellings. This keeps later lexical
errors from being hidden by an earlier mapping failure. The separate original
source validation hook uses the same scanner, without token materialization,
projection, parser invocation, or logical input-token charging. Source loading
shares one validation budget across the root, children and verified provider
within the preparation operation. Original lexical validation precedes
configuration and child filesystem observations.

The scanner retains exact spelling, literal suffixes, punctuation adjacency,
explicit EOF provenance, and doc-comment records. Configured and syntax-only
source modes do not reinject doc attributes or repeat source policy. The
literal decoder walks validated normal-string contents, not printed or
reconstructed token transport.

Unicode XID classification uses the minimal `unicode-ident =1.0.24` utility,
with Unicode table version 17.0.0 and Cargo.lock checksum pinned. Its notices
are retained in THIRD-PARTY-LICENSES.md. No compiler or parser implementation is
embedded. Unicode lexical validity is distinct from semantic admission and
normalization. Production source follows the initial edition-2024 Linux
x86-64 compatibility track. Explicit earlier-edition lexical observations do
not establish full earlier-edition support.

## Parser views and authority

The owned native parser input retains its token session, budgeted logical
views, decoded normal strings, and constituent SpanIds. The borrowed cursor
continues to provide immutable checkpoints and type-only punctuation splits.
Splits select checked constituent provenance rather than mutating token text
or doing SpanId arithmetic. Numeric subranges needed for tuple field views are
projected before the session is frozen. No authoritative expanded-text
print/relex path is introduced.

Nonroot contextual identifiers, raw and Unicode identifiers, and newly lexed
literal classes remain explicitly unsupported at the semantic ingress unless
a separate implementation admits them. String continuations that are valid
Rust but outside that ingress are Unsupported, not malformed Syntax. Arbitrary
literal suffixes can be lexically valid in token trees even when their use as
an expression is invalid or unsupported. Lexical tests keep those questions
separate.

The existing AST, source replay, trait/ownership checks and universal unused
body validation remain mandatory. Span/session IDs and metadata records grant
neither canonical source authority nor private provider attribute permission.

## Diagnostics and acceptance obligations

Syntax, Unsupported and Limit remain distinct from Contract and Infrastructure
transport failures. Source locations use original-file byte provenance and
Unicode-scalar columns. Unlocated structural resource errors do not fabricate
an original token span. The graph retains complete failure accounting, while
CHECK does not request code generation. BUILD may request an object task whose
dependency fails, but it cannot publish a successful executable on rejection.

Required gates include pinned stock lexical witnesses, native facade checks,
all existing source/probe/replay/depth/resource regressions, workers 1/8
CHECK/BUILD graph tests, complete hosted tests, strict all-target lint and
formatting. Initial failures and their repairs are retained rather than
reported as clean first-pass acceptance. Full Rust and macro execution remain
open in IMPLEMENTATION.md.

## Resource regression repairs

The first native cutover exhausted the shared work budget on the existing
4096-parameter positive source because symbol lookup compared each spelling
against all earlier entries. The replacement is a charged, capacity-accounted
open-address index with session-randomized hashes and full spelling equality on
hash matches. Symbol IDs still follow insertion order, not hash-table order.
Forced collisions, rehashing and the unchanged positive source are regression
tests. No work cap was increased.

The unchanged 65537-import negative source initially hit the 256 MiB storage
budget before its parser expansion-count guard. Two measured failing admissions
were 231736373 live bytes plus a 67108864-byte request, and 240321465 live bytes
plus a 33554432-byte request. The parser adapter now reuses genuine native leaf
and EOF SpanIds, stores checked 32-bit prepared bounds, and does not duplicate
prepared ranges alongside every constituent SpanId. Whole compounds and suffix
views still have independently checked provenance. The original source and
parser guard are unchanged. These are bounded-resource compatibility repairs,
not representative compile-time speedup measurements.

Prepared input and its session must share both the sealed source snapshot and
the exact expansion-budget authority. Equal limit values are not sufficient.
Foreign pairing rejects before scanning or charging either ledger. Allocation
and infrastructure failures retain their typed category through the source
diagnostic adapter rather than becoming malformed-source errors.

## Frozen acceptance

On October 17, 2026, the final hosted gates passed:

- All-target check: `20261017-064504065-590678`, no warnings.
- Complete tests with `--no-fail-fast`: `20261017-064506676-590807`, all 1887 tests
  in 96 binaries, no failed, ignored or filtered tests.
- Strict all-target Clippy: `20261017-064524075-592057`.
- `cargo fmt --check` and `git diff --check`: clean.

The complete run includes 16 native lexical integration tests with 54 stock
witnesses at editions 2024 and 2021. Edition-2024 stock outcomes are 21 valid and
33 invalid, compared with 23 valid and 31 invalid at edition 2021. The native
lexical phase accepts 23 and rejects 31: two stock-invalid expression suffix
uses still have lexically valid spellings, as independently checked inside
swallowed token trees. Native parser ingress tests separately reject unsupported
semantic classes. The actual hosted oracle was rustc 1.101.0-nightly,
`1d81eb4ad9cd207e3e638bd32b17ec4fce8412a6`, dated October 7, 2026. The separately
measured local baseline was rustc 1.98.1,
`48a229ceaefd4985c50990b14116b6d856af0985`; these are not claimed to be identical
source revisions.

The complete run also covers existing source, unchanged identity, nested
reference, wide-integer, ownership and replay suites. Independent source/adapter
and lexer/literal reviewers found no remaining concrete blocker in this bounded
scope. Twelve exact redacted records in
`data/runs/xps14/2026-10-17-native-lexer.jsonl` preserve the initial 655/658 and
678/679 unit failures, both measured storage failures, stale typed-test
expectations, the repaired 684-unit run, and final acceptance. This is not a
clean-first-pass claim, a complete Rust lexical-conformance proof, macro support,
or full Rust compatibility.
