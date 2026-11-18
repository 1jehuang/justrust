#!/usr/bin/env python3
"""Hosted-only, opt-in root parsed-AST disk snapshot experiment.

Never builds or installs a compiler. This is a narrow root parse snapshot test,
not restoration of full compiler state or a general compiler speedup claim.
All timings exclude fresh consumer verification. Native execution is forbidden
unless the host guard and explicit acknowledgement both pass.
"""
import argparse
from contextlib import contextmanager
import json
import math
import os
from pathlib import Path
import platform
import statistics
import subprocess
import time

import stable_resident as shared
import stable_resident_ast as ast

MODES = ('fresh', 'incremental', 'disk-ast', 'live-ast')
DISK_ENV = ('JUSTRUST_RUSTC_DISK_AST', 'JUSTRUST_RUSTC_DISK_AST_STATS')
HOST_MARKER = Path('/etc/justrust-hosted')


def host_guard(execute_host_workloads):
    if not execute_host_workloads:
        raise ValueError('explicit --execute-host-workloads required')
    if not HOST_MARKER.is_file():
        raise ValueError('hosted-only: missing /etc/justrust-hosted identity marker')
    info = HOST_MARKER.stat()
    if info.st_uid != 0 or info.st_mode & 0o022:
        raise ValueError('host identity marker must be root-owned and not group/world writable')
    return dict(marker=str(HOST_MARKER), marker_sha256=shared.digest(HOST_MARKER),
                hostname=platform.node(), kernel=platform.release(), machine=platform.machine(),
                affinity=sorted(os.sched_getaffinity(0)), cpuinfo=Path('/proc/cpuinfo').read_text())


def environment():
    env = ast.environment('0')
    for key in DISK_ENV:
        env.pop(key, None)
    return env


@contextmanager
def no_inherited_disk():
    saved = {key: os.environ.pop(key) for key in DISK_ENV if key in os.environ}
    try:
        yield
    finally:
        os.environ.update(saved)


def run(command, timeout, cwd, env):
    """Full streams retained, timeout reaped, PID and compiler-only elapsed time."""
    command = list(map(str, command))
    started = time.monotonic()
    process = subprocess.Popen(command, cwd=cwd, env=env, stdout=subprocess.PIPE,
                               stderr=subprocess.PIPE)
    timed_out = False
    try:
        stdout, stderr = process.communicate(timeout=timeout)
    except subprocess.TimeoutExpired:
        timed_out = True
        process.kill()
        stdout, stderr = process.communicate()
    return dict(command=command, pid=process.pid, status=process.returncode,
                timed_out=timed_out, elapsed_seconds=time.monotonic() - started,
                stdout_b64=shared.encoded(stdout), stderr_b64=shared.encoded(stderr))


