#!/usr/bin/env python3
"""Linux benchmark session guardian. No process-name matching or group signals.

The guardian remains the session leader until every live session member is gone.
Payloads may create process groups, but must not escape with setsid(). Ownership
records are private, fsynced and atomically replaced before any payload executes.
"""
import argparse
import fcntl
import json
import os
from pathlib import Path
import signal
import stat
import subprocess
import sys
import time


def boot():
    return Path('/proc/sys/kernel/random/boot_id').read_text().strip()


def identity(pid):
    try:
        text = Path(f'/proc/{pid}/stat').read_text()
        fields = text[text.rindex(')') + 2:].split()
        return {'pid': int(pid), 'sid': int(fields[3]),
                'start': int(fields[19]), 'state': fields[0]}
    except (FileNotFoundError, ProcessLookupError):
        return None


def atomic(path, record):
    tmp = path.with_name(path.name + f'.{os.getpid()}.tmp')
    fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    try:
        with os.fdopen(fd, 'w') as stream:
            json.dump(record, stream)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(tmp, path)
        fd = os.open(path.parent, os.O_DIRECTORY)
        try:
            os.fsync(fd)
        finally:
            os.close(fd)
    finally:
        tmp.unlink(missing_ok=True)


def read_record(path):
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
    with os.fdopen(fd) as stream:
        info = os.fstat(stream.fileno())
        if not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid() or info.st_mode & 0o077:
            raise ValueError('ownership record is not a private owned regular file')
        record = json.load(stream)
    if (not isinstance(record, dict) or record.get('version') != 1 or record.get('boot') != boot()
            or any(type(record.get(k)) is not int or record[k] <= 0
                   for k in ('pid', 'sid', 'start'))
            or record['sid'] != record['pid']
            or record.get('status') not in ('running', 'finished', 'failed')):
        raise ValueError('invalid ownership identity or boot mismatch')
    return record


def same(record, current):
    return current is not None and all(record[k] == current[k] for k in ('pid', 'sid', 'start'))


def members(sid):
    result = []
    for entry in Path('/proc').iterdir():
        if entry.name.isdecimal():
            item = identity(int(entry.name))
            if item and item['sid'] == sid and item['pid'] != sid and item['state'] not in ('Z', 'X'):
                result.append(item)
    return result


def sweep(sid, sig):
    for item in members(sid):
        try:
            fd = os.pidfd_open(item['pid'])
        except ProcessLookupError:
            continue
        try:
            if same(item, identity(item['pid'])):
                signal.pidfd_send_signal(fd, sig)
        except ProcessLookupError:
            pass
        finally:
            os.close(fd)


def cleanup(sid, grace):
    # Repeat enumeration to include children forked during termination. The
    # live guardian pins SID identity throughout, including between scans.
    for sig, duration in ((signal.SIGTERM, grace), (signal.SIGKILL, 3.0)):
        deadline = time.monotonic() + duration
        while True:
            if not members(sid):
                return True
            sweep(sid, sig)
            if time.monotonic() >= deadline:
                break
            time.sleep(0.05)
    return not members(sid)


def run(args):
    os.umask(0o077)
    path = Path(args.record).absolute()
    lock = os.open(str(path) + '.lock', os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    if path.exists():
        old = read_record(path)
        if old.get('status') != 'finished':
            raise ValueError('existing session has no verified terminal state')
    me = identity(os.getpid())
    if me['sid'] != me['pid']:
        raise ValueError('guardian must be a new session leader')
    cancelled = []
    for sig in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP, signal.SIGQUIT, signal.SIGUSR1):
        signal.signal(sig, lambda number, frame: cancelled.append(number))
    record = dict(me, version=1, boot=boot(), status='running')
    record.pop('state')
    atomic(path, record)
    atomic(Path(args.pidfile), me['pid'])
    status = 125
    clean = False
    child = None
    try:
        child = subprocess.Popen(args.command)
        deadline = time.monotonic() + args.timeout
        while child.poll() is None and not cancelled and time.monotonic() < deadline:
            time.sleep(0.05)
        status = child.poll()
        if cancelled:
            status = 128 + cancelled[0]
        elif status is None:
            status = 124
    finally:
        clean = cleanup(me['sid'], args.grace)
        if not clean:
            status = 125
        elif status is not None and status < 0:
            status = 128 - status
        if clean and child is not None:
            child.wait(timeout=1)
        record.update(status='finished' if clean else 'failed', exit=status)
        atomic(path, record)
        Path(args.pidfile).unlink(missing_ok=True)
        print(f'EXIT {status}', flush=True)
    return status


