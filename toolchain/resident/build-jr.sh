#!/usr/bin/env bash
# Build the owned, source-identified resident engine, never an upstream download.
# Local build is required: the engine uses the exact compiler pinned below and
# the full vendored workspace for its build identity. Shipping hosted compiler
# substitution is not interchangeable with this bootstrap toolchain.
set -euo pipefail
if [[ $# == 1 && "$1" == --help ]]; then
  echo 'usage: toolchain/resident/build-jr.sh TOOLCHAIN_DIRECTORY [INSTALL_DIRECTORY]'
  echo 'Builds via justrust with the pinned native toolchain, then installs jr-native.'
  echo 'Default install directory: ${CARGO_HOME:-$HOME/.cargo}/bin'
  exit 0
fi
if [[ $# -lt 1 || $# -gt 2 ]]; then
  echo 'error: expected TOOLCHAIN_DIRECTORY [INSTALL_DIRECTORY], see --help' >&2
  exit 2
fi
ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)
TC=$(cd "$1" && pwd -P)
DEST=${2:-${CARGO_HOME:-$HOME/.cargo}/bin}
if [[ "$DEST" != /* ]]; then DEST="$PWD/$DEST"; fi
PINS="$ROOT/crates/ra-jit/src/cargo/toolchain/pins.txt"
EXPECTED=$(awk '$1 == "commit" { print $2 }' "$PINS")
ACTUAL=$("$TC/bin/rustc" -vV | awk '$1 == "commit-hash:" { print $2 }')
if [[ -z "$EXPECTED" || "$ACTUAL" != "$EXPECTED" ]]; then
  echo "error: native engine requires pinned rustc commit $EXPECTED, found $ACTUAL" >&2
  exit 1
fi
if [[ ! -f "$TC/lib/rustlib/src/rust/library/Cargo.toml" ]]; then
  echo 'error: pinned toolchain needs the rust-src component' >&2
  exit 1
fi
command -v justrust >/dev/null || { echo 'error: install the stable justrust client first' >&2; exit 1; }
export PATH="$TC/bin:$PATH"
export JUSTRUST_REAL_CARGO="$TC/bin/cargo" JUSTRUST_REAL_RUSTC="$TC/bin/rustc" RUSTC="$TC/bin/rustc"
export JUSTRUST_TOOLCHAIN=system JUSTRUST_REMOTE=off JUSTRUST_BUILDD=0 JUSTRUST_BUILDD_PREBUILD=0
export JUSTRUST_NO_TELEMETRY=1 JUSTRUST_PREBUILT=off JUSTRUST_PROCMACRO_CACHE=0
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$ROOT/target}
unset RUSTC_WRAPPER RUSTC_WORKSPACE_WRAPPER RA_JIT_GIT_SHA
cd "$ROOT"
justrust build --release --locked -p ra-jit --bin jr-engine
ENGINE="$CARGO_TARGET_DIR/release/jr-engine"
VERSION=$("$ENGINE" --version)
if [[ ! "$VERSION" =~ ^jr-engine\ experimental.*build\ resident-v1-[0-9a-f]{64}\)$ ]]; then
  echo "error: refusing an unidentified or non-owned native engine: $VERSION" >&2
  exit 1
fi
mkdir -p "$DEST"
# Publish a complete executable atomically, without replacing the shipping client.
STAGED=$(mktemp "$DEST/.jr-native.XXXXXXXX")
trap 'rm -f "$STAGED"' EXIT
cp "$ENGINE" "$STAGED"
chmod 755 "$STAGED"
mv -f "$STAGED" "$DEST/jr-native"
mkdir -p "$DEST/jr-native-licenses"
cp "$ROOT/LICENSE-MIT" "$ROOT/LICENSE-APACHE" "$ROOT/JUSTRUST-SOURCE.md" "$DEST/jr-native-licenses/"
printf 'Installed %s\n%s\n' "$DEST/jr-native" "$VERSION"
printf 'Use: justrust native test --lib -- TEST_FILTER\n'
