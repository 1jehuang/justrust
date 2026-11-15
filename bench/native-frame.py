#!/usr/bin/env python3
"""Focused TLS loopback ABBA diagnostic, not a general build speed gate.

Run: python3 bench/native-frame.py
The coordinator must serialize this hosted job with other Rust jobs.
"""
import datetime
import hashlib
import json
import os
from pathlib import Path
import signal
import statistics
import subprocess
import sys
import threading

ROOT = Path(__file__).resolve().parents[1]
PREFIX = "NATIVE_FRAME_BENCH "
COMMAND = ["justrust", "test", "--lib", "--locked", "--release", "native_frame_tls_benchmark",
           "--", "--ignored", "--nocapture"]
SIZES = {64: 32768, 32768: 4096}


def expected_order():
    return [(round_, size, coalesced) for size in SIZES for round_ in range(3)
            for coalesced in (False, True, True, False)]


def unique_fields(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"duplicate benchmark field: {key}")
        result[key] = value
    return result


def parse_samples(lines):
    """Reject malformed, extra, missing, or out-of-order benchmark records."""
    samples = []
    expected = expected_order()
    for line in lines:
        if not line.startswith("NATIVE_FRAME_BENCH"):
            continue
        if not line.startswith(PREFIX):
            raise ValueError("malformed NATIVE_FRAME_BENCH prefix")
        try:
            sample = json.loads(line[len(PREFIX):], object_pairs_hook=unique_fields)
        except (ValueError, TypeError) as error:
            raise ValueError("invalid NATIVE_FRAME_BENCH JSON") from error
        if not isinstance(sample, dict) or set(sample) != {"coalesced", "size", "frames", "round", "wall"}:
            raise ValueError("invalid benchmark fields")
        if (type(sample["coalesced"]) is not bool
                or type(sample["size"]) is not int or sample["size"] not in SIZES
                or type(sample["frames"]) is not int or sample["frames"] != SIZES[sample["size"]]
                or type(sample["round"]) is not int or sample["round"] not in range(3)
                or type(sample["wall"]) not in (int, float)
                or not 0 < sample["wall"] <= sys.float_info.max):
            raise ValueError("invalid benchmark values or nonpositive/nonfinite wall")
        position = len(samples)
        actual = (sample["round"], sample["size"], sample["coalesced"])
        if position >= len(expected) or actual != expected[position]:
            raise ValueError(f"benchmark order mismatch at sample {position}: {actual}")
        samples.append(sample)
    if len(samples) != len(expected):
        raise ValueError(f"incomplete benchmark: {len(samples)}/24 samples")
    return samples


def source_hash():
    return hashlib.sha256((ROOT / "src/remote_native.rs").read_bytes()).hexdigest()


def results(samples, env, identity, timestamp):
    pair = []
    for coalesced, label in ((False, "before"), (True, "after")):
        scenarios = {}
        for size in SIZES:
            group = [{**sample, "recorded_wall": sample["wall"], "noisy": None}
                     for sample in samples if sample["size"] == size and sample["coalesced"] is coalesced]
            walls = [sample["wall"] for sample in group]
            scenarios[f"native-frame-tls-{size}"] = {
                "samples": group, "expect_test_fail": False,
                "stats": {"n": len(group), "median": statistics.median(walls),
                          "min": min(walls), "max": max(walls),
                          "quiet_n": 0, "quiet_median": None,
                          "first_error": None, "first_fail": None},
            }
        pair.append({"schema": "native_frame_tls_v1", "timestamp": timestamp,
                     "variant": label, "coalesced": coalesced, "env": env,
                     "source_sha256": identity, "command": COMMAND,
                     "scope": "focused TLS loopback only, excludes build and agent wait",
                     "speed_claim_eligible": False, "default_promotion": False,
                     "foreign_contention": "unmeasured", "order": expected_order(),
                     "scenarios": scenarios})
    return pair


def stream_command(command, timeout=900):
    """Stream every line, killing only this new process group on timeout."""
    lines = []
    expired = threading.Event()
    with subprocess.Popen(command, cwd=ROOT, stdout=subprocess.PIPE,
                          stderr=subprocess.STDOUT, text=True, bufsize=1,
                          start_new_session=True) as process:
        def expire():
            expired.set()
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass

        watchdog = threading.Timer(timeout, expire)
        watchdog.daemon = True
        watchdog.start()
        try:
            for line in process.stdout:
                print(line, end="", flush=True)
                lines.append(line)
            code = process.wait()
        finally:
            watchdog.cancel()
            watchdog.join()
    if expired.is_set():
        raise ValueError(f"benchmark command exceeded {timeout}s, no results published")
    return code, lines


def main():
    import suite
    from runtime_context import require_matching_context

    identity = source_hash()
    env = suite.env_info()
    env.update(execution="hosted-default", foreign_rustc_policy="unmeasured",
               noise_policy="unknown foreign contention; noisy=null")
    print("Running: " + " ".join(COMMAND), flush=True)
    code, lines = stream_command(COMMAND)
    if source_hash() != identity:
        raise ValueError("src/remote_native.rs changed during benchmark, refusing mixed results")
    suite.verify_binary_identity(env)
    if code:
        raise ValueError(f"benchmark command exited {code}, no results published")
    samples = parse_samples(lines)
    timestamp = datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%S.%fZ")
    before, after = results(samples, env, identity, timestamp)
    require_matching_context(before, after)
    paths = []
    for label, result in (("before", before), ("after", after)):
        path = ROOT / "bench/results" / f"native-frame-{timestamp}-{label}.json"
        path.parent.mkdir(parents=True, exist_ok=True)
        with path.open("x") as output:
            json.dump(result, output, indent=2, allow_nan=False)
            output.write("\n")
        print(f"Saved {path}", flush=True)
        suite.upload_result(path)
        paths.append(path)
    return subprocess.run([sys.executable, str(ROOT / "bench/suite.py"), "compare",
                           *map(str, paths)], cwd=ROOT).returncode


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (ValueError, OSError) as error:
        sys.exit(f"native-frame: {error}")
