# Function-local source macros and checked mixed-site locals

Status: the bounded first function-local `tt` / `ident` source client is accepted
on the frozen hosted gate after `12826bfe3`. OPAQUE-LITERALS.md records the later
accepted `literal` consumer and its 2,089-test / 103-binary gate. Neither is the
full Rust macro implementation. The first client passed 2,070 tests / 102 binaries in
`20261017-090528020-1227907`, all-target check
`20261017-090525946-1227837`, strict all-target lint
`20261017-090544741-1229123`, formatting and whitespace checks.

## Admitted source path

The bounded client accepts one function-local `macro_rules!` definition and
expression-position invocation in the same function or nested blocks. It reuses
the existing ordered `tt` / `ident` / `literal` matcher and transcriber. Opaque
literal consumption follows the separate OPAQUE-LITERALS.md boundary. Output must form one
complete expression in the existing grammar. Existing `println!` handling remains
its separate supported builtin path.

The source loader still validates complete original lexemes before attribute
filtering and child filesystem observations. Effective cfg filtering precedes
macro registration. A false-cfg definition cannot register a macro, and a later
unknown invocation is an ordinary syntax error, not a request for stock fallback.

`native_adapter` performs a charged typed-token audit and retains a canonical
flat token tree only for macro-bearing input. Ordinary macro-free functions do
not allocate lexical scope arenas or a second canonical token inventory. The
audit is still an extra charged scan, not a claim of zero time overhead.

The parser selects actual grouped ranges through checked opening `SpanId`s.
Quoted punctuation and empty strings are not delimiter or EOF sentinels. The
source driver compiles rules and invokes the pure engine in the same session
and expansion ledger. It does not print tokens, relex generated text, run a
compiler subprocess, or grant body-proof authority.

## Lexical binding identity

`semantic_scope` owns append-only checked function, block, frontier and binding
records. Every handle contains an issuing owner. Frontiers preserve the exact
source-order environment, including the point where a macro was defined.
Leaving a block restores its previous frontier rather than mutating an older
snapshot. Flat storage and backward references avoid recursive arena drop.

The driver registers a checked association between an existing expansion record,
its definition frontier and invocation frontier. A compiler-private admission
proof authorizes only that expansion, function and token session. Low-level span
or syntax-context construction alone does not issue the proof.

Generated local identifiers first resolve a matching marked lexical binding,
then the anchored definition environment. Captured identifiers retain their
incoming context and can intentionally introduce bindings in the current scope.
Item/member names use the checked invocation-side projection for this bounded
single-mark case. Label, type-parameter and lifetime-parameter projection is not
generalized by stripping marks.

Original root-context identifier admission remains unchanged. Expanded admission
is a separate compiler-private factory. It validates the actual native identifier,
exact span/context consistency, expansion record and admission proof. Root-context
captured occurrences still carry expansion provenance and must belong to the
registered function. They cannot be transplanted to another function merely
because their syntax context is root.

`Program` retains an optional immutable scope arena, and each macro-bearing
`Function` retains its checked function scope. The universal validator checks
these owners even for unused bodies. A recorded unresolved reference still has
a function association. An unrecorded occurrence or missing anchored local is a
contract failure, never an invitation to retry item resolution.

The checker maps `BindingId` to fresh IR `LocalId`. The source inventory uses the
same binding identity to select the actual structural source node. Semantic name
keys remain namespace-sensitive lookup keys, not declaration or replay identity.
The original body, plan, facts and verified identity capabilities retain their
existing separate publication contracts.

## Expanded token ingress and diagnostics

The expanded adapter walks checked slices and logical macro tokens directly.
It preserves each punctuation constituent and identifier context. Generated
coordinates do not pretend to be affine prepared-source byte ranges.

The parser's compact token part representation uses a checked disjoint tag for
prepared offsets versus expanded line/column display coordinates. Expanded
`bytes()` is a non-coordinate sentinel and is never passed to a prepared-source
mapping fallback. Exact issued spans remain the provenance authority, including
explicit expansion-owned EOF.

Expanded errors resolve their primary origin through checked span provenance and
render a bounded expansion trace. Variable message capacity is retained by a
`ByteLease`, charged to `DiagnosticBytes`, and transferred with the same String
through parser and structured source errors. These error types are not cloned
with an unaccounted String copy. The legacy final String presentation API formats
while the structured owner is still alive.

The source projection cache stores compact typed failure causes rather than
cloning a leased diagnostic. Contract messages and exact budget failures can be
recreated without rerunning the origin scan or losing the latched cause.

## Explicit boundaries

The following remain unsupported by this client:

- Multiple definitions per function, recursive or nested expansion, generated
  definitions and item-position output.
- Macro imports/exports, cross-crate `$crate`, procedural macros and richer
  fragment parsers.
- General string expression typing and richer literal classes. The accepted
  opaque numeric/bool expression and normal-string println consumer is bounded
  separately in OPAQUE-LITERALS.md.
- General nested/opaque hygiene, expanded labels/lifetimes, raw identifiers and
  Unicode semantic identifier normalization.
- Receiver-bearing contextual macro functions until the receiver has an explicit
  checked lexical binding path.
- New literal classes and expanded tuple-float decomposition requiring missing
  subrange provenance metadata.
- Macro-bearing documentation audit records without explicit source-owned
  documentation lowering. They are not silently dropped or reinjected.

All existing expansion caps remain unchanged. Work, scope transitions, probes,
old-capacity rehash scans and diagnostic construction are charged before work.
Retained arenas and temporary stacks use fallible capacity admission. Resource,
allocation and infrastructure failures remain typed failures, not `NoMatch`,
`NotFound` or unsupported-source fallback. Shared clones preserve the exact scope
arena and its original lease until its final owner drops.

## Acceptance program

The stock-first source corpus records actual `rustc -vV` independently of its
historical local baseline. Native CHECK and BUILD run at workers 1 and 8 with
compiler lookup disabled. BUILD must publish an ELF image that executes with
exact expected stdout and exit status. CHECK and rejected cases publish no
executable. Original source bytes and invalid expectations are not replaced with
preexpanded programs.

The final source matrix includes 17 stock-first cases (8 runtime successes and
9 compile failures) and 48 native worker/mode cells: five runtime clients and
seven ordinary invalid clients at workers 1/8 in CHECK/BUILD. The runtime clients
include the exact definition-site shadowing witness, retained captured context,
transparent `tt`, introduced marked binders and intentional captured binders.
The invalid clients cover source-order errors, unknown capture, missing arm,
missing helper, scope escape and false-cfg registration. Five stock-only controls
retain explicit recursion/block-item/lexical boundaries rather than being
reclassified as native invalid syntax. The focused final matrix passed in
`20261017-090119390-1209633` and is included in the complete frozen suite above.

Eighteen independent public scope controls passed in
`20261017-085415120-1177760`. Private checked-handle mutations, resource denial,
expanded diagnostics and unchanged compiler regressions also pass in the final
suite. Joint `<-` remains a single lexical/macro token but is rejected by the
expression parser; spaced `< -` remains valid. This distinction was measured
against stock before changing the native and expanded ingress paths.

Earlier failed gates are retained, including false-cfg invocation classification,
new fixture ownership/canonical-path mistakes and the final BudgetVec test API
compile repair. They are not acceptance receipts. All 18 cited successful and
failed records are in `data/runs/xps14/2026-10-17-source-macros.jsonl`.

No compiler speed claim or full Rust compatibility claim follows from this
bounded client. Generated-item discovery/cfg-before-IO, graph expansion closure,
full matcher grammar and broader hygiene remain separate implementation gates.
