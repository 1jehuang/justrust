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
    snapshot = suite.prepare("desktop")
    root = snapshot / "jcode"
    env = dict(os.environ)
    env.update(JUSTRUST_LOCAL="1", JUSTRUST_NO_UPDATE="1",
               JUSTRUST_LOCAL_REASON="Controlled clean runner pinned runtime preflight",
               CARGO_TARGET_DIR=str(args.work / "runtime-preflight-target"))
    for key in ("JUSTRUST_REMOTE_REQUIRED", "JUSTRUST_TOOLCHAIN_REQUIRED", "JUSTRUST_TOOLCHAIN"):
        env.pop(key, None)
    os.environ[runtime_context.SELECTOR] = runtime_context.POLICY
    # Run the entire Bedrock unit binary, not just the formerly failing test:
    # earlier tests write bedrock.env and exposed the JCODE_HOME contamination.
    # Catalog discovery shares the Responses listener. Exercise the forced
    # catalog-first regression and ALL provider tests with normal parallelism,
    # then the exact formerly failing full-workspace command. No filtered test
    # result can stand in for workspace acceptance before the forty-point gate.
    commands = [
        ["test", "-p", "jcode-provider-bedrock", "--lib"],
        ["test", "-p", "jcode-provider-bedrock", "--lib"],
        ["test", "-p", "jcode-provider-openai-runtime", "--lib"],
        ["test", "-p", "jcode-provider-openai-runtime", "--lib"],
        ["test", "--workspace"],
    ]
    # Desktop has independent integration tests, including real Git fixture
    # commits. The jcode workspace cannot substitute for Desktop acceptance.
    commands.append(["test", "--workspace"])
    for index, command in enumerate(commands):
        cwd = snapshot / "jcode-desktop" if index == len(commands) - 1 else root
        log = args.work / "evidence" / f"runtime-preflight-{index}.log"
        result = versus.run_counted(
            [str(args.binary.resolve()), *command], cwd, env, log)
        print(log.read_text(), end="", flush=True)
        if result["exit"]:
            raise SystemExit(result["exit"])
    shutil.rmtree(args.work / "runtime-preflight-target")


if __name__ == "__main__":
    main()
