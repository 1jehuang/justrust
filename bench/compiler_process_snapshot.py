#!/usr/bin/env python3
"""Real CRIU disk checkpoint experiment for --jr-resident-owned rustc.

No native build, installation, fork snapshot, or encoded-query substitute. Requires
Linux and a CRIU-capable dedicated host. Receipts retain unsupported failures.
Run: python3 bench/compiler_process_snapshot.py --compiler /abs/jr-rustc
     --out /abs/new-evidence --criu /abs/criu
"""
import argparse
import hashlib
import json
import os
import platform
from pathlib import Path
import select
import signal
import subprocess
import sys
import time

import stable_resident as shared
import stable_resident_owned as owned


MODES = ('fresh', 'warm', 'restored')


def mode_order(value):
    order = tuple(value.split(',')) if isinstance(value, str) else tuple(value)
    if len(order) != len(MODES) or set(order) != set(MODES):
        raise ValueError('mode order must be an exact permutation of fresh,warm,restored')
    return order


def provenance():
    """Local observations, not a remote attestation or anonymized host claim."""
    info = dict(hostname=platform.node(), platform=platform.platform(),
                machine=platform.machine(), kernel=platform.release(),
                python=sys.version, executable=sys.executable, uid=os.getuid(),
                invocation=[sys.executable, *sys.argv], captured_unix_seconds=time.time(),
                logical_cpus=os.cpu_count())
    if hasattr(os, 'sched_getaffinity'):
        info['cpu_affinity'] = sorted(os.sched_getaffinity(0))
    for key, filename in (('cpuinfo', '/proc/cpuinfo'), ('meminfo', '/proc/meminfo'),
                          ('boot_id', '/proc/sys/kernel/random/boot_id')):
        try:
            info[key] = Path(filename).read_text()
        except OSError as error:
            info[key + '_error'] = str(error)
    info['harness_sources'] = [dict(path=str(Path(module.__file__).resolve()),
                                   sha256=shared.digest(module.__file__))
                               for module in (sys.modules[__name__], shared, owned)]
    return info


def launcher(compiler, root, channel):
    """Only the compiler owns these FIFO descriptions at checkpoint time."""
    root = Path(root)
    request = os.open(root / (channel + '.request'), os.O_RDWR)
    response = os.open(root / (channel + '.response'), os.O_RDWR)
    os.dup2(request, 0)
    if request != 0:
        os.close(request)
    for fd, suffix in ((1, '.stdout'), (2, '.stderr')):
        stream = os.open(root / (channel + suffix), os.O_WRONLY | os.O_CREAT, 0o600)
        os.dup2(stream, fd)
        if stream != fd:
            os.close(stream)
    os.set_inheritable(response, True)
    env = owned.environment()
    env['JUSTRUST_RUSTC_RESIDENT_FD'] = str(response)
    os.execve(compiler, [compiler, '--jr-resident-owned'], env)


class DiskWorker:
    def __init__(self, compiler, root, channel, timeout):
        self.compiler, self.root, self.channel = compiler, root, channel
        self.timeout, self.sequence, self.offsets, self.stats = timeout, 0, [0, 0], None
        for suffix in ('.request', '.response'):
            os.mkfifo(root / (channel + suffix), 0o600)
        self.command = [sys.executable, str(Path(__file__).resolve()),
                        '--launch', str(compiler), str(root), channel]
        self.process = subprocess.Popen(self.command, stdin=subprocess.DEVNULL,
                                        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                                        cwd=root, start_new_session=True)
        self.pid = self.process.pid
        self.alive = True

    def compile(self, argv):
        self.sequence += 1
        request = dict(id=self.sequence, args=list(map(str, argv)))
        start = time.monotonic()
        with (self.root / (self.channel + '.jsonl')).open('ab') as transcript:
            transcript.write(json.dumps({'request': request}).encode() + b'\n')
        # Never keep the controller's endpoints open across dump/restore.
        response_fd = os.open(self.root / (self.channel + '.response'), os.O_RDWR | os.O_NONBLOCK)
        request_fd = os.open(self.root / (self.channel + '.request'), os.O_RDWR | os.O_NONBLOCK)
        data = b''
        try:
            payload = json.dumps(request).encode() + b'\n'
            if os.write(request_fd, payload) != len(payload):
                raise RuntimeError('Short FIFO request write')
            while b'\n' not in data:
                remaining = self.timeout - (time.monotonic() - start)
                if remaining <= 0:
                    raise TimeoutError('Compiler FIFO response timed out')
                if select.select([response_fd], [], [], min(.05, remaining))[0]:
                    data += os.read(response_fd, 65536)
                if len(data) > 64 * 1024 * 1024:
                    raise ValueError('Oversized compiler reply')
            if not data.endswith(b'\n') or data.count(b'\n') != 1:
                raise ValueError('Unexpected extra response frames')
            with (self.root / (self.channel + '.jsonl')).open('ab') as transcript:
                transcript.write(json.dumps({'response_b64': shared.encoded(data)}).encode() + b'\n')
            record = json.loads(data)
            if type(record.get('pid')) is not int or record['pid'] != self.pid:
                raise AssertionError('Worker PID mismatch')
            if record.get('id') != self.sequence or type(record.get('status')) is not int:
                raise AssertionError('Worker request/status mismatch')
            self.stats = owned.validate_owned(record.get('owned'), self.stats)
            if self.stats['revisions'] != self.sequence:
                raise AssertionError('Restored revision counter mismatch')
            if record['status'] == 0 and not self.stats['contexts']:
                raise AssertionError('Missing native owned context')
            record.update(request=request, raw_response_b64=shared.encoded(data),
                          worker_elapsed_seconds=record.get('elapsed_seconds'),
                          elapsed_seconds=time.monotonic() - start)
            for index, suffix in enumerate(('.stdout', '.stderr')):
                contents = (self.root / (self.channel + suffix)).read_bytes()
                record[('stdout_b64', 'stderr_b64')[index]] = shared.encoded(contents[self.offsets[index]:])
                self.offsets[index] = len(contents)
            return record
        finally:
            os.close(request_fd)
            os.close(response_fd)

    def close(self):
        if self.alive and Path(f'/proc/{self.pid}/exe').exists():
            actual = Path(f'/proc/{self.pid}/exe').resolve()
            if actual != self.compiler:
                raise RuntimeError('Refusing to signal unexpected process: ' + str(actual))
            os.kill(self.pid, signal.SIGKILL)
        if self.process.returncode is None:
            self.process.wait(timeout=10)
        self.alive = False


