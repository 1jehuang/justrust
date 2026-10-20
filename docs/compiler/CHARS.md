# Bounded Unicode character scalars

Status: implemented and validated for this bounded slice on 2026-10-18. Full Rust remains open.

Implement a distinct Rust `char` source and runtime type, Unicode scalar literals and supported escapes, typed locals/parameters/results/fields, genuine built-in Copy/Clone/Sized, and scalar equality and ordering. Character values must remain distinct from integer values. Arithmetic, implicit integer conversion, explicit casts, and character literal/range patterns are outside this milestone. Existing wildcard/binding patterns may operate on characters without widening pattern syntax.

The source AST uses `Type::Char` and `Expr::Char(char)`. Public IR uses `Ty::Char` and `ExprKind::Char(CharValue { value: u32 })`; every validation/publication boundary must reject surrogate and out-of-range values, including unused arena entries. Native storage is private implementation detail and must preserve valid scalar values and unsigned Unicode scalar ordering. No invalid value may gain authority through a cache, clone recipe, CFG constant, or source replay.

Preserve checked source identity, metadata admission, shared resource budgets and existing diagnostics. Freeze independently measured stock sources before native expectations, then exercise CLI and observed graph CHECK/BUILD with workers1/8 and actual executable results. Add public malformed-IR and real-source authority controls. Run full regression, optimized focused gates, formatting and strict lint before marking complete. Do not infer full Rust compatibility or speedups from this bounded feature.

## Validation

The 36 byte-frozen stock sources contain 10 accepted executable cases and 26 rejected cases. Native rejection partitions are 10 syntax and 16 type errors. CLI and observed graph paths each cover 144 worker1/8 CHECK/BUILD cells and 20 executable runs. Five public authority tests and 27 optimized private character-filter tests pass. The optimized corpus and authority suite passes eight tests. Original lexical character and disabled-character witnesses are now admitted without changing their source bytes; character literal patterns remain explicitly unsupported.

All-target check, strict all-target Clippy and formatting pass. The final complete suite passes 3179 tests in 164 binaries (`20261017-193351333-2275251`). Twenty-four successful and failed redacted gate receipts are retained in `data/runs/xps14/2026-10-18-chars.jsonl`. The independently measured stock receipt SHA256 is `7bc819a16b51543c63923f0f0f6f54de9b9820fd7519789e2e7c09f4c5b12e53`. No performance speedup or full Rust compatibility is claimed.
