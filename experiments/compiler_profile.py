#!/usr/bin/env python3
r"""Opt-in Linux CPU profiling of an existing, actual jr-rust CLI. No builds.

Run this script *inside* the authorized hosted job, not around the local
justrust network client. This script does not submit jobs, install packages,
change privileges/kernel settings, execute generated programs, or clear caches.
Examples (each output directory must be new):

  python3 experiments/compiler_profile.py --output-dir /scratch/jr-stat \
    --mode stat -- /scratch/bin/jr-rust input.rs --workers 1 --check
  python3 experiments/compiler_profile.py --output-dir /scratch/jr-stacks \
    --mode record --execution-context hosted --input input.rs \
    -- /scratch/bin/jr-rust input.rs --workers 1 -o /scratch/output

Default mode is resources only. stat/record explicitly opt in to system perf.
A failed profiler preflight falls back to one resource-only compiler invocation,
unless --require-profiler is given. A started compiler is NEVER retried.

Requirements: Linux, Python 3.9+, an already-built compiler, writable private
artifact storage. CPU modes additionally need an existing compatible perf and
permission for per-process user-space events. record defaults to DWARF stacks:
preserve the exact compiler and its debug information, and use perf with an
unwinder. --call-graph fp instead requires a frame-pointer-enabled compiler.
No symbols, samples, or successful unwinding are promised by a successful run.
On a host where scoped administration is already authorized, --perf-sudo opts
in to `sudo -n perf ... -- runuser ...` only. sudo/runuser must already exist.
The compiler worker verifies the invoking UID/GID/supplementary groups before
launch, never runs the compiler as root, and does not change sudoers or sysctls.
sudo/runuser sanitize the workload environment, so this mode is not equivalent
to an ordinary invocation. Empty perf.data files are precreated by the caller
to retain caller ownership and readability. No recursive chmod/chown is used.

Compiler wall/user/system/maxRSS are measured independently with wait4, not
RUSAGE_CHILDREN high-water differences and not the enclosing perf process.
Linux maxRSS is KiB, a peak for the child and its waited-for descendants, NOT a
sum of simultaneously resident process memory. CPU counters/stacks also include
the small Python measurement launcher and descendants (including any linker).
The readable stack report is filtered to the native compiler PID and its
threads; raw perf.data retains the wider process tree. stat counters are not
PID-filtered after collection and must not be called compiler-only counters.
These diagnostic timings include profiler overhead and are not speedup evidence.

This is NOT an allocation profiler. RSS, page faults, CPU samples in allocator
functions, and compiler budget charges are NOT allocation counts or bytes.
Allocation counts, lifetimes, retained bytes, and allocation stacks require
separate allocator instrumentation. No CFG --trace is accepted or added.

Logs have bounded retained prefixes. perf sample size and a monitored data-file
cap bound capture size, with a final hard retained-file cap. Hitting either cap
can lose samples, terminate profiling, or yield an unusable partial perf.data;
this is recorded, never called a complete profile. Compiler-created outputs
are outside these caps. Private artifacts can contain source paths, diagnostics,
and sampled stack memory. No environment dump or network symbol lookup is used.
"""

import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import platform
import grp
import pwd
import selectors
import shlex
import shutil
import signal
import stat
import subprocess
import sys
import time

MIB = 1024 * 1024


def write_json(path, value):
    temporary = path.with_suffix(path.suffix + ".pending")
    with temporary.open("x") as stream:
        json.dump(value, stream, indent=2, sort_keys=True)
        stream.write("\n")
    temporary.replace(path)


def identity(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(MIB), b""):
            digest.update(block)
        stat = os.fstat(stream.fileno())
    return {"path": str(path), "sha256": digest.hexdigest(), "bytes": stat.st_size,
            "mtime_ns": stat.st_mtime_ns}


def read_optional(path, limit=16384):
    try:
        with open(path) as stream:
            return stream.read(limit).strip()
    except OSError as error:
        return {"unavailable": str(error)}


def shell_exit(returncode):
    return 128 - returncode if returncode < 0 else returncode


def kill_group(pid, sig):
    try:
        os.killpg(pid, sig)
    except ProcessLookupError:
        pass
    except PermissionError:
        # sudo's caller-owned monitor can relay signals even when group peers
        # are root. Never add a second privileged kill command or change policy.
        try:
            os.kill(pid, sig)
        except (ProcessLookupError, PermissionError):
            pass


