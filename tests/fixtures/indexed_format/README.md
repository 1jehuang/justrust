# Frozen indexed format fixtures

Oracle: stock rustc 1.98.1, edition 2024, Linux x86-64.
Command: `rustc --edition=2024 <fixture>.rs -o <binary>`, then execute the binary.
`SHA256SUMS` freezes source and recorded stdout. The transport fixtures exit successfully.
The transport includes side-effect indices, shared and mutable slices, and non-Copy library Display operands.