def ordered_modes(sample, case):
    # Balanced Latin rotation with alternating direction, deterministic and recorded.
    offset = (sample + case) % len(MODES)
    order = MODES[offset:] + MODES[:offset]
    return tuple(reversed(order)) if (sample // len(MODES)) % 2 else order


def cases():
    selected = ast.cases()[:9]
    changed = ast.ROOT.replace('^ 7', '^ 19')
    helper = ast.HELPER.replace('^ 11', '^ 23')
    return selected + [('type-recovery', changed, helper, 19, 23, 'recover')]


def events_since(path, offset, pid):
    if not path.exists():
        return [], offset
    data = path.read_bytes()
    if len(data) < offset:
        raise AssertionError('append-only disk telemetry was truncated')
    events = []
    for line in data[offset:].splitlines():
        event = json.loads(line)
        if not isinstance(event, dict) or event.get('event') not in ('hit', 'miss', 'store', 'reject'):
            raise AssertionError('invalid disk telemetry event')
        if type(event.get('pid')) is not int or event['pid'] != pid:
            raise AssertionError('disk telemetry PID does not match fresh compiler process')
        events.append(event)
    return events, len(data)


def gate_row(row, previous_ast, live_pid, seen_pids, seen_nonces):
    reference = row['records']['fresh']['compile']
    for mode, record in row['records'].items():
        compile_record = record['compile']
        if compile_record.get('timed_out'):
            raise AssertionError('compiler timed out: ' + mode)
        if shared.observable(compile_record) != shared.observable(reference):
            raise AssertionError('exact fresh diagnostics/status parity failed: ' + mode)
        pid = compile_record.get('pid')
        if type(pid) is not int or pid <= 0:
            raise AssertionError('missing compiler PID')
        if mode == 'live-ast':
            if pid != live_pid:
                raise AssertionError('live worker PID changed')
        else:
            if pid == live_pid or pid in seen_pids:
                raise AssertionError('fresh compiler PID reused')
            seen_pids.add(pid)
        if row['a'] is None:
            if compile_record['status'] != 1 or 'runtime' in record:
                raise AssertionError('expected compilation error, not stale success')
        else:
            if compile_record['status'] != 0:
                raise AssertionError('expected compilation success')
            ast.validate_runtime(record['runtime'], row['a'], row['b'])
            nonce = record['runtime']['nonce']
            if nonce in seen_nonces:
                raise AssertionError('consumer nonce reused')
            seen_nonces.add(nonce)
    disk = row['records']['disk-ast']
    if row['name'] in ('unchanged', 'unrelated-helper-edit'):
        if not any(event['event'] == 'hit' for event in disk['disk_events']):
            raise AssertionError('positive genuine cross-process disk AST hit required')
    return ast.validate_stats(row['records']['live-ast']['compile'], previous_ast,
                              '1', row['kind'])


def write_receipt(path, data):
    # The evidence directory itself is exclusively created. Atomic checkpoint writes
    # replace only our own receipt, never a previous experiment or external file.
    pending = path.with_suffix('.pending')
    pending.write_text(json.dumps(data, indent=2) + '\n')
    pending.replace(path)


def verify_consumer(compiler, root, timeout, a, b):
    # Existing helper uses fresh consumer compiles and a fresh random execution nonce.
    # Disable disk snapshots so consumers cannot supply the measured hit evidence.
    with no_inherited_disk():
        return ast.execute(compiler, root, '0', timeout, a, b)


def capture_sample(compiler, root, sample, timeout):
    root.mkdir(mode=0o700)
    result = dict(sample=sample, verdict='incomplete', observations=[], failures=[])
    worker = None
    try:
        for name in ('incremental', 'disk-incremental', 'disk-cache'):
            (root / name).mkdir(mode=0o700)
        stats_path = root / 'disk-events.jsonl'
        (root / 'consumer.rs').write_text(ast.consumer_source())
        setup = time.monotonic()
        with no_inherited_disk():
            worker = ast.AstResident(compiler, root, '1', timeout)
        result.update(live_pid=worker.process.pid,
                      worker_startup_seconds=time.monotonic() - setup)
        previous, offset = None, 0
        seen_pids, seen_nonces = set(), set()
        for index, (name, source, helper, a, b, kind) in enumerate(cases()):
            row = dict(name=name, kind=kind, a=a, b=b, records={},
                       order=ordered_modes(sample, index), source=source, helper=helper)
            result['observations'].append(row)
            for filename, text in (('fixture.rs', source), ('helper.rs', helper)):
                (root / filename).write_text(text)
                (root / f'{index:02d}-{filename}').write_text(text)
            for mode in row['order']:
                artifact = root / 'libast_fixture.rlib'
                artifact.unlink(missing_ok=True)
                argv = ast.arguments(root)
                env = environment()
                if mode in ('incremental', 'disk-ast'):
                    cache = 'incremental' if mode == 'incremental' else 'disk-incremental'
                    argv += ['-Cincremental=' + str(root / cache)]
                if mode == 'disk-ast':
                    env.update(JUSTRUST_RUSTC_DISK_AST=str(root / 'disk-cache'),
                               JUSTRUST_RUSTC_DISK_AST_STATS=str(stats_path))
                record = row['records'][mode] = dict(argv=argv)
                record['compile'] = (worker.compile(argv) if mode == 'live-ast' else
                                     run([compiler, *argv], timeout, root, env))
                # Checkpoint raw streams before interpreting potentially malformed telemetry.
                write_receipt(root / 'receipt.json', result)
                if mode == 'disk-ast':
                    record['disk_events'], offset = events_since(stats_path, offset,
                                                                  record['compile']['pid'])
                if record['compile']['status'] == 0:
                    record['artifact_sha256'] = shared.digest(artifact)
                    record['runtime'] = verify_consumer(compiler, root, timeout, a, b)
                write_receipt(root / 'receipt.json', result)
            try:
                previous = gate_row(row, previous, worker.process.pid, seen_pids, seen_nonces)
            except AssertionError as error:
                result['failures'].append(dict(case=name, error=str(error)))
                current = row['records']['live-ast']['compile'].get('ast')
                if isinstance(current, dict):
                    previous = current
        result['controls'] = []
        corruption_controls(compiler, root, timeout, stats_path, offset,
                            seen_pids, seen_nonces, result['controls'])
        result['verdict'] = 'failed' if result['failures'] else 'passed'
    except BaseException as error:
        result.update(verdict='failed', error=f'{type(error).__name__}: {error}')
    finally:
        if worker is not None:
            try:
                worker.close()
            except Exception as error:
                result.update(verdict='failed', shutdown_error=str(error))
        write_receipt(root / 'receipt.json', result)
    return result


def corruption_controls(compiler, root, timeout, stats_path, offset, seen_pids,
                        seen_nonces, results):
    """Untimed fail-open checks, only perturbing this run's private disk cache."""
    for action in ('corrupt', 'truncate'):
        files = sorted(p for p in (root / 'disk-cache').rglob('*') if p.is_file())
        if not files or any(p.is_symlink() for p in files):
            raise AssertionError('controls require private genuine snapshot files, no symlinks')
        row = dict(action=action, damaged=[], records={})
        results.append(row)
        for path in files:
            row['damaged'].append(dict(path=str(path.relative_to(root)),
                                       sha256_before=shared.digest(path)))
            path.write_bytes(b'invalid parsed AST snapshot\n' if action == 'corrupt' else b'')
        for mode in ('fresh', 'disk-ast'):
            artifact = root / 'libast_fixture.rlib'
            artifact.unlink(missing_ok=True)
            argv, env = ast.arguments(root), environment()
            if mode == 'disk-ast':
                argv += ['-Cincremental=' + str(root / 'disk-incremental')]
                env.update(JUSTRUST_RUSTC_DISK_AST=str(root / 'disk-cache'),
                           JUSTRUST_RUSTC_DISK_AST_STATS=str(stats_path))
            record = row['records'][mode] = dict(compile=run([compiler, *argv], timeout, root, env))
            write_receipt(root / 'controls.json', results)
            compile_record = record['compile']
            pid = compile_record['pid']
            if pid in seen_pids:
                raise AssertionError('control compiler PID reused')
            seen_pids.add(pid)
            if mode == 'disk-ast':
                record['disk_events'], offset = events_since(stats_path, offset, pid)
                if not any(e['event'] in ('reject', 'miss') for e in record['disk_events']):
                    raise AssertionError('damaged snapshot produced no fail-open telemetry')
                if any(e['event'] == 'hit' for e in record['disk_events']):
                    raise AssertionError('damaged snapshot reported a genuine hit')
            if compile_record.get('timed_out') or compile_record['status'] != 0:
                raise AssertionError('damaged snapshot failed to compile successfully')
            record['runtime'] = verify_consumer(compiler, root, timeout, 19, 23)
            ast.validate_runtime(record['runtime'], 19, 23)
            nonce = record['runtime']['nonce']
            if nonce in seen_nonces:
                raise AssertionError('control consumer nonce reused')
            seen_nonces.add(nonce)
            write_receipt(root / 'controls.json', results)
        if shared.observable(row['records']['fresh']['compile']) != shared.observable(row['records']['disk-ast']['compile']):
            raise AssertionError('damaged snapshot diagnostics/status differ from fresh')
        row['verdict'] = 'passed'
        write_receipt(root / 'controls.json', results)


def summaries(samples):
    if not samples or any(s['verdict'] != 'passed' for s in samples):
        raise AssertionError('no medians before all correctness gates pass')
    return {name: {mode: statistics.median(
                sample['observations'][index]['records'][mode]['compile']['elapsed_seconds']
                for sample in samples) for mode in MODES}
            for index, (name, *_rest) in enumerate(cases()) if name != 'initial'}


def capture(compiler, output, samples=5, timeout=180, execute_host_workloads=False):
    host = host_guard(execute_host_workloads)
    compiler, output = Path(compiler), Path(output)
    if not compiler.is_absolute() or not compiler.is_file() or not os.access(compiler, os.X_OK):
        raise ValueError('--compiler must be an absolute existing executable')
    if type(samples) is not int or samples < 1 or not math.isfinite(timeout) or timeout <= 0:
        raise ValueError('positive samples and finite positive timeout required')
    output = output.absolute()
    output.mkdir(mode=0o700, parents=True, exist_ok=False)
    report = dict(schema=1, verdict='incomplete', host=host, samples=[],
                  harness_sha256=shared.digest(Path(__file__)),
                  claim='Root parsed-AST snapshot only, not full compiler-state restoration or general speedup.',
                  timing='Compiler command only. Initial cold compiles and worker setup separate. Consumer verification excluded.')
    try:
        with no_inherited_disk():
            report['compiler_before'] = shared.identity(compiler)
        for sample in range(samples):
            report['samples'].append(capture_sample(compiler, output / f'sample-{sample:03d}', sample, timeout))
            write_receipt(output / 'receipt.json', report)
        with no_inherited_disk():
            report['compiler_after'] = shared.identity(compiler)
        if report['compiler_before'] != report['compiler_after']:
            raise AssertionError('frozen compiler changed during experiment')
        if any(s['verdict'] != 'passed' for s in report['samples']):
            raise AssertionError('sample correctness gate failed')
        report['median_compiler_seconds'] = summaries(report['samples'])
        report['cold_compiler_seconds'] = [{mode: s['observations'][0]['records'][mode]['compile']['elapsed_seconds']
                                            for mode in MODES} for s in report['samples']]
        report['verdict'] = 'passed'
    except BaseException as error:
        report.update(verdict='failed', error=f'{type(error).__name__}: {error}')
        report.pop('median_compiler_seconds', None)
    finally:
        write_receipt(output / 'receipt.json', report)
    return report


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--compiler', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--samples', type=int, default=5)
    parser.add_argument('--timeout', type=float, default=180)
    parser.add_argument('--execute-host-workloads', action='store_true')
    args = parser.parse_args(argv)
    result = capture(**vars(args))
    print(json.dumps(dict(verdict=result['verdict'], receipt=str(args.output / 'receipt.json'))))
    return 0 if result['verdict'] == 'passed' else 1


if __name__ == '__main__':
    raise SystemExit(main())