def image_inventory(images):
    rows = [dict(path=str(p), size=p.stat().st_size, sha256=shared.digest(p))
            for p in sorted(images.rglob('*')) if p.is_file()]
    if not any(Path(row['path']).name == 'inventory.img' and row['size'] for row in rows):
        raise AssertionError('CRIU did not produce a disk inventory.img')
    if not any(Path(row['path']).name.startswith('pages-') and row['size'] for row in rows):
        raise AssertionError('CRIU did not persist process memory pages')
    return rows


def checkpoint(worker, criu, directory, evidence):
    directory.mkdir()
    evidence['dump'] = shared.run([criu, 'dump', '-t', str(worker.pid),
        '--images-dir', directory, '--shell-job', '--log-file', 'dump.log', '-v4'], worker.timeout)
    if evidence['dump']['status'] or evidence['dump']['timed_out']:
        raise RuntimeError('CRIU dump unsupported/failed, see original dump receipt/log')
    worker.process.wait(timeout=10) if worker.process.returncode is None else None
    # A zombie is not a running original, but an executable still present is.
    if Path(f'/proc/{worker.pid}/exe').exists():
        raise AssertionError('Original process survived dump, refusing snapshot claim')
    worker.alive = False
    evidence['original_terminated'] = True
    evidence['images'] = image_inventory(directory)


def restore(worker, criu, directory, evidence):
    pidfile = directory / 'restored.pid'
    evidence['restore'] = shared.run([criu, 'restore', '--images-dir', directory,
        '--shell-job', '--restore-detached', '--pidfile', pidfile,
        '--log-file', 'restore.log', '-v4'], worker.timeout)
    if evidence['restore']['status'] or evidence['restore']['timed_out']:
        raise RuntimeError('CRIU disk restore unsupported/failed, see original receipt/log')
    pid = int(pidfile.read_text().strip())
    if pid != worker.pid:
        raise AssertionError('CRIU restored unexpected PID')
    worker.alive = True
    if Path(f'/proc/{pid}/exe').resolve() != worker.compiler:
        raise AssertionError('Restored executable mismatch')
    evidence['restored_pid'] = pid


