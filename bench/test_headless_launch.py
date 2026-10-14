#!/usr/bin/env python3
"""LOCAL ONLY. No real remote tools, builds, installs, or caller-terminal changes.

Run: python3 bench/test_headless_launch.py [-v] [TestName]
All fixtures live under JCODE_SCRATCH_DIR. Public-script integration uses copied
source and fake JR/ssh/rsync/tmux/git, then executes the captured tmux command
under a real privately owned controlling PTY. This is not a tmux fidelity test.
Private real tmux diagnostics explicitly prove the REJECTED setsid candidate
leaks after early kill-session. A green diagnostic is NOT launch approval.
GNU timeout fixture is direct exec in a non-orphaned background group, not a
reproduction of a legacy causal signal or an interactive shell job table.
HEADLESS_LAUNCH_SOURCE may select a pinned original copy for red/green checks.
"""
import ctypes
import errno
try:
    import fcntl
    import pty
    import termios
except ImportError:
    fcntl = pty = termios = None
import hashlib
import json
import os
from pathlib import Path
import select
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import unittest

HERE = Path(__file__).resolve()
ROOT = HERE.parent.parent
LIMIT = 5


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def receive(fd, seconds=LIMIT):
    deadline = time.monotonic() + seconds
    data = b""
    while b"\n" not in data:
        remaining = deadline - time.monotonic()
        if remaining <= 0 or not select.select([fd], [], [], max(0, remaining))[0]:
            raise TimeoutError("explicit child handshake deadline")
        chunk = os.read(fd, 65536)
        if not chunk:
            raise RuntimeError("child closed handshake before message: " + repr(data))
        data += chunk
    return json.loads(data.split(b"\n", 1)[0])


def send(fd, value):
    os.write(fd, (json.dumps(value) + "\n").encode())


def probe():
    report, go, jobread, jobwrite = map(int, sys.argv[2:6])
    # Exec inherited these descriptors, including both existing jobserver ends.
    os.fstat(jobread)
    os.fstat(jobwrite)
    if os.read(jobread, 1) != b'J':
        raise RuntimeError('jobserver token not preserved')
    os.write(jobwrite, b'J')
    try:
        tty = os.open('/dev/tty', os.O_RDONLY)
    except OSError as exc:
        if exc.errno != errno.ENXIO:
            raise
        tty = None
    send(report, dict(pid=os.getpid(), sid=os.getsid(0), pgid=os.getpgrp(),
                      tty=tty is not None, stdin=os.readlink('/proc/self/fd/0'),
                      jobserver=True))
    if os.read(go, 1) != b'G':
        raise RuntimeError('missing go handshake')
    if tty is not None:
        # Real kernel SIGTTIN when our group is background and non-orphaned.
        os.read(tty, 1)
    time.sleep(0.15)
    if '--signal' in sys.argv:
        os.kill(os.getpid(), signal.SIGTERM)
    raise SystemExit(23)


