# Irrefutable let-pattern milestone

This slice follows nominal aggregates and cached move analysis. It does not add
match, borrow checking, destructors, enum variants or general Rust inference.

## Source and checked representations

Source patterns are Wildcard, Binding { name, mutable }, Tuple(Vec<Pattern>) and
Struct { name, fields: Vec<(String, Pattern)>, rest }. A LetPattern statement
contains the pattern, whole-pattern type annotation (or Infer), and initializer.
Plain binding Let remains available for existing direct-AST users.

Supported syntax includes grouped patterns, unit/singleton/nested tuples, named
struct patterns, renamed fields, field shorthand, per-binding mutability and final
struct `..`. Tuple rest, ref/ref mut, @, or/literal/range patterns, array patterns,
let-else, parameter patterns and destructuring assignment remain explicit gaps.

Checked IR retains only LetPattern { value: ExprId, ty: Ty, bindings } and flat
PatternBinding { local: LocalId, fields: Vec<usize> }. Binding paths are relative
to the initializer root, in depth-first source pattern order. Each binder gets a
fresh local ID. Ignored fields and rest create no bindings or local slots.

## Typing, privacy and scope

Resolve and type the initializer in the old lexical scope. Validate the complete
pattern, shape, annotation, field completeness, duplicate fields and duplicate
binding names before installing all bindings together. Struct headers resolve
through the nominal type namespace and must match exact declaration identity.

Every explicitly projected field uses the existing field visibility and semantic
type-accessibility rules, including a field matched with `_`. An explicit public
field whose type is private is not a privacy escape. Omitted fields under `..`
are not projected, so ignored private fields remain allowed. Unit requires Unit,
tuple arity must match, and invalid dead syntax remains rejected.

For a pure-Never initializer an explicit annotation or a nominal/unit pattern may
supply the effective type. An unannotated tuple requiring backward inference from
later uses is a legal-unsupported inference boundary, not invalid Rust.

## Evaluation and ownership

Do not eagerly consume the initializer root. A pattern-source operation addresses
Local directly and recursively appends Field projections. A non-place root is
materialized exactly once. This distinction applies to temporary-rooted field
chains as well as local-rooted ones.

After initializer effects, each binding becomes StorageLive followed by assignment
from Copy or Move of its selected projected type. Existing analysis proves the
selected paths initialized and handles partial moves and joins. The source root
is not made a new lifetime. Hidden temporary roots remain alive through transfers.

Wildcard is not a binding: `_` performs no read, move or borrow. `_x` is a real
binding and consumes normally. `let (_, b) = t` reads only t.1. Ignored moved
places do not need to be initialized, but initializer effects are preserved:
`let _ = make()` still calls once, and `let _ = (s, t)` still constructs its RHS
and moves non-Copy operands. A block result `{s}` is a value evaluation, whereas
parentheses `(s)` preserve place context. Divergent initializers retain effects
and syntax typing but emit no impossible projection transfers.

## Validation

Require independent stock-classified positive/negative/boundary fixtures, bounded
native execution at workers 1/8, exact stdout, wildcard versus underscore binding,
partial/whole moved inputs, temporary roots, evaluation once, shadowing, privacy,
annotation and divergent initializers. Inspect CFGs for absence of ignored reads
and absence of a whole-root Move for place destructuring. Both graph roots and
standalone checked emission retain their existing cached-analysis contract.
