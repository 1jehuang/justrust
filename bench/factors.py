#!/usr/bin/env python3
"""System 2: what each speed factor is worth, measured one at a time.

Every factor that can make the agent loop faster gets an on/off switch and
a workload where it matters. Each factor runs with everything else at the
current default ("leave one out"), on the pinned snapshots of bench/suite.py,
with on and off runs interleaved (ABBA order) so machine drift hits both
sides equally. Each variant has its own target dir (and, for caching
factors, its own private cache), so variants never warm or invalidate each
other.

Every sample also records the reuse ledger: units compiled, units cargo
found fresh, units restored from the depcache and the rustc seconds that
saved, build-script runs replayed, units copied in by slot refresh.

  bench/factors.py list                  factors, workloads, switches
  bench/factors.py run [factor...]       run (default: every factor not marked slow)
  bench/factors.py run --all             include slow factors
  bench/factors.py report                rebuild bench/FACTORS.md from the newest result per factor
  bench/factors.py clean                 delete the factor target dirs and caches

Results: bench/results/factors-<time>-<rev>.json (commit them, and FACTORS.md).
Not covered here: the machine-wide scheduler under contention
(bench/sched-contention.sh) and remote builds.
"""

from __future__ import annotations

import argparse
import dataclasses
import hashlib
import json
import os
import shutil
import statistics
import subprocess
import sys
import time
from dataclasses import dataclass, field
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import suite  # noqa: E402

FACTOR_WORK = Path(os.environ.get("JUSTRUST_FACTORS_WORK", suite.JR_HOME / "bench" / "factors"))
FACTORS_MD = suite.ROOT / "bench" / "FACTORS.md"
NIGHTLY = "nightly-2026-10-08"  # the pinned toolchain (docs/toolchain.md)
# Cross-crate cutoff needs a jr toolchain built with -Zjr-iface-hash and jr-cargo
# with the cutoff (JR_CUTOFF_TOOLCHAIN overrides).
CUTOFF_TOOLCHAIN = os.environ.get("JR_CUTOFF_TOOLCHAIN", "jr-08fa8889b33c")
# Cargo reads per-package profile rustflags and hint-mostly-unused only with
# these unstable features (the pinned nightly's cargo accepts them).
UNSTABLE_PROFILE = {"CARGO_UNSTABLE_PROFILE_RUSTFLAGS": "true",
                    "CARGO_UNSTABLE_PROFILE_HINT_MOSTLY_UNUSED": "true"}
# The jr-rustc build fork-change factors run on both sides
# (JR_RUSTC_MODE picks another saved mode under ~/jr-rustc/build/jr).
JR_RUSTC_ENV = {"JUSTRUST_TOOLCHAIN": "system",
                "RUSTC": str(Path(os.environ.get("JR_RUSTC_DIR", Path.home() / "jr-rustc")) / "build/jr" / os.environ.get("JR_RUSTC_MODE", "pgo-bolt-stage2") / "bin/rustc")}
# The slowest dependencies of a cold Desktop UI test build (run
# 20261011-043427013-426230: gpui 115 s, aws-sdk-bedrock 68 s, wgpu-core 67 s, ...).
BIG_DEPS = ["gpui", "aws-sdk-bedrock", "wgpu-core", "gpui_linux", "read-fonts", "wgpu-hal",
            "mermaid-rs-renderer", "aws-sdk-bedrockruntime", "x11rb-protocol", "zbus"]


def ui_rustflags(*flags: str) -> list[str]:
    return ["--config", f"profile.dev.package.jcode-desktop-ui.rustflags={json.dumps(list(flags))}"]


def hint_mostly_unused_args() -> list[str]:
    out = []
    for p in BIG_DEPS:
        out += ["--config", f'profile.dev.package."{p}".hint-mostly-unused=true']
    return out


@dataclass
class Factor:
    name: str
    what: str  # what the factor is, in one line
    # loop: warm edit loop (suite scenario). cold: fresh target dir per run.
    # sequence: fresh target, several commands, summed. concurrent: two
    # agents at once, the second one measured.
    kind: str
    scenario: str  # suite scenario (loop/cold) or label
    on: dict[str, str]
    off: dict[str, str]
    n: int = 5
    warmup: int = 1
    slow: bool = False
    # Both variants share one private cache dir (so `off` still gets what
    # the factor does not control). Cold factors only.
    private_cache: bool = False
    cmds: list[list[str]] = field(default_factory=list)  # sequence factors
    note: str = ""
    # Extra cargo arguments (`--config ...`) inserted after the subcommand,
    # for settings cargo only reads from config, such as per-package profile
    # rustflags. Loop factors only.
    on_args: list[str] = field(default_factory=list)
    off_args: list[str] = field(default_factory=list)