def supervisor(config):
    # This process alone owns a new session and its controlling PTY.
    # Become a subreaper so launcher forks cannot escape owned cleanup.
    if ctypes.CDLL(None, use_errno=True).prctl(36, 1, 0, 0, 0) != 0:
        raise OSError(ctypes.get_errno(), 'PR_SET_CHILD_SUBREAPER')
    os.setsid()
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSCTTY, 0)
    os.tcsetpgrp(slave, os.getpgrp())
    report_r, report_w = os.pipe()
    go_r, go_w = os.pipe()
    job_r, job_w = os.pipe()
    os.write(job_w, b'J')
    for fd in (report_w, go_r, job_r, job_w):
        os.set_inheritable(fd, True)
    child = os.fork()
    if child == 0:
        # Deliberately non-orphaned background group: supervisor stays in session.
        os.setpgid(0, 0)
        null = os.open('/dev/null', os.O_RDONLY)
        os.dup2(null, 0)
        os.close(null)
        for fd in (master, slave, report_r, go_w):
            os.close(fd)
        args = [sys.executable, str(HERE), '--probe', str(report_w), str(go_r),
                str(job_r), str(job_w)]
        if config.get('signal'):
            args.append('--signal')
        if config['mode'] == 'setsid':
            args = [shutil.which('setsid'), '--wait', *args]
        elif config['mode'] == 'timeout':
            args = [shutil.which('timeout'), '4s', *args]
        elif config['mode'] == 'command':
            args = ['bash', '-c', config['command']]
        os.execvpe(args[0], args, os.environ)
    os.close(report_w)
    os.close(go_r)
    result = dict(supervisor=os.getpid(), child=child, mode=config['mode'])
    def expired(signum, frame):
        raise TimeoutError('supervisor total deadline or cancellation')
    signal.signal(signal.SIGALRM, expired)
    signal.signal(signal.SIGTERM, expired)
    signal.alarm(LIMIT + 1)
    try:
        if config['mode'] != 'command':
            result['probe'] = receive(report_r)
            os.write(go_w, b'G')
        deadline = time.monotonic() + LIMIT
        while time.monotonic() < deadline:
            pid, status = os.waitpid(child, os.WNOHANG | os.WUNTRACED)
            if pid:
                if os.WIFSTOPPED(status):
                    result['stop_signal'] = os.WSTOPSIG(status)
                else:
                    result['exit'] = os.waitstatus_to_exitcode(status)
                break
            # GNU timeout ignores SIGTTIN itself, but its payload can stop.
            if config['mode'] == 'timeout':
                p = result['probe']['pid']
                state = Path(f'/proc/{p}/status').read_text().split('State:', 1)[1].splitlines()[0].strip()
                if state.startswith('T'):
                    result['payload_stopped'] = state
                    break
            time.sleep(0.01)
        else:
            raise TimeoutError('owned launcher completion deadline')
    finally:
        signal.alarm(0)
        # Never signal a saved PID/PGID after waitpid has reaped it: that ID
        # might already belong to an unrelated process. Each ID read here is
        # an un-reaped direct child of this sole-threaded subreaper, so cannot
        # be recycled until OUR next waitpid. Killing parents adopts their
        # descendants, which the next iteration cleans the same safe way.
        deadline = time.monotonic() + 1
        killed, reaped = set(), set()
        while time.monotonic() < deadline:
            try:
                while True:
                    owned, status = os.waitpid(-1, os.WNOHANG)
                    if not owned:
                        break
                    reaped.add(owned)
            except ChildProcessError:
                break
            children = Path(f'/proc/self/task/{os.getpid()}/children').read_text().split()
            for owned in children:
                try:
                    os.kill(int(owned), signal.SIGKILL)
                    killed.add(int(owned))
                except ProcessLookupError:
                    pass
            time.sleep(0.01)
        remaining = Path(f'/proc/self/task/{os.getpid()}/children').read_text().split()
        result['cleanup'] = dict(killed=sorted(killed), reaped=sorted(reaped), remaining=remaining)
        # Closing our PTY master sends HUP to its foreground group (us).
        # Change only supervisor disposition after children have been cleaned.
        signal.signal(signal.SIGHUP, signal.SIG_IGN)
        for fd in (master, slave, report_r, go_w, job_r, job_w):
            os.close(fd)
        if remaining:
            raise RuntimeError('owned descendants exceeded cleanup deadline: ' + repr(remaining))
    print(json.dumps(result), flush=True)


