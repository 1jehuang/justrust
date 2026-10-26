#!/usr/bin/env python3
"""Pinned clean-room test-edit diagnostic using suite.py's measurement harness.

Run both profiles, then compare with suite.py. This is a scoped diagnostic,
not the session-weighted gate for changing justrust's global defaults.
Execution is local unless --remote is explicitly passed to the suite.

  python3 bench/cleanroom-test-profile.py --debug-info 2 run -n 3 -o before.json
  python3 bench/cleanroom-test-profile.py --debug-info 1 run -n 3 -o after.json
  python3 bench/suite.py compare before.json after.json
"""
import argparse
import sys

import suite


PIN = "7bc3f6e5f"
NAME = "rewrite-lib-test-edit"


def configure(debug_info: int, lto: str | None = None) -> None:
    suite.OPTIONAL_SNAPSHOTS["rewrite"]["repos"]["justrust-clean-room"] = (
        suite.HOME / "justrust-clean-room", PIN
    )
    scenario = suite.Scenario(
        NAME,
        "rewrite",
        ["test", "--config", f"profile.dev.debug={debug_info}", "--lib",
         "mutable_callable_instantiation_preserves_writable_carrier_envelope"],
        suite.Edit(
            "justrust-clean-room/src/lib.rs",
            "pub mod compiler;",
            "pub mod compiler;\n\n#[test]\nfn latency_probe() {{\n"
            "    assert_eq!(std::hint::black_box({n}u64), {n}u64);\n}}\n",
        ),
        "One test-body edit in the pinned compiler library, then a filtered semantic test; "
        f"opt-level=1, debug={debug_info}, assertions and overflow checks unchanged",
    )
    if lto is not None:
        scenario.cmd.extend(["--config", f'profile.test.lto="{lto}"'])
        scenario.what += f", test LTO={lto}"
    suite.SCENARIOS.append(scenario)
    suite.BY_NAME[NAME] = scenario


def main() -> None:
    parser = argparse.ArgumentParser(add_help=False)
    parser.add_argument("--debug-info", type=int, choices=(0, 1, 2), required=True)
    parser.add_argument("--lto", choices=("off",))
    args, remaining = parser.parse_known_args()
    configure(args.debug_info, args.lto)
    if not remaining or remaining[0] != "run":
        parser.error("expected suite run arguments after --debug-info")
    # Never silently run the full default suite from this diagnostic entry point.
    sys.argv = [sys.argv[0], "run", NAME, *remaining[1:]]
    suite.main()


if __name__ == "__main__":
    main()
