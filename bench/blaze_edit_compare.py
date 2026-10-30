#!/usr/bin/env python3
"""Private, local edit diagnostic, not a suite replacement or promotion gate.

Blaze isolation follows upstream docs/cargo.md: RA_JIT_HOME, RA_JIT_CACHE,
XDG_CACHE_HOME and XDG_RUNTIME_DIR are private. CARGO_HOME stays the existing
normal dependency home. No `enable` command or normal configuration is changed.
All output files contain private paths and must not be published without review.
"""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import resource
import signal
import statistics
import subprocess
import time

STACKS = ('owned-off', 'owned-on', 'blaze')
FLAGS = ('jr-empty-lint-level-fast-path', 'jr-cache-small-query-values',
         'jr-cache-associated-items')
TIMEOUT_SECONDS = 360
# Exact keys only. Never log inherited credentials, proxy URLs or arbitrary env.
ENV_LOG_ALLOWLIST = frozenset(('HOME', 'CARGO_HOME', 'RUSTUP_HOME', 'CARGO_TARGET_DIR',
    'XDG_CACHE_HOME', 'XDG_RUNTIME_DIR', 'XDG_CONFIG_HOME', 'XDG_DATA_HOME',
    'CARGO', 'RUSTC', 'RUSTFLAGS', 'BLAZE_HOME', 'BLAZE_CACHE', 'BLAZE_BACKEND', 'BLAZE_OPT',
    'RA_JIT_HOME', 'RA_JIT_CACHE', 'JUSTRUST_TOOLCHAIN', 'JUSTRUST_TOOLCHAIN_NIGHTLY', 'JUSTRUST_REMOTE',
    'JUSTRUST_DEPCACHE', 'JUSTRUST_REMOTE_CACHE', 'JUSTRUST_PEER_CACHE',
    'JUSTRUST_CACHE_PROC_MACROS', 'JUSTRUST_BUILDD', 'JUSTRUST_BUILDD_PREBUILD',
    'JUSTRUST_CARGO_FORK', 'JUSTRUST_TOOLCHAIN_AUTO_INSTALL', 'JUSTRUST_REAL_CARGO',
    'JUSTRUST_HOME', 'JUSTRUST_HOTPATCH', 'JUSTRUST_LOCALCACHE', 'JUSTRUST_LOCALCACHE_TARGET_BOUND',
    'JUSTRUST_PREBUILT', 'JUSTRUST_PREBUILT_ALWAYS'))


def logged_environment(env):
    return {k: env[k] for k in sorted(ENV_LOG_ALLOWLIST) if k in env}


def succeeded(row):
    return row['status'] == 0 and not row.get('timed_out') and not row.get('error')


def type_error_bytes(original):
    return edit_bytes(original, 0).replace(
        b'std::hint::black_box(1u64);',
        b'let _: u64 = "blaze_edit_compare_type_error";', 1)


def type_error_detected(args, stack, row):
    if row['status'] != 101 or row.get('timed_out') or row.get('error'):
        return False
    prefix = args.output / f'type-error-000-{stack}'
    diagnostic = b''.join(prefix.with_suffix('.' + ext).read_bytes()
                          for ext in ('stdout', 'stderr'))
    return (b'blaze_edit_compare_type_error' in diagnostic
            and (b'E0308' in diagnostic or b'mismatched types' in diagnostic))


def digest(path):
    h = hashlib.sha256()
    with Path(path).open('rb') as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b''):
            h.update(chunk)
    return h.hexdigest()