def capture(command, directory, name, timeout, cap, env=None, watched=None):
    """Bounded pipes + per-child wait4. Never change the child's rlimits.

    A pidfd gives prompt completion even for tiny/quiet compiler invocations.
    The fallback selector polling can add up to 10ms to wall measurements.
    """
    started = time.perf_counter_ns()
    deadline = time.monotonic() + timeout
    result = {"command": command, "returncode": None, "timed_out": False,
              "interrupted": None, "data_cap_hit": False, "streams": {}}
    try:
        process = subprocess.Popen(command, stdout=subprocess.PIPE,
                                   stderr=subprocess.PIPE, stdin=subprocess.DEVNULL,
                                   start_new_session=True, env=env)
    except OSError as error:
        result.update(launch_error=str(error), wall_ns=time.perf_counter_ns() - started)
        return result
    result["pid"] = process.pid
    selector = selectors.DefaultSelector()
    handles = []
    pidfd = None
    status = usage = None
    completed_at = None
    stop_at = None
    try:
        for kind, pipe in (("stdout", process.stdout), ("stderr", process.stderr)):
            path = directory / (name + "." + kind)
            output = path.open("xb")
            handles.append(output)
            result["streams"][kind] = {"path": path.name, "bytes_seen": 0,
                                        "bytes_retained": 0, "truncated": False}
            os.set_blocking(pipe.fileno(), False)
            selector.register(pipe, selectors.EVENT_READ, (kind, output))
        try:
            pidfd = os.pidfd_open(process.pid)
            selector.register(pidfd, selectors.EVENT_READ, None)
            result["completion_notification"] = "pidfd"
        except (AttributeError, OSError):
            result["completion_notification"] = "10ms poll"
        while completed_at is None or any(key.data is not None for key in selector.get_map().values()):
            now = time.monotonic()
            if completed_at is None:
                if stop_at is None and now >= deadline:
                    result["timed_out"] = True
                    stop_at = now
                    kill_group(process.pid, signal.SIGTERM)
                if watched and stop_at is None:
                    path, maximum = watched
                    if path.exists() and path.stat().st_size > maximum:
                        result["data_cap_hit"] = True
                        stop_at = now
                        kill_group(process.pid, signal.SIGTERM)
                if stop_at is not None and now - stop_at >= 2:
                    kill_group(process.pid, signal.SIGKILL)
                exited = os.waitid(os.P_PID, process.pid, os.WEXITED | os.WNOHANG | os.WNOWAIT)
                if exited is not None:
                    completed_at = now
                    result["wall_ns"] = time.perf_counter_ns() - started
                    if pidfd is not None:
                        selector.unregister(pidfd)
                    # Keep the exited leader unreaped until cleanup, preventing
                    # PID reuse from ever redirecting a group signal elsewhere.
                    kill_group(process.pid, signal.SIGTERM)
            if completed_at is not None and now - completed_at > 3:
                result["pipe_drain_incomplete"] = True
                break
            for key, _ in selector.select(0.01):
                if key.data is None:
                    continue
                kind, output = key.data
                chunk = os.read(key.fileobj.fileno(), 65536)
                if not chunk:
                    selector.unregister(key.fileobj)
                    key.fileobj.close()
                    continue
                stream = result["streams"][kind]
                stream["bytes_seen"] += len(chunk)
                retained = chunk[:max(0, cap - stream["bytes_retained"])]
                output.write(retained)
                stream["bytes_retained"] += len(retained)
                stream["truncated"] = stream["bytes_seen"] > stream["bytes_retained"]
    except (KeyboardInterrupt, InterruptedError) as error:
        result["interrupted"] = type(error).__name__
    finally:
        if completed_at is None:
            kill_group(process.pid, signal.SIGTERM)
            # Wait briefly for the measurement worker to save its receipt before
            # escalating. Its compiler is in a separate, worker-owned session.
            deadline = time.monotonic() + 3
            while time.monotonic() < deadline:
                exited = os.waitid(os.P_PID, process.pid, os.WEXITED | os.WNOHANG | os.WNOWAIT)
                if exited is not None:
                    break
                time.sleep(0.01)
            result["wall_ns"] = time.perf_counter_ns() - started
        # The unreaped leader anchors ownership for this final group cleanup.
        kill_group(process.pid, signal.SIGKILL)
        _, status, usage = os.wait4(process.pid, 0)
        process.returncode = os.waitstatus_to_exitcode(status)
        selector.close()
        if pidfd is not None:
            os.close(pidfd)
        for pipe in (process.stdout, process.stderr):
            pipe.close()
        for output in handles:
            output.close()
    result.update(returncode=process.returncode,
                  signal=-process.returncode if process.returncode < 0 else None,
                  user_seconds=usage.ru_utime, system_seconds=usage.ru_stime,
                  max_rss_kib=usage.ru_maxrss, minor_faults=usage.ru_minflt,
                  major_faults=usage.ru_majflt,
                  voluntary_context_switches=usage.ru_nvcsw,
                  involuntary_context_switches=usage.ru_nivcsw)
    return result


