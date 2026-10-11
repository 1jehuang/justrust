#!/usr/bin/env python3
"""Acceptance check of jr-rustc's attribute proc-macro cache through justrust.

Drives the real agent path: `justrust test -p jcode-desktop-ui --lib` on the
pinned Desktop snapshot with RUSTC = a jr-rustc build and -Zcache-proc-macros
on the UI crate (cargo per-package profile rustflags, as bench/factors.py).
Each step's expected outcome is checked from justrust's output:

  suite     full UI lib suite, cache on vs off: same passed/failed/ignored
  new       a new #[gpui::test] appears and runs
  rename    a renamed test runs under its new name, the old name is gone
  sequence  edit A, edit B, revert A: each build reflects the current source
  args      #[gpui::test(iterations = 3)] on an existing test: the macro
            re-expands (the test body runs 3 times, counted by a static)
  macro-err a broken attribute (unknown argument) is reported, then fixed

Usage: bench/attr-cache-accept.py RUSTC
"""
import os
import re
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import suite  # noqa: E402

RUSTC = sys.argv[1]
J = os.environ.get("JUSTRUST", "justrust")
root = suite.prepare("desktop")
cwd = root / suite.SNAPSHOTS["desktop"]["cwd"]
REL = "jcode-desktop/crates/jcode-desktop-ui/src/build_info.rs"
ANCHOR = "    #[gpui::test]\n    fn footer_exposes_exact_build_details_on_hover(cx: &mut gpui::TestAppContext) {\n"
NAME = "footer_exposes_exact_build_details_on_hover"
UI_FLAGS = ["--config", 'profile.dev.package.jcode-desktop-ui.rustflags=["-Zcache-proc-macros"]']


def env(cache: bool) -> dict:
    e = {k: v for k, v in os.environ.items() if not k.startswith(("RUSTFLAGS", "CARGO_ENCODED"))}
    e.update(RUSTC=RUSTC, JUSTRUST_TOOLCHAIN="system", JUSTRUST_REMOTE="0", JUSTRUST_DEPCACHE="0",
             JUSTRUST_PEER_CACHE="0", JUSTRUST_REMOTE_CACHE="0",
             CARGO_UNSTABLE_PROFILE_RUSTFLAGS="true",
             CARGO_TARGET_DIR=str(suite.JR_HOME / "bench" / f"attr-accept-{'on' if cache else 'off'}"))
    return e


def jr_test(cache: bool, *filt: str) -> subprocess.CompletedProcess:
    args = [J, "test", *(UI_FLAGS if cache else []), "-p", "jcode-desktop-ui", "--lib"]
    if filt:
        args += ["--", *filt]
    return subprocess.run(args, cwd=cwd, env=env(cache), capture_output=True, text=True)


def write(src: str | None) -> None:
    orig = suite.pristine(root, REL).read_text()
    assert orig.count(ANCHOR) == 1, "anchor"
    (root / REL).write_text(orig if src is None else orig.replace(ANCHOR, src, 1))


def results(p: subprocess.CompletedProcess) -> dict[str, str]:
    """Per-test outcomes. justrust hides passing-test lines, so read the
    full saved output of that run (`justrust log`)."""
    out = p.stdout + p.stderr
    m = re.search(r"`justrust log (\S+)`", out)
    if m:
        out = subprocess.run([J, "log", m.group(1)], capture_output=True, text=True).stdout
    return {m.group(1): m.group(2) for m in re.finditer(r"^test (\S+) \.\.\. (ok|FAILED|ignored)", out, re.M)}


fails = 0


def check(cond: bool, what: str, p: subprocess.CompletedProcess | None = None) -> None:
    global fails
    print(("PASS " if cond else "FAIL ") + what, flush=True)
    if not cond:
        fails += 1
        if p is not None:
            print((p.stdout + p.stderr)[-2500:])


def ran(p, name) -> str | None:
    for k, v in results(p).items():
        if k.endswith(name):
            return v
    return None