def identities(args):
    paths = {name: Path(getattr(args, name)).resolve()
             for name in ('rustc', 'cargo', 'justrust', 'blaze')}
    # A compiler executable alone does not identify a dynamically linked rustc.
    drivers = sorted(paths['rustc'].parent.parent.glob('lib/**/librustc_driver*.so'))
    if not drivers:
        raise ValueError('No rustc driver found beside --rustc toolchain')
    paths.update({f'rustc_driver_{i}': p for i, p in enumerate(drivers)})
    # Include Blaze companion executables and its already installed toolchain.
    for p in sorted(paths['blaze'].parent.glob('*')):
        if p.is_file() and ('blaze' in p.name or 'ra-jit' in p.name):
            paths['blaze_companion:' + p.name] = p.resolve()
    blaze_home = Path(os.environ.get('BLAZE_HOME', str(Path.home() / '.blaze')))
    for pattern in ('toolchains/**/bin/rustc', 'toolchains/**/bin/cargo',
                    'toolchains/**/lib/**/librustc_driver*.so'):
        for p in sorted(blaze_home.glob(pattern)):
            if p.is_file():
                paths['blaze_toolchain:' + str(p.relative_to(blaze_home))] = p.resolve()
    return {k: {'path': str(p), 'sha256': digest(p)} for k, p in paths.items()}


def edit_bytes(original, replicate):
    matches = list(re.finditer(rb'(?m)^fn ago\(t: f64\) -> String \{', original))
    if len(matches) != 1:
        raise ValueError('Expected exactly one pinned fn ago(t: f64) -> String {')
    pos = matches[0].end()
    return original[:pos] + f'\n    std::hint::black_box({replicate + 1}u64);'.encode() + original[pos:]


def environment(args, stack):
    # Remove inherited overrides, including flags not part of this experiment.
    env = {k: v for k, v in os.environ.items()
           if not k.startswith(('JUSTRUST_', 'RA_JIT_', 'BLAZE_', 'CARGO_PROFILE_'))
           and k not in ('RUSTC', 'RUSTFLAGS', 'RUSTDOCFLAGS', 'CARGO_ENCODED_RUSTFLAGS',
                         'RUSTC_WRAPPER', 'RUSTC_WORKSPACE_WRAPPER', 'CARGO',
                         'RUSTUP_TOOLCHAIN', 'CARGO_BUILD_RUSTC', 'CARGO_BUILD_RUSTFLAGS')}
    root = args.output / stack
    for name in ('home', 'cache', 'runtime', 'config', 'data', 'target', 'daemon', 'store', 'bin'):
        (root / name).mkdir(parents=True, exist_ok=True, mode=0o700)
    normal_home = Path(os.environ.get('HOME', str(Path.home())))
    env.update(HOME=str(root / 'home'),
               CARGO_HOME=os.environ.get('CARGO_HOME', str(normal_home / '.cargo')),
               RUSTUP_HOME=os.environ.get('RUSTUP_HOME', str(normal_home / '.rustup')),
               CARGO_TARGET_DIR=str(root / 'target'), XDG_CACHE_HOME=str(root / 'cache'),
               XDG_RUNTIME_DIR=str(root / 'runtime'), XDG_CONFIG_HOME=str(root / 'config'),
               XDG_DATA_HOME=str(root / 'data'))
    if stack == 'blaze':
        blaze_home = root / 'blaze-home'
        blaze_home.mkdir(mode=0o700)
        installed = Path(os.environ.get('BLAZE_HOME', str(normal_home / '.blaze'))) / 'toolchains'
        if not installed.is_dir():
            raise ValueError('Blaze toolchains must be installed by caller before benchmarking')
        (blaze_home / 'toolchains').symlink_to(installed)
        env.update(RA_JIT_HOME=str(root / 'daemon'), RA_JIT_CACHE=str(root / 'store'),
                   BLAZE_CACHE=str(root / 'cache' / 'blaze'),
                   BLAZE_HOME=str(blaze_home))
    else:
        cargo_link = root / 'bin' / 'cargo'
        cargo_link.symlink_to(args.cargo)
        env.update(PATH=str(root / 'bin') + os.pathsep + env.get('PATH', ''),
                   CARGO=str(args.cargo), RUSTC=str(args.rustc),
                   JUSTRUST_REAL_CARGO=str(args.cargo), JUSTRUST_HOME=str(root / 'home' / '.justrust'),
                   JUSTRUST_LOCALCACHE='0', JUSTRUST_LOCALCACHE_TARGET_BOUND='0',
                   JUSTRUST_PREBUILT='off', JUSTRUST_PREBUILT_ALWAYS='0',
                   JUSTRUST_TOOLCHAIN='system', JUSTRUST_REMOTE='off',
                   JUSTRUST_DEPCACHE='0', JUSTRUST_REMOTE_CACHE='0',
                   JUSTRUST_PEER_CACHE='0', JUSTRUST_CACHE_PROC_MACROS='0',
                   JUSTRUST_BUILDD='0', JUSTRUST_BUILDD_PREBUILD='0',
                   JUSTRUST_CARGO_FORK='0', JUSTRUST_TOOLCHAIN_AUTO_INSTALL='0',
                   RUSTFLAGS=' '.join(f'-Z{f}={"yes" if stack == "owned-on" else "no"}' for f in FLAGS))
    return env


