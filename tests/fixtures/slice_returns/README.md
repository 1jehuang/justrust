# Slice return stock oracle

`transport.rs` was compiled using stock rustc 1.98.1 (48a229cea 2026-09-01), edition 2024, Linux x86-64, with `-C panic=abort`. Its successful stdout is frozen in `transport.stdout`; both files are covered by SHA256SUMS. The fixture covers generic shared and mutable slice return descriptors, shared subslice return, reference-free non-Copy element transport, and an empty input. It does not claim raw constructor or mutable range support.