FACTORS = [
    Factor(
        "depcache", "shared dependency cache: restore dependency rlibs instead of compiling",
        "cold", "desktop-cold-check",
        on={}, off={"JUSTRUST_DEPCACHE": "0", "JUSTRUST_BUILD_SCRIPT_CACHE": "0"},
        n=3, private_cache=True,
        note="fresh target dir (a new checkout, worktree, or slot); the cache was filled by one earlier build",
    ),
    Factor(
        "build-script-cache", "replay registry build-script runs (OUT_DIR + output) instead of running them",
        "cold", "desktop-cold-check",
        on={}, off={"JUSTRUST_BUILD_SCRIPT_CACHE": "0"},
        n=3, private_cache=True,
        note="fresh target dir with the depcache warm in both variants",
    ),
    Factor(
        "localcache-revert", "local crate cache: restore workspace crates whose sources match an earlier build "
                             "(JUSTRUST_LOCALCACHE=1), on undoing an edit",
        "revert", "desktop-test-edit",
        on={"JUSTRUST_LOCALCACHE": "1", "JUSTRUST_LOCALCACHE_DIR": str(FACTOR_WORK / "cache" / "localcache-warm")}, off={"JUSTRUST_LOCALCACHE": "0"}, n=5,
        note="edit fps_counter.rs, build (unmeasured, `edit_wall` = the miss path with staging), undo, build "
             "(measured); each variant keeps its own warm target dir",
    ),
    Factor(
        "localcache-revert-sde", "same, with SOURCE_DATE_EPOCH fixed (Desktop's build.rs then stops stamping the "
                                 "build time into the UI crate through env!)",
        "revert", "desktop-test-edit",
        on={"JUSTRUST_LOCALCACHE": "1", "JUSTRUST_LOCALCACHE_DIR": str(FACTOR_WORK / "cache" / "localcache-warm"),
            "SOURCE_DATE_EPOCH": "1791700000"},
        off={"JUSTRUST_LOCALCACHE": "0", "SOURCE_DATE_EPOCH": "1791700000"}, n=5,
        note="without it, JCODE_DESKTOP_BUILD_ID (build time in ms) differs every build and the UI crate never hits",
    ),
    Factor(
        "localcache-revert-small", "same, on this repo's snapshot (one crate, full `test`)",
        "revert", "justrust-body-test",
        on={"JUSTRUST_LOCALCACHE": "1", "JUSTRUST_LOCALCACHE_DIR": str(FACTOR_WORK / "cache" / "localcache-warm")},
        off={"JUSTRUST_LOCALCACHE": "0"}, n=5,
    ),
    Factor(
        "localcache-loop", "same cache, cost on the plain edit loop (every build misses and stages its outputs)",
        "loop", "desktop-test-edit",
        on={"JUSTRUST_LOCALCACHE": "1", "JUSTRUST_LOCALCACHE_DIR": str(FACTOR_WORK / "cache" / "localcache-warm")}, off={"JUSTRUST_LOCALCACHE": "0"}, n=6,
        note="the background finisher hashes the 285 MB test binary niced while the next sample may start",
    ),
    Factor(
        "localcache-fresh", "same cache, on a fresh target dir of unchanged sources (new slot, cargo clean)",
        "fresh", "desktop-test-edit",
        on={"JUSTRUST_LOCALCACHE": "1", "SOURCE_DATE_EPOCH": "1791700000"},
        off={"JUSTRUST_LOCALCACHE": "0", "SOURCE_DATE_EPOCH": "1791700000"}, n=3, private_cache=True,
        note="no edit; dependencies come from the private depcache in both variants, the warmup round fills both caches",
    ),
    Factor(
        "feature-unify", "resolve dependency features once per workspace, so -p switches reuse builds",
        "sequence", "desktop-pkg-switch",
        on={}, off={"JUSTRUST_UNIFY_FEATURES": "0"},
        n=2, warmup=0, slow=True,
        cmds=[["check", "-p", "jcode-desktop-model"], ["check", "-p", "jcode-desktop-harness"],
              ["check", "-p", "jcode-desktop-ui"]],
        note="fresh target dir, depcache off, three `check -p` in a row (the switch cost without caches hiding it)",
    ),
    Factor(
        "slots", "per-agent target dirs: a second agent does not wait on the first one's build lock",
        "concurrent", "desktop-two-agents",
        on={}, off={"JUSTRUST_SLOTS": "0"},
        n=4,
        note="agent A runs `build -p jcode-desktop-ui` after a ui-core body edit; agent B starts 1s later with the "
             "reference loop (test-fn edit, `test --lib -- fps_counter`); B's wall is measured",
    ),
    Factor(
        "early-exit", "return rust-analyzer's coded errors in the changed file at once (JUSTRUST_EARLY_EXIT=1) while "
                      "cargo finishes under the build daemon, vs printing them early and waiting for cargo",
        "early", "desktop-type-error-early",
        on={"JUSTRUST_EARLY": "1", "JUSTRUST_EARLY_EXIT": "1", "JUSTRUST_BUILDD": "1", "JUSTRUST_BUILDD_PREBUILD": "0"},
        off={"JUSTRUST_EARLY": "1", "JUSTRUST_BUILDD": "1", "JUSTRUST_BUILDD_PREBUILD": "0"}, n=8,
        note="type-error edit in fps_counter.rs, `check -p jcode-desktop-ui`, warm early daemon (started untimed) on "
             "both sides; wall is until the command returns. The on side waits (untimed) for the background build",
    ),
    Factor(
        "buildd-coalesce", "resident build daemon: a second agent asking for the identical build attaches to the "
                           "running one instead of waiting for its slot and re-running cargo and the tests "
                           "(default on; JUSTRUST_BUILDD=0)",
        "coalesce", "desktop-two-agents-same",
        on={"JUSTRUST_BUILDD": "1", "JUSTRUST_BUILDD_PREBUILD": "0"}, off={"JUSTRUST_BUILDD": "0"}, n=6,
        note="one test-fn edit, then agent A and, 1s later, agent B run the identical reference-loop command; "
             "B's wall is measured. Off, B joins A's slot (slots.rs join_identical) and runs a no-op cargo after it",
    ),
    Factor(
        "buildd-noop", "same daemon, its cost on a lone no-op `test` (client, socket, source stamp, extra exec)",
        "loop", "desktop-noop-test",
        on={"JUSTRUST_BUILDD": "1", "JUSTRUST_BUILDD_PREBUILD": "0"}, off={"JUSTRUST_BUILDD": "0"}, n=10,
        note="prebuild off on both sides; nothing to coalesce, pure overhead. Benchmarks default to JUSTRUST_BUILDD=0",
    ),
    Factor(
        "buildd-loop", "same daemon on the reference loop (one agent, nothing to coalesce)",
        "loop", "desktop-test-edit",
        on={"JUSTRUST_BUILDD": "1", "JUSTRUST_BUILDD_PREBUILD": "0"}, off={"JUSTRUST_BUILDD": "0"}, n=6,
    ),
    Factor(
        "buildd-prebuild", "resident build daemon prebuilds the agent's last request when its sources change "
                           "(default on; JUSTRUST_BUILDD_PREBUILD=0)",
        "prebuild", "desktop-test-edit",
        on={"JUSTRUST_BUILDD": "1", "JUSTRUST_BUILDD_PREBUILD": "1"}, off={"JUSTRUST_BUILDD": "0"}, n=6,
        note="reference loop with agent think time: edit, wait BUILDD_THINK seconds (default 4.6, the median gap "
             "between an agent's last edit and its build call in 2,241 mined calls), then the build; its wall is "
             "measured. Both sides wait the same",
    ),
    Factor(
        "buildd-prebuild-interference", "cost of another agent's prebuild: agent A edits a ui-core body and its "
                                        "prebuild of `build -p jcode-desktop-ui` starts; agent B (other checkout) "
                                        "runs the reference loop meanwhile",
        "interference", "desktop-two-agents-prebuild",
        on={"JUSTRUST_BUILDD": "1"}, off={"JUSTRUST_BUILDD": "1", "JUSTRUST_BUILDD_PREBUILD": "0"}, n=5,
        note="B's wall is measured. A never asks for its build in the sample: the worst case, a wrong guess that "
             "runs to completion (niced 10)",
    ),
    Factor(
        "parallel-frontend", "rustc -Zthreads=8 for local crates (Desktop's rustc wrapper)",
        "loop", "desktop-test-edit",
        on={}, off={"JCODE_PARALLEL_FRONTEND": "0"},
    ),
    Factor(
        "parallel-frontend-deep", "same, on the worst warm case (jcode-core body edit, 16 crates rebuilt)",
        "loop", "desktop-jcode-core-body",
        on={}, off={"JCODE_PARALLEL_FRONTEND": "0"}, n=4,
    ),
    Factor(
        "mold", "mold linker instead of the system default (cc here links with lld)",
        "loop", "desktop-test-edit",
        on={"JUSTRUST_LINKER": "off"},
        off={"JUSTRUST_LINKER": "off", "CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER": "cc"},
        note="justrust's wild injection off on both sides, so the project's linker is what is compared",
    ),
    Factor(
        "wild", "wild linker for local crates (justrust default once `justrust toolchain wild` ran) instead of "
                "the project's linker (Desktop: clang -fuse-ld=mold)",
        "loop", "desktop-test-edit",
        on={}, off={"JUSTRUST_LINKER": "off"}, n=6,
        note="the shim adds -B ~/.justrust/bin/ld to local links; wild 0.10 is not incremental yet. "
             "Link args are untracked, so both sides share incremental state",
    ),
    Factor(
        "hotpatch", "hot patch the test binary (link only the regenerated codegen units against the base "
                    "binary, loader patches before main) instead of relinking it (opt-in JUSTRUST_HOTPATCH=1)",
        "loop", "desktop-test-edit",
        on={"JUSTRUST_HOTPATCH": "1"}, off={}, n=6,
        note="opt-in, so `on` is the non-default side; both sides link with wild. The first `on` build links a "
             "base (--no-gc-sections + loader), later edits patch. Outcomes in <run dir>/hotpatch.jsonl",
    ),
    Factor(
        "hotpatch-body", "same, on a non-test body edit in the UI crate",
        "loop", "desktop-body-test",
        on={"JUSTRUST_HOTPATCH": "1"}, off={}, n=6,
    ),
    Factor(
        "incremental", "rustc incremental compilation (reuse of unchanged query results and CGUs)",
        "loop", "desktop-test-edit",
        on={}, off={"CARGO_INCREMENTAL": "0"}, n=4,
    ),
    Factor(
        "shadow", "shadow leaf crate (JUSTRUST_SHADOW=1): a test filter naming a leaf module whose file is the only "
                  "change runs in a generated one-file crate instead of the 118k-line UI crate",
        "loop", "desktop-test-edit",
        on={"JUSTRUST_SHADOW": "1"}, off={"JUSTRUST_SHADOW": "0"}, n=6,
        note="warmup is a normal build that records the snapshot; every measured edit touches only fps_counter.rs",
    ),
    Factor(
        "pinned-nightly", "justrust's pinned nightly toolchain instead of the system stable rustc",
        "loop", "desktop-test-edit",
        on={"JUSTRUST_TOOLCHAIN": NIGHTLY}, off={"JUSTRUST_TOOLCHAIN": "system"}, slow=True,
    ),
    Factor(
        "cranelift", "cranelift codegen backend for every crate (pinned nightly in both variants)",
        "loop", "desktop-test-edit",
        on={"JUSTRUST_TOOLCHAIN": NIGHTLY, "RUSTFLAGS": "-Zcodegen-backend=cranelift"},
        off={"JUSTRUST_TOOLCHAIN": NIGHTLY}, slow=True,
    ),
    Factor(
        "cargo-fork", "jr-cargo (justrust's cargo fork, resolve stamp) instead of the toolchain's stock cargo",
        "loop", "desktop-noop-test",
        on={"JUSTRUST_TOOLCHAIN": NIGHTLY},
        off={"JUSTRUST_TOOLCHAIN": NIGHTLY, "JUSTRUST_CARGO_FORK": "0"}, n=10,
        note="no edit, so the wall is cargo + justrust overhead; needs `justrust toolchain cargo-fork` first",
    ),
    Factor(
        "cargo-fork-loop", "same, on the reference edit loop",
        "loop", "desktop-test-edit",
        on={"JUSTRUST_TOOLCHAIN": NIGHTLY},
        off={"JUSTRUST_TOOLCHAIN": NIGHTLY, "JUSTRUST_CARGO_FORK": "0"}, n=8,
    ),
    # jr-cargo fork server (jr-cargo src/jr_daemon.rs): a parked process per
    # exact invocation that already ran create_bcx forks per request. The
    # first `on` run (warmup) starts the server.
    *[Factor(
        name, what, "loop", scenario,
        on={"JUSTRUST_TOOLCHAIN": NIGHTLY, "JUSTRUST_CARGO_DAEMON": "1"},
        off={"JUSTRUST_TOOLCHAIN": NIGHTLY}, n=n,
        note="opt-in JUSTRUST_CARGO_DAEMON=1; needs jr-cargo with jr_daemon installed in the nightly toolchain",
    ) for name, what, scenario, n in [
        ("cargo-daemon", "jr-cargo daemon: fork a parked post-create_bcx cargo per request (no-op test)",
         "desktop-noop-test", 12),
        ("cargo-daemon-check", "same, one-line body edit then `check`", "desktop-body-check", 8),
        ("cargo-daemon-loop", "same, on the reference edit loop", "desktop-test-edit", 8),
    ]],
    # jr-rustc fork changes: both sides run the same jr-rustc build, `on`
    # enables the change.
    *[Factor(
        name, what, "loop", scenario,
        on={**JR_RUSTC_ENV, **UNSTABLE_PROFILE}, off={**JR_RUSTC_ENV, **UNSTABLE_PROFILE},
        on_args=ui_rustflags("-Zcache-proc-macros"), n=6, slow=True, note=note,
    ) for name, what, scenario, note in [
        ("jr-attr-cache", "jr-rustc caches attribute proc-macro expansions (#[gpui::test]) under -Zcache-proc-macros",
         "desktop-test-edit", "FINDINGS 40: 724 #[gpui::test] expansions per edit, 0.96 s; on runs 1"),
        ("jr-attr-cache-check", "same, on the check-only loop", "desktop-body-check", ""),
    ]],
    *[Factor(
        name, what, "loop", scenario,
        on={**JR_RUSTC_ENV, **UNSTABLE_PROFILE},
        off={**JR_RUSTC_ENV, **UNSTABLE_PROFILE, "JUSTRUST_RUSTC_PAR_UPSTREAM_MONO": "0"},
        on_args=ui_rustflags("-Zcache-proc-macros"), off_args=ui_rustflags("-Zcache-proc-macros"),
        n=8, slow=True, note=note,
    ) for name, what, scenario, note in [
        ("jr-par-upstream-mono", "jr-rustc decodes upstream crates' exported generics in parallel, "
         "started during lints (upstream_monomorphizations) instead of serially at the start of the mono walk",
         "desktop-test-edit", "self-profile: all threads blocked 285 ms on upstream_drop_glue_for (623 crates)"),
        ("jr-par-upstream-mono-deep", "same, jcode-core body edit (16 crates rebuilt)",
         "desktop-jcode-core-body", ""),
    ]],
    *[Factor(
        name, what, "loop", scenario,
        on={"JUSTRUST_TOOLCHAIN": CUTOFF_TOOLCHAIN},
        off={"JUSTRUST_TOOLCHAIN": CUTOFF_TOOLCHAIN, "JUSTRUST_CUTOFF": "0", "JUSTRUST_CARGO_CUTOFF": "0"},
        n=n, slow=True, note=note,
    ) for name, what, scenario, n, note in [
        ("jr-cutoff", "cross-crate early cutoff: jr-rustc records each crate's interface hash "
         "(-Zjr-iface-hash, metadata minus spans), jr-cargo skips dependents of a crate whose "
         "interface did not change, jr-rustc loads them against the new build",
         "desktop-upstream-body", 8, "body edit in jcode-desktop-ui-core: 2 sibling dependents cut off"),
        ("jr-cutoff-deep", "same, body edit in jcode-core (Jcode repo, 15 dependents)",
         "desktop-jcode-core-body", 8, "the 15-crate cascade of FINDINGS (jcode_base 5.6 s)"),
        ("jr-cutoff-sig", "same, but the edit adds a pub item: the interface changes, nothing may be cut off",
         "desktop-upstream-sig", 6, "control: on must equal off"),
        ("jr-cutoff-cost", "same, on the UI crate's own body-check loop: nothing to cut off, measures what "
         "the interface hash costs (hashing, larger rmeta of every dependency with padded spans)",
         "desktop-body-check", 12, "cost side: on should equal off"),
    ]],
    *[Factor(
        name, what, "loop", scenario,
        on={"JUSTRUST_TOOLCHAIN": CUTOFF_TOOLCHAIN, "JUSTRUST_TEST_FILTER_COMPILE": "1"},
        off={"JUSTRUST_TOOLCHAIN": CUTOFF_TOOLCHAIN},
        n=n, slow=True, note=note,
    ) for name, what, scenario, n, note in [
        ("jr-test-filter", "test-filter-aware compilation: jr-rustc's harness holds only the tests the "
         "libtest filter selects (JUSTRUST_RUSTC_TEST_FILTER); the other 1311 are type-checked, not codegened",
         "desktop-test-edit", 10, "reference loop, filter fps_counter (3 of 1314 tests)"),
        ("jr-test-filter-body", "same, body edit in non-test code", "desktop-body-test", 8, ""),
    ]],
    Factor(
        "jr-overlap-persist", "jr-rustc persists the incremental state (dep graph, query cache) on one "
        "thread while joining codegen and linking on another, instead of persisting before the link",
        "loop", "desktop-test-edit",
        on={**JR_RUSTC_ENV, **UNSTABLE_PROFILE},
        off={**JR_RUSTC_ENV, **UNSTABLE_PROFILE, "JUSTRUST_RUSTC_OVERLAP_PERSIST": "0"},
        on_args=ui_rustflags("-Zcache-proc-macros"), off_args=ui_rustflags("-Zcache-proc-macros"),
        n=8, slow=True,
        note="profile: serialize_dep_graph 0.57 s + link 0.56 s, serial, of a 4.73 s warm rustc",
    ),
    Factor(
        "jr-early-reply", "jr-rustc replies to justrust's shim after the link and persists its incremental "
        "state in the background (JUSTRUST_RUSTC_EARLY_REPLY=1); the next session waits for it",
        "loop", "desktop-test-edit",
        on={**JR_RUSTC_ENV, **UNSTABLE_PROFILE, "JUSTRUST_RUSTC_EARLY_REPLY": "1"},
        off={**JR_RUSTC_ENV, **UNSTABLE_PROFILE},
        on_args=ui_rustflags("-Zcache-proc-macros"), off_args=ui_rustflags("-Zcache-proc-macros"),
        n=10, slow=True,
        note="needs JR_RUSTC_MODE with the early-reply commit; overlap persist on both sides; "
        "replay: link done 3.41 s, exit 3.82 s",
    ),
    Factor(
        "jr-rustc", "justrust's rustc fork (~/jr-rustc/build/jr/$JR_RUSTC_MODE, default pgo-bolt-stage2) vs the official nightly at the same commit",
        "loop", "desktop-test-edit",
        on={"JUSTRUST_TOOLCHAIN": "system",
            "RUSTC": str(Path.home() / "jr-rustc/build/jr" / os.environ.get("JR_RUSTC_MODE", "pgo-bolt-stage2") / "bin/rustc")},
        off={"JUSTRUST_TOOLCHAIN": "system",
             "RUSTC": str(suite.JR_HOME / "toolchains" / NIGHTLY / "bin" / "rustc")},
        n=4, slow=True,
        note="same rustc commit both sides; build-jr.sh saves each mode (dev, release, pgo-use) under build/jr/",
    ),
    # Pinned-nightly flag experiments (docs/toolchain.md phase 2). Both sides
    # run the pinned nightly; `on` adds one flag through cargo config, so
    # project manifests stay untouched.
    *[Factor(
        name, what, "loop", scenario,
        on={"JUSTRUST_TOOLCHAIN": NIGHTLY, **UNSTABLE_PROFILE}, off={"JUSTRUST_TOOLCHAIN": NIGHTLY},
        on_args=args, n=n, slow=True, note=note,
    ) for name, what, scenario, args, n, note in [
        ("cache-proc-macros", "-Zcache-proc-macros on the UI crate: reuse derive expansions from the incremental cache",
         "desktop-test-edit", ui_rustflags("-Zcache-proc-macros"), 6,
         "only derive macros are cached; `macro_expand_crate` is the largest pass of the loop"),
        ("cache-proc-macros-check", "same, on the check-only loop", "desktop-body-check",
         ui_rustflags("-Zcache-proc-macros"), 6, ""),
        ("share-generics-deps", "-Zshare-generics=y on every dependency (opt-level 2 deps default to off)",
         "desktop-test-edit", ["--config", 'profile.dev.package."*".rustflags=["-Zshare-generics=y"]'], 6,
         "the UI crate could reuse generic instances from gpui and friends instead of codegenning its own; "
         "the first build of this configuration recompiles every dependency"),
        ("hint-mostly-unused", "-Zhint-mostly-unused on the largest dependencies (gpui, wgpu, aws-sdk, ...)",
         "desktop-test-edit", hint_mostly_unused_args(), 6,
         "defers codegen of the hinted crates' items to their users: cheaper dependency builds, "
         "possibly more codegen in the loop"),
    ]],
    Factor(
        "hint-mostly-unused-cold", "same hint, on a cold build of the UI test binary (what it is meant for)",
        "sequence", "desktop-cold-test-build",
        on={"JUSTRUST_TOOLCHAIN": NIGHTLY, **UNSTABLE_PROFILE}, off={"JUSTRUST_TOOLCHAIN": NIGHTLY},
        on_args=hint_mostly_unused_args(), n=2, warmup=0, slow=True,
        cmds=[["test", "-p", "jcode-desktop-ui", "--lib", "--no-run"]],
        note="fresh target dir, depcache off, `test --no-run` (codegen included, unlike check)",
    ),
    Factor(
        "nightly-flags-combo", "-Zcache-proc-macros on the UI crate plus -Zshare-generics=y on dependencies",
        "loop", "desktop-test-edit",
        on={"JUSTRUST_TOOLCHAIN": NIGHTLY, **UNSTABLE_PROFILE}, off={"JUSTRUST_TOOLCHAIN": NIGHTLY},
        on_args=[*ui_rustflags("-Zcache-proc-macros"),
                 "--config", 'profile.dev.package."*".rustflags=["-Zshare-generics=y"]'],
        n=8, slow=True, note="the two flags with a positive (within-noise) median alone",
    ),
    Factor(
        "frontend-threads-16", "rustc -Zthreads=16 instead of 8 for local crates (Desktop's wrapper)",
        "loop", "desktop-test-edit",
        on={"JUSTRUST_TOOLCHAIN": NIGHTLY, "JCODE_FRONTEND_THREADS": "16"},
        off={"JUSTRUST_TOOLCHAIN": NIGHTLY}, n=6, slow=True,
        note="16 hardware threads here; -Zthreads is untracked, so incremental state is the same",
    ),
    Factor(
        "frontend-threads-4", "rustc -Zthreads=4 instead of 8 for local crates (Desktop's wrapper)",
        "loop", "desktop-test-edit",
        on={"JUSTRUST_TOOLCHAIN": NIGHTLY, "JCODE_FRONTEND_THREADS": "4"},
        off={"JUSTRUST_TOOLCHAIN": NIGHTLY}, n=6, slow=True,
    ),
]
BY_NAME = {f.name: f for f in FACTORS}


