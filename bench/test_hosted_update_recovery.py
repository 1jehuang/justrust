#!/usr/bin/env python3
"""Exercise the real client executable against isolated API/daemon transports.
No compiler runs and no live tenant is interrupted. Set JUSTRUST_ACCEPT_BIN to
an executable built on hosted. Covers actual framing, admission, replay, final
exit status, generic failures, stale receipts, and bounded retries.
"""
import http.server
import json
import os
from pathlib import Path
import socket
import struct
import subprocess
import tempfile
import threading
import time
import unittest

BIN = os.environ.get('JUSTRUST_ACCEPT_BIN')


def recv_exact(sock, length):
    data = b''
    while len(data) < length:
        piece = sock.recv(length - len(data))
        if not piece:
            raise EOFError()
        data += piece
    return data


def receive(sock):
    header = json.loads(recv_exact(sock, struct.unpack('>I', recv_exact(sock, 4))[0]))
    payload = recv_exact(sock, struct.unpack('>I', recv_exact(sock, 4))[0])
    return header, payload


def send(sock, message, payload=b''):
    data = json.dumps(message).encode()
    sock.sendall(struct.pack('>I', len(data)) + data + struct.pack('>I', len(payload)) + payload)


@unittest.skipUnless(BIN, 'set JUSTRUST_ACCEPT_BIN to a hosted-built client')
class HostedUpdateAcceptance(unittest.TestCase):
    def scenario(self, mode, final_code=0):
        with tempfile.TemporaryDirectory(prefix='jr-update-') as tmp:
            root = Path(tmp)
            home = root / 'home'
            remote = home / 'remote'
            remote.mkdir(parents=True)
            (remote / 'backend.json').write_text('{"kind":"hosted"}')
            project = root / 'project'
            (project / 'src').mkdir(parents=True)
            (project / 'Cargo.toml').write_text('[package]\nname="update_acceptance"\nversion="0.1.0"\nedition="2024"\n')
            (project / 'src/lib.rs').write_text('')
            attempts, charges, events, requests = [], [], [], []
            started = int(time.time())
            class API(http.server.BaseHTTPRequestHandler):
                def log_message(self, *args):
                    pass
                def reply(self, data):
                    value = json.dumps(data).encode()
                    self.send_response(200)
                    self.send_header('Content-Length', str(len(value)))
                    self.end_headers()
                    self.wfile.write(value)
                def do_GET(self):
                    requests.append(self.path)
                    update = None if mode == 'generic' else {
                        'id': 'acceptance-update', 'reason': 'isolated update acceptance',
                        'state': ('failed' if mode == 'failed' else
                                  'announced' if mode == 'waiting' and len(requests) == 1 else 'completed'),
                        'started_at': started - 5,
                        'completed_at': started - 100 if mode == 'stale' else int(time.time()) + 1,
                        'interrupted_work_stopped': mode != 'failed',
                    }
                    self.reply({'update': update})
                def do_POST(self):
                    data = json.loads(self.rfile.read(int(self.headers.get('Content-Length', 0))) or '{}')
                    if self.path == '/v1/build/runs':
                        charges.append(data)
                    elif self.path.endswith('/event'):
                        events.append(data)
                    self.reply({})
            api = http.server.ThreadingHTTPServer(('127.0.0.1', 0), API)
            api_thread = threading.Thread(target=api.serve_forever, daemon=True)
            api_thread.start()
            listener = socket.socket(socket.AF_UNIX)
            listener.bind(str(remote / 'daemon.sock'))
            listener.listen()
            listener.settimeout(0.2)
            stop = threading.Event()
            errors = []
            stat = os.stat(BIN)
            build = f'{stat.st_size}-{int(stat.st_mtime)}'
            def serve():
                while not stop.is_set():
                    try:
                        conn, _ = listener.accept()
                    except socket.timeout:
                        continue
                    except OSError:
                        return
                    try:
                        with conn:
                            msg, _ = receive(conn)
                            if msg['t'] != 'Ping':
                                raise AssertionError(msg)
                            send(conn, {'t': 'Pong', 'roots': [], 'machine': 'isolated-hosted',
                                        'pushed': 0, 'connected': True, 'build': build})
                            msg, _ = receive(conn)
                            if msg['t'] != 'RunSync':
                                raise AssertionError(msg)
                            attempts.append(msg)
                            if mode == 'presync' and len(attempts) == 1:
                                send(conn, {'t': 'Error', 'id': 0, 'msg': 'connection to the remote machine was lost'})
                                continue
                            send(conn, {'t': 'RunState', 'id': 0, 'may_have_started': True})
                            send(conn, {'t': 'Flushed', 'id': 0, 'files': 0, 'ms': 0, 'machine': 'isolated-hosted'})
                            send(conn, {'t': 'Out', 'id': 0, 'stream': 1}, f'attempt-{len(attempts)}\n'.encode())
                            if len(attempts) == 1 and mode != 'waiting' or mode == 'repeat':
                                send(conn, {'t': 'Error', 'id': 0, 'msg': 'connection to the remote machine was lost'})
                            else:
                                send(conn, {'t': 'Exit', 'id': 0, 'code': final_code}, b'{}')
                    except Exception as exc:
                        errors.append(str(exc))
            daemon = threading.Thread(target=serve, daemon=True)
            daemon.start()
            env = dict(os.environ, JUSTRUST_HOME=str(home), JCODE_API_KEY='isolated-acceptance-not-a-real-key',
                       JCODE_API_BASE=f'http://127.0.0.1:{api.server_port}/v1',
                       JUSTRUST_BUILD_BASE=f'http://127.0.0.1:{api.server_port}/v1',
                       JUSTRUST_NO_UPDATE='1', JUSTRUST_TELEMETRY='0', JUSTRUST_REMOTE_REQUIRED='1')
            try:
                result = subprocess.run([BIN, 'remote', 'check'], cwd=project, env=env,
                                        capture_output=True, text=True, timeout=30)
                time.sleep(0.2)
            finally:
                stop.set()
                listener.close()
                api.shutdown()
                api.server_close()
                daemon.join(timeout=2)
            self.assertEqual(errors, [])
            self.assertEqual(len(charges), 1, result.stderr)
            if mode in ('success', 'presync', 'waiting'):
                self.assertEqual(result.returncode, final_code, result.stderr)
                self.assertEqual(len(attempts), 1 if mode == 'waiting' else 2, result.stderr)
                if mode == 'waiting':
                    self.assertIn('waiting before starting work', result.stderr)
                else:
                    self.assertEqual(attempts[0]['run_id'], attempts[1]['run_id'])
                    self.assertIn('interrupted by planned hosted update', result.stderr)
                    self.assertIn('restarting the interrupted hosted command', result.stderr)
                    self.assertIn('attempt-2', result.stdout)
                terminal = [e for e in events if e.get('event') in ('done', 'abandoned')]
                self.assertEqual([e['event'] for e in terminal], ['done'], events)
                self.assertEqual(terminal[0]['exit_code'], final_code)
            else:
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(len(attempts), 2 if mode == 'repeat' else 1, result.stderr)
                if mode == 'generic':
                    self.assertNotIn('interrupted by planned hosted update', result.stderr)
                if mode == 'repeat':
                    self.assertIn('recovery limit reached', result.stderr)
            self.assertNotIn('building locally', result.stderr)
            return result

    def test_update_restarts_hosted_once_and_returns_success(self):
        self.scenario('success')

    def test_recovered_command_returns_actual_failure_exit(self):
        self.scenario('success', final_code=17)

    def test_unexplained_disconnect_is_not_an_update(self):
        self.scenario('generic')

    def test_stale_update_does_not_replay_work(self):
        self.scenario('stale')

    def test_failed_update_does_not_replay_work(self):
        self.scenario('failed')

    def test_same_update_cannot_trigger_unbounded_replays(self):
        self.scenario('repeat')

    def test_update_during_sync_retries_without_local_fallback(self):
        self.scenario('presync')

    def test_announced_update_waits_before_admission(self):
        self.scenario('waiting')


if __name__ == '__main__':
    unittest.main()
