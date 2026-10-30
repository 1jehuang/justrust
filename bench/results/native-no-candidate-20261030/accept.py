"""Actual two-project daemon acceptance. Run only against a frozen candidate."""
import json
import os
from pathlib import Path
import socket
import struct
import subprocess
import sys
import time

base = Path(__file__).resolve().parent
binary = str(Path(sys.argv[1]).resolve())
expected_native = int(sys.argv[2]) if len(sys.argv) > 2 else 2
label = sys.argv[3] if len(sys.argv) > 3 else 'acceptance'
home = base / ('home-' + str(time.time_ns()))
home.mkdir()
(home / 'toolchains').symlink_to(Path.home() / '.justrust/toolchains', target_is_directory=True)
env = dict(os.environ, JUSTRUST_HOME=str(home), JUSTRUST_REMOTE='off',
           JUSTRUST_BUILDD='1', JUSTRUST_BUILDD_PREBUILD='0',
           JUSTRUST_BUILDD_NATIVE_EMPTY='0',
           JUSTRUST_BUILDD_NATIVE_NO_CANDIDATE='1',
           JUSTRUST_TOOLCHAIN='nightly-2026-10-08', JUSTRUST_NO_TELEMETRY='1')

def connect(kind, payload=b''):
    stream = socket.socket(socket.AF_UNIX)
    stream.settimeout(90)
    stream.connect(str(home / 'buildd/sock'))
    stream.sendall(kind + struct.pack('>I', len(payload)) + payload)
    return stream

def receive(stream):
    def exact(size):
        data = b''
        while len(data) < size:
            block = stream.recv(size - len(data))
            assert block, 'unexpected daemon EOF'
            data += block
        return data
    header = exact(5)
    return header[:1], exact(struct.unpack('>I', header[1:])[0])

def request(project, session):
    child_env = dict(env, JUSTRUST_SLOT_SESSION=session)
    return dict(build='', cwd=str(base / project), argv=['check', '--lib'],
                env=sorted(child_env.items()), prebuild=False)

subprocess.run([binary, 'daemon', 'start'], env=env, check=True)
owner = None
try:
    pidfile = base / 'blocker/script.pid'
    assert not pidfile.exists(), 'fixture already used, preserve evidence and use fresh fixture'
    owner = connect(b'R', json.dumps(request('blocker', 'accept-blocker')).encode())
    deadline = time.monotonic() + 45
    while not pidfile.exists() and time.monotonic() < deadline:
        time.sleep(.02)
    assert pidfile.exists(), 'actual sleeping build script never started'
    script_pid = int(pidfile.read_text())
    with connect(b'P') as ping:
        kind, body = receive(ping)
        assert kind == b'I'
        status_before = json.loads(body)
    assert status_before['running'] == 1, status_before
    start = time.monotonic()
    output = []
    with connect(b'R', json.dumps(request('probe', 'accept-probe')).encode()) as probe:
        while True:
            kind, body = receive(probe)
            if kind == b'X':
                result = json.loads(body)
                break
            assert kind != b'F', body.decode(errors='replace')
            output.append(body.decode(errors='replace'))
    wall_ms = (time.monotonic() - start) * 1000
    assert result['code'] == 0, (result, output)
    assert result['attached'] is False, result
    with connect(b'P') as ping:
        kind, body = receive(ping)
        status_after = json.loads(body)
    assert status_after['running'] >= 1, status_after
    assert Path(f'/proc/{script_pid}/stat').exists(), 'independent blocker was cancelled'
    assert status_after['coalesced'] == 0, status_after
    owner.close()
    owner = None
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        stat = Path(f'/proc/{script_pid}/stat')
        if not stat.exists() or stat.read_text().split()[2] == 'Z':
            break
        time.sleep(.02)
    else:
        raise AssertionError('blocker survived requesting socket disconnect')
    log = (home / 'buildd/daemon.log').read_text()
    assert log.count('stamp native') == expected_native, log
    evidence = dict(home=str(home), binary=binary, status_before=status_before,
                    status_after=status_after, probe_exit=result, probe_wall_ms=wall_ms,
                    blocker_cancelled=True, output=output)
    (base / (label + '-result.json')).write_text(json.dumps(evidence, indent=2) + '\n')
    (base / (label + '-daemon.log')).write_text(log)
    print(json.dumps(evidence, indent=2))
    print(log)
finally:
    if owner is not None:
        owner.close()
    subprocess.run([binary, 'daemon', 'stop'], env=env, check=False)