def ledger_sum(samples: list[dict]) -> dict:
    """Median reuse ledger across samples (each sample may be several runs)."""
    keys = samples[0]["reuse"].keys()
    return {k: round(statistics.median(s["reuse"][k] for s in samples), 2) for k in keys}


def add_ledgers(rs: list[dict]) -> dict:
    out: dict = {}
    for r in rs:
        for k, v in r.items():
            out[k] = round(out.get(k, 0) + v, 2)
    return out


def variant_scenario(f: Factor, v: str) -> suite.Scenario:
    base = suite.BY_NAME[f.scenario]
    env = {**base.env, **(f.on if v == "on" else f.off)}
    extra = f.on_args if v == "on" else f.off_args
    cmd = [base.cmd[0], *extra, *base.cmd[1:]]
    return dataclasses.replace(base, env=env, cmd=cmd)


def cache_env(f: Factor) -> dict[str, str]:
    if not f.private_cache:
        return {}
    return {"JUSTRUST_DEPCACHE_DIR": str(FACTOR_WORK / "cache" / f.name),
            "JUSTRUST_LOCALCACHE_DIR": str(FACTOR_WORK / "cache" / f"{f.name}-local")}


def sample_loop(f: Factor, v: str, i: int, base_n: int) -> dict:
    s = variant_scenario(f, v)
    root = suite.prepare(s.snapshot)
    # Keyed by the build configuration, not the factor: every factor's `on`
    # side of the same workload is the same build and shares one warm dir.
    # Extra cargo args are part of the configuration; plain commands keep
    # their old key so existing warm dirs stay valid.
    parts = [s.snapshot, sorted(s.env.items())] + ([s.cmd] if f.on_args or f.off_args else [])
    key = hashlib.sha1(json.dumps(parts).encode()).hexdigest()[:10]
    target = FACTOR_WORK / "target" / f"loop-{s.snapshot}-{key}"
    try:
        if s.edit:
            suite.apply_edit(root, s.edit, base_n + 2 * i + (v == "off"))
        return suite.run_once(s, root, target)
    finally:
        suite.restore(root, s.edit)