def cpu_snapshot():
    try:
        fields = Path('/proc/stat').read_text().splitlines()[0].split()[1:9]
        ticks = list(map(int, fields))
        return {'busy_seconds': (sum(ticks) - ticks[3] - ticks[4]) / os.sysconf('SC_CLK_TCK'),
                'loadavg': list(os.getloadavg())}
    except (OSError, ValueError):
        return None


def child_cpu():
    usage = resource.getrusage(resource.RUSAGE_CHILDREN)
    return usage.ru_utime + usage.ru_stime


def run_command(args, stack, phase, replicate, env):
    prefix = args.output / f'{phase}-{replicate:03d}-{stack}'
    command = [str(args.blaze if stack == 'blaze' else args.justrust), args.command, '--lib']
    before, child_before = cpu_snapshot(), child_cpu()
    start_wall = datetime.datetime.now(datetime.timezone.utc).isoformat()
    start = time.perf_counter()
    error = None
    status = None
    timed_out = False
    try:
        with prefix.with_suffix('.stdout').open('xb') as out, prefix.with_suffix('.stderr').open('xb') as err:
            with subprocess.Popen(command, cwd=args.source, env=env, stdout=out, stderr=err,
                                  start_new_session=True) as process:
                try:
                    status = process.wait(timeout=TIMEOUT_SECONDS)
                except subprocess.TimeoutExpired:
                    timed_out = True
                    try:
                        os.killpg(process.pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                    status = process.wait()
                except BaseException:
                    try:
                        os.killpg(process.pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                    process.wait()
                    raise
    except OSError as exc:
        error = repr(exc)
    elapsed = time.perf_counter() - start
    after, child_after = cpu_snapshot(), child_cpu()
    noise = {'before': before, 'after': after, 'child_cpu_seconds': child_after - child_before,
             'contamination': 'unknown',
             'reason': 'System CPU minus waited child CPU is only a proxy. Daemons, scheduling, IO and thermal noise are not attributed.'}
    if before and after:
        noise['other_cpu_seconds_estimate'] = max(0, after['busy_seconds'] - before['busy_seconds'] - (child_after - child_before))
    result = dict(stack=stack, phase=phase, replicate=replicate, command=command,
                  cwd=str(args.source), environment=logged_environment(env), started_utc=start_wall,
                  perf_counter_start=start, elapsed_seconds=elapsed, status=status,
                  error=error, noise=noise, timed_out=timed_out, timeout_seconds=TIMEOUT_SECONDS,
                  source_sha256=digest(args.source / 'src/runs.rs'))
    prefix.with_suffix('.json').write_text(json.dumps(result, indent=2) + '\n')
    return result


def summarize(records, samples, identity_valid):
    medians = {}
    for stack in STACKS:
        prep = [r for r in records if r['stack'] == stack and r['phase'] == 'prepare']
        gates = [r for r in records if r['stack'] == stack and r['phase'] == 'type-error']
        recoveries = [r for r in records if r['stack'] == stack and r['phase'] == 'recovery']
        rows = [r for r in records if r['stack'] == stack and r['phase'] == 'edit']
        if (identity_valid and len(prep) == 1 and succeeded(prep[0])
                and len(gates) == 1 and gates[0].get('gate_passed') is True
                and len(recoveries) == 1 and succeeded(recoveries[0])
                and len(rows) == samples and all(succeeded(r) for r in rows)):
            medians[stack] = statistics.median(r['elapsed_seconds'] for r in rows)
    return {'medians_seconds': medians,
            'descriptive_ratios_owned_off_over': {k: medians['owned-off'] / v for k, v in medians.items() if 'owned-off' in medians and v > 0},
            'interpretation': 'Descriptive only. No significance, cleanliness, default-promotion or general speed claim. Failed and incomplete stacks are excluded.',
            'identities_unchanged': identity_valid}


def execute(args):
    args.output.mkdir(parents=True, exist_ok=False, mode=0o700)
    pinned = args.source / 'src/runs.rs'
    original = pinned.read_bytes()
    (args.output / 'runs.rs.original').write_bytes(original)
    edit_bytes(original, 0)  # Validate before invoking any stack.
    initial = identities(args)
    metadata = {'identities_before': initial, 'machine': {'platform': platform.platform(),
                'uname': list(platform.uname()), 'cpu_count': os.cpu_count(),
                'cpuinfo': Path('/proc/cpuinfo').read_text(),
                'meminfo': Path('/proc/meminfo').read_text()},
                'source_sha256': hashlib.sha256(original).hexdigest(),
                'policy': 'Sequential same source; isolated targets/HOME/XDG; shared normal CARGO_HOME dependency downloads; Blaze installed toolchain home shared read-only by intent; no enable or global config writes. Only explicit policy allowlist environment keys logged. Each stack must reject deliberate E0308 then recover before sampling.',
                'samples': args.samples, 'command': args.command}
    (args.output / 'metadata.json').write_text(json.dumps(metadata, indent=2) + '\n')
    records = []
    valid = False
    failure = None
    try:
        environments = {s: environment(args, s) for s in STACKS}
        active = []
        for stack in STACKS:
            pinned.write_bytes(original)
            row = run_command(args, stack, 'prepare', 0, environments[stack])
            records.append(row)
            if not succeeded(row):
                continue
            pinned.write_bytes(type_error_bytes(original))
            gate = run_command(args, stack, 'type-error', 0, environments[stack])
            gate['gate_passed'] = type_error_detected(args, stack, gate)
            records.append(gate)
            (args.output / f'type-error-000-{stack}.json').write_text(json.dumps(gate, indent=2) + '\n')
            pinned.write_bytes(original)
            recovery = run_command(args, stack, 'recovery', 0, environments[stack])
            records.append(recovery)
            if gate['gate_passed'] and succeeded(recovery):
                active.append(stack)
        for replicate in range(args.samples):
            edited = edit_bytes(original, replicate)
            order = STACKS[replicate % len(STACKS):] + STACKS[:replicate % len(STACKS)]
            for stack in order:
                if stack not in active:
                    continue
                pinned.write_bytes(edited)
                row = run_command(args, stack, 'edit', replicate, environments[stack])
                row['source_sha256'] = hashlib.sha256(edited).hexdigest()
                records.append(row)
                with (args.output / 'measurements.jsonl').open('a') as stream:
                    stream.write(json.dumps(row) + '\n')
    except BaseException as exc:
        failure = repr(exc)
        raise
    finally:
        pinned.write_bytes(original)
        try:
            final = identities(args)
            valid = initial == final
            metadata['identities_after'] = final
        except Exception as exc:
            metadata['identity_error'] = repr(exc)
        metadata.update(identities_unchanged=valid, failure=failure,
                        source_restored=pinned.read_bytes() == original)
        (args.output / 'metadata.json').write_text(json.dumps(metadata, indent=2) + '\n')
        summary = summarize(records, args.samples, valid)
        (args.output / 'summary.json').write_text(json.dumps(summary, indent=2) + '\n')
    return 0 if valid and len(summary['medians_seconds']) == len(STACKS) else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for key in ('source', 'output', 'blaze', 'justrust', 'rustc', 'cargo'):
        parser.add_argument('--' + key, required=True, type=Path)
    parser.add_argument('--samples', type=int, default=3)
    parser.add_argument('--command', choices=('check', 'build'), default='check')
    args = parser.parse_args()
    if args.samples < 1:
        parser.error('--samples must be positive')
    for key in ('source', 'output', 'blaze', 'justrust', 'rustc', 'cargo'):
        setattr(args, key, getattr(args, key).expanduser().resolve())
    raise SystemExit(execute(args))


if __name__ == '__main__':
    main()
