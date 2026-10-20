# Nested original-source local macros

Status: the bounded nested original-source client is accepted after opaque
literal checkpoint `db575a416`. The frozen hosted gate passes 2,105 tests across
104 binaries in `20261017-094044045-1387251`, all-target check
`20261017-094041236-1386804`, strict all-target lint
`20261017-094102336-1388370`, formatting and whitespace checks.
SOURCE-MACROS.md and OPAQUE-LITERALS.md retain the historical earlier boundaries.

## Added bounded client

This native slice admits multiple original-source function-local macro_rules
definitions, source-order redefinition, block shadowing and nested/recursive
expression invocations of those definitions. It does not admit generated macro
definitions, item expansion, macro exports/imports, path-qualified macros,
procedural macros or general stacked syntax contexts.

Definitions are immutable charged records in an append-only driver arena. A
macro lookup scans newest to oldest, charging each candidate and checking both
the Macro namespace key and the exact lexical visibility frontier. Definitions
not yet encountered by the source parser cannot be selected. Leaving a block
makes its definitions inaccessible rather than deleting or rebinding records.

Selection returns a private owner-qualified definition handle together with its
exact definition and call frontiers. Expansion consumes that selection, never a
second name lookup. A selected arm failure does not retry an older same-name
definition. Macro textual lookup uses the actual invocation frontier, not local
variable definition-site projection or a printed name.

## Nested proof and context authority

An original definition's tokens have root contexts. Its generated literal
identifiers therefore carry one semi-transparent mark with a root parent. A
nested original definition introduces its own such mark; forwarded captures
retain their incoming root or ancestor-expansion mark. This client does not
require admitting arbitrary stacked marks or generated definitions.

The parser passes the active admission proof's expansion as the actual call's
parent. A nested anchor must verify the parent proof against the immutable arena
record, the exact child expansion parent, the checked call occurrence, matching
function/session/ledger ownership and the selected lexical frontiers. A span or
syntax-context handle alone cannot manufacture this association.

Every output token's occurrence span must still belong to the current admitted
expansion. Only its single semi-transparent context mark may reference a checked
ancestor. Ancestor traversal is iterative, work-charged and bounded by the
existing expansion depth limit. Sibling, foreign, unregistered and conflicting
parent associations fail closed. Root-context captures do not bypass occurrence
or function checks.

Local lookup retains the existing exact-context lexical binding search followed
by that mark's own definition frontier. An inner definition's generated free
local therefore uses the inner definition environment, while an outer-marked
capture retains its outer binder/environment. Item/member projection remains
the bounded original-source single-mark policy, not global mark stripping.

## Native input and lifetime

The expanded buffer retains its actual checked TokenSlice. Nested invocation
arguments are selected from that stream using issued group spans, not searched
in the original input or printed and relexed. Opaque literal captures remain
opaque matching input through nested forwarding. The final semantic consumer
still uses its isolated one-expression parser from OPAQUE-LITERALS.md.

The parser keeps the checked absolute expansion depth in its child frame through
expansion, lowering and the entire child parse, alongside the existing syntax
nesting budget. A counter alone is not proof of native-stack safety. The first
real recursive gate `20261017-093040913-1342385` overflowed its Rust test thread
stack before reaching the typed limit, despite passing all-target compilation
and 824 library controls. That receipt is a failure, not acceptance. The repair
uses fallible charged singleton root owners and tiny dispatch/preparation
helpers to keep large scratch frames off the recursive chain. The unchanged
public recursive control returns typed Limit in `20261017-093515329-1362645`
without changing stack settings, sources or caps. That intermediate run still
failed the separate matcher-punctuation transport gate and is not final acceptance. Recursive failure must be a typed resource denial, not stack overflow,
Unsupported, NoMatch retry or a reset nesting counter. Work/output/invocation
accounting is monotonic across failed nested calls. Existing limits are unchanged.

## Acceptance evidence

The added stock-first corpus contains 17 exact sources (11 runtime successes and
6 rejections), with 68 native workers-1/8 CHECK/BUILD cells. Typed public parser
controls independently pin syntax, committed-arm type checking and recursive
resource failure. The focused unchanged source/literal plus new nested matrices
pass 8 tests in `20261017-093821761-1376557`. All exact scoped receipts, including
the genuine stack failure and intermediate fixture compile error, are retained in
`data/runs/xps14/2026-10-17-nested-source-macros.jsonl`.

Canonical macro-bearing streams transport lexically valid punctuation outside
the existing expression grammar as DeferredUnsupported, never as fake semantic
punctuation. Matcher-only countdown tokens can be captured/discarded; direct
semantic consumption remains explicitly Unsupported. Macro-free and isolated
literal paths retain their prior gates. The final full suite includes lowered
expansion-depth, poisoned-work, owner/container lifetime and wrong-proof controls.
No native stack-size, harness, resource cap or existing source corpus was changed.

The following acceptance requirements remain invariants of this client:

Stock-first sources must cover two distinct definitions, source-order
redefinition, block shadow/restore, scope escape, before-definition errors,
selected-definition commitment, literal forwarding opacity versus transparent
tt forwarding, nested definition-site/captured locals, repeated fresh expansion
binders and terminating recursion. Unbounded recursion must fail with a typed
resource limit. Generated-definition and broader context boundaries must remain
explicit.

Actual native CHECK/BUILD at workers 1 and 8 must preserve no-compiler-fallback,
ELF execution/stdout/exit checks and no-artifact failures. Private controls must
reject wrong parent proofs, sibling/foreign contexts, reused selected handles
from another driver/function, poisoned ledgers and invalid current streams.
The full unchanged regressions, all-target check, strict lint, formatting and
recorded exact run receipts are required. No full Rust or performance claim
follows from this slice.
