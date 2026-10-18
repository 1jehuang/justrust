# Bounded Unicode character scalars

Status: pending implementation and validation. Full Rust remains open.

Implement a distinct Rust `char` source and runtime type, Unicode scalar literals and supported escapes, typed locals/parameters/results/fields, genuine built-in Copy/Clone/Sized, and scalar equality and ordering. Character values must remain distinct from integer values. Arithmetic, implicit integer conversion, explicit casts, and character literal/range patterns are outside this milestone. Existing wildcard/binding patterns may operate on characters without widening pattern syntax.

The source AST uses `Type::Char` and `Expr::Char(char)`. Public IR uses `Ty::Char` and `ExprKind::Char(CharValue { value: u32 })`; every validation/publication boundary must reject surrogate and out-of-range values, including unused arena entries. Native storage is private implementation detail and must preserve valid scalar values and unsigned Unicode scalar ordering. No invalid value may gain authority through a cache, clone recipe, CFG constant, or source replay.

Preserve checked source identity, metadata admission, shared resource budgets and existing diagnostics. Freeze independently measured stock sources before native expectations, then exercise CLI and observed graph CHECK/BUILD with workers1/8 and actual executable results. Add public malformed-IR and real-source authority controls. Run full regression, optimized focused gates, formatting and strict lint before marking complete. Do not infer full Rust compatibility or speedups from this bounded feature.
