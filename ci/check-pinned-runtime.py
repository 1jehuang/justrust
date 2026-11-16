#!/usr/bin/env python3
"""Exercise pinned runtime-sensitive tests before the expensive full gate."""
import argparse
import os
from pathlib import Path
import shutil
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "bench"))
import runtime_context
import suite
import versus


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("work", type=Path)
    args = parser.parse_args()
    root = suite.prepare("desktop") / "jcode"
    env = dict(os.environ)
    env.update(JUSTRUST_LOCAL="1", JUSTRUST_NO_UPDATE="1",
               JUSTRUST_LOCAL_REASON="Controlled clean runner pinned runtime preflight",
               CARGO_TARGET_DIR=str(args.work / "runtime-preflight-target"))
    for key in ("JUSTRUST_REMOTE_REQUIRED", "JUSTRUST_TOOLCHAIN_REQUIRED", "JUSTRUST_TOOLCHAIN"):
        env.pop(key, None)
    os.environ[runtime_context.SELECTOR] = runtime_context.POLICY
    # Run the entire Bedrock unit binary, not just the formerly failing test:
    # earlier tests write bedrock.env and exposed the JCODE_HOME contamination.
    for index in range(2):
        log = args.work / "evidence" / f"runtime-preflight-{index}.log"
        result = versus.run_counted(
            [str(args.binary.resolve()), "test", "-p", "jcode-provider-bedrock", "--lib"],
            root, env, log)
        print(log.read_text(), end="", flush=True)
        if result["exit"]:
            raise SystemExit(result["exit"])
    shutil.rmtree(args.work / "runtime-preflight-target")


if __name__ == "__main__":
    main()
