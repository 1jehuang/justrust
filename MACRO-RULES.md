# Pure declarative-rule token utilities

This is an implementation checkpoint below the production Rust frontend. It is
not native `macro_rules!` source admission, a hygiene implementation, or full Rust
compatibility. `MACRO-EXPANSION.md` defines the complete integration program.

## Ownership and transport

`compiler::token_primitives` exposes checked `TokenSlice`, `MacroCursor`,
`CursorMark`, `LogicalToken`, and opaque literal-fragment transport. A slice owns
its immutable token stream and checked tree range. Clones retain its view
identity, while a checked subslice receives a distinct identity. A cursor mark
cannot be restored into a foreign session or view.

The macro cursor presents Rust logical operator and lifetime tokens over physical
leaves. Joint punctuation is not indiscriminately combined: `>>=` is one logical
operator, whereas `++` remains two tokens. Every constituent retains its own
checked span. Nested delimiter groups remain trees rather than printed text.
Neither grouping nor matching prints and relexes expansions.

Literal fragments are session-issued opaque payloads. Forwarding an opaque
literal cannot reveal its spelling to a fixed-token matcher. Recapturing it as a
literal retains opacity and its recorded leading-minus grammar state. `tt` and
`ident` remain transparent fragment classes. There is no fabricated expression,
type, pattern, or item AST certificate for an unsupported fragment grammar.

## Pure public API

`compiler::macro_rules` provides:

- `compile_rules(body, DefinitionSite, session) -> RulesDef`.
- `expand_rules(definition, args, CallSite, session) -> ExpandedTokens`.
- Checked output accessors for the token slice and expansion record.

The definition body is the grouped arm-list interior, not an entire textual
`macro_rules!` declaration. Invocation arguments are the invocation outer-group
interior. Both remain in the same checked token session as the definition and
output. Definitions and expansion results have private issuing-owner fields.

The current engine supports `tt`, `ident`, and `literal`, ordered arms, nested
repetitions with `*`, `+`, `?`, and separators. Other fragment grammars are
explicitly unsupported rather than treated as a failed arm. Identifier matching
has an ASCII boundary pending the canonical Unicode-name implementation.

Compilation builds immutable flat matcher instructions and repetition schemas.
Matching uses bounded lockstep states and flat capture histories rather than
recursive backtracking. Only a genuine matcher `NoMatch` advances to another
arm. A local ambiguity, committed fragment syntax failure, resource failure, or
selected-arm transcription error is final. A successful match does not retry a
later arm because its output is invalid.

Definition errors and selected-transcriber errors have different phases. In
particular, an unbound transcriber metavariable is not rejected merely because
an unused arm contains it. Selecting that arm fails without trying a fallback.

## Resources and provenance

All components use the existing shared `ExpansionBudget`, without raising its
caps or borrowing unrelated generic-planning caps. Work is monotonic across
failed alternatives. Live matcher states and temporary buffers are leased;
retained definitions, fragments, output, and provenance keep their own storage
leases. Each invocation has one invocation budget. Output admission counts
physical token occurrences, including delimiters and opaque payload weight,
not merely cheap shared handles.

Typed budget failures retain their original limit, overflow, allocation, or
infrastructure cause. They are never converted into `NoMatch`. Foreign handles
and invalid checked ranges remain contract failures, not source syntax errors.

Expansion output receives a checked expansion record. Definition literal tokens
receive expansion context, while captured identifiers retain their input
context. Reissued occurrences anchor the previous occurrence plus the current
expansion, preserving forwarding history and separate delimiter origins. These
low-level facts do not constitute a mixed-site resolver, privacy permission,
trusted original-body publication, or private identity-metadata authority.

## Validation and acceptance boundary

Focused hosted foundation/engine tests passed 16/16 in
`20261017-071409342-722090`. The independent public API target passed 15/15 in
`20261017-071543969-731024`, including 81 stock Rust cases and 14 direct utility
tests. That run recorded actual stock `1.101.0-nightly` commit `1d81eb4ad`
(2026-10-07), independently of the earlier local `1.98.1` measurements and the
Reference pin. Exact failed and successful checkpoint summaries are retained in
`data/runs/xps14/2026-10-17-semantic-macro.jsonl`.

These output token streams are utility evidence, not proof that the public
compiler can check or execute a Rust macro program. The complete coordinated
suite subsequently passed 1,951 tests across 98 binaries without retry output in
`20261017-072549738-775993`. All-target check
`20261017-072603261-776840`, strict all-target lint
`20261017-072603747-776966`, formatting, and whitespace checks passed. The
publication-event race found during validation and its separate repair are
recorded in FINDINGS.md rather than hidden by the stable client's automatic retry.

The specification pin is Rust Reference commit
`f1e8ba23e46236877fe90c86e4cb7075ad37b776`,
`src/macros-by-example.md`, 27,548 bytes, SHA-256
`32e55cf4b4f62b969d1348b15d30b6f807501e32cfa04133e2b0bc5e944dcc67`.
The measured stock compiler version is recorded independently. The Reference
revision is not a claim that the installed compiler was built from that source.

Production admission still requires actual owned semantic AST identifiers,
namespace-specific mixed-site resolution, token-origin editions, lexical macro
scope and lookup, generated-item configuration before module I/O, opaque
fragment consumption by the parser, and graph completion after late producer
failures. CHECK and BUILD workers 1/8 must then pass source-level stock/native
cases, including unused generated errors, shadowing, forwarding, and negative
filesystem observations. No utility-only test substitutes for those gates.
