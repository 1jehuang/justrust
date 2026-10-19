# Byte literals

`b'a'`, the simple byte escapes (`\n`, `\r`, `\t`, `\0`, `\\`, `\'`,
`\"`), and two-ASCII-hex-digit `b'\xNN'` are exact `u8` values. Hex escapes
cover the full 0 through 255 domain, unlike char/string ASCII hex escapes.
Unescaped non-ASCII source characters are rejected, never truncated. Unicode
escapes, empty/multiple-byte bodies, unescaped tab/newline/carriage return,
unknown escapes, malformed hex escapes and suffixes are invalid.

Original source and macro expansion admit the existing lexical Byte class as a
private parser token. Both routes decode through the same allocation-free
checked decoder. The parser constructs the existing integer literal AST with a
rigid `u8` suffix. Inference, patterns/range patterns, reference temporaries,
arrays, arithmetic, casts, ownership, source replay and code generation therefore
use existing graph facts and validation, not a parallel byte evaluator. No new
IR operations, graph kinds, side state or ratchets are introduced.

Byte strings and raw byte strings remain explicitly unsupported. They require
correct static array-reference and lifetime semantics, not treating `b"..."` as
a by-value array or a UTF-8 string. No such approximation is added here.

Validation: `justrust test --local --test byte_literals` requires stock rustc,
compiles edition 2024 with panic=abort, and compares acceptance/execution and
invalid-input rejection. It includes all 256 hex byte values, simple escapes,
ASCII/non-ASCII boundaries, rigid u8 typing, arrays/references/ranges, and macro
literal/tt forwarding. Decoder tests independently reject malformed expanded
spellings. Graph-integrity and strict all-target clippy are separate required
gates. This is a literal-syntax increment, not full Rust or library support.