def sample_cold(f: Factor, v: str, i: int, base_n: int) -> dict:
    s = variant_scenario(f, v)
    s = dataclasses.replace(s, env={**s.env, **cache_env(f)})
    root = suite.prepare(s.snapshot)
    target = FACTOR_WORK / "target" / f"{f.name}-{v}-cold-{os.getpid()}-{i}"
    try:
        return suite.run_once(s, root, target)
    finally:
        shutil.rmtree(target, ignore_errors=True)


def sample_sequence(f: Factor, v: str, i: int, base_n: int) -> dict:
    env = {"JUSTRUST_DEPCACHE": "0", "JUSTRUST_BUILD_SCRIPT_CACHE": "0",
           **(f.on if v == "on" else f.off)}
    root = suite.prepare("desktop")
    target = FACTOR_WORK / "target" / f"{f.name}-{v}-seq-{os.getpid()}-{i}"
    extra = f.on_args if v == "on" else f.off_args
    runs = []
    try:
        for cmd in ([c[0], *extra, *c[1:]] for c in f.cmds):
            s = suite.Scenario(f"{f.name}:{' '.join(cmd)}", "desktop", cmd, None, "", cold=True, env=env)
            runs.append(suite.run_once(s, root, target))
    finally:
        shutil.rmtree(target, ignore_errors=True)
    return {
        "id": ",".join(r["id"] for r in runs),
        "wall": round(sum(r["wall"] for r in runs), 3),
        "cpu": round(sum(r["cpu"] for r in runs), 2),
        "units": sum(r["units"] for r in runs),
        "steps": [{"cmd": " ".join(c), "wall": r["wall"], "units": r["units"]} for c, r in zip(f.cmds, runs)],
        "reuse": add_ledgers([r["reuse"] for r in runs]),
        "noisy": any(r["noisy"] for r in runs),
        "other_cores": max(r["other_cores"] for r in runs),
    }


