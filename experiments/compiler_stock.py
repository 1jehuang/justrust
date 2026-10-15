#!/usr/bin/env python3
"""Matched fresh-build, warm-machine timings. Not a cold-cache benchmark."""
import argparse
import hashlib
import json
from pathlib import Path
import platform
import subprocess
import tempfile
import time
from compiler import source_for


def run(command):
    result = subprocess.run(command, capture_output=True, timeout=120)
    if result.returncode:
        raise RuntimeError(f"{command!r}\n{result.stdout!r}\n{result.stderr!r}")
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--rustc", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    with args.output.open("x") as receipt, tempfile.TemporaryDirectory(prefix="jr-stock-") as temporary:
        directory = Path(temporary)
        def record(value):
            receipt.write(json.dumps(value, sort_keys=True) + "\n")
            receipt.flush()
        record({"record":"environment", "revision":run(["git","rev-parse","HEAD"]).stdout.decode().strip(),
                "rustc":run([str(args.rustc),"-Vv"]).stdout.decode(),
                "binary_sha256":hashlib.sha256(binary.read_bytes()).hexdigest(),
                "platform":platform.platform(), "topology":run(["lscpu"]).stdout.decode(),
                "measurement":"fresh-process/fresh-output/warm-machine/contended-uncontrolled",
                "cargo":False,"incremental":False,"repeats":7,"warmups_per_tool_and_source":1,
                "rustc_flags":["--edition=2024","--crate-name","reference","-C","opt-level=0","-C","overflow-checks=yes"],
                "prototype_workers":1})
        for count in (4,256):
            text = source_for(count)
            source = directory / "input.rs"
            source.write_text(text)
            for repeat in range(-1,7):
                for tool in (("rustc","prototype") if repeat % 2 else ("prototype","rustc")):
                    output = directory / f"out-{count}-{repeat}-{tool}"
                    if tool == "rustc":
                        command = [str(args.rustc),"--edition=2024","--crate-name","reference","-C","opt-level=0","-C","overflow-checks=yes",str(source),"-o",str(output)]
                    else:
                        command = [str(binary),str(source),"--workers","1","-o",str(output)]
                    start = time.perf_counter_ns()
                    result = run(command)
                    elapsed = time.perf_counter_ns()-start
                    execution = run([str(output)])
                    if execution.stdout != b"compiled correctly\n" or execution.stderr:
                        raise RuntimeError(f"invalid generated program: {execution!r}")
                    row = {"record":"sample","functions":count+2,"source_bytes":len(text.encode()),
                           "source_sha256":hashlib.sha256(text.encode()).hexdigest(),"tool":tool,
                           "repeat":repeat,"warmup":repeat<0,"process_wall_ns":elapsed,
                           "executable_bytes":output.stat().st_size,"executable_checked":True}
                    if tool == "prototype":
                        detail=json.loads(result.stdout)
                        if not detail["capture_complete"] or detail["failures"] or detail["events_dropped"]:
                            raise RuntimeError(f"incomplete prototype capture: {detail}")
                        row["in_process_ns"]=detail["total_ns"]
                    record(row)
                    print(json.dumps(row),flush=True)


if __name__ == "__main__":
    main()
