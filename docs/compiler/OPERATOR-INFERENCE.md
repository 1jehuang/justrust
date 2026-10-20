# Canonical body-local operator inference

Status: operator-term inference correctness repair. This does not claim complete
Rust operator compatibility or complete Rust type inference.

## Frozen contract

A function-body solver records independent receiver, RHS and result terms for
binary, unary and compound-assignment operations. A nominal receiver must not be
equated with the RHS or associated Output. Its canonical embedded `std::ops` or
`std::cmp` declaration and retained method token select the same validated trait
application used by authoritative expression lowering. The selected signature
constrains RHS and Output separately. Unknown receivers stay pending, not fake
primitive or opaque types. Numeric fallback waits for constraint quiescence.
Unsuffixed numeric RHS terms first reuse canonical application discovery and
selection: exactly one proved numeric application may constrain the width.
Receiver-matching applications with unresolved impl parameters remain deferred,
not disproved. Multiple widths retain ordinary numeric fallback/ambiguity, and
nominal `Self` applications cannot consume integer or float literal domains.

Primitive arithmetic retains its operand type constraints. Shift RHS integer
widths remain independent. Comparisons produce bool; canonical comparison
signatures borrow operands during authoritative lowering, not in inference.
Nested expression hints also select the canonical signature's Output, rather
than pretending an overloaded binary or unary result has its receiver's type.

All scratch is local to one body operation. No new graph kinds, immutable fact
shapes, shared budgets, locks or whole-program joins are introduced. Solved hints
still use inferred expectation provenance and cannot authorize a reborrow, move,
callee, trait application, source occurrence or associated equality.

## IR and CFG

There is no new operator IR or backend path. Authoritative lowering retains the
existing exact checked trait call, selected target, signature, source call
identity and equality/carrier evidence. CFG and ownership validation consume that
original checked call. A hint alone never publishes an executable.

## Acceptance

- The unchanged `operator_overloading` regression checks Add/Sub/Mul<i32>/Neg/
  AddAssign, including the original `V * 3` blocker and missing-impl rejection.
- `operator_inference` compares stock and native CHECK/BUILD/run at workers 1/8
  for different receiver/RHS/Output, nested outputs, exact distinct RHS impls,
  compound RHS widths and independent primitive shifts. Invalid unused bodies,
  missing or shadow trait impls, malformed signatures and contradictory results
  remain rejected for CHECK and BUILD.
- `operator_inference_authority` starts from real canonical operator calls and
  rejects foreign facts/targets, reordered operands, a changed RHS formal and a
  changed Output before checked IR and CFG ingress.
- Graph-integrity, full-program harness and existing scalar/callable inference
  authority gates must continue passing.
