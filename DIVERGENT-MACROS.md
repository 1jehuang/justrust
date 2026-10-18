# Bounded divergent builtin macros

Status: implementation in progress. Full Rust remains open.

Add `unreachable!`, `todo!`, and `unimplemented!` through the existing checked literal panic path. Admit empty and single string-literal forms, optional trailing comma and all three delimiters. Preserve stock default/custom message prefixes as independently measured on the installed stock compiler. No formatting arguments or nonliteral message expressions in this slice.

Select builtins only after authoritative lexical/module macro lookup misses. Preserve checked invocation span and macro expansion call-site location, bounded literal decoding and storage accounting. Lower to existing SourcePanic/IR Panic with exact Never type and Never completion, terminal CFG and no result assignment. Reuse the existing hardened Linux x86-64 stderr/SIGABRT backend without new runtime or IR variants. Native panic text remains the documented diagnostic subset, not complete stock stderr equivalence.

Acceptance requires stock-first frozen source receipts, live/dead and default/custom cases, exact native message/location, signal 6 with no postpanic effects, lexical shadowing, explicit unsupported cases, all-arena metadata checks, and workers1/8 CHECK/BUILD through CLI and observed graph. Root runs debug/release acceptance, complete regressions, formatting and strict lint before committing. Do not weaken existing panic, assertion or output fixtures.
