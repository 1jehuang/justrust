#!/usr/bin/env python3
"""Fresh-checkout optimized release build of uv, on the README uv harness.

Reuses bench/readme_uv.py unchanged (pinned source, frozen client, rotating
stack order, hosted-fallback detection, receipts) with a single row:
`build --release -p uv --locked`. uv's release profile is fat LTO with one
codegen unit, so this is the full optimized binary, not a check.

    python3 bench/readme_uv_release.py --stacks stock,remote --rounds 3
"""
import sys

import readme_uv as harness

ROW = "fresh-release-build"
harness.ROWS = {ROW: ["build", "--release", "-p", "uv", "--locked"]}


def run_rounds(result, output, archive, binaries, work):
    for round_index in range(result["rounds"]):
        for stack in harness.rotating_order(round_index, tuple(result["stacks"])):
            if harness.completed_sample(result, ROW, stack, round_index):
                continue
            checkout = harness.fresh_checkout(archive, work, ROW, stack, round_index)
            harness.execute(result, output, stack, ROW, round_index, checkout, binaries, work)


harness.run_rounds = run_rounds

if __name__ == "__main__":
    sys.exit(harness.main())
