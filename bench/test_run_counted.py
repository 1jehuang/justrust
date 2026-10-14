"""Bounded real-exec coverage for run_counted's noninteractive boundary."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


# Run the fork/wait4 owner in a separate process. Its watchdog kills and reaps
# only its recorded child, never a process group or the test runner.
DRIVER = r'''
import json, os, signal, sys, time
from pathlib import Path
import versus
from unittest.mock import patch

child = None
real_fork = os.fork
real_open = os.open
env = dict(os.environ)
env['RUN_COUNTED_SENTINEL'] = 'inherited-context-unchanged'
env['RUN_COUNTED_DRIVER_PGID'] = str(os.getpgrp())

def fork():
    global child
    pid = real_fork()
    if pid:
        child = pid
    return pid

def open_fd(path, flags, *args):
    if str(path) == os.devnull:
        if sys.argv[3] == 'zero':
            os.close(0)
        fd = real_open(path, flags, *args)
        # Deliberately inheritable: exec's CLOEXEC must not mask a missing close.
        if fd != 0:
            os.set_inheritable(fd, True)
        env['NULL_FD'] = str(fd)
        return fd
    return real_open(path, flags, *args)

def expired(signum, frame):
    raise TimeoutError('run_counted exceeded 3 seconds')

signal.signal(signal.SIGALRM, expired)
signal.alarm(3)
try:
    with patch.object(versus, 'open_instruction_counters', return_value=[]), \
         patch.object(os, 'fork', fork), patch.object(os, 'open', open_fd):
        result = versus.run_counted(
            [sys.executable, '-c', sys.argv[2]], Path.cwd(), env, Path(sys.argv[1]))
    child = None  # wait4 already reaped it, so never signal a recycled PID.
    print(json.dumps(result), flush=True)
finally:
    signal.alarm(0)
    if child is not None:
        try:
            os.kill(child, signal.SIGKILL)
        except ProcessLookupError:
            pass
        deadline = time.monotonic() + 1
        while True:
            try:
                reaped, _ = os.waitpid(child, os.WNOHANG)
            except ChildProcessError:
                break
            if reaped:
                break
            if time.monotonic() >= deadline:
                raise TimeoutError('child did not exit after SIGKILL')
            time.sleep(0.01)
'''

PROBE = r'''
import errno, json, os, sys
fd = int(os.environ.get('NULL_FD', '-1'))
closed = None
if fd > 2:
    try:
        os.fstat(fd)
        closed = False
    except OSError as error:
        closed = error.errno == errno.EBADF
print(json.dumps({'isatty': os.isatty(0), 'data': os.read(0, 1).hex(),
                  'null_fd': fd, 'temporary_closed': closed,
                  'pgid': os.getpgrp(),
                  'driver_pgid': int(os.environ['RUN_COUNTED_DRIVER_PGID']),
                  'sentinel': os.environ.get('RUN_COUNTED_SENTINEL')}), flush=True)
print('stderr marker', file=sys.stderr, flush=True)
sys.exit(int(sys.argv[1]) if len(sys.argv) > 1 else 0)
'''


@unittest.skipUnless(hasattr(os, 'fork') and hasattr(os, 'wait4'), 'requires fork/wait4')
class RunCountedTests(unittest.TestCase):
    def run_helper(self, source=PROBE, *, terminal=False, zero=False):
        with tempfile.TemporaryDirectory() as directory:
            log = Path(directory) / 'child.log'
            master = slave = None
            if terminal:
                import pty

                # No setsid/TIOCSCTTY: this PTY cannot become the runner's
                # controlling terminal or stop it through background reads.
                master, slave = pty.openpty()
            process = subprocess.Popen(
                [sys.executable, '-c', DRIVER, str(log), source,
                 'zero' if zero else 'normal'],
                cwd=Path(__file__).resolve().parent,
                stdin=slave if terminal else subprocess.PIPE,
                stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            try:
                # Do not communicate(): it would close the open input pipe and
                # accidentally give the unfixed implementation EOF.
                process.wait(timeout=6)
                stdout = process.stdout.read()
                stderr = process.stderr.read()
                self.assertEqual(process.returncode, 0, stderr)
                return json.loads(stdout), log.read_text()
            finally:
                if process.poll() is None:
                    process.kill()
                    process.wait(timeout=2)
                for stream in (process.stdin, process.stdout, process.stderr):
                    if stream is not None:
                        stream.close()
                for fd in (master, slave):
                    if fd is not None:
                        os.close(fd)

    def assert_probe(self, text):
        probe = json.loads(text.splitlines()[0])
        self.assertFalse(probe['isatty'])
        self.assertEqual(probe['data'], '')
        self.assertIn('stderr marker', text)
        self.assertEqual(probe['pgid'], probe['driver_pgid'])
        self.assertEqual(probe['sentinel'], 'inherited-context-unchanged')
        return probe

    def test_open_pipe_becomes_empty_non_tty_stdin(self):
        result, text = self.run_helper()
        self.assertEqual(result['exit'], 0)
        self.assert_probe(text)
        self.assertEqual(set(result), {'wall', 'exit', 'cpu', 'max_rss_mb', 'instructions'})
        for key in ('wall', 'cpu', 'max_rss_mb'):
            self.assertGreaterEqual(result[key], 0)
        self.assertIsNone(result['instructions'])

    def test_pty_becomes_empty_non_tty_stdin(self):
        result, text = self.run_helper(terminal=True)
        self.assertEqual(result['exit'], 0)
        self.assert_probe(text)

    def test_temporary_null_descriptor_is_closed(self):
        _, text = self.run_helper()
        probe = self.assert_probe(text)
        self.assertGreater(probe['null_fd'], 2)
        self.assertTrue(probe['temporary_closed'])

    def test_null_open_returning_zero_keeps_stdin_open(self):
        _, text = self.run_helper(zero=True)
        self.assertEqual(self.assert_probe(text)['null_fd'], 0)

    def test_real_exec_failure_status_and_output_are_preserved(self):
        result, text = self.run_helper("import sys; print('failure marker', file=sys.stderr); sys.exit(23)")
        self.assertEqual(result['exit'], 23)
        self.assertIn('failure marker', text)

    def test_signal_exit_is_preserved(self):
        result, _ = self.run_helper('import os, signal; os.kill(os.getpid(), signal.SIGTERM)')
        self.assertEqual(result['exit'], -15)


if __name__ == '__main__':
    unittest.main()