def capture(compiler, root, criu, timeout, order=MODES):
    order = mode_order(order)
    report = dict(schema=2, verdict='incomplete', mechanism='CRIU disk full-process dump/restore',
                  claim='Prototype timings only, not production or general speedup evidence.',
                  observations=[], cwd=str(root), compiler=str(compiler), criu=str(criu),
                  mode_order=list(order), host_provenance=provenance())
    workers = []
    source_paths = [root / 'fixture.rs', root / 'helper.rs']
    originals = {p: p.read_bytes() if p.exists() else None for p in source_paths}
    try:
        report['criu_version'] = shared.run([criu, '--version'], timeout)
        report['criu_check'] = shared.run([criu, 'check'], timeout)
        if report['criu_check']['status'] or report['criu_check']['timed_out']:
            raise RuntimeError('CRIU check unsupported/failed, original output retained')
        report['identity_before'] = shared.identity(compiler)
        report['environment_sha256'] = hashlib.sha256(
            json.dumps(owned.environment(), sort_keys=True).encode()).hexdigest()
        report['native_worker_command'] = [str(compiler), '--jr-resident-owned']
        argv = owned.arguments(root)
        report['argv'] = argv
        workload = owned.cases()
        for p, text in zip(source_paths, workload[0][1:3]):
            p.write_text(text)
        warm = DiskWorker(compiler, root, 'warm', timeout)
        workers.append(warm)
        snap = DiskWorker(compiler, root, 'snapshot', timeout)
        workers.append(snap)
        report['workers'] = [dict(pid=w.pid, command=w.command) for w in workers]
        report['initialization'] = [w.compile(argv) for w in workers]
        for index, (name, source, helper, expected) in enumerate(workload[1:]):
            row = dict(name=name, source=source, helper=helper, expected=expected, modes={}, checkpoint={})
            report['observations'].append(row)
            # Snapshot the PREVIOUS revision, before this edit is installed.
            images = root / ('images-%02d' % index)
            checkpoint(snap, criu, images, row['checkpoint'])
            for p, text in zip(source_paths, (source, helper)):
                p.write_text(text)
            for mode in order:
                binary = root / 'fixture'
                binary.unlink(missing_ok=True)
                result = row['modes'][mode] = {}
                if mode == 'restored':
                    start = time.monotonic()
                    restore(snap, criu, images, row['checkpoint'])
                    result['compile'] = snap.compile(argv)
                    result['restore_plus_compile_seconds'] = time.monotonic() - start
                elif mode == 'warm':
                    result['compile'] = warm.compile(argv)
                else:
                    result['compile'] = shared.run([compiler, *argv], timeout, cwd=root, env=owned.environment())
                if result['compile']['status'] == 0:
                    result['artifact_sha256'] = shared.digest(binary)
                    result['runtime'] = owned.runtime(binary, timeout, root)
            for mode, worker in (('warm', warm), ('restored', snap)):
                owned.validate_pair({'fresh': row['modes']['fresh'], 'owned': row['modes'][mode]}, expected, worker.pid)
                if row['modes']['fresh'].get('artifact_sha256') != row['modes'][mode].get('artifact_sha256'):
                    raise AssertionError(mode + ' artifact differs byte-for-byte from fresh')
        report['identity_after'] = shared.identity(compiler)
        if report['identity_after'] != report['identity_before']:
            raise AssertionError('Compiler identity changed')
        report['verdict'] = 'passed'
    except BaseException as error:
        report.update(verdict='failed', error=f'{type(error).__name__}: {error}')
    finally:
        for worker in workers:
            try:
                worker.close()
            except Exception as error:
                report.setdefault('cleanup_errors', []).append(str(error))
                report['verdict'] = 'failed'
        for p, content in originals.items():
            if content is None:
                p.unlink(missing_ok=True)
            else:
                p.write_bytes(content)
        (root / 'receipt.json').write_text(json.dumps(report, indent=2) + '\n')
    return report


def main():
    if len(sys.argv) == 5 and sys.argv[1] == '--launch':
        launcher(*sys.argv[2:])
        return 0
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--compiler', type=Path, required=True)
    parser.add_argument('--out', '--output', type=Path, required=True)
    parser.add_argument('--criu', type=Path, required=True)
    parser.add_argument('--timeout', type=float, default=180)
    parser.add_argument('--mode-order', default=','.join(MODES),
                        help='Comma-separated exact permutation of fresh,warm,restored')
    parser.add_argument('--repeats', type=int, default=1,
                        help='Independent worker sessions, each in a new repeat directory')
    args = parser.parse_args()
    if args.timeout <= 0:
        parser.error('--timeout must be positive')
    if args.repeats <= 0:
        parser.error('--repeats must be positive')
    try:
        order = mode_order(args.mode_order)
    except ValueError as error:
        parser.error(str(error))
    root = args.out.resolve()
    root.mkdir(parents=True, exist_ok=False)
    if args.repeats == 1:
        report = capture(args.compiler.resolve(), root, args.criu.resolve(), args.timeout, order)
    else:
        report = dict(schema=2, verdict='incomplete', repeats_requested=args.repeats,
                      mode_order=list(order), host_provenance=provenance(), repeats=[])
        try:
            for index in range(args.repeats):
                directory = root / ('repeat-%03d' % index)
                directory.mkdir()
                row = capture(args.compiler.resolve(), directory, args.criu.resolve(), args.timeout, order)
                report['repeats'].append(dict(index=index, receipt=str(directory / 'receipt.json'),
                                              verdict=row['verdict']))
                if row['verdict'] != 'passed':
                    break
            report['verdict'] = ('passed' if len(report['repeats']) == args.repeats and
                                 all(r['verdict'] == 'passed' for r in report['repeats']) else 'failed')
        except BaseException as error:
            report.update(verdict='failed', error=f'{type(error).__name__}: {error}')
        finally:
            (root / 'receipt.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report, indent=2))
    return 0 if report['verdict'] == 'passed' else 1


if __name__ == '__main__':
    sys.exit(main())
