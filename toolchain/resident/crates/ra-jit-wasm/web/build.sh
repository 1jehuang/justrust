#!/bin/sh
# Builds what the page loads into `pkg/`: the compiler as wasm32, and the source of `core`.
# Needs the `rust-src` component. Serve this directory afterwards, with any static server:
#
#     python3 -m http.server -d crates/ra-jit-wasm/web 8377
set -eu
cd "$(dirname "$0")/.."
target=../../target
mkdir -p web/pkg

cargo build --release --lib --target-dir "$target" --target wasm32-unknown-unknown
cp "$target/wasm32-unknown-unknown/release/ra_jit_wasm.wasm" web/pkg/

cargo run --release --target-dir "$target" -- \
    bundle "$(rustc --print sysroot)/lib/rustlib/src/rust/library" web/pkg/core.bundle