try:
    write(None)
    # suite: whole lib suite both ways (each side warms its own target dir).
    on = jr_test(True)
    off = jr_test(False)
    ron, roff = results(on), results(off)
    diff = sorted(k for k in set(ron) | set(roff) if ron.get(k) != roff.get(k))
    # A differing test counts only if it differs again when rerun alone on
    # both sides 5 times (Desktop has timing-sensitive UI tests that flake
    # under full-suite parallelism with or without the cache).
    real = []
    for t in diff:
        def outcomes(cache: bool) -> set[str]:
            return {"ok" if jr_test(cache, t, "--exact").returncode == 0 else "FAILED" for _ in range(5)}
        o_on, o_off = outcomes(True), outcomes(False)
        print(f"  {t}: alone on={sorted(o_on)} off={sorted(o_off)}")
        if o_on != o_off:
            real.append(t)
    check(len(ron) > 1000 and set(ron) == set(roff) and not real,
          f"full suite: cache on {len(ron)} tests ({sum(v == 'ok' for v in ron.values())} ok), "
          f"off {len(roff)}; {len(diff)} differed in the suite run, {len(real)} consistently")

    # new test
    write(ANCHOR + "        let _ = cx;\n    }\n\n    #[gpui::test]\n"
          "    fn attr_accept_new_test(_cx: &mut gpui::TestAppContext) {\n        assert_eq!(1 + 1, 2);\n    }\n\n"
          "    #[gpui::test]\n    fn footer_exposes_exact_build_details_on_hover_orig(cx: &mut gpui::TestAppContext) {\n")
    p = jr_test(True, "attr_accept_new_test")
    check(ran(p, "attr_accept_new_test") == "ok", "new #[gpui::test] appears and passes", p)

    # rename
    write(ANCHOR.replace(NAME, "attr_accept_renamed"))
    p = jr_test(True, "build_info")
    check(ran(p, "attr_accept_renamed") == "ok" and ran(p, "::" + NAME) is None,
          "renamed test runs under the new name, old name gone", p)

    # sequence A, B, revert A
    a = ANCHOR + '        if std::env::var("ATTR_ACCEPT").is_ok() { panic!("variant-A"); }\n'
    b = ANCHOR + '        if std::env::var("ATTR_ACCEPT").is_ok() { panic!("variant-B"); }\n'
    seen = []
    for src in (a, b, a):
        write(src)
        e = env(True) | {"ATTR_ACCEPT": "1"}
        p = subprocess.run([J, "test", *UI_FLAGS, "-p", "jcode-desktop-ui", "--lib", "--", NAME],
                           cwd=cwd, env=e, capture_output=True, text=True)
        seen.append("A" if "variant-A" in p.stdout + p.stderr else "B" if "variant-B" in p.stdout + p.stderr else "?")
    check(seen == ["A", "B", "A"], f"edit A, B, revert A runs {seen}")

    # attribute args only
    counter = ("    static ATTR_ACCEPT_RUNS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);\n")
    body = (f"    fn {NAME}(cx: &mut gpui::TestAppContext) {{\n"
            "        let n = ATTR_ACCEPT_RUNS.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;\n"
            '        eprintln!("attr-accept-run {n}");\n')
    write(counter + "    #[gpui::test]\n" + body)
    p1 = jr_test(True, NAME, "--nocapture")
    write(counter + "    #[gpui::test(iterations = 3)]\n" + body)
    p3 = jr_test(True, NAME, "--nocapture")
    r1 = len(re.findall(r"attr-accept-run \d", p1.stdout + p1.stderr))
    r3 = len(re.findall(r"attr-accept-run \d", p3.stdout + p3.stderr))
    check(r1 == 1 and r3 == 3, f"attribute args change re-expands: runs {r1} -> {r3}", p3 if r3 != 3 else None)

    # macro error then fix
    write("    #[gpui::test(not_an_option = 1)]\n" + ANCHOR.split("\n", 1)[1])
    p = jr_test(True, NAME)
    check(p.returncode != 0 and "not_an_option" in p.stdout + p.stderr, "attribute macro error is reported", p)
    write(None)
    p = jr_test(True, NAME)
    check(p.returncode == 0 and ran(p, NAME) == "ok", "fixed attribute builds and passes again", p)
finally:
    write(None)

print(f"{fails} failed")
sys.exit(1 if fails else 0)
