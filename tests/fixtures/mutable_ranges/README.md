# Frozen mutable ranges fixtures

Oracle: stock rustc 1.98.1, edition 2024, Linux x86-64.
Command: `rustc --edition=2024 <fixture>.rs -o <binary>`, then execute the binary.
`SHA256SUMS` freezes source and recorded stdout. The transport fixtures exit successfully.
`order` exits successfully with recorded stdout. `reverse`, `beyond`, and `overflow` print `before\n` and panic with the stock default unwind exit 101. Native tests assert trap/stdout parity only, not panic text or general exit-code parity.