def measure(config_path):
    config = json.loads(config_path.read_text())
    directory = config_path.parent
    actual_identity = {"uid": os.getuid(), "euid": os.geteuid(), "gid": os.getgid(),
                       "egid": os.getegid(), "groups": sorted(os.getgroups())}
    if actual_identity != config["native_identity"]:
        write_json(directory / "compiler.json", {
            "started": False, "returncode": None, "actual_identity": actual_identity,
            "launch_error": "Refusing compiler launch: worker identity differs from invoking user"})
        return 125
    # A durable marker before launch prevents any automatic double execution,
    # even if the profiler/worker fails between Popen and saving the result.
    write_json(directory / "compiler-started.json", {"command": config["command"]})

    def interrupt(_number, _frame):
        raise InterruptedError("measurement worker interrupted")

    signal.signal(signal.SIGTERM, interrupt)
    signal.signal(signal.SIGINT, interrupt)
    result = capture(config["command"], directory, "compiler", config["timeout"],
                     config["log_cap"])
    result["started"] = "pid" in result
    result["actual_identity"] = actual_identity
    result["resource_scope"] = "native child lifetime (including fork/exec setup) and descendants it waits for, excluding perf and launcher"
    write_json(directory / "compiler.json", result)
    return shell_exit(result["returncode"]) if result["returncode"] is not None else 127


def perf_command(perf, mode, directory, options, workload):
    if mode == "stat":
        # No interval/repetition: one bounded counter report, separate stderr
        # artifact. Raw values preserve <not supported>/<not counted> markers.
        flags = ["stat", "--no-big-num", "-x", ";", "-e", options.stat_events]
    else:
        flags = ["record", "--no-buildid-cache", "-e", "cpu-clock:u",
                 "-F", str(options.frequency), "--call-graph", options.call_graph,
                 "--mmap-pages", "128", "--max-size", str(options.data_cap_mib) + "M",
                 "-o", str(directory / "perf.data")]
    return options.perf_prefix + [perf] + flags + ["--"] + options.workload_prefix + workload


def configure_elevation(options):
    """Build opt-in argv only. No sudo probe or command is run here."""
    options.perf_prefix = []
    options.workload_prefix = []
    if not options.perf_sudo:
        return None
    if os.getuid() == 0 or os.geteuid() != os.getuid() or os.getegid() != os.getgid():
        return "--perf-sudo requires an ordinary non-root invoking user"
    system_path = "/usr/sbin:/usr/bin:/sbin:/bin"
    sudo = shutil.which("sudo", path=system_path)
    runuser = shutil.which("runuser", path=system_path)
    if not sudo or not runuser:
        return "--perf-sudo requires existing sudo and runuser; nothing was installed"
    perf = shutil.which(options.perf)
    if not perf or Path(perf).resolve().name != "perf":
        return "--perf-sudo only authorizes an existing system executable named perf"
    for tool in (sudo, runuser, perf):
        if not trusted_system_tool(tool):
            return "--perf-sudo refuses a tool or parent directory writable by non-root: " + tool
    try:
        user = pwd.getpwuid(os.getuid()).pw_name
        group = grp.getgrgid(os.getgid()).gr_name
        groups = [grp.getgrgid(gid).gr_name for gid in sorted(os.getgroups())]
    except KeyError as error:
        return "Cannot map invoking identity to runuser account/group names: " + str(error)
    if not groups:
        return "Cannot preserve an empty supplementary group list with runuser; elevation unavailable"
    options.perf_prefix = [str(Path(sudo).resolve()), "-n", "--"]
    options.workload_prefix = [str(Path(runuser).resolve()), "-u", user, "-g", group]
    for group in groups:
        options.workload_prefix += ["-G", group]
    options.workload_prefix += ["--"]
    return None