class HeadlessLaunch(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        missing = [name for name in ('fork', 'setsid') if not hasattr(os, name)]
        missing += [name for name in ('bash', 'setsid', 'timeout') if not shutil.which(name)]
        if sys.platform != 'linux' or missing or fcntl is None or pty is None or termios is None:
            raise unittest.SkipTest('needs Linux PTY/fork/util-linux setsid/GNU timeout: ' + repr(missing))
        if 'util-linux' not in subprocess.check_output(['setsid', '--version'], text=True, timeout=2):
            raise unittest.SkipTest('setsid is not util-linux')
        if 'GNU coreutils' not in subprocess.check_output(['timeout', '--version'], text=True, timeout=2):
            raise unittest.SkipTest('timeout is not GNU')
        base = os.environ.get('JCODE_SCRATCH_DIR')
        if not base:
            raise unittest.SkipTest('JCODE_SCRATCH_DIR required, no /tmp fallback')
        try:
            master, slave = pty.openpty()
        except OSError as exc:
            raise unittest.SkipTest('real PTY unavailable: ' + str(exc))
        else:
            os.close(master)
            os.close(slave)
        cls.fixture = Path(tempfile.mkdtemp(prefix='headless-launch-', dir=base))
        cls.source_file = cls.fixture / 'on-remote.sh'
        cls.source_file.write_bytes(Path(os.environ.get('HEADLESS_LAUNCH_SOURCE', ROOT / 'bench/on-remote.sh')).read_bytes())
        cls.source_hash = digest(cls.source_file)
        print(f'FIXTURE {cls.fixture}\nSOURCE_SHA256 {cls.source_hash}', flush=True)

    def run_owned(self, mode, **extra):
        # Outer watchdog can kill only this supervisor, never caller's group.
        # Inner supervisor performs group cleanup in finally on normal failures.
        config = dict(mode=mode, **extra)
        proc = subprocess.Popen([sys.executable, str(HERE), '--supervise', json.dumps(config)],
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        try:
            out, err = proc.communicate(timeout=LIMIT + 3)
        except subprocess.TimeoutExpired:
            proc.terminate()
            try:
                proc.communicate(timeout=2)
            except subprocess.TimeoutExpired:
                proc.kill()
                proc.communicate(timeout=1)
            self.fail('supervisor hard deadline (unexpected infrastructure failure)')
        self.assertEqual(proc.returncode, 0, err)
        if err:
            print('CHILD_STDERR ' + repr(err), flush=True)
        result = json.loads(out)
        self.assertEqual(result['cleanup']['remaining'], [], 'all owned descendants reaped')
        result['stderr'] = err
        print('OBSERVED ' + json.dumps(result), flush=True)
        return result

    def test_devnull_still_sigttin(self):
        result = self.run_owned('plain')
        self.assertEqual(result['stop_signal'], signal.SIGTTIN)
        self.assertTrue(result['probe']['tty'])
        self.assertEqual(result['probe']['stdin'], '/dev/null')
        self.assertEqual(result['probe']['sid'], result['supervisor'])
        self.assertNotEqual(result['probe']['pgid'], result['supervisor'])

    def test_setsid_wait_exit23_inherited_jobserver(self):
        result = self.run_owned('setsid')
        self.assertEqual(result['exit'], 23)
        self.assertFalse(result['probe']['tty'])
        self.assertTrue(result['probe']['jobserver'])
        self.assertNotEqual(result['probe']['sid'], result['supervisor'])

    def test_setsid_wait_reports_child_signal_number(self):
        result = self.run_owned('setsid', signal=True)
        # Observed util-linux semantics: --wait exits with WTERMSIG itself,
        # NOT 128+signal and NOT a negative Python signal return code.
        self.assertEqual(result['exit'], signal.SIGTERM)

    def test_gnu_timeout_background_group(self):
        result = self.run_owned('timeout')
        self.assertEqual(result['probe']['pgid'], result['child'])
        self.assertTrue(result.get('payload_stopped') or result.get('stop_signal') == signal.SIGTTIN)

    def private_tmux(self, early_kill):
        if not shutil.which('tmux'):
            self.skipTest('actual local tmux is unavailable')
        if not hasattr(os, 'pidfd_open') or not hasattr(signal, 'pidfd_send_signal'):
            self.skipTest('private tmux cleanup requires Linux pidfd APIs')
        try:
            prerequisite_fd = os.pidfd_open(os.getpid())
        except OSError as exc:
            self.skipTest('Linux pidfd_open unavailable: ' + str(exc))
        else:
            try:
                signal.pidfd_send_signal(prerequisite_fd, 0)
            except OSError as exc:
                self.skipTest('Linux pidfd_send_signal unavailable: ' + str(exc))
            finally:
                os.close(prerequisite_fd)
        import shlex
        directory = self.fixture / ('tmux-kill' if early_kill else 'tmux-natural')
        directory.mkdir()
        socket = directory / 'server.sock'
        marker, release, output = (directory / name for name in ('ready.json', 'release', 'result'))
        payload = directory / 'payload.py'
        payload.write_text('''import os, json, pathlib, sys, time
marker, release, output = map(pathlib.Path, sys.argv[1:])
try:
    os.open('/dev/tty', os.O_RDONLY)
except OSError as exc:
    tty_error = exc.errno
else:
    tty_error = None
marker_tmp = marker.with_suffix('.tmp')
marker_tmp.write_text(json.dumps(dict(pid=os.getpid(), sid=os.getsid(0), pgid=os.getpgrp(),
    start=pathlib.Path('/proc/self/stat').read_text().rsplit(')', 1)[1].split()[19],
    tty_error=tty_error)))
marker_tmp.replace(marker)
deadline=time.monotonic()+5
while not release.exists():
    if time.monotonic()>deadline: raise SystemExit(99)
    time.sleep(0.01)
output.write_text('EXIT 23\\n')
raise SystemExit(23)
''')
        cmd = ['tmux', '-f', '/dev/null', '-S', str(socket)]
        def tmux(*args, check=True):
            return subprocess.run([*cmd, *args], capture_output=True, text=True,
                                  timeout=2, check=check, env=dict(os.environ, TMUX=''))
        def wait_for(predicate, label, seconds=2):
            deadline = time.monotonic() + seconds
            while not predicate():
                if time.monotonic() > deadline:
                    self.fail('private tmux deadline: ' + label)
                time.sleep(0.01)
        def alive(info):
            try:
                fields = Path(f"/proc/{info['pid']}/stat").read_text().rsplit(')', 1)[1].split()
                return fields[19] == info['start'] and fields[0] != 'Z'
            except FileNotFoundError:
                return False
        info = None
        pidfd = None
        pidfd_verified = False
        try:
            candidate = 'setsid --wait '+shlex.join([sys.executable, str(payload), str(marker), str(release), str(output)])+' </dev/null'
            tmux('new-session', '-d', '-s', 'owned', candidate)
            wait_for(marker.exists, 'payload ready handshake')
            info = json.loads(marker.read_text())
            # Pin a stable kernel process handle BEFORE checking birth identity.
            # Never signal a numeric saved PID, including on failure paths.
            pidfd = os.pidfd_open(info['pid'])
            pidfd_verified = alive(info)
            self.assertTrue(pidfd_verified, 'ready payload birth identity changed')
            self.assertEqual(info['tty_error'], errno.ENXIO)
            self.assertEqual(info['sid'], info['pid'])
            self.assertEqual(info['pgid'], info['pid'])
            self.assertTrue(alive(info))
            self.assertEqual(tmux('has-session', '-t', 'owned', check=False).returncode, 0)
            if early_kill:
                tmux('kill-session', '-t', 'owned')
                time.sleep(0.25)
                leak = alive(info)
                print('PRIVATE_TMUX_EARLY_KILL '+json.dumps(dict(info, detached_alive=leak)), flush=True)
                self.assertTrue(leak, 'diagnostic expects rejected plain setsid candidate to leak')
            else:
                # Session remains present until payload release and real completion.
                release.touch()
                wait_for(output.exists, 'natural payload completion')
                wait_for(lambda: tmux('has-session', '-t', 'owned', check=False).returncode != 0,
                         'natural session exit')
                self.assertEqual(output.read_text(), 'EXIT 23\n')
                print('PRIVATE_TMUX_NATURAL_EXIT '+json.dumps(info), flush=True)
        finally:
            cleanup = dict(pid=info['pid'] if info else None,
                           pidfd_opened=pidfd is not None,
                           identity_verified=pidfd_verified,
                           pidfd_send_signal=False, remaining_alive=None,
                           fdclosed=False)
            try:
                if pidfd is not None and pidfd_verified:
                    if not select.select([pidfd], [], [], 0)[0]:
                        cleanup['pidfd_send_signal'] = True
                        try:
                            signal.pidfd_send_signal(pidfd, signal.SIGKILL)
                        except ProcessLookupError:
                            pass
                    wait_for(lambda: bool(select.select([pidfd], [], [], 0)[0]),
                             'stable pidfd owned payload cleanup')
                    cleanup['remaining_alive'] = False
            finally:
                if pidfd is not None:
                    os.close(pidfd)
                    cleanup['fdclosed'] = True
                print('PRIVATE_TMUX_CLEANUP '+json.dumps(cleanup), flush=True)
                # Before-ready failures rely on the payload's bounded 5s
                # selfdeadline plus ONLY this private server's teardown.
                tmux('kill-server', check=False)

    def test_private_tmux_natural_session_lifetime(self):
        self.private_tmux(False)

    def test_diagnostic_rejected_setsid_tmux_kill_leaks(self):
        self.private_tmux(True)

    def public_start(self, label, status=None):
        work = self.fixture / label
        (work / 'bench').mkdir(parents=True)
        (work / 'bin').mkdir()
        home = work / 'home'
        home.mkdir()
        shutil.copyfile(self.source_file, work / 'bench/on-remote.sh')
        (work / 'bench/suite.py').write_text('SNAPSHOTS = {}\n')
        fake = r'''#!/usr/bin/env python3
import os, sys, pathlib, subprocess, shlex
name = pathlib.Path(sys.argv[0]).name
home = pathlib.Path(os.environ['HOME'])
a = sys.argv[1:]
if name == 'jr':
    assert a == ['remote', 'env'], a
    print('JR_REMOTE_KIND=fixture JR_REMOTE_DEST=local JR_REMOTE_SSH=fake-ssh')
elif name == 'git':
    assert a in [['rev-parse', '--short=12', 'HEAD'], ['status', '--porcelain', '--untracked-files=no']], a
    if a[0] == 'rev-parse': print('0123456789ab')
elif name == 'rsync':
    if a[-1].endswith('bench-bin/.justrust.upload'):
        (home/'bench-bin/.justrust.upload').write_text('#!/bin/sh\nexit 0\n')
    elif a[-1].endswith('bench-justrust/'):
        (home/'bench-justrust').mkdir(exist_ok=True)
    else: raise RuntimeError(a)
elif name == 'tmux':
    if a[0] == 'has-session': sys.exit(1)
    assert a[:3] == ['new-session', '-d', '-s'], a
    (home/'tmux-command').write_text(a[-1])
elif name == 'fake-ssh':
    assert a[0] == 'local', a
    command = a[1]
    if command.startswith('test -f /var/lib/justrust/ready'): sys.exit(0)
    if command == 'cat /proc/sys/kernel/perf_event_paranoid': print(1)
    elif command.startswith('if [ -d /var/lib/justrust ]'): (home/'.jcode-build-active').touch()
    elif command.startswith(('tmux ', 'mkdir ', 'chmod ', 'cat >~/bench-')):
        sys.exit(subprocess.call(['bash', '-c', command], cwd=home))
    else: raise RuntimeError('refusing unknown fake SSH command: '+command)
else: raise RuntimeError(name)
'''
        for name in ('jr', 'git', 'rsync', 'tmux', 'fake-ssh'):
            file = work / 'bin' / name
            file.write_text(fake)
            file.chmod(0o755)
        payload = home / 'payload.py'
        payload.write_text("import os, errno, time\ntry:\n fd=os.open('/dev/tty', os.O_RDONLY)\nexcept OSError as e:\n assert e.errno == errno.ENXIO\n print('HEADLESS', flush=True)\n time.sleep(0.15)\n raise SystemExit(23)\nelse:\n os.close(fd)\n print('HAS_CONTROLLING_TTY', flush=True)\n raise SystemExit(87)\n")
        if status is not None:
            if status == 143:
                payload.write_text("import os, signal\nos.kill(os.getpid(), signal.SIGTERM)\n")
            else:
                payload.write_text(f"raise SystemExit({status})\n")
        env = dict(os.environ, HOME=str(home), PATH=str(work/'bin')+':'+os.environ['PATH'],
                   JUSTRUST=str(work/'bin/jr'), TMPDIR=str(work))
        start = subprocess.run(['bash', str(work/'bench/on-remote.sh'), 'start', '--name', 'fixture',
                                '--', sys.executable, str(payload)], env=env,
                               capture_output=True, text=True, timeout=LIMIT)
        self.assertEqual(start.returncode, 0, start.stderr)
        command = (home/'tmux-command').read_text()
        print('CAPTURED_TMUX_COMMAND ' + command, flush=True)
        # Export fake home/path to the private PTY supervisor only.
        import shlex
        command = 'export HOME='+shlex.quote(str(home))+' PATH='+shlex.quote(env['PATH'])+'; '+command
        result = self.run_owned('command', command=command)
        log = (home/'bench-fixture.log').read_text()
        print('GENERATED_LOG ' + repr(log), flush=True)
        return result, log

    def test_diagnostic_public_start_still_has_tty(self):
        result, log = self.public_start('public-tty-diagnostic')
        self.assertIn('HAS_CONTROLLING_TTY\n', log, 'headless default was not promoted')
        self.assertEqual(result['exit'], 87)

    def check_public_status(self, status):
        result, log = self.public_start(f'public-status-{status}', status=status)
        self.assertIn(f'EXIT {status}\n', log, 'generated script must retain actual command status')
        self.assertEqual(result['exit'], status, 'date/echo must not overwrite command exit status')

    def test_public_start_preserves_success0(self):
        self.check_public_status(0)

    def test_public_start_preserves_failure23(self):
        self.check_public_status(23)

    def test_public_start_preserves_signal143(self):
        self.check_public_status(143)

    def test_terminal_failure_is_not_suppressed(self):
        import shlex
        command = shlex.join(['setsid', '--wait', sys.executable, '-c',
                             "import os; os.open('/dev/tty', os.O_RDONLY)"])
        result = self.run_owned('command', command=command)
        self.assertEqual(result['exit'], 1)
        self.assertIn('Traceback', result['stderr'])
        self.assertIn('[Errno 6]', result['stderr'])


if __name__ == '__main__':
    if len(sys.argv) > 1 and sys.argv[1] == '--probe':
        probe()
    elif len(sys.argv) > 1 and sys.argv[1] == '--supervise':
        supervisor(json.loads(sys.argv[2]))
    else:
        unittest.main()