def concurrent_root(v: str) -> Path:
    """A reflink copy of the Desktop snapshot sources with the default
    target dir inside (slots only engage when no target dir is forced)."""
    src = suite.prepare("desktop")
    root = FACTOR_WORK / "slots" / v
    if not (root / ".copied").exists():
        shutil.rmtree(root, ignore_errors=True)
        root.mkdir(parents=True)
        for repo in ("jcode", "jcode-desktop"):
            subprocess.run(["cp", "-a", "--reflink=auto", str(src / repo), str(root / repo)], check=True)
        (root / ".copied").write_text("")
    return root


def run_agent(cmd: list[str], cwd: Path, env: dict[str, str]) -> tuple[subprocess.Popen, str, float]:
    rid = suite.new_id()
    e = dict(os.environ)
    e.pop("CARGO_TARGET_DIR", None)
    e.pop("JUSTRUST_DISABLE", None)
    e.setdefault("JUSTRUST_BUILDD", "0")
    e.update({"JUSTRUST_RUN_ID_OVERRIDE": rid, "JUSTRUST_REMOTE": "0", "JUSTRUST_QUIET": "1",
              "JUSTRUST_TOOLCHAIN": "system", "JUSTRUST_PREBUILT": "off", **env})
    t0 = time.monotonic()
    p = subprocess.Popen([suite.J, *cmd], cwd=cwd, env=e, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True)
    return p, rid, t0


