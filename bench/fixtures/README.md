# Pinned PTY acceptance input

`terminal_setup_command.abff94eeb.rs` is an exact test-only copy of
`crates/jcode-tui/src/tui/app/tests/terminal_setup_command.rs` from Jcode commit
`abff94eeb`. Its SHA256 is checked by `bench/test_pty_fixture.py` against
`bench/pty_fixture.py`. It is not compiled as part of justrust or copied into
measured snapshots. Those are extracted from the immutable Git commit and
validated independently before applying the overlay.

`pty_failure_acceptance.rs` is acceptance-only failure injection, never a
benchmark overlay and never part of the measured project test inventory.

The copied Jcode test input is distributed under its original license:

MIT License

Copyright (c) 2025 Jeremy Huang

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
