# Bounded character casts

Status: implemented and validated on 2026-10-18 (tests/char_casts.rs, tests/char_cast_authority.rs). Full Rust remains open.

Support explicit Rust casts from `char` to each supported signed or unsigned integer type, and from `u8` to `char`. Preserve Unicode scalar identity until the explicit integer conversion, including truncation for narrow integer destinations. Reject every other integer-to-character conversion and float, Bool, reference or aggregate conversions rather than extending the cast domain implicitly.

Use distinct checked `CharToInt` and `U8ToChar` operations. Existing integer casts remain integer-only. Validate operand domains, exact result types and completion for every IR arena entry and CFG operation, including unused rows. Preserve source identity, single evaluation, shared budgets, and malformed-scalar rejection. No character arithmetic, character patterns or general conversion traits are delivered by this milestone.

Freeze independently compiled stock sources before native expectations. Verify CLI and observed graph CHECK/BUILD with workers1/8, actual output, narrow and wide conversions, invalid dead code and public authority forgery controls. Run debug/release focused gates, the complete regression suite, formatting and strict lint before marking complete. Retain failed and successful receipts and document historical expectation changes without changing source witnesses.

## Validation

16 executable positives and 22 type rejections frozen against stock rustc; CLI and observed-graph CHECK/BUILD with workers 1/8 pass, plus public authority controls. Full local suite: 3248 tests passed. Oracle tests need local `/usr/bin/rustc` (run with `--local`).
