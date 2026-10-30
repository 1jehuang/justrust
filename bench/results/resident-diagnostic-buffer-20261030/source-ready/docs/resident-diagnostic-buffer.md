# Resident diagnostic text buffer experiment (v20)

This is a source-ready hypothesis in the licensed Blaze-derived experimental resident engine, not the clean-room engine. It is not a measured speedup or an approved default promotion.

`errors_of` now owns one initially empty `RefCell<String>` per call. Its `asked` closure temporarily borrows that storage for each function source, clears the length, and appends every `SyntaxText::for_each_chunk` chunk without normalization or filtering. The helper hashes the resulting complete UTF-8 string with `FxBuildHasher.hash_one(scratch.as_str())`. The original owned `String` hashes through the same Rust string hash semantics, including the string terminator. Comments, whitespace, Unicode, and all other trivia remain in the input. Hash collision properties are unchanged. Hashing chunks separately would not be equivalent and is deliberately avoided.

Only allocation storage is reused. There is no persisted or shared scratch, query wrapper, new cache, altered cache key, diagnostic selection/filter change, or root graph/function-cache policy change. The borrow ends before the existing function hash insertion and selection logic. Storage is dropped on return from `errors_of`, retaining at most the largest examined function's capacity during that invocation.

Two focused native tests in the existing test module have names prefixed `resident_diagnostic_buffer`. They compare complete strings and Fx hashes against the original `text().to_string()` path across empty syntax, Unicode, comments and trivia, nested syntax, token/chunk boundaries, and multiple functions. A large-to-small-to-empty sequence additionally checks capacity and allocation-pointer reuse.

Source-only handoff checks: standalone rustfmt with child-module formatting disabled, diff whitespace validation, local SyntaxText API inspection, and helper/caller inspection. No builds, tests, native workloads, source commits, or performance measurements were run for this handoff. Root owns correctness/error/recovery gates and matched warm runtime measurements. Reject this small allocation hypothesis if it does not improve real agent wait.