def trusted_system_tool(tool):
    """Never turn a user-writable perf override into a privileged program."""
    path = Path(tool).resolve()
    try:
        if not stat.S_ISREG(path.stat().st_mode):
            return False
        return all(entry.stat().st_uid == 0 and not entry.stat().st_mode & 0o022
                   for entry in (path, *path.parents))
    except OSError:
        return False


def counters(path):
    """Keep perf's raw CSV fields, including explicit unavailable markers."""
    rows = []
    if path.exists():
        for line in path.read_text(errors="replace").splitlines():
            fields = [field.strip() for field in line.split(";")]
            if len(fields) < 3 or not fields[2]:
                continue
            try:
                value = float(fields[0])
            except ValueError:
                value = None
            rows.append({"event": fields[2], "value": value, "raw_value": fields[0],
                         "unit": fields[1], "remaining_fields": fields[3:]})
    return rows


def positive_float(value):
    parsed = float(value)
    if not 0 < parsed <= 86400:
        raise argparse.ArgumentTypeError("must be greater than 0 and at most 86400")
    return parsed


def arguments():
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--output-dir", type=Path, required=True, help="new private artifact directory")
    parser.add_argument("--mode", choices=("resources", "stat", "record"), default="resources")
    parser.add_argument("--perf", default="perf", help="existing system perf executable, never installed")
    parser.add_argument("--perf-sudo", action="store_true", help="explicit authorized sudo -n perf only; runuser keeps compiler unprivileged")
    parser.add_argument("--require-profiler", action="store_true", help="fail without running compiler if preflight fails")
    parser.add_argument("--timeout", type=positive_float, default=120, help="native timeout seconds, default 120")
    parser.add_argument("--tool-timeout", type=positive_float, default=20, help="preflight/report timeout seconds")
    parser.add_argument("--log-cap-mib", type=int, choices=range(1, 65), default=1, metavar="1..64")
    parser.add_argument("--data-cap-mib", type=int, choices=range(1, 1025), default=64, metavar="1..1024")
    parser.add_argument("--frequency", type=int, choices=range(1, 10000), default=99, metavar="1..9999")
    parser.add_argument("--call-graph", choices=("dwarf,8192", "dwarf,16384", "fp"), default="dwarf,8192")
    parser.add_argument("--stat-events", default="task-clock:u,cycles:u,instructions:u,branches:u,branch-misses:u,page-faults:u,context-switches:u")
    parser.add_argument("--input", type=Path, action="append", default=[], help="explicit input to hash, repeat for module files")
    parser.add_argument("--revision-label", help="caller-supplied label only, not verified executable provenance")
    parser.add_argument("--build-profile-label", help="caller-supplied release/debug label, not verified")
    parser.add_argument("--execution-context", choices=("unknown", "local", "hosted"), default="unknown", help="caller assertion, not a hosted billing/provenance receipt")
    parser.add_argument("command", nargs=argparse.REMAINDER, help="-- /absolute/path/jr-rust source.rs [compiler options]")
    args = parser.parse_args()
    if args.command[:1] == ["--"]:
        args.command.pop(0)
    if sys.platform != "linux":
        parser.error("Linux is required for wait4 maxRSS units and perf")
    if not args.command:
        parser.error("provide the existing native compiler command after --")
    if any(token == "--trace" or token.startswith("--trace=") for token in args.command[1:]):
        parser.error("CFG --trace is intentionally forbidden: profile the ordinary native CLI")
    if args.require_profiler and args.mode == "resources":
        parser.error("--require-profiler requires --mode stat or record")
    if args.perf_sudo and args.mode == "resources":
        parser.error("--perf-sudo requires --mode stat or record")
    if len(args.input) > 64 or len(json.dumps(args.command)) > 65536:
        parser.error("at most 64 explicit input files and 64KiB of command arguments")
    executable = shutil.which(args.command[0])
    if not executable:
        parser.error("native compiler executable not found or not executable")
    args.command[0] = str(Path(executable).resolve(strict=True))
    # Reject synthetic/remote front doors, while allowing renamed preserved
    # jr-rust binaries. The receipt retains exact identity, not a name assertion.
    if Path(args.command[0]).name in {"justrust", "jr-prototype", "cargo", "rustc"}:
        parser.error("invoke the actual jr-rust CLI, not a build client or synthetic graph runner")
    args.output_dir = args.output_dir.absolute()
    if not args.output_dir.parent.is_dir():
        parser.error("output directory parent must already exist")
    try:
        args.native_identity = identity(Path(args.command[0]))
        args.input_identities = [identity(path.resolve(strict=True)) for path in args.input]
        args.output_dir.mkdir(mode=0o700)
    except OSError as error:
        parser.error(str(error))
    return args


