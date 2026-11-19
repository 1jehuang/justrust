#!/usr/bin/env python3
"""Linux PTY smoke: atomic client replacement reloads the dashboard in place."""
import fcntl
import os
from pathlib import Path
import pty
import select
import shutil
import struct
import subprocess
import sys
import tempfile
import termios
import time


def wait_for(check, master):
    end = time.monotonic() + 20
    output = bytearray()
    while time.monotonic() < end:
        if select.select([master], [], [], .1)[0]:
            output.extend(os.read(master, 65536))
        if check(output):
            return
    raise AssertionError(f'dashboard timed out: {output[-500:]!r}')


with tempfile.TemporaryDirectory() as directory:
    root = Path(directory)
    exe = root / 'justrust'
    shutil.copy2(sys.argv[1], exe)
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 42, 120, 0, 0))
    before = termios.tcgetattr(slave)
    env = dict(os.environ, HOME=directory, JUSTRUST_NO_UPDATE='1', JUSTRUST_NO_TELEMETRY='1', TERM='xterm-256color')
    def own_terminal():
        os.setsid()
        fcntl.ioctl(0, termios.TIOCSCTTY, 0)

    with subprocess.Popen([exe, 'interactive', 'dashboard'], stdin=slave, stdout=slave,
                          stderr=slave, env=env, preexec_fn=own_terminal) as proc:
        try:
            wait_for(lambda out: b'Personal Dashboard' in out, master)
            os.write(master, b'/needle42\r')
            # Input is drawn as cell diffs. Only the first frame after exec
            # necessarily contains the entire search as contiguous text.
            candidate = root / 'replacement'
            shutil.copy2(exe, candidate)
            candidate.replace(exe)
            wait_for(lambda out: os.path.samefile(f'/proc/{proc.pid}/exe', exe)
                     and b'needle42' in out, master)
            os.write(master, b'q')
            assert proc.wait(timeout=10) == 0
            assert termios.tcgetattr(slave) == before, 'terminal mode not restored'
        finally:
            if proc.poll() is None:
                proc.kill()
            os.close(master)
            os.close(slave)
print('Dashboard reloaded in the same PTY and PID, restored its search, and quit cleanly.')
