#!/usr/bin/env python3
"""Run pinned suite scenarios in ABBA order with one experimental Cargo binary.

Requires an isolated JUSTRUST_HOME containing the exact compiler and candidate
bin/jr-cargo. Does not install or activate a production toolchain. Measurements
are controlled host-local builds, not hosted-service transport benchmarks.
"""
import argparse
import json
import math
import os
from pathlib import Path
import subprocess
import sys
import tempfile

import suite

MODES = ("off", "on", "on", "off")


def edge_observations(receipt):
    """After native validation: latest *declared* input availability, not queue wait."""
    nodes = {n['id']: n for n in receipt['nodes']}
    metadata = {e['unit']: e['at_ns'] for e in receipt['events'] if e['event'] == 'artifact-ready'}
    last = {}
    counts = {'rmeta_event': 0, 'rmeta_at_completion': 0, 'all_at_completion': 0}
    for edge in receipt['edges']:
        consumer, producer = edge['from'], edge['to']
        if edge['artifact'] == 'rmeta' and producer in metadata:
            available, source = metadata[producer], 'rmeta_event'
        else:
            available, source = nodes[producer]['end_ns'], edge['artifact'] + '_at_completion'
        if nodes[consumer]['outcome'] not in ('success', 'fresh') or available is None:
            raise ValueError('unobserved scheduler edge lifecycle')
        counts[source] += 1
        last[consumer] = max(last.get(consumer, 0), available)
    if not last or not any(nodes[k]['outcome'] == 'success' for k in last):
        raise ValueError('no executed non-fresh dependent in scheduler evidence')
    return {'edge_sources': counts, 'last_input_to_start_ns': {
        k: nodes[k]['start_ns'] - t for k, t in last.items()}}


def sample_evidence(directory):
    manifests = list(directory.glob('owner-*.manifest.json'))
    if len(manifests) != 1:
        raise ValueError(f'{directory.name}: missing or ambiguous Cargo scheduler owner manifest')
    manifest, = manifests
    checked = subprocess.run([suite.J, 'graph', str(directory), '--manifest', str(manifest), '--json'],
                             check=True, stdout=subprocess.PIPE, text=True)
    coverage = json.loads(checked.stdout)
    if (coverage.get('scheduler_entry_complete') is not True or coverage.get('issues') != []
            or coverage.get('expected_receipts') != 1 or coverage.get('accounted_receipts') != 1):
        raise ValueError(f'{directory.name}: incomplete native scheduler graph')
    owner, = suite.bounded_identity_json(manifest, 16384)['owners']
    ticket, = owner['tickets']
    if ticket['lifecycle'] != 'succeeded' or owner['outcome'] != 'success':
        raise ValueError('scheduler evidence requires a successful BuildRunner entry')
    path = directory / (ticket['capture_id'] + '.json')
    receipt = suite.bounded_identity_json(path, 64 * 1024 * 1024)
    if (receipt['capture_id'] != ticket['capture_id'] or receipt['producer'] != 'cargo-scheduler'
            or receipt['structural_complete'] is not True):
        raise ValueError('incomplete or mismatched scheduler receipt')
    return {'scope': 'one Cargo scheduler entry, not queue wait or predicted savings',
            'sha256': suite.binary_sha256(path), **edge_observations(receipt)}


def identities(toolchain):
    rustc = suite.toolchain_rustc(toolchain)
    cargo = rustc.with_name("jr-cargo")
    if not cargo.is_file() or not os.access(cargo, os.X_OK):
        raise ValueError("scheduler sweep requires an explicit candidate bin/jr-cargo")
    if not cargo.resolve().is_relative_to(suite.JR_HOME.resolve()):
        raise ValueError("candidate Cargo escapes the isolated toolchain prefix")
    if any(not path.is_file() or not os.access(path, os.X_OK) for path in (rustc, cargo.with_name("cargo"))):
        raise ValueError("isolated toolchain needs executable rustc and official Cargo")
    marker = rustc.parent.parent / "justrust-toolchain.json"
    try:
        info = json.loads(marker.read_text())
        required_strings = ("id", "manifest_date", "manifest_sha256", "rustc_version")
        if (not isinstance(info, dict)
                or any(not isinstance(info.get(key), str) for key in required_strings)
                or not isinstance(info.get("components"), list)
                or any(not isinstance(value, str) for value in info["components"])
                or any(type(info.get(key)) not in (int, float) or not math.isfinite(info[key])
                       for key in ("installed_at", "install_secs"))):
            raise ValueError("isolated toolchain must already have a valid installed marker")
        if info["id"] != toolchain:
            raise ValueError("isolated toolchain marker does not match the requested compiler")
    except (OSError, json.JSONDecodeError) as error:
        raise ValueError("isolated toolchain must already have a valid installed marker") from error
    return {"justrust_sha256": suite.binary_sha256(),
            "toolchain_marker_sha256": suite.binary_sha256(marker),
            "rustc_sha256": suite.binary_sha256(rustc),
            "cargo_sha256": suite.binary_sha256(cargo),
            "rustc_version": suite.sh([str(rustc), "-vV"]),
            "cargo_version": suite.sh([str(cargo), "-vV"])}


