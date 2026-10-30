#!/usr/bin/env python3
"""Pinned clean-room frontend diagnostic, not the global promotion gate.

Uses the suite harness, including exact run IDs, hosted-route validation,
noise flags and byte-for-byte restoration of the pinned source snapshot.
"""
import argparse
import sys

import suite

PIN = "7bc3f6e5f"
NAME = "rewrite-frontend-check"


def configure(incremental: bool) -> None:
    suite.OPTIONAL_SNAPSHOTS["rewrite"]["repos"]["justrust-clean-room"] = (
        suite.HOME / "justrust-clean-room", PIN
    )
    scenario = suite.Scenario(
        NAME,
        "rewrite",
        ["check", "--lib", "--config", f"profile.dev.incremental={str(incremental).lower()}"],
        suite.Edit(
            "justrust-clean-room/src/lib.rs",
            "pub mod compiler;",
            "pub mod compiler;\nconst _: u64 = {n};\n",
        ),
        "Pinned compiler library check after a crate item edit, "
        f"opt-level=1, debug=2, incremental={incremental}",
    )
    suite.SCENARIOS.append(scenario)
    suite.BY_NAME[NAME] = scenario


def main() -> None:
    parser = argparse.ArgumentParser(add_help=False)
    parser.add_argument("--incremental", choices=("on", "off"), default="on")
    args, remaining = parser.parse_known_args()
    configure(args.incremental == "on")
    if not remaining or remaining[0] != "run":
        parser.error("expected suite run arguments")
    sys.argv = [sys.argv[0], "run", NAME, *remaining[1:]]
    suite.main()


if __name__ == "__main__":
    main()
