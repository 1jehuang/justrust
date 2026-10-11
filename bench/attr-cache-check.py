#!/usr/bin/env python3
"""Correctness and effect of jr-rustc's attribute proc-macro cache.

Runs on the pinned Desktop snapshot (takes the bench lock) in its own
target dir, with -Zcache-proc-macros on the UI crate only (a RUSTC_WRAPPER
adds it, so dependencies keep their artifacts):

  1. warm build, the chosen #[gpui::test] passes
  2. its body edited to panic: the test now fails (the edit reached the
     expansion, no stale cached output)
  3. its body edited to a type error: the build fails with that error
  4. restored: the test passes again
  5. a one-line edit in that test, self-profiled: how many #[gpui::test]
     expansions ran (no cache: 724)

Usage: bench/attr-cache-check.py RUSTC [--no-cache]
"""
import os
import shutil
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import suite  # noqa: E402

rustc = sys.argv[1]
use_cache = "--no-cache" not in sys.argv
cargo = os.path.expanduser("~/.justrust/toolchains/nightly-2026-10-08/bin/cargo")
root = suite.prepare("desktop")
cwd = root / suite.SNAPSHOTS["desktop"]["cwd"]
rel = "jcode-desktop/crates/jcode-desktop-ui/src/build_info.rs"
anchor = "    fn footer_exposes_exact_build_details_on_hover(cx: &mut gpui::TestAppContext) {\n"
FILTER = "footer_exposes_exact_build_details_on_hover"
target = suite.JR_HOME / "bench" / "attr-cache-target"
wrapper = target / "ui-flags-wrapper.sh"
target.mkdir(parents=True, exist_ok=True)
wrapper.write_text(
    "#!/bin/sh\n"
    "# Adds $UI_RUSTFLAGS when compiling jcode_desktop_ui.\n"
    'rc="$1"; shift\n'
    'case " $* " in *" --crate-name jcode_desktop_ui "*) exec "$rc" "$@" $UI_RUSTFLAGS ;; esac\n'
    'exec "$rc" "$@"\n'
)
wrapper.chmod(0o755)
env = {k: v for k, v in os.environ.items() if not k.startswith(("RUSTFLAGS", "CARGO_ENCODED", "RUSTC"))}
env.update(RUSTC=rustc, RUSTC_WRAPPER=str(wrapper), CARGO_TARGET_DIR=str(target), CARGO_INCREMENTAL="1",
           UI_RUSTFLAGS="-Zcache-proc-macros" if use_cache else "")


def edit(new: str | None) -> None:
    orig = suite.pristine(root, rel).read_text()
    assert orig.count(anchor) == 1
    (root / rel).write_text(orig if new is None else orig.replace(anchor, anchor + new, 1))


def cargo_test(*args: str, extra_env: dict | None = None) -> subprocess.CompletedProcess:
    return subprocess.run([cargo, "test", "-p", "jcode-desktop-ui", "--lib", *args], cwd=cwd,
                          env={**env, **(extra_env or {})}, capture_output=True, text=True)


def ok(cond: bool, what: str, p: subprocess.CompletedProcess | None = None) -> None:
    print(("PASS " if cond else "FAIL ") + what, flush=True)
    if not cond:
        if p is not None:
            print(p.stderr[-3000:])
        edit(None)
        sys.exit(1)


try:
    print(f"cache {'on' if use_cache else 'off'}, rustc {rustc}", flush=True)
    edit(None)
    p = cargo_test("--no-run"); ok(p.returncode == 0, "warm build", p)
    p = cargo_test("--", FILTER); ok(p.returncode == 0, "test passes before the edit", p)

    edit('        panic!("attr-cache-check");\n')
    p = cargo_test("--", FILTER)
    ok(p.returncode != 0 and "attr-cache-check" in p.stdout + p.stderr, "edited #[gpui::test] body runs (panics)", p)

    edit('        let _: u32 = "attr-cache-type-error";\n')
    p = cargo_test("--no-run")
    ok(p.returncode != 0 and "attr-cache-type-error" in p.stderr, "type error inside the test is reported", p)

    edit(None)
    p = cargo_test("--", FILTER); ok(p.returncode == 0, "test passes again after restore", p)

    prof = target / "selfprof"
    shutil.rmtree(prof, ignore_errors=True)
    prof.mkdir()
    edit("        std::hint::black_box(4242u64);\n")
    p = cargo_test("--no-run", extra_env={
        "UI_RUSTFLAGS": env["UI_RUSTFLAGS"] + f" -Zself-profile={prof} -Zself-profile-events=default,args"})
    ok(p.returncode == 0, "profiled one-line edit builds", p)
    edit(None)
    mmview = os.path.expanduser(os.environ.get("MMVIEW", "~/.jcode/scratch/mm/bin/mmview"))
    out = subprocess.run([mmview, str(next(prof.glob("jcode_desktop_ui-*.mm_profdata")))],
                         capture_output=True, text=True).stdout
    calls, lab = 0, None
    for line in out.splitlines():
        line = line.strip()
        if line.startswith("label:"):
            lab = line[6:].strip(" ,")
        elif line.startswith("additional_data:") and lab == "expand_proc_macro" and "#[gpui::test]" in line:
            calls += 1
    print(f"#[gpui::test] expansions on a one-line edit: {calls}")
finally:
    edit(None)