def summary(rid: str) -> dict:
    return json.loads((suite.JR_HOME / "runs" / rid / "summary.json").read_text())


def sample_concurrent(f: Factor, v: str, i: int, base_n: int) -> dict:
    root = concurrent_root(v)
    cwd = root / "jcode-desktop"
    venv = f.on if v == "on" else f.off
    a_edit = suite.BY_NAME["desktop-upstream-body"].edit
    b_edit = suite.BY_NAME["desktop-test-edit"].edit
    n = base_n + 2 * i + (v == "off")
    # Agent A is the intended load. Noise = foreign rustc before A starts.
    _, rustc = suite.foreign_load()
    try:
        suite.apply_edit(root, a_edit, n)
        pa, ra, ta = run_agent(["build", "-p", "jcode-desktop-ui"], cwd,
                               {"JCODE_SESSION_ID": "bench-agent-a", **venv})
        time.sleep(1.0)
        suite.apply_edit(root, b_edit, n)
        pb, rb, tb = run_agent(["test", "-p", "jcode-desktop-ui", "--lib", "--", "fps_counter"], cwd,
                               {"JCODE_SESSION_ID": "bench-agent-b", "JUSTRUST_PASSES": "always", **venv})
        _, err_b = pb.communicate()
        wall_b = time.monotonic() - tb
        _, err_a = pa.communicate()
        wall_a = time.monotonic() - ta
        if pa.returncode or pb.returncode:
            sys.exit(f"factors: {f.name}: agent exit A={pa.returncode} B={pb.returncode}\n{err_a[-2000:]}\n{err_b[-2000:]}")
    finally:
        suite.restore(root, a_edit)
        suite.restore(root, b_edit)
    sb = summary(rb)
    res = sb.get("resources", {})
    slot = sb.get("slot") or {}
    return {
        "id": f"{ra},{rb}",
        "wall": round(wall_b, 3),
        "wall_a": round(wall_a, 3),
        "cpu": round(sb.get("cpu_secs", 0.0), 2),
        "units": sb["units"]["compiled"],
        "lock_wait": round(sb["phases"].get("lock_wait", 0.0), 3),
        "slot": slot.get("how") or "none",
        "reuse": suite.reuse_ledger(sb, suite.JR_HOME / "runs" / rb),
        "noisy": rustc > 0,
        "other_cores": round(res.get("avg_other_cores", 0.0), 2),
    }


def sample_revert(f: Factor, v: str, i: int, base_n: int) -> dict:
    """Edit, build (unmeasured, its wall is kept as `edit_wall`: the miss
    path), undo the edit, build again (measured). The undo writes the
    original text with a fresh mtime, as an editor or `git checkout` does,
    so cargo sees a change."""
    s = variant_scenario(f, v)
    root = suite.prepare(s.snapshot)
    key = hashlib.sha1(json.dumps([s.snapshot, sorted(s.env.items()), "revert"]).encode()).hexdigest()[:10]
    target = FACTOR_WORK / "target" / f"revert-{s.snapshot}-{key}"
    try:
        suite.apply_edit(root, s.edit, base_n + 2 * i + (v == "off"))
        first = suite.run_once(s, root, target)
        (root / s.edit.file).write_text(suite.pristine(root, s.edit.file).read_text())
        r = suite.run_once(s, root, target)
    finally:
        suite.restore(root, s.edit)
    r["edit_wall"] = first["wall"]
    r["edit_id"] = first["id"]
    return r


def sample_fresh_dir(f: Factor, v: str, i: int, base_n: int) -> dict:
    """A fresh target dir of the same sources (new slot, `cargo clean`, a
    second checkout at the same path). Both caches are private and shared
    by both variants, so dependencies hit either way."""
    s = variant_scenario(f, v)
    # cold: no forced -Ztime-passes, which keeps units out of the depcache.
    s = dataclasses.replace(s, env={**s.env, **cache_env(f)}, cold=True)
    root = suite.prepare(s.snapshot)
    target = FACTOR_WORK / "target" / f"{f.name}-{v}-fresh-{os.getpid()}-{i}"
    try:
        return suite.run_once(s, root, target)
    finally:
        shutil.rmtree(target, ignore_errors=True)


def sample_coalesce(f: Factor, v: str, i: int, base_n: int) -> dict:
    """Two agents, identical request: one edit, A starts, B 1s later with
    the same command. B's wall is measured (A's is kept as wall_a)."""
    root = concurrent_root(v)
    cwd = root / "jcode-desktop"
    venv = f.on if v == "on" else f.off
    edit = suite.BY_NAME["desktop-test-edit"].edit
    cmd = ["test", "-p", "jcode-desktop-ui", "--lib", "--", "fps_counter"]
    _, rustc = suite.foreign_load()
    try:
        suite.apply_edit(root, edit, base_n + 2 * i + (v == "off"))
        pa, ra, ta = run_agent(cmd, cwd, {"JCODE_SESSION_ID": "bench-agent-a", **venv})
        time.sleep(1.0)
        pb, rb, tb = run_agent(cmd, cwd, {"JCODE_SESSION_ID": "bench-agent-b", **venv})
        _, err_b = pb.communicate()
        wall_b = time.monotonic() - tb
        _, err_a = pa.communicate()
        wall_a = time.monotonic() - ta
        if pa.returncode or pb.returncode:
            sys.exit(f"factors: {f.name}: agent exit A={pa.returncode} B={pb.returncode}\n{err_a[-2000:]}\n{err_b[-2000:]}")
    finally:
        suite.restore(root, edit)
    sa, sb = summary(ra), summary(rb)
    res = sb.get("resources", {})
    shared = "shared an identical build" in err_b
    return {
        "id": f"{ra},{rb}",
        "wall": round(wall_b, 3),
        "wall_a": round(wall_a, 3),
        # CPU of both agents' runs (one run when B attached).
        "cpu": round(sa.get("cpu_secs", 0.0) + (0.0 if shared else sb.get("cpu_secs", 0.0)), 2),
        "units": sb["units"]["compiled"],
        "shared": shared,
        "slot": (sb.get("slot") or {}).get("how") or "none",
        "reuse": suite.reuse_ledger(sb, suite.JR_HOME / "runs" / rb),
        "noisy": rustc > 0,
        "other_cores": round(res.get("avg_other_cores", 0.0), 2),
    }


