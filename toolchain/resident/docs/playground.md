# The playground

`site/playground/` is a page of the website where ra-jit compiles a Rust program to
WebAssembly inside the browser and runs it. It is an experiment, and smaller than the daemon.

## What runs in the browser

The compiler is ra-jit's own: rust-analyzer's parser, name resolution and type inference, then
ra-jit's code generator, built for `wasm32-unknown-unknown` as one module (`crates/ra-jit-wasm`).
It runs in a worker of the page. No server compiles anything, and neither rustc nor LLVM is
involved at any point: the page is static files.

The first build of a program compiles it together with every function of `core` it reaches,
from `core`'s source, which is downloaded with the compiler. After an edit ra-jit compiles the
functions whose code changed and writes a small module with them, and the page instantiates
it over the running program's memory and function table. Statics are left alone, so the
program keeps its state.

## What it does not do

- **One file, `core` only.** The program is a single `#![no_std]` file. There is no `std` and
  no `alloc`, so no `Vec`, `String` or `println!`, and no crates from crates.io.
- **No procedural macros.** `macro_rules!` and the built-in macros work. There is no rustc to
  build a procedural macro with.
- **Few error messages.** ra-jit has no diagnostics of its own. A function that does not parse
  or type-check is reported by name with a terse reason, and the program keeps running with
  the last version of it that compiled. Nothing checks borrows or moves, so code rustc would
  reject can compile here.
- **Not the native code generator.** Natively ra-jit emits machine code with Cranelift or
  LLVM. For WebAssembly it lowers the same Cranelift IR with a lowering of its own
  (`wasm.rs`), which is newer and less tested, and the page's linker is a small one written
  for it rather than `wasm-ld`. Nothing is optimised across functions.
- **It can be wrong.** If a program misbehaves in the page, check it with rustc.
- **Only what the page provides.** The program draws through four functions the page gives it
  (`fill`, `rect`, `circle`, `text`) and the page calls its `frame` and `click`. There is no
  DOM access and no `wasm-bindgen`.

The larger experiment, a Dioxus app with `std` and its dependencies edited live in a page, is
in `crates/ra-jit-wasm/web` (`build-dioxus.sh`) and described in
[reports/WASM.md](reports/WASM.md). It is not on the website: it links into a module rustc
built, downloads a 74 MB pack of the workspace's source, and its compiler needs about 1 GB of
memory.

## How it is built and served

`crates/ra-jit-wasm/web/build-site.sh` builds the compiler and the bundle of `core`'s source,
runs `check.mjs` on them under Node, and puts them gzipped into `site/playground/ra-jit/`,
which is not committed. `.github/workflows/site.yaml` runs it before uploading `site/`. If it
fails the site is deployed without `ra-jit/pkg/`, and the page says the demo is unavailable.

To look at it locally, under a path as GitHub Pages serves it:

    crates/ra-jit-wasm/web/build-site.sh
    mkdir -p /tmp/pages && ln -sfn "$PWD/site" /tmp/pages/ra-jit
    python3 -m http.server -d /tmp/pages 8000    # http://localhost:8000/ra-jit/playground/

Every URL in the page is relative. It needs no `SharedArrayBuffer` and no cross-origin
isolation. The files are gzipped by the build and inflated in the worker with
`DecompressionStream`, so the download does not depend on the host compressing `.wasm`.

Measured once, in Chrome 137 on Linux against a local server, so with no network time:

| | |
|---|---|
| Downloaded on a cold load | 6.9 MB (compiler 5.5 MB, source of `core` 1.4 MB, the page 30 KB) |
| Compiler, inflated | 27.0 MB |
| First paint | 0.1 s |
| First program running | 3.6 s after the page opened (analysis 1.9 s, code generation 1.4 s) |
| An edit to one function | 40 to 100 ms |
| Compiler memory | about 210 MiB |

It has not been run in Firefox or Safari.
