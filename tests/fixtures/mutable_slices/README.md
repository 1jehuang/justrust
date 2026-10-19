# Mutable slice witnesses

Oracle: stock rustc 1.98.1, edition 2024, Linux x86-64, `-C panic=abort`.
`ops.stdout` and `order.stdout` are the captured successful outputs of the corresponding unchanged `.rs` sources. `SHA256SUMS` pins sources and outputs.

The native acceptance suite runs each source through CHECK and BUILD at workers 1 and 8, then compares stdout. Separate generated bounds probes verify native trap behavior, not numeric exit-status parity with stock aborts.