def buildd_idle(timeout: float = 120.0) -> None:
    """Wait until the build daemon runs nothing (a prebuild of the restored
    file must not overlap the next sample)."""
    t = time.monotonic()
    while time.monotonic() - t < timeout:
        out = subprocess.run([suite.J, "daemon", "status"], capture_output=True, text=True).stdout
        if " 0 running" in out or "no build daemon" in out:
            return
        time.sleep(0.5)


def sample_prebuild(f: Factor, v: str, i: int, base_n: int) -> dict:
    s = variant_scenario(f, v)
    root = suite.prepare(s.snapshot)
    key = hashlib.sha1(json.dumps([s.snapshot, sorted(s.env.items()), "prebuild"]).encode()).hexdigest()[:10]
    target = FACTOR_WORK / "target" / f"prebuild-{s.snapshot}-{key}"
    think = float(os.environ.get("BUILDD_THINK", "4.6"))
    try:
        buildd_idle()
        # The daemon prebuilds requests it has seen: one unmeasured run first.
        suite.apply_edit(root, s.edit, base_n + 4 * i + 2 * (v == "off"))
        suite.run_once(s, root, target)
        buildd_idle()
        suite.apply_edit(root, s.edit, base_n + 4 * i + 2 * (v == "off") + 1)
        time.sleep(think)
        r = suite.run_once(s, root, target)
    finally:
        suite.restore(root, s.edit)
    buildd_idle()
    r["think"] = think
    return r


def sample_early(f: Factor, v: str, i: int, base_n: int) -> dict:
    s = variant_scenario(f, v)
    root = suite.prepare(s.snapshot)
    key = hashlib.sha1(json.dumps([s.snapshot, "early-exit"]).encode()).hexdigest()[:10]
    # One target for both sides: the background build of `on` and the
    # foreground build of `off` compile the same thing.
    target = FACTOR_WORK / "target" / f"early-{s.snapshot}-{key}"
    cwd = root / suite.SNAPSHOTS[s.snapshot]["cwd"]
    if i == 0:
        suite.restore(root, s.edit)
        subprocess.run([suite.J, "early", "start"], cwd=cwd, check=True)
    try:
        buildd_idle()
        suite.apply_edit(root, s.edit, base_n + 2 * i + (v == "off"))
        r = suite.run_once(s, root, target)
    finally:
        suite.restore(root, s.edit)
    buildd_idle()
    return r


def sample_interference(f: Factor, v: str, i: int, base_n: int) -> dict:
    venv = f.on if v == "on" else f.off
    a_root = concurrent_root("ia")
    s = variant_scenario(f, v) if False else suite.BY_NAME["desktop-test-edit"]
    s = dataclasses.replace(s, env={**s.env, "JUSTRUST_BUILDD": "0"})
    b_root = suite.prepare("desktop")
    target = FACTOR_WORK / "target" / "interference-b"
    a_edit = suite.BY_NAME["desktop-upstream-body"].edit
    n = base_n + 2 * i + (v == "off")
    try:
        buildd_idle()
        # Teach the daemon A's request (unmeasured).
        pa, _, _ = run_agent(["build", "-p", "jcode-desktop-ui"], a_root / "jcode-desktop",
                             {"JCODE_SESSION_ID": "bench-agent-a", **venv})
        pa.communicate()
        buildd_idle()
        suite.apply_edit(a_root, a_edit, n)  # A's prebuild starts 0.4 s later (on)
        time.sleep(0.5)
        suite.apply_edit(b_root, s.edit, n)
        r = suite.run_once(s, b_root, target)
    finally:
        suite.restore(b_root, s.edit)
        suite.restore(a_root, a_edit)
        buildd_idle(300)
    return r


SAMPLERS = {"early": sample_early, "interference": sample_interference, "prebuild": sample_prebuild, "coalesce": sample_coalesce, "loop": sample_loop, "cold": sample_cold, "sequence": sample_sequence, "concurrent": sample_concurrent,
            "revert": sample_revert, "fresh": sample_fresh_dir}


def vstats(samples: list[dict]) -> dict:
    w = [x["wall"] for x in samples]
    return {
        "n": len(w),
        "median": suite.med(w),
        "min": round(min(w), 3),
        "max": round(max(w), 3),
        "cpu": suite.med([x["cpu"] for x in samples]),
        "units": suite.med([x["units"] for x in samples]),
        "noisy": sum(1 for x in samples if x["noisy"]),
        "reuse": ledger_sum(samples),
    }


def verdict(on: dict, off: dict, floor: float) -> str:
    if not off["median"]:
        return "-"
    pct = 100 * (off["median"] - on["median"]) / off["median"]
    if on["max"] < off["min"] and pct > floor:
        return "helps"
    if on["min"] > off["max"] and -pct > floor:
        return "hurts"
    return "within noise"


def binary_fingerprint() -> str:
    """Identity of the justrust binary on PATH. Other agents reinstall it
    while a run is going (seen: a depcache key-scheme change mid-run turned
    one 'on' sample into 833 misses), which silently mixes versions."""
    path = shutil.which(suite.J) or suite.J
    st = os.stat(path)
    return f"{path}:{st.st_size}:{st.st_mtime_ns}"


def run_factor(f: Factor, n: int | None, warmup: int | None, quiet_wait: float) -> dict:
    n = f.n if n is None else n
    warmup = f.warmup if warmup is None else warmup
    sampler = SAMPLERS[f.kind]
    base_n = int(time.time()) % 100000 * 100
    samples: dict[str, list[dict]] = {"on": [], "off": []}
    fp = binary_fingerprint()
    for i in range(warmup + n):
        # ABBA: alternate which variant goes first, so drift cancels.
        order = ("on", "off") if i % 2 == 0 else ("off", "on")
        for v in order:
            if binary_fingerprint() != fp:
                sys.exit(f"factors: {f.name}: justrust was reinstalled during the run ({fp} -> "
                         f"{binary_fingerprint()}); results would mix versions. Rerun this factor.")
            if quiet_wait:
                suite.wait_quiet(quiet_wait)
            r = sampler(f, v, i, base_n)
            measured = i >= warmup
            print(f"  {f.name} {v:3s} {i + 1}/{warmup + n}: {r['wall']:.2f}s units {r['units']} "
                  f"reuse {r['reuse']} id {r['id']}"
                  f"{' link %.2fs' % r['split']['link'] if 'split' in r else ''}"
                  f"{' hotpatch %s' % r['hotpatch'] if r.get('hotpatch') else ''}"
                  f"{' noisy' if r['noisy'] else ''}{'' if measured else ' (warmup)'}",
                  file=sys.stderr)
            if measured:
                samples[v].append(r)
    on, off = vstats(samples["on"]), vstats(samples["off"])
    return {
        "what": f.what, "kind": f.kind, "workload": f.scenario, "note": f.note,
        "on_env": f.on, "off_env": f.off,
        "samples": samples, "on": on, "off": off,
        "saved_secs": round(off["median"] - on["median"], 3),
        "justrust": fp,
        "saved_pct": round(100 * (off["median"] - on["median"]) / off["median"], 1) if off["median"] else 0.0,
    }


