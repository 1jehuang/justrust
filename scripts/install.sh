#!/bin/sh
# justrust installer: curl -fsSL https://jcode.sh/rust.sh | sh
#
# Environment:
#   JUSTRUST_INSTALL_DIR  install directory (default: ~/.local/bin)
#   JUSTRUST_BASE_URL     release asset base URL
#                         (default: https://github.com/1jehuang/justrust/releases/latest/download)
#   JUSTRUST_NO_LOGIN     if set, skip running `justrust login`
set -eu

BASE_URL="${JUSTRUST_BASE_URL:-https://github.com/1jehuang/justrust/releases/latest/download}"
INSTALL_DIR="${JUSTRUST_INSTALL_DIR:-$HOME/.local/bin}"

say() { printf 'justrust: %s\n' "$*"; }
err() { printf 'justrust: error: %s\n' "$*" >&2; exit 1; }

detect_target() {
    os=$(uname -s)
    arch=$(uname -m)
    case "$arch" in
        x86_64 | amd64) arch=x86_64 ;;
        aarch64 | arm64) arch=aarch64 ;;
        *) return 1 ;;
    esac
    case "$os" in
        Linux) echo "${arch}-unknown-linux-musl" ;;
        Darwin) echo "${arch}-apple-darwin" ;;
        *) return 1 ;;
    esac
}

fetch() {
    # fetch URL DEST
    case "$1" in
        file://*) cp "${1#file://}" "$2" ;;
        *)
            if command -v curl >/dev/null 2>&1; then
                curl -fsSL "$1" -o "$2"
            elif command -v wget >/dev/null 2>&1; then
                wget -qO "$2" "$1"
            else
                err "need curl or wget"
            fi
            ;;
    esac
}

sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d ' ' -f 1
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | cut -d ' ' -f 1
    else
        err "need sha256sum or shasum to verify the download"
    fi
}

cargo_fallback() {
    say "no prebuilt binary for this platform ($1); building from source with cargo"
    command -v cargo >/dev/null 2>&1 || err "cargo not found. Install Rust from https://rustup.rs and rerun."
    cargo install justrust --locked
    BIN="$(command -v justrust || echo "$HOME/.cargo/bin/justrust")"
}

main() {
    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' EXIT INT TERM

    if target=$(detect_target); then
        asset="justrust-${target}.tar.gz"
        say "downloading $asset"
        if fetch "$BASE_URL/$asset" "$tmp/$asset" && fetch "$BASE_URL/$asset.sha256" "$tmp/$asset.sha256"; then
            expected=$(cut -d ' ' -f 1 "$tmp/$asset.sha256")
            actual=$(sha256_of "$tmp/$asset")
            [ "$expected" = "$actual" ] || err "checksum mismatch for $asset (expected $expected, got $actual)"
            tar -xzf "$tmp/$asset" -C "$tmp"
            src="$tmp/justrust-${target}/justrust"
            [ -f "$src" ] || err "archive did not contain justrust"
            mkdir -p "$INSTALL_DIR"
            cp "$src" "$INSTALL_DIR/justrust.tmp"
            chmod 755 "$INSTALL_DIR/justrust.tmp"
            mv "$INSTALL_DIR/justrust.tmp" "$INSTALL_DIR/justrust"
            BIN="$INSTALL_DIR/justrust"
            say "installed $BIN"
            case ":$PATH:" in
                *":$INSTALL_DIR:"*) ;;
                *)
                    say "$INSTALL_DIR is not on PATH. Run it as $BIN"
                    say "or add it: export PATH=\"$INSTALL_DIR:\$PATH\""
                    ;;
            esac
        else
            cargo_fallback "$target"
        fi
    else
        cargo_fallback "$(uname -s)/$(uname -m)"
    fi

    if [ -n "${JUSTRUST_NO_LOGIN:-}" ]; then
        say "Next: justrust login"
    elif [ -t 0 ] || { [ -t 1 ] && (: </dev/tty) 2>/dev/null; }; then
        # A human is at a terminal: run the interactive login.
        "$BIN" login </dev/tty || say "login skipped. Next: justrust login"
    else
        # No TTY (coding agent): never block on input.
        if "$BIN" login --help </dev/null 2>&1 | grep -q -- '--no-wait'; then
            out=$("$BIN" login --no-wait </dev/null 2>&1) || true
            printf '%s\n' "$out"
            url=$(printf '%s\n' "$out" | grep -o 'https\{0,1\}://[^[:space:]"<>]*' | head -n 1)
            echo
            echo "Installed: $BIN"
            if [ -n "$url" ]; then
                echo "Ask your user to open $url to enable remote builds (25 free)"
            else
                echo "Ask your user to run: $BIN login (to enable remote builds, 25 free)"
            fi
            echo "Then just use justrust check/test"
        else
            echo
            echo "Installed: $BIN"
            echo "Next: justrust login"
        fi
    fi
}

main "$@"
