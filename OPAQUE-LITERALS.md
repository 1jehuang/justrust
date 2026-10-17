# Opaque literal fragments at the source expression boundary

Status: the bounded opaque literal source consumer is accepted after source-macro
commit `712d83e7b`. The final hosted gate passed all 2,089 tests across 103 binaries
in `20261017-092115516-1299372`, all-target check
`20261017-092115024-1299346`, strict all-target lint
`20261017-092129356-1300588`, formatting and whitespace checks. This extends the
accepted `tt`/`ident` client, not the full Rust macro implementation.

The later accepted NESTED-SOURCE-MACROS.md slice adds multiple original local
definitions and nested/recursive expression invocations. Earlier single-definition
restrictions below describe this historical checkpoint, not the later client.

## Exact added client

The added slice permits the existing pure engine's `literal` fragment in the same
one-function-local-definition, expression-position source driver. It does not
add nested/recursive invocations, item generation, macro exports/imports or new
fragment grammars. The matcher and transcriber keep captured literals opaque.

Only the final native expression consumer can inspect the session-owned literal
payload. A compiler-private helper requires the exact opaque native token and
its driver-issued expansion admission proof. It validates session, expansion,
source span, payload kind and shape before following backward fragment handles.
Every chain step and inspected node is work-charged. A fixed arena-count bound
makes cycles fail closed without recursion or an unbudgeted visited set.

The result contains at most one actual minus token and one actual literal/bool
terminal, plus the outer opaque occurrence span. The helper reissues only those
real payload constituents into the admitted expansion. It retains their physical
origins and the current expansion call trace. It does not accept arbitrary span
requests or expose a mutable token session.

## Atomic parser semantics

The native buffer retains a private opaque atom, not a transparent replacement
sequence available to a macro matcher. An isolated buffer of checked literal
constituents is parsed by the existing expression parser with an explicit EOF.
Its resulting expression is one atom in the surrounding parser. A negative
literal therefore cannot accidentally acquire the outer postfix/precedence of a
flattened minus and positive integer.

The existing integer parser and checker remain responsible for suffixes,
unsigned magnitude, contextual defaults, negation and signed minimum values.
The child must preserve a direct integer or unary-negation-of-integer expression;
wrapping it in an artificial block or typechecking the positive magnitude first
would break signed i32/i128 minimum acceptance. Float spelling, suffix and signed
zero handling likewise remain with the existing parser and checker.

Booleans remain literal expressions, not references or fabricated lexical
bindings. A negative boolean is syntactically captured and fails ordinary type
checking. Raw/byte/character/C literals remain explicitly Unsupported at the
existing semantic boundary, not malformed lexical input.

Normal strings are consumed only by the existing `println!` literal slot.
Standalone string expressions remain Unsupported until an actual string AST,
type and runtime representation exists. No fake string variable or print
expression stands in for that missing implementation.

The formatter must reserve work and output capacity before allocation, use
fallible string reservation and reconcile actual capacity. Its escaping AST
String is not covered by an isolated child buffer's soon-to-drop text lease.
Instead, the original parser's output-capacity reservation is conservatively
retained in the same NameStore wrapper lease through the final Program/name-store
Arc owner. Early AST destruction does not refund this conservative charge.
Subsequent planned AST clones retain their existing separate clone preflight
accounting. This is not a claim that a copied bare String carries an individual
lease or that arbitrary unplanned public clones have exact aggregate accounting.

## Phase and authority controls

Arbitrary literal suffixes are legal lexical and macro-fragment input. A macro
that discards such a capture can therefore succeed. A returned invalid numeric
suffix is rejected by the existing expression semantics, not by a fabricated
lexical failure. Original macro-bearing input carries suffixed normal strings as
private `DeferredLiteral` parser tokens without decoding them or pretending they
are semantically admitted strings. Only the canonical macro input is matched;
ordinary parser consumption of the deferred token remains Syntax. Consumed opaque
string suffixes still fail the existing decoder. Malformed radix digits fail the
complete original lexical scan before macro registration or projection.

Consumed leaves are parser-only views. They never replace the opaque arena entry
or become transparent matching input. Existing pure-engine forwarding-opacity
controls remain unchanged. Token, scope, resource and infrastructure failures
retain their typed causes and are never retried as an alternative matcher arm.
No operation grants original-body, plan, facts or verified publication authority.

## Acceptance evidence

The added stock-first corpus contains 21 sources: 15 stock-valid and six
stock-invalid. Its 76 native worker/mode cells cover 12 runtime clients, five
ordinary invalid clients and two explicit unimplemented string-expression
boundaries, each at workers 1/8 in CHECK/BUILD. Two stock-valid postfix atomicity
controls remain stock-only because their method is not implemented natively.
Typed parser controls independently pin suffix/radix Syntax, standalone string
Unsupported, and successful parsing of negative bool before its checker error.
The unchanged 17-stock/48-native source corpus also passes. The focused combined
gate passed in `20261017-092051812-1297792`.

Private controls cover atomic AST shape, signed minima, payload origins,
proof/session/buffer rejection, recursive forwarding, work/byte denial, retained
buffer cleanup and surviving formatted String capacity through the last name-store
owner. All 817 library tests passed in `20261017-092100259-1297791`, including the
unchanged 65,537-use and 4,096-parameter resource witnesses.

The first runtime matrix correctly exposed premature rejection of a discarded
suffixed string in `20261017-091615659-1277231`. The deferred-token repair keeps
that legal capture separate from actual string consumption. Earlier failed
compile snapshots and successful frozen receipts are retained in
`data/runs/xps14/2026-10-17-opaque-literals.jsonl`. The complete frozen suite above
reports zero failed, ignored or filtered tests. There is no full Rust or speed
claim.
