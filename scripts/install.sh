#!/bin/sh
# justrust installer: curl -fsSL https://jcode.sh/rust.sh | sh
#
# Environment:
#   JUSTRUST_INSTALL_DIR  install directory (default: ~/.local/bin)
#   JUSTRUST_BASE_URL     release asset base URL
#                         (default: https://github.com/1jehuang/justrust/releases/latest/download)
#   JUSTRUST_NO_LOGIN     if set, skip running `justrust login`
#   JUSTRUST_INSTALL_TOKEN  one-time token from the signed-in install command
#                         on jcode.sh/rust (rust.sh?t=...). Exchanged once for
#                         an API key saved to the Jcode credentials file, so
#                         this machine is signed in with no `justrust login`.
#   JUSTRUST_NO_AGENTS    if set, do not add justrust instructions to the
#                         global CLAUDE.md / AGENTS.md of installed agents
#   JUSTRUST_NO_TELEMETRY or DO_NOT_TRACK
#                         if set, send no install events. Otherwise the
#                         installer reports installer_start and
#                         installer_finish (success, or the failed step) with
#                         a random install id, OS, and CPU arch to
#                         build.jcode.sh, and saves the id in
#                         ~/.justrust/install_id so `justrust login` can tie
#                         the sign-in to it. Nothing else is sent.
set -eu

BASE_URL="${JUSTRUST_BASE_URL:-https://github.com/1jehuang/justrust/releases/latest/download}"
INSTALL_DIR="${JUSTRUST_INSTALL_DIR:-$HOME/.local/bin}"
EVENT_URL="${JUSTRUST_INSTALL_EVENT_URL:-https://build.jcode.sh/v1/install/event}"
STAGE=start
FINISHED=

say() { printf 'justrust: %s\n' "$*"; }
err() { printf 'justrust: error: %s\n' "$*" >&2; exit 1; }

telemetry_off() {
    [ -n "${JUSTRUST_NO_TELEMETRY:-}" ] && return 0
    [ -n "${DO_NOT_TRACK:-}" ] && [ "${DO_NOT_TRACK}" != 0 ]
}

# A random v4 uuid, or nothing. The website passes one in (JUSTRUST_INSTALL_ID)
# so the copied command and this install are the same funnel row.
new_install_id() {
    if [ -r /proc/sys/kernel/random/uuid ]; then
        cat /proc/sys/kernel/random/uuid
    elif command -v uuidgen >/dev/null 2>&1; then
        uuidgen | tr '[:upper:]' '[:lower:]'
    fi
}

valid_install_id() {
    printf '%s' "$1" | grep -Eq '^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$'
}

# report STAGE OUTCOME DETAIL: fire and forget, never fails or delays the
# install by more than 3 seconds.
report() {
    telemetry_off && return 0
    [ -n "${INSTALL_ID:-}" ] || return 0
    command -v curl >/dev/null 2>&1 || return 0
    body=$(printf '{"install_id":"%s","stage":"%s","outcome":"%s","source":"installer","detail":"%s","os":"%s","arch":"%s"}' \
        "$INSTALL_ID" "$1" "$2" "$3" "$(uname -s 2>/dev/null | tr -cd 'A-Za-z0-9_.-')" "$(uname -m 2>/dev/null | tr -cd 'A-Za-z0-9_.-')")
    curl -fsS -m 3 -o /dev/null -H 'Content-Type: application/json' -d "$body" "$EVENT_URL" >/dev/null 2>&1 || true
}

setup_install_id() {
    INSTALL_ID="${JUSTRUST_INSTALL_ID:-}"
    valid_install_id "$INSTALL_ID" || INSTALL_ID=$(new_install_id)
    valid_install_id "$INSTALL_ID" || INSTALL_ID=
    telemetry_off && return 0
    [ -n "$INSTALL_ID" ] || return 0
    jr_home="${JUSTRUST_HOME:-$HOME/.justrust}"
    (umask 077; mkdir -p "$jr_home" && printf '%s\n' "$INSTALL_ID" > "$jr_home/install_id") 2>/dev/null || true
}

on_exit() {
    code=$?
    rm -rf "${tmp:-/nonexistent}"
    if [ -z "$FINISHED" ]; then
        FINISHED=1
        if [ "$code" = 0 ]; then report installer_finish success ""; else report installer_finish failure "$STAGE"; fi
    fi
    exit "$code"
}

# The Jcode credentials file justrust and Jcode share (remote_hosted.rs
# config_dir): $JCODE_HOME/config/jcode, else the platform config dir.
creds_file() {
    if [ -n "${JCODE_HOME:-}" ]; then
        dir="$JCODE_HOME/config/jcode"
    elif [ "$(uname -s)" = Darwin ]; then
        dir="$HOME/Library/Application Support/jcode"
    else
        dir="${XDG_CONFIG_HOME:-$HOME/.config}/jcode"
    fi
    printf '%s/jcode-subscription.env' "$dir"
}

