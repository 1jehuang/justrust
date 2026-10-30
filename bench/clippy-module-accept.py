#!/usr/bin/env python3
"""Real Clippy acceptance for the opt-in module-local subset, not a speed benchmark.

Runs ordered cross-module edits with warm state and -Zincremental-verify-ich.
Compares complete JSON diagnostics (including spans, children and suggestions),
ignoring only emission order. Also compares each warm result with a cold compile.
Use --ui-root to compare selected existing UI fixtures in both modes.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time

LINTS = ["needless_bool", "needless_bool_assign", "bool_comparison", "neg_multiply", "needless_question_mark"]


def run(driver, sysroot, source, cache, module, extra=()):
    command = [str(driver), str(source), "--sysroot", str(sysroot), "--edition=2024",
               "--crate-type=lib", "--emit=metadata", "--error-format=json", "-Zincremental-verify-ich",
               "-Adead_code", "-Aunused", "-Aclippy::all", "--out-dir", str(cache.parent),
               "-Cincremental=" + str(cache)]
    command += ["-Wclippy::" + lint for lint in LINTS]
    if module:
        command += ["--cfg=jr_clippy_module_lints"]
    command += list(extra)
    env = dict(os.environ)
    env["LD_LIBRARY_PATH"] = str(sysroot / "lib") + ":" + env.get("LD_LIBRARY_PATH", "")
    started = time.monotonic()
    result = subprocess.run(command, env=env, text=True, capture_output=True)
    elapsed = time.monotonic() - started
    diagnostics = []
    for line in result.stderr.splitlines():
        try:
            item = json.loads(line)
        except json.JSONDecodeError:
            raise AssertionError(f"non-JSON compiler output: {line}\n{result.stderr}")
        if item.get("$message_type") == "diagnostic":
            diagnostics.append(item)
    infrastructure_errors = {"E0463", "E0514"}
    assert not any((item.get("code") or {}).get("code") in infrastructure_errors for item in diagnostics), result.stderr
    assert not any("internal compiler error" in item.get("message", "") for item in diagnostics), result.stderr
    return {"status": result.returncode, "diagnostics": sorted(diagnostics, key=lambda x: json.dumps(x, sort_keys=True)),
            "seconds": elapsed}


def equivalent(a, b):
    return a["status"] == b["status"] and a["diagnostics"] == b["diagnostics"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--driver", type=Path, required=True)
    parser.add_argument("--sysroot", type=Path, required=True)
    parser.add_argument("--ui-root", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.driver, args.sysroot = args.driver.resolve(), args.sysroot.resolve()
    results = []
    with tempfile.TemporaryDirectory(prefix="clippy-module-", dir=os.environ.get("JCODE_SCRATCH_DIR")) as directory:
        root = Path(directory)
        source = root / "lib.rs"
        source.write_text("#![allow(unexpected_cfgs)]\nmod left;\nmod right;\n")
        left = root / "left.rs"
        right = root / "right.rs"
        left_original = "pub const ENABLED: bool = true;\npub fn f(x: bool) -> bool { if x { true } else { false } }\npub fn n(x: i32) -> i32 { x * -1 }\n"
        right_original = "pub fn f(x: bool) -> bool { x == true }\npub fn q(x: Option<u8>) -> Option<u8> { Some(x?) }\npub fn a(x: bool) -> bool { let mut y = false; if x { y = true; } else { y = false; } y }\npub fn cross(x: bool) -> bool { if crate::left::f(x) { true } else { false } }\npub mod nested { pub fn f(x: bool) -> bool { if x { true } else { false } } }\n"
        left.write_text(left_original)
        right.write_text(right_original)
        edits = [
            ("initial", None, None, ()),
            ("left-body", left, left_original.replace("if x { true } else { false }", "x").replace("x * -1", "-x"), ()),
            ("right-body", right, right_original.replace("x == true", "x").replace("Some(x?)", "x"), ()),
            ("left-reintroduce", left, left_original.replace("true;", "false;"), ()),
            ("right-scoped-allow", right, "#![allow(clippy::needless_bool)]\n" + right_original, ()),
            ("right-expect", right, "#![expect(clippy::needless_bool)]\n" + right_original, ()),
            ("right-expect-unfulfilled", right, "#![expect(clippy::needless_bool)]\n" + right_original.replace("if crate::left::f(x) { true } else { false }", "crate::left::f(x)").replace("if x { true } else { false }", "x"), ()),
            ("deny-lint", None, None, ("-Dclippy::neg_multiply",)),
            ("type-error", left, left_original + "pub fn broken() -> u8 { false }\n", ()),
            ("recover", left, left_original, ()),
        ]
        for index, (label, path, text, extra) in enumerate(edits):
            if path is not None:
                path.write_text(text)
            before = run(args.driver, args.sysroot, source, root / "default", False, extra)
            after = run(args.driver, args.sysroot, source, root / "module", True, extra)
            cold = run(args.driver, args.sysroot, source, root / f"cold{index}", True, extra)
            entry = {"label": label, "default": before, "module": after, "cold_module": cold}
            results.append(entry)
            args.output.write_text(json.dumps(results, indent=2) + "\n")
            if not equivalent(before, after) or not equivalent(after, cold):
                args.output.write_text(json.dumps(results, indent=2) + "\n")
                raise AssertionError(f"diagnostic/status mismatch at {label}, see {args.output}")
            if label in ("deny-lint", "type-error"):
                assert before["status"] != 0, label
            else:
                assert before["status"] == 0, label
            print(f"PASS {label}: {len(after['diagnostics'])} diagnostics, status {after['status']}")
        # A single incremental directory must also safely cross the cfg partition.
        assert (root / "module" / "jr-clippy-module-lints-v2").is_dir(), "driver did not activate experiment"
        expected = results[-1]["default"]
        for index, module in enumerate([False, True, False, True]):
            toggled = run(args.driver, args.sysroot, source, root / "toggle", module)
            results.append({"label": f"toggle-{index}-{module}", "actual": toggled})
            args.output.write_text(json.dumps(results, indent=2) + "\n")
            assert equivalent(toggled, expected), "cfg toggle reused different diagnostics"
            print(f"PASS cfg-toggle {index}: module={module}")
        if args.ui_root:
            for index, name in enumerate(["bool_comparison", "needless_bool/simple", "needless_bool/fixable", "needless_bool/early_return", "needless_bool_assign", "neg_multiply", "needless_question_mark"]):
                fixture = args.ui_root.resolve() / (name + ".rs")
                before = run(args.driver, args.sysroot, fixture, root / f"ui-default{index}", False)
                after = run(args.driver, args.sysroot, fixture, root / f"ui-module{index}", True)
                results.append({"label": "ui/" + name, "default": before, "module": after})
                args.output.write_text(json.dumps(results, indent=2) + "\n")
                assert equivalent(before, after), "UI mismatch: " + name
                print(f"PASS ui/{name}: {len(after['diagnostics'])} diagnostics, status {after['status']}")
    args.output.write_text(json.dumps(results, indent=2) + "\n")


if __name__ == "__main__":
    main()
