#!/usr/bin/env bash
# End-to-end check of the default toolchain path (jr-rustc + proc-macro
# cache) through the installed `justrust`, on a tiny workspace with its own
# attribute, derive and env-reading proc macros (bench/e2e).
#
# Each step prints PASS/FAIL against the expected value. Copies the fixture
# to a scratch dir first; never touches this repo's sources.
#
# Usage: bench/e2e-default-toolchain.sh
set -u
src=$(cd "$(dirname "$0")/e2e" && pwd)
work=${JUSTRUST_E2E_DIR:-$HOME/.justrust/bench/e2e-default-toolchain}
mkdir -p "$work"
cp -r "$src/." "$work/"
cd "$work"
export JUSTRUST_REMOTE=0
fails=0
gen() { python3 - "$@" <<'EOF'
import sys
t = open("app/src/lib.rs.tpl").read()
for k, v in zip(["TIMES", "BODY", "FIELDS"], sys.argv[1:]):
    t = t.replace(k, v)
open("app/src/lib.rs", "w").write(t)
EOF
}
values() { justrust test --local -p app -- --nocapture 2>&1 | grep -o 'E2E [^"]*' | head -1; }
expect() { # what got want
  if [ "$2" = "$3" ]; then echo "PASS $1"; else echo "FAIL $1: got '$2', want '$3'"; fails=$((fails+1)); fi
}
toolchain() {
  local id
  id=$(justrust runs --here -n 1 --json | python3 -c "import sys,json;print(json.loads(sys.stdin.readline())['id'])")
  python3 -c "import json;print(json.load(open('$HOME/.justrust/runs/$id/meta.json')).get('toolchain'))"
}

gen 2 1 ""; got=$(values); expect "first build" "$got" "E2E work=1 times=2 fields=2 tag="
case "$(toolchain)" in jr-*) echo "PASS default toolchain is jr";; *) echo "FAIL default toolchain: $(toolchain)"; fails=$((fails+1));; esac
gen 2 7 "";        expect "body edit"            "$(values)" "E2E work=7 times=2 fields=2 tag="
gen 5 7 "";        expect "attribute args only"  "$(values)" "E2E work=7 times=5 fields=2 tag="
gen 5 7 ", c: u8"; expect "derive input changed" "$(values)" "E2E work=7 times=5 fields=3 tag="
gen 2 7 "";        expect "revert"               "$(values)" "E2E work=7 times=2 fields=2 tag="
gen 2 '"x"' ""
if justrust test --local -p app 2>&1 | grep -q "E0308"; then echo "PASS type error in an attributed fn is reported"; else echo "FAIL type error not reported"; fails=$((fails+1)); fi
gen 2 7 "";        expect "fixed"                "$(values)" "E2E work=7 times=2 fields=2 tag="
if justrust test --local -p app --doc 2>&1 | grep -q "^justrust: ok"; then echo "PASS doctests (jr rustdoc)"; else echo "FAIL doctests"; fails=$((fails+1)); fi
gen 4 7 ""
if justrust test --local -p app --doc 2>&1 | grep -q "^justrust: ok"; then echo "PASS doctests after attribute change"; else echo "FAIL doctests after attribute change"; fails=$((fails+1)); fi
gen 2 7 ""
if justrust clippy --local -p app 2>&1 | grep -q "^justrust: ok"; then echo "PASS clippy (jr clippy)"; else echo "FAIL clippy"; fails=$((fails+1)); fi
JUSTRUST_TOOLCHAIN=system values >/dev/null
[ "$(toolchain)" = None ] && echo "PASS JUSTRUST_TOOLCHAIN=system opts out" || { echo "FAIL system opt-out"; fails=$((fails+1)); }
printf '[toolchain]\nchannel = "jr-0123456789ab"\n' > justrust.toml
got=$(values); rm justrust.toml
expect "pin to a missing jr toolchain fails open" "$got" "E2E work=7 times=2 fields=2 tag="
# The documented caveat: a macro reading the environment is not re-run on
# the cached path; with the cache off it is (after any edit).
gen 2 11 ""; E2E_TAG=one values >/dev/null; gen 2 12 ""
expect "caveat: env-reading macro keeps cached output" "$(E2E_TAG=two values)" "E2E work=12 times=2 fields=2 tag=one"
gen 2 13 ""; JUSTRUST_CACHE_PROC_MACROS=0 E2E_TAG=one values >/dev/null; gen 2 14 ""
expect "cache off re-runs it" "$(JUSTRUST_CACHE_PROC_MACROS=0 E2E_TAG=two values)" "E2E work=14 times=2 fields=2 tag=two"
echo "$fails failed"
exit $((fails > 0))
