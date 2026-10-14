# Working on the owned analyzer

This source is part of justrust at `toolchain/rustc/src/tools/rust-analyzer`.
The justrust root `AGENTS.md` controls development and validation.
No separate upstream reviewer, approval, branch, or AI-use policy applies.

Retained technical references:

- [Architecture and source layout](architecture.md)
- [Testing conventions and fixtures](testing.md)
- [Source style and implementation idioms](style.md)

Use `justrust check` and focused `justrust test -p <crate>` commands from this
workspace when testing analyzer changes outside compiler bootstrap.
