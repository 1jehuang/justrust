#!/usr/bin/env bash
# Build jr-cargo exactly as the pinned toolchain's cargo identifies itself:
# same release version and channel (cargo's defaults, such as
# -Zembed-metadata=no on nightly, depend on them, and so do fingerprints).
# Usage: ./build-jr.sh [toolchain-id]   then
#        justrust toolchain cargo-fork target/release/cargo --toolchain <id>
set -euo pipefail
cd "$(dirname "$0")"
tc=${1:-nightly-2026-10-08}
stock=${JUSTRUST_HOME:-$HOME/.justrust}/toolchains/$tc/bin/cargo
release=$("$stock" -vV | sed -n 's/^release: //p')
case "$release" in
  *-nightly) channel=nightly ;;
  *-beta*) channel=beta ;;
  *) channel=stable ;;
esac
echo "jr-cargo: building as cargo $release ($channel) to match $tc" >&2
# all-static: libgit2, OpenSSL and curl compiled in, like rustup's cargo, so
# the binary runs on any glibc host (an Arch-built jr-cargo needed
# libgit2.so.1.9 and failed on Ubuntu 24.04 build hosts). Set
# JR_CARGO_FEATURES= to link the system libraries instead.
features=${JR_CARGO_FEATURES-all-static}
justrust build --release --bin cargo --locked \
  --config "env.CFG_RELEASE=\"$release\"" --config "env.CFG_RELEASE_CHANNEL=\"$channel\"" \
  ${features:+--features "$features"}
binary=${CARGO_TARGET_DIR:-target}/release/cargo
if [ -x "$binary" ]; then
  "$binary" -vV
  ldd "$binary" | grep -E 'libgit2|libssl|libcurl|libssh2' && echo "jr-cargo: WARNING: still links system libraries above" >&2 || true
else
  echo "jr-cargo: build succeeded without a local binary (hosted builds keep artifacts remote)." >&2
  echo "jr-cargo: use JUSTRUST_REMOTE=off ./build-jr.sh $tc when a local installable binary is required." >&2
fi