def verify_recorded_selection(result, toolchain, identity, jobs):
    selected = suite.toolchain_rustc(toolchain)
    rustc = selected.resolve()
    cargo = selected.with_name("jr-cargo").resolve()
    if (suite.binary_sha256(cargo) != identity["cargo_sha256"]
            or suite.binary_sha256(rustc) != identity["rustc_sha256"]):
        raise ValueError("recorded selection binary identity changed")
    scenarios = result.get("scenarios", {})
    if not scenarios:
        raise ValueError("recorded selection has no scenarios")
    for scenario in scenarios.values():
        if not scenario.get("samples"):
            raise ValueError("recorded selection has no samples")
        for sample in scenario["samples"]:
            rid = sample.get("id")
            if not isinstance(rid, str) or not rid or Path(rid).name != rid or rid in (".", ".."):
                raise ValueError("recorded selection has an invalid run ID")
            try:
                meta = json.loads((suite.JR_HOME / "runs" / rid / "meta.json").read_text())
                if (not isinstance(meta, dict) or meta.get("id") != rid
                        or meta.get("toolchain") != toolchain or meta.get("place") != "local"
                        or not isinstance(meta.get("real_cargo"), str)
                        or not isinstance(meta.get("real_rustc"), str)
                        or not Path(meta["real_cargo"]).is_absolute()
                        or not Path(meta["real_rustc"]).is_absolute()
                        or Path(meta["real_cargo"]).resolve() != cargo
                        or Path(meta["real_rustc"]).resolve() != rustc
                        or not isinstance(meta.get("env"), dict)
                        or meta["env"].get("CARGO_BUILD_JOBS") != str(jobs)):
                    raise ValueError("recorded selection does not match the scheduler experiment")
            except (OSError, json.JSONDecodeError) as error:
                raise ValueError("recorded selection lacks a valid invocation record") from error