json_field() {
    printf '%s' "$1" | sed -n "s/.*\"$2\"[[:space:]]*:[[:space:]]*\"\([^\"]*\)\".*/\1/p" | tr -cd 'A-Za-z0-9@._+-'
}

# Exchange JUSTRUST_INSTALL_TOKEN (single use, 30 minutes) for an API key and
# save it like `justrust login` does. Fails soft: the install still succeeds,
# and the normal login runs instead.
redeem_install_token() {
    token=$(printf '%s' "$JUSTRUST_INSTALL_TOKEN" | tr -cd 'A-Za-z0-9_-')
    api="${JCODE_API_BASE:-https://api.jcode.sh/v1}"
    resp=$(curl -sS -m 20 -H 'Content-Type: application/json' \
        -d "{\"install_token\":\"$token\"}" "$api/auth/install-token" 2>/dev/null) || resp=
    key=$(json_field "$resp" api_key)
    case "$key" in
        jck_live_*) ;;
        *)
            msg=$(printf '%s' "$resp" | sed -n 's/.*"message"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')
            say "could not sign in with the install command: ${msg:-no response from $api}"
            report login_token failure ""
            return 1
            ;;
    esac
    file=$(creds_file)
    (
        umask 077
        mkdir -p "$(dirname "$file")"
        tmpf="$file.tmp"
        if [ -f "$file" ]; then
            grep -Ev '^(export[[:space:]]+)?(JCODE_API_KEY|JCODE_ACCOUNT_ID|JCODE_ACCOUNT_EMAIL|JCODE_TIER)=' "$file" > "$tmpf" || true
        else
            : > "$tmpf"
        fi
        {
            printf 'JCODE_API_KEY=%s\n' "$key"
            printf 'JCODE_ACCOUNT_ID=%s\n' "$(json_field "$resp" account_id)"
            printf 'JCODE_ACCOUNT_EMAIL=%s\n' "$(json_field "$resp" email)"
            printf 'JCODE_TIER=%s\n' "$(json_field "$resp" tier)"
        } >> "$tmpf"
        chmod 600 "$tmpf"
        mv "$tmpf" "$file"
    ) || { say "could not write $file"; return 1; }
    say "signed in as $(json_field "$resp" email) (saved to $file)"
    report login_token success ""
    return 0
}

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
    STAGE=cargo_install
    say "no prebuilt binary for this platform ($1); building from source with cargo"
    command -v cargo >/dev/null 2>&1 || err "cargo not found. Install Rust from https://rustup.rs and rerun."
    cargo install justrust --locked
    BIN="$(command -v justrust || echo "$HOME/.cargo/bin/justrust")"
}

main() {
    tmp=$(mktemp -d)
    trap on_exit EXIT
    trap 'exit 130' INT TERM
    setup_install_id
    report installer_start success ""

    if target=$(detect_target); then
        asset="justrust-${target}.tar.gz"
        say "downloading $asset"
        STAGE=download
        if fetch "$BASE_URL/$asset" "$tmp/$asset" && fetch "$BASE_URL/$asset.sha256" "$tmp/$asset.sha256"; then
            STAGE=verify
            expected=$(cut -d ' ' -f 1 "$tmp/$asset.sha256")
            actual=$(sha256_of "$tmp/$asset")
            [ "$expected" = "$actual" ] || err "checksum mismatch for $asset (expected $expected, got $actual)"
            STAGE=extract
            tar -xzf "$tmp/$asset" -C "$tmp"
            src="$tmp/justrust-${target}/justrust"
            [ -f "$src" ] || err "archive did not contain justrust"
            STAGE=copy
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
    # The binary is in place: that is a successful install, whatever the
    # sign-in below does (it is tracked separately as login_start).
    FINISHED=1
    report installer_finish success ""

    # Tell installed coding agents to use justrust (global CLAUDE.md /
    # AGENTS.md, a marked block that `justrust agents --remove` takes out).
    if [ -z "${JUSTRUST_NO_AGENTS:-}" ] && "$BIN" agents --help </dev/null >/dev/null 2>&1; then
        "$BIN" agents --quiet </dev/null 2>&1 | sed 's/^/justrust: /' || true
    fi

    if [ -n "${JUSTRUST_INSTALL_TOKEN:-}" ] && redeem_install_token; then
        # Signed in: `login` now only selects hosted builds and shows the
        # builds left.
        "$BIN" login </dev/null 2>&1 | sed 's/^/justrust: /' || true
        echo
        echo "Installed and signed in: $BIN"
        echo "Use justrust check / test / clippy instead of cargo. They run on remote build machines."
    elif [ -n "${JUSTRUST_NO_LOGIN:-}" ]; then
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