def control(args):
    path = Path(args.record)
    record = read_record(path)
    if record.get('status') == 'finished':
        if members(record['sid']):
            raise ValueError('terminal record has live session members')
        print('finished', record.get('exit'))
        return 0
    if record.get('status') == 'failed':
        raise ValueError('previous session cleanup failed')
    fd = os.pidfd_open(record['pid'])
    try:
        if not same(record, identity(record['pid'])):
            raise ValueError('session leader identity changed')
        if args.action == 'status':
            print('running' if record['status'] == 'running' else 'cleanup failed')
            return 0 if record['status'] == 'running' else 1
        # Re-read immediately before signalling, never trust a replaced record.
        if read_record(path) != record:
            raise ValueError('ownership record changed')
        signal.pidfd_send_signal(fd, signal.SIGUSR1)
        deadline = time.monotonic() + 12
        while time.monotonic() < deadline:
            final = read_record(path)
            if any(final[k] != record[k] for k in ('pid', 'sid', 'start', 'boot')):
                raise ValueError('ownership record changed during stop')
            if final.get('status') == 'finished':
                if members(record['sid']):
                    raise ValueError('session members remain')
                print('stopped', final['exit'])
                return 0
            time.sleep(0.05)
        raise ValueError('bounded stop failed, session not verified clean')
    finally:
        os.close(fd)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=('start', 'run', 'stop', 'status'))
    parser.add_argument('--record', required=True)
    parser.add_argument('--log')
    parser.add_argument('--pidfile')
    parser.add_argument('--timeout', type=float, default=21600)
    parser.add_argument('--grace', type=float, default=5)
    argv = sys.argv[1:]
    split = argv.index('--') if '--' in argv else len(argv)
    args = parser.parse_args(argv[:split])
    args.command = argv[split + 1:]
    if args.action in ('start', 'run') and (not args.command or not args.pidfile
                                or not 0 < args.timeout <= 86400 or not 0 < args.grace <= 5):
        parser.error('run requires payload, pidfile and bounded positive timeout/grace')
    try:
        if args.action == 'start':
            if not args.log:
                parser.error('start requires log')
            command = [sys.executable, str(Path(__file__).absolute()), 'run',
                       '--record', args.record, '--pidfile', args.pidfile,
                       '--timeout', str(args.timeout), '--grace', str(args.grace),
                       '--', *args.command]
            with open(args.log, 'ab') as log:
                child = subprocess.Popen(command, start_new_session=True,
                                         stdin=subprocess.DEVNULL, stdout=log, stderr=log)
            deadline = time.monotonic() + 10
            while time.monotonic() < deadline:
                try:
                    record = read_record(Path(args.record))
                    if record['pid'] == child.pid:
                        if record['status'] == 'failed':
                            raise ValueError('guardian cleanup failed, inspect log')
                        if record['status'] == 'finished' or json.loads(Path(args.pidfile).read_text()) == child.pid:
                            print('started', child.pid)
                            return 0
                except FileNotFoundError:
                    pass
                if child.poll() is not None:
                    raise ValueError('guardian failed before ownership publication, inspect log')
                time.sleep(0.05)
            # No payload can start without publication. Ask the guardian to
            # clean up rather than killing its session anchor.
            child.terminate()
            raise ValueError('guardian publication timed out')
        return run(args) if args.action == 'run' else control(args)
    except (OSError, ValueError, KeyError, TypeError) as error:
        print(f'remote-session: {error}', file=sys.stderr)
        return 125


if __name__ == '__main__':
    sys.exit(main())