def aggregate(segments, mode, identity, jobs):
    chosen = [segment for segment in segments if segment["mode"] == mode]
    result = json.loads(json.dumps(chosen[0]["result"]))
    result["started"] = chosen[0]["result"]["started"]
    result["finished"] = chosen[-1]["result"]["finished"]
    result["scheduler_experiment"] = {"mode": mode, "jobs": jobs,
                                      "identities": identity,
                                      "ordering": [segment["mode"] for segment in segments],
                                      "interpretation": "controlled host-local builds, not service latency"}
    for name, scenario in result["scenarios"].items():
        samples = []
        for segment in chosen:
            current = segment["result"]
            if current["env"] != result["env"]:
                raise ValueError("mixed environment in scheduler measurements")
            if current["scenarios"][name]["cmd"] != scenario["cmd"]:
                raise ValueError("mixed invocation in scheduler measurements")
            samples.extend(current["scenarios"][name]["samples"])
        scenario["samples"] = samples
        scenario["stats"] = suite.stats(samples)
    result["runs_per_scenario"] = len(next(iter(result["scenarios"].values()))["samples"])
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--toolchain", required=True)
    parser.add_argument("--out-dir", type=Path, required=True)
    parser.add_argument("--jobs", type=int, choices=(1, 2, 4, 8, 16, 24), default=24)
    parser.add_argument("--quiet-wait", type=float, default=30)
    parser.add_argument("--samples", type=int, default=2)
    parser.add_argument('--require-scheduling-evidence', action='store_true',
                        help='require a validated native Cargo graph for every measured ABBA sample')
    parser.add_argument("--costs", type=Path,
                        help="explicit bounded package hints, copied and digest-guarded")
    parser.add_argument("scenarios", nargs="+", choices=tuple(suite.BY_NAME))
    args = parser.parse_args()
    if args.samples < 1:
        parser.error("samples must be positive")
    if not os.environ.get("JUSTRUST_HOME"):
        parser.error("use an isolated JUSTRUST_HOME, never the production prefix")
    if Path(os.environ["JUSTRUST_HOME"]).resolve() == (Path.home() / ".justrust").resolve():
        parser.error("JUSTRUST_HOME must not be the production prefix")
    if os.environ.get("JUSTRUST_CARGO_FORK") in ("0", "off", "false", "no"):
        parser.error("candidate Cargo fork selection is disabled")
    if os.environ.get("JUSTRUST_CARGO_CRITICAL_PATH_COSTS"):
        parser.error("use explicit --costs, not unrecorded environment hint tables")
    hints = suite.load_scheduler_hints(args.costs) if args.costs is not None else None
    args.out_dir = args.out_dir.resolve()
    args.out_dir.mkdir(parents=True, exist_ok=False, mode=0o700)
    identity = identities(args.toolchain)
    hint_snapshot = args.out_dir / "costs.json"
    if hints is not None:
        with hint_snapshot.open("xb") as handle:
            handle.write(hints[0])
        hint_snapshot.chmod(0o400)
        identity["cost_hints"] = hints[1]
    os.environ.update({"JUSTRUST_CARGO_DAEMON": "0", "JUSTRUST_CARGO_FORK": "1",
                       "CARGO_BUILD_JOBS": str(args.jobs), "JUSTRUST_NO_UPDATE": "1",
                       "JUSTRUST_TOOLCHAIN_AUTO_INSTALL": "0", "JUSTRUST_SHADOW": "0"})
    segments = []
    seen = set()
    for index, mode in enumerate(MODES):
        if hints is not None and suite.load_scheduler_hints(hint_snapshot) != hints:
            raise ValueError("cost hint snapshot changed before the segment")
        if hints is not None and mode == "on":
            os.environ["JUSTRUST_CARGO_CRITICAL_PATH_COSTS"] = str(hint_snapshot)
        else:
            os.environ.pop("JUSTRUST_CARGO_CRITICAL_PATH_COSTS", None)
        os.environ["JUSTRUST_CARGO_CRITICAL_PATH"] = "1" if mode == "on" else "0"
        out = args.out_dir / f"{index}-{mode}.json"
        command = [sys.executable, str(Path(suite.__file__)), "run", "--toolchain", args.toolchain,
                   "--warmup", "1", "-n", str(args.samples), "--quiet-wait", str(args.quiet_wait),
                   "--out", str(out), *args.scenarios]
        if args.require_scheduling_evidence:
            graph_root = args.out_dir / f'{index}-{mode}-graphs'
            graph_root.mkdir(mode=0o700)
            # Unvalidated suite files are neither published nor retained as reusable results.
            with tempfile.TemporaryDirectory(dir=args.out_dir) as scratch:
                private = Path(scratch) / 'segment.json'
                command[command.index('--out') + 1] = str(private)
                subprocess.run([*command, '--scheduler-graph-root', str(graph_root)], check=True)
                result = json.loads(private.read_text())['unchecked_scheduler_segment']
        else:
            subprocess.run(command, check=True)
            result = json.loads(out.read_text())
        current = identities(args.toolchain)
        if hints is not None:
            current["cost_hints"] = hints[1]
        if current != identity:
            raise ValueError("toolchain or justrust changed during scheduler sweep")
        if hints is not None and suite.load_scheduler_hints(hint_snapshot) != hints:
            raise ValueError("cost hint snapshot changed during the segment")
        expected_mode = "1" if mode == "on" else "0"
        if result.get("env", {}).get("optimization_env", {}).get("JUSTRUST_CARGO_CRITICAL_PATH") != expected_mode:
            raise ValueError("recorded segment mode does not match the scheduler experiment")
        expected_costs = hints[1]["sha256"] if hints is not None and mode == "on" else None
        if result.get("env", {}).get("optimization_env", {}).get("JUSTRUST_CARGO_CRITICAL_PATH_COSTS_SHA256") != expected_costs:
            raise ValueError("recorded cost hints do not match the scheduler experiment")
        verify_recorded_selection(result, args.toolchain, identity, args.jobs)
        if args.require_scheduling_evidence:
            if set(result['scenarios']) != set(args.scenarios):
                raise ValueError('missing requested scheduler evidence scenario')
            for scenario in result['scenarios'].values():
                if len(scenario['samples']) != args.samples:
                    raise ValueError('incomplete scheduler sample inventory')
                for sample in scenario['samples']:
                    rid = sample['id']  # checked in verify_recorded_selection
                    if sample.get('scheduler_launch_mode') != expected_mode:
                        raise ValueError(f'{rid}: missing or mismatched scheduler launch mode')
                    if rid in seen:
                        raise ValueError('reused scheduler sample ID')
                    seen.add(rid)
                    sample['scheduling_evidence'] = sample_evidence(graph_root / rid)
        segments.append({"mode": mode, "result": result})
        if not args.require_scheduling_evidence:
            (args.out_dir / "partial.json").write_text(json.dumps({"segments": segments,
                                                                    "identities": identity}, indent=2) + "\n")
    pair = {mode: aggregate(segments, mode, identity, args.jobs) for mode in ('off', 'on')}
    for mode, result in pair.items():
        (args.out_dir / f"{mode}.json").write_text(json.dumps(result, indent=2) + "\n")
    subprocess.run([sys.executable, str(Path(suite.__file__)), "compare",
                    str(args.out_dir / "off.json"), str(args.out_dir / "on.json")], check=True)


if __name__ == "__main__":
    main()