def table(factors: dict, floor: float = 10.0) -> str:
    rows = [f"| factor | workload | on | off | saved | units on/off | reused on/off | depcache saved on | verdict |",
            "|---|---|---:|---:|---:|---:|---:|---:|---|"]
    for name, r in factors.items():
        on, off = r["on"], r["off"]
        ro, rf = on["reuse"], off["reuse"]
        reused_on = ro["fresh"] + ro["depcache_hits"] + ro["slot_refreshed_units"]
        reused_off = rf["fresh"] + rf["depcache_hits"] + rf["slot_refreshed_units"]
        rows.append(
            f"| {name} | {r['workload']} | {on['median']:.2f}s ({on['min']:.1f}-{on['max']:.1f}) | "
            f"{off['median']:.2f}s ({off['min']:.1f}-{off['max']:.1f}) | {r['saved_secs']:+.2f}s ({r['saved_pct']:+.0f}%) | "
            f"{on['units']:.0f}/{off['units']:.0f} | {reused_on:.0f}/{reused_off:.0f} | "
            f"{ro['depcache_saved_secs']:.0f}s | {verdict(on, off, floor)} |")
    return "\n".join(rows)


def cmd_list(_args) -> None:
    for f in FACTORS:
        tag = " [slow, --all]" if f.slow else ""
        print(f"{f.name:24s} {f.what}{tag}\n{'':24s} workload: {f.kind} {f.scenario}; off: {f.off or '(default)'}"
              + (f"; on: {f.on}" if f.on else ""))


def cmd_run(args) -> None:
    names = args.factors or [f.name for f in FACTORS if args.all or not f.slow]
    for n in names:
        if n not in BY_NAME:
            sys.exit(f"factors: unknown factor {n} (see `bench/factors.py list`)")
    if any(BY_NAME[n].on.get("JUSTRUST_TOOLCHAIN") == NIGHTLY or BY_NAME[n].off.get("JUSTRUST_TOOLCHAIN") == NIGHTLY
           for n in names):
        subprocess.run([suite.J, "toolchain", "install", NIGHTLY], check=True, stdout=subprocess.DEVNULL)
    info = suite.env_info()
    out = Path(args.out) if args.out else suite.RESULTS / (
        f"factors-{time.strftime('%Y%m%d-%H%M%S')}-{info['justrust_rev']}"
        + ("-dirty" if info["justrust_dirty"] else "") + ".json")
    result = {"started": time.strftime("%Y-%m-%dT%H:%M:%S%z"), "env": info, "factors": {}}
    for k, name in enumerate(names):
        print(f"factors: {name} ({k + 1}/{len(names)})", file=sys.stderr)
        result["factors"][name] = run_factor(BY_NAME[name], args.n, args.warmup, args.quiet_wait)
        # Save after every factor: a full run takes an hour or more.
        result["finished"] = time.strftime("%Y-%m-%dT%H:%M:%S%z")
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(json.dumps(result, indent=1) + "\n")
    print(table(result["factors"]))
    print(f"\nsaved {out}")
    write_report()


def newest_per_factor() -> dict[str, tuple[str, dict, dict]]:
    best: dict[str, tuple[str, dict, dict]] = {}
    for p in sorted(suite.RESULTS.glob("factors-*.json")):
        r = json.loads(p.read_text())
        for name, fr in r["factors"].items():
            best[name] = (p.name, r["env"], fr)
    return best


def write_report() -> None:
    best = newest_per_factor()
    lines = [
        "# What each factor is worth (System 2)",
        "",
        "Generated by `bench/factors.py report` from the newest result per factor in",
        "`bench/results/factors-*.json`. Each factor is switched off alone, with",
        "everything else at the current default, on the pinned snapshots of",
        "`bench/suite.py`. On/off runs interleaved, medians with min-max. `saved` is",
        "off minus on: what the factor buys on that workload. `helps` needs",
        "disjoint ranges and a median move above 10%.",
        "",
        "Reuse columns: `units` = rustc units actually compiled; `reused` = units",
        "not recompiled (cargo-fresh + depcache hits + slot-refresh copies);",
        "`depcache saved` = recorded rustc time of the original builds those hits",
        "replaced.",
        "",
        table({k: v[2] for k, v in best.items()}),
        "",
        "## Factors and workloads",
        "",
    ]
    for f in FACTORS:
        src = best.get(f.name)
        where = f"`{src[0]}` (justrust {src[1]['justrust_rev']})" if src else "not run yet"
        lines.append(f"- **{f.name}**: {f.what}. Workload `{f.scenario}` ({f.kind})"
                     + (f": {f.note}" if f.note else "") + f". Off: `{f.off}`. Result: {where}.")
    lines += ["", "Not here: the scheduler under contention (`bench/sched-contention.sh`),",
              "remote builds, crate splits (`bench/split-*.sh`, they change the source)."]
    FACTORS_MD.write_text("\n".join(lines) + "\n")
    print(f"wrote {FACTORS_MD}", file=sys.stderr)


def cmd_clean(_args) -> None:
    shutil.rmtree(FACTOR_WORK, ignore_errors=True)
    print(f"removed {FACTOR_WORK}")


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("list").set_defaults(fn=cmd_list)
    r = sub.add_parser("run")
    r.add_argument("factors", nargs="*")
    r.add_argument("--all", action="store_true", help="include slow factors")
    r.add_argument("-n", type=int, help="measured runs per variant (default: per factor)")
    r.add_argument("--warmup", type=int, help="discarded runs per variant first (default: per factor)")
    r.add_argument("--quiet-wait", type=float, default=0, metavar="SECS")
    r.add_argument("-o", "--out")
    r.set_defaults(fn=cmd_run)
    sub.add_parser("report").set_defaults(fn=lambda a: write_report())
    sub.add_parser("clean").set_defaults(fn=cmd_clean)
    args = ap.parse_args()
    args.fn(args)


if __name__ == "__main__":
    main()
