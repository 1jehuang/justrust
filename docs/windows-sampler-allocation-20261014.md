# Windows sampler allocation failure, October 14, 2026

Jcode v97's x86_64 Windows job using justrust alpha17 aborted at
09:19:29 UTC with `memory allocation of 30064771072 bytes failed`, following
cargo profile warnings and without a justrust report. The inspected log was
`v97-failures.log` supplied by the coordinating session.

## Root cause assessment

`record::tree_of` treated parent PID links as an acyclic tree. It appended a
`ProcStat` and pushed its PID for every edge, with no visited set. A self-parent
or reachable cycle therefore grows the output vector forever. On x64 a
`ProcStat` occupies 56 bytes. A vector growth to 536,870,912 entries requests
exactly **30,064,771,072 bytes**, matching the abort. Cloned names consume
additional memory before that allocation.

Windows Toolhelp parent IDs identify the original creator, not necessarily a
currently living parent. Once that creator exits its PID may be reused,
producing false ancestry, including cycles. Microsoft's explanation:
https://devblogs.microsoft.com/oldnewthing/20150403-00/?p=44313

The actual runner process snapshot was not retained, so attribution of this
particular abort remains highly likely, not a native Windows reproduction.
The bug itself is demonstrable from the traversal and synthetic cyclic input.
The Windows API buffers inspected are fixed-size, not allocated from machine
memory size or a reported API return length.

## Repair

Visit each PID at most once. Reject an edge when the alleged parent's start
is later than the child's. Preserve equal start times, since timestamp
resolution may put valid processes in the same tick. Preserve descendants
when the root has disappeared between enumeration and stat reads. This bounds
traversal by the snapshot's process count and excludes obviously reused parent
identities without changing cargo commands or caches.

Platform-independent synthetic regressions cover self-parenting, two- and
three-process cycles with equal start times, duplicate entries, reused parent
IDs, valid descendants, absent roots, and the exact x64 allocation arithmetic.
They do not constitute native Windows API tests.

## Immediate alpha17 mitigation

For the Windows build only, set `JUSTRUST_DISABLE=1`, retaining `--local`,
`JUSTRUST_TOOLCHAIN=system`, and `JUSTRUST_SLOTS=0`. Alpha17's
`record::run` checks `should_record` before setup, and directly executes real
cargo when disabled. On Windows `sys::exec` waits and propagates its exit code.
This bypasses sampler and rustc instrumentation, not compilation or release
validation. CI disables the build daemon by default. No release is needed.
No Jcode files or release state were changed by this investigation.
