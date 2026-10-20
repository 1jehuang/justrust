# Macro receiver occurrence authority

Status: focused implementation of receiver identity and binding replay, not full
macro-generated callable, Iterator or Rust support. Initial prototype parent is
`f17ed225394f452e1f9a72a2d8d83258c82b9b23`. Publication requires the focused
stock/native and authority/graph gates below plus graph integrity on its actual
parent. Generated callable container/header issuance remains a separate gate.

## Frozen contract

* A bare value `self` retains its actual token `AstIdent`, including source owner,
  occurrence span and expansion context. It is not reconstructed from its text,
  nor represented only by an empty `SelfModule` path.
* `ReceiverOccurrence` has a private constructor. Public AST transport may clone
  an issued occurrence, but cannot mint a receiver from an arbitrary identifier.
  A copied occurrence is not authority to use another method's receiver.
* A function's immutable lexical root retains its parser-issued receiver binder.
  Existing hygienic lexical lookup must resolve each use to that exact binder
  in that exact function. Canonical receiver local IDs remain unchanged.
* The parser binds value, mutable value, shared and mutable-reference receiver
  tokens before ordinary parameters. Original functions without lexical macro
  scopes retain their existing nonmacro receiver behavior. Synthetic internal
  `LocalName::Receiver` is not a substitute for a missing macro source occurrence.
* Receiver place conversion, assignment and indexed borrows preserve the sealed
  occurrence. Resolver, complete name replay, original-source inventory and typed
  local lookup check it before collapsing onto the canonical receiver key.
* `self::module_item` remains module-path syntax and `Self::Item` remains type
  syntax. A receiver occurrence cannot be structurally edited into another path
  category. No macro mark is stripped or normalized to the root context.
* Definition-site and invocation-site receiver uses follow existing hygienic
  lookup, including nested expansion anchors and shadowed nonreceiver locals.
  A macro defined outside a receiver's scope cannot capture it by spelling self.
* Module imports retain the checked original module/source-crate selection.
  Their definition-site local frontier is empty, not the invocation frontier.
  This also prevents ordinary local-name leakage. Metavariable captures keep
  their actual invocation contexts and local definitions keep their real
  definition environments.

## Graph and IR shape

There is no new compiler node, global map, shared mutable fact, whole-program
join or IR operation. The existing immutable source/scope facts carry one
optional receiver token per function root. Scope allocations remain metered by
`BudgetVec` using the actual row size. Exact reference and binder lookups use
existing charged scope/name operations. Existing source admission is required
before the receiver protocol, including generated callable header/container
certificates. CHECK does not demand emission or linking.

## Acceptance

* `tests/macro_receivers.rs`: stock Rust versus native CHECK/BUILD/run at workers
  1 and 8 for definition/invocation tokens, nested uses, nonself shadowing,
  mutable receiver writes and distinct module-self syntax.
* `tests/macro_receivers_authority.rs`: unused invalid bodies, foreign definition
  context, cross-method receiver transplantation, foreign scope/source, omitted
  receiver kind/scope/occurrence and path-category mutation.
* `tests/macro_receivers_graph.rs`: no-codegen CHECK at workers 1 and 8 and the
  same existing node keys for ordinary and macro receiver bodies.
* `justrust test --test graph_integrity -j1`: unchanged ratchets mandatory.

## Observed validation

On the frozen initial parent, the four focused targets passed 18 tests, including
all 14 unchanged graph-integrity gates (`20261019-115159620-3829948`). The first
negative campaign exposed a genuine module-import definition-frontier leak:
module-defined literal `self` incorrectly captured the invocation receiver. The
repair separates the authenticated module definition environment from function
locals. Six stock-invalid unused bodies, including nested receiver and ordinary
local-name leakage, now reject CHECK and BUILD at workers 1 and 8. Six positive
programs execute against stock Rust at both worker counts. The sealed-origin
API then passed check, authority controls, all 14 graph-integrity tests and
focused clippy (38 pre-existing warnings, no receiver-module warnings). Exact
parent `7570b1e042db4a13ac573602df14d6badec78a46` refreshed cleanly, preserving
alignment, callable inference, runtime-site and mutable-range changes. Its
bounded check plus both authority tests and no-codegen graph test passed
(`20261019-115707761-3868158`). Selected receipts live in
`data/runs/macro-receivers/validation.jsonl`.

The existing expression-only macro output boundary is unchanged. Statement
mutation fixtures use a block expression, not an ignored trailing semicolon.
Generated associated items additionally require their independently authenticated
container/header protocol. Receiver capture into closures, arbitrary macro
products and full Iterator integration are not implied by this feature alone.
