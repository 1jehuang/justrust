# Raw slice fixture

`transport.rs` is compiled without source rewriting by stock
`rustc 1.98.1 (48a229cea 2026-09-01)`, edition 2024, `-C panic=abort`.
Its successful exit and exact stdout are frozen alongside the source hashes.
It covers shared/mutable generic wrappers, Copy writes, non-Copy element
transport, zero-length slices from aligned non-null dangling pointers and
pointer-before-length evaluation. It does not dereference dangling storage.

The native acceptance harness uses the same source at workers 1 and 8 in CHECK
and BUILD/run modes. This is a bounded feature witness, not evidence of full
Rust or library compatibility.