def main():
    args = arguments()
    directory = args.output_dir
    cap = args.log_cap_mib * MIB
    # These overrides suppress interactive perf configuration, global caches,
    # and network symbol downloads. They are inherited by the compiler when
    # perf wraps it, and recorded explicitly rather than dumping user secrets.
    environment_overrides = {"LC_ALL": "C", "PERF_PAGER": "cat", "PERF_CONFIG": "/dev/null",
                             "PERF_BUILDID_DIR": str(directory / "buildid-cache"), "DEBUGINFOD_URLS": ""}
    environment = dict(os.environ, **environment_overrides)
    elevation_error = configure_elevation(args)
    native_identity = {"uid": os.getuid(), "euid": os.geteuid(), "gid": os.getgid(),
                       "egid": os.getegid(), "groups": sorted(os.getgroups())}
    cpuinfo = read_optional("/proc/cpuinfo")
    cpu_model = next((line.split(":", 1)[1].strip() for line in cpuinfo.splitlines()
                      if line.startswith("model name")), None) if isinstance(cpuinfo, str) else cpuinfo
    report = {
        "schema": "jr-rust-cpu-profile-v1", "started_utc": datetime.now(timezone.utc).isoformat(),
        "native_command": args.command, "native_shell_display": shlex.join(args.command),
        "cwd": str(Path.cwd()), "compiler_identity_before": args.native_identity,
        "profiled_environment_overrides": environment_overrides,
        "runner_sha256": identity(Path(__file__).resolve())["sha256"],
        "explicit_inputs": args.input_identities,
        "labels_unverified": {"revision": args.revision_label, "build_profile": args.build_profile_label,
                              "execution_context": args.execution_context},
        "host": {"uname": list(platform.uname()), "cpu_model": cpu_model,
                 "logical_cpus": os.cpu_count(), "affinity": sorted(os.sched_getaffinity(0)),
                 "perf_event_paranoid": read_optional("/proc/sys/kernel/perf_event_paranoid"),
                 "perf_event_max_sample_rate": read_optional("/proc/sys/kernel/perf_event_max_sample_rate"),
                 "uid": os.getuid()},
        "profiler": {"requested": args.mode, "status": "disabled", "errors": []},
        "privilege": {"sudo_requested": args.perf_sudo, "native_identity": native_identity,
                      "perf_prefix": args.perf_prefix, "workload_prefix": args.workload_prefix,
                      "environment_sanitized_by_sudo_runuser": args.perf_sudo},
        "allocation_diagnosis": {"available": False, "allocation_count": None, "allocated_bytes": None,
                                 "scope": "runner-added instrumentation only; any separately enabled native allocation report remains in compiler.stdout",
                                 "reason": "This runner adds no allocator hooks. RSS, page faults, CPU stacks and budget counts are not allocation counts."},
        "limits": {"log_bytes_per_stream": cap, "perf_sample_bytes": args.data_cap_mib * MIB,
                   "perf_retained_bytes": args.data_cap_mib * MIB, "timeout_seconds": args.timeout},
        "limitations": ["Diagnostics only, profiled and contended, not a benchmark speedup claim.",
                        "perf counters/stacks include measurement launcher and inherited subprocesses; native wait4 does not.",
                        "maxRSS is a per-process peak including waited descendants, not simultaneous process-tree RSS.",
                        "Only explicitly listed inputs are hashed, not an automatically complete dependency closure.",
                        "Retain the matching binary/debug files for symbolization; reports may contain unknown frames.",
                        "perf.data can contain sensitive user stack bytes. Artifacts are private, not redacted.",
                        "Compiler-created outputs are not capped. No cache eviction or isolation is provided."],
    }
    write_json(directory / "metadata.json", report)
    config = {"command": args.command, "timeout": args.timeout, "log_cap": cap,
              "native_identity": native_identity}
    write_json(directory / "launch.json", config)
    worker = [sys.executable, str(Path(__file__).resolve()), "_measure", str(directory / "launch.json")]
    profiler = report["profiler"]
    perf = shutil.which(args.perf) if args.mode != "resources" else None
    ready = False
    if args.mode != "resources":
        profiler["status"] = "unavailable"
        if elevation_error:
            profiler["errors"].append(elevation_error)
        elif not perf:
            profiler["errors"].append("perf executable is not available; nothing was installed")
        else:
            perf = str(Path(perf).resolve())
            profiler["executable"] = perf
            profiler["version"] = capture([perf, "--version"], directory, "perf-version", args.tool_timeout, cap, environment)
            probe = directory / "probe"
            probe.mkdir(mode=0o700)
            if args.mode == "record":
                (probe / "perf.data").touch(mode=0o600, exist_ok=False)
            # Probe the exact requested event/callgraph options, not just whether
            # perf exists. The real compiler is not invoked during this check.
            command = perf_command(perf, args.mode, probe, args, [sys.executable, "-c", "sum(i*i for i in range(10000))"])
            profiler["preflight"] = capture(command, probe, "perf", args.tool_timeout, cap, environment,
                                             (probe / "perf.data", args.data_cap_mib * MIB))
            ready = profiler["preflight"]["returncode"] == 0 and not profiler["preflight"]["data_cap_hit"]
            if args.mode == "stat":
                profiler["preflight_counters"] = counters(probe / "perf.stderr") + counters(probe / "perf.stdout")
                ready = ready and any(row["value"] is not None for row in profiler["preflight_counters"])
            if not ready:
                profiler["errors"].append("Exact perf preflight failed; see probe/perf.stderr for permissions, unsupported events or options")
    if args.require_profiler and not ready:
        report["compiler"] = {"started": False, "reason": "required profiler unavailable"}
        finish(directory, report, args)
        return 2
    if ready:
        profiler["status"] = "attempted"
        if args.mode == "record":
            (directory / "perf.data").touch(mode=0o600, exist_ok=False)
        command = perf_command(perf, args.mode, directory, args, worker)
        profiler["capture"] = capture(command, directory, "perf", args.timeout + args.tool_timeout + 5,
                                       cap, environment, (directory / "perf.data", args.data_cap_mib * MIB))
        if not (directory / "compiler-started.json").exists():
            profiler["status"] = "unavailable"
            profiler["errors"].append("perf failed before the compiler launch marker")
            if not args.require_profiler:
                report["resource_fallback"] = capture(worker, directory, "launcher", args.timeout + 5, cap)
        else:
            profiler["status"] = "captured_unverified"
    else:
        report["resource_fallback"] = capture(worker, directory, "launcher", args.timeout + 5, cap)
    receipt = directory / "compiler.json"
    if receipt.exists():
        report["compiler"] = json.loads(receipt.read_text())
    else:
        report["compiler"] = {"launch_attempted": (directory / "compiler-started.json").exists(),
                              "started": None, "returncode": None,
                              "error": "No compiler receipt. Never retry a potentially started compiler."}
        profiler["status"] = "failed"
    if ready and profiler["status"] == "captured_unverified":
        capture_result = profiler["capture"]
        compiler = report["compiler"]
        # Some perf versions propagate workload failures. Preserve both exit
        # statuses rather than interpreting a failed compiler as a profiler error.
        expected = compiler.get("returncode")
        statuses = {0, shell_exit(expected)} if expected is not None else set()
        if capture_result["returncode"] not in statuses or capture_result["timed_out"] or capture_result["data_cap_hit"]:
            profiler["status"] = "partial_or_failed"
            profiler["errors"].append("Profiler failed, timed out or hit its data cap. Compiler is not retried.")
        else:
            profiler["status"] = "captured"
        if args.mode == "stat":
            profiler["counters"] = counters(directory / "perf.stderr") + counters(directory / "perf.stdout")
            if not any(row["value"] is not None for row in profiler["counters"]):
                profiler["status"] = "partial_or_failed"
                profiler["errors"].append("No numeric counters available; raw unavailable markers are retained")
        if args.mode == "record" and (directory / "perf.data").exists():
            data_size = (directory / "perf.data").stat().st_size
            profiler["data_size_before_final_cap"] = data_size
            command = [perf, "report", "--stdio", "--stdio-color", "never", "--header", "--demangle",
                       "--max-stack", "64", "--call-graph", "graph,0.5,caller", "-i", str(directory / "perf.data")]
            if compiler.get("pid") is not None:
                command += ["--pid", str(compiler["pid"])]
            profiler["symbol_report_scope"] = "native compiler PID and its threads, excluding launcher and separate linker processes"
            profiler["symbol_report_command"] = command
            profiler["symbolization"] = "attempted, inspect perf-report.stdout for samples and unknown frames; success is not proof of complete stacks"
            if compiler.get("pid") is None:
                profiler["symbolization"] = "not attempted: no native compiler PID captured"
            elif data_size > args.data_cap_mib * MIB:
                profiler["symbolization"] = "not attempted: oversized perf.data will be truncated at finalization"
            else:
                profiler["symbol_report"] = capture(command, directory, "perf-report", args.tool_timeout, cap, environment)
            if profiler.get("symbol_report", {}).get("returncode") != 0:
                profiler["errors"].append("perf report failed; raw perf.data retained subject to cap")
        elif args.mode == "record":
            profiler["status"] = "partial_or_failed"
            profiler["errors"].append("No perf.data was produced")
    finish(directory, report, args)
    compiler = report["compiler"]
    if compiler.get("timed_out"):
        return 124
    if compiler.get("returncode") is None:
        return 125
    return shell_exit(compiler["returncode"])


