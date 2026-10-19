# Raw text and byte literals

Status: native literal value implementation, Linux x86-64, edition 2024.
This is one G1/G2/G3/G5/G6/G10 slice, not full Rust compatibility.

## Frozen contract

- `r"..."` and `r#"..."#` through 255 hashes decode to `&'static str`.
  Only a quote followed by the full opening hash count closes a raw literal.
  Backslashes are ordinary content. Unicode, embedded NUL and CRLF normalization
  follow Rust's lexer rules.
- `b'...'` remains a rigid `u8` through the existing byte-character path.
- `b"..."` and `br#"..."#` produce **`&'static [u8; N]`**, where N is the
  decoded byte count. Non-ASCII source characters and Unicode escapes are invalid
  in byte literals. Normal byte strings admit `\x00` through `\xff`, simple
  escapes and escaped-newline whitespace continuation. Raw bytes do not unescape.
- Byte data can contain NUL, including source NUL. It is not UTF-8 text and is
  not implicitly NUL-terminated. Empty byte strings still have a nonnull pointer.
- Existing expected-site shared array-to-slice coercion creates `&[u8]` values.
  Indexing and `.len()` of shared array references use that same checked carrier,
  evaluating the source exactly once. Const initializers reuse ordinary constant
  typing and inlining, including unused initializer checks.
- Static storage is native program data, never a stack temporary or compiler-side
  shared mutable table. An explicit static origin is issued by the borrow task
  only for the byte-literal CFG operation. No reference type grants provenance.
- C and raw C strings remain explicitly Unsupported. They never become `&str`.
  Other language gaps, including explicit lifetime annotations on local references
  and unsupported reference-parameter lifetime forms, remain separately diagnosed
  rather than approximated.

## IR, CFG and graph

`Expr::ByteStr(Vec<u8>)` becomes an exact-length immutable typed
`ir::ExprKind::ByteStr`, then operand-free `cfg::Rvalue::ByteStr(Arc<[u8]>)`.
IR validation requires exact shared byte-array type and returning completion.
The backend emits a thin pointer relocation into owned program data, using
exactly the existing private array element layout for loads and slice indexing
rather than packed UTF-8 storage. Slice coercion alone introduces a data/length
descriptor. Raw UTF-8 strings reuse the existing Str operation. Scanner validation
is shared with lexical token ingress. Const slice initializer checking still
passes through the ordinary hidden const-body checker and requires the shared
slice-return admission, without weakening return provenance validation.

No new node kinds, mutable published facts, whole-program joins or graph baseline
changes. Payloads travel through the existing per-body/lower/object facts. CHECK
must not demand native codegen.

## Acceptance gates

`tests/raw_byte_literals.rs` compares identical sources against stock rustc,
CHECK, BUILD and native execution at workers 1 and 8, including Unicode raw text,
multiple delimiters, CRLF, escapes, high bytes, actual and escaped NUL,
continuations, empty arrays, static returns, const slices, opaque macro forwarding
and the unchanged nix-base32 alphabet initializer. Invalid lexical, extent,
mutability, string/byte typing, unused-body and frame-escape controls must reject.
`raw_byte_literals_authority.rs` rejects forged type, extent and completion.
`raw_byte_literals_graph.rs` checks immutable CFG payload and CHECK codegen
exclusion. The unchanged `graph_integrity` gates are mandatory.
