# Bounded matches macro

Status: implementation in progress. Full Rust remains open.

Add checked builtin `matches!(expression, pattern)` with optional trailing comma and all three delimiters. Reuse existing supported source pattern forms and checked match lowering: evaluate the scrutinee once, yield true for its matching arm and false for the wildcard arm. The result is Bool, subject to ordinary Never coercion/completion rules. Do not add new IR or backend operations, general match guards, range patterns, or-patterns or unsupported pattern forms in this slice.

Builtin selection follows authoritative lexical/module macro lookup. Preserve source occurrence ownership, binding scope and existing pattern metadata/ownership checks. A lowered Match must not make parenthesized/bracket macro syntax implicitly block-like for semicolon/comma omission. Guard and other unsupported syntax must fail explicitly rather than silently ignoring tokens. All original bodies and unused metadata remain validated.

Acceptance uses independently measured frozen stock sources, exact once-only side effects, Bool results, supported literal/tuple/enum/wildcard cases, local macro shadowing, strict unsupported/invalid boundaries, and CLI plus observed graph workers1/8 CHECK/BUILD gates. Public authority controls, debug/release runs, full regressions, strict lint and formatting precede publication. General Rust pattern/macro support remains open.