def finish(directory, report, args):
    try:
        after = identity(Path(args.command[0]))
        report["compiler_identity_after"] = after
        report["compiler_identity_unchanged"] = after == args.native_identity
    except OSError as error:
        report["compiler_identity_unchanged"] = False
        report["compiler_identity_after"] = {"error": str(error)}
    artifacts = []
    # Do not recursively harvest compiler-created output or follow its symlinks.
    # This fixed inventory is also the maximum number of retained runner files.
    names = ["compiler-started.json", "compiler.json", "compiler.stdout", "compiler.stderr",
             "launcher.stdout", "launcher.stderr", "perf-version.stdout", "perf-version.stderr",
             "perf.stdout", "perf.stderr", "perf.data", "perf-report.stdout", "perf-report.stderr",
             "probe/perf.stdout", "probe/perf.stderr", "probe/perf.data"]
    for name in names:
        path = directory / name
        if path.is_symlink():
            report["profiler"]["errors"].append(name + " is a symlink, not harvested or truncated")
            continue
        if not path.is_file():
            continue
        maximum = args.data_cap_mib * MIB if path.name == "perf.data" else args.log_cap_mib * MIB
        size = path.stat().st_size
        clipped = size > maximum
        try:
            if clipped:
                with path.open("r+b") as stream:
                    stream.truncate(maximum)
                report["profiler"]["errors"].append(name + " exceeded retained cap and was truncated; may be unusable")
                report["profiler"]["status"] = "partial_or_failed"
            artifact = identity(path)
        except OSError as error:
            report["profiler"]["status"] = "partial_or_failed"
            report["profiler"]["errors"].append(name + ": " + str(error))
            artifacts.append({"path": name, "error": str(error), "bytes": size,
                              "retained_cap_unenforced": clipped})
            continue
        artifact["path"] = str(path.relative_to(directory))
        artifact["truncated_at_finalization"] = clipped
        artifact["bytes_before_final_cap"] = size
        artifacts.append(artifact)
    report["artifacts"] = artifacts
    report["finished_utc"] = datetime.now(timezone.utc).isoformat()
    write_json(directory / "metadata.json", report)
    print(json.dumps({"receipt": str(directory / "metadata.json"),
                      "compiler_returncode": report.get("compiler", {}).get("returncode"),
                      "profiler_status": report["profiler"]["status"]}, sort_keys=True))


if __name__ == "__main__":
    if sys.argv[1:2] == ["_measure"]:
        sys.exit(measure(Path(sys.argv[2])))
    sys.exit(main())
