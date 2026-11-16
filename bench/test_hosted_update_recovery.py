#!/usr/bin/env python3
"""Exercise the real client executable against isolated API/daemon transports.
No compiler runs and no live tenant is interrupted. Set JUSTRUST_ACCEPT_BIN to
an executable built on hosted. Covers actual framing, admission, replay, final
exit status, generic failures, stale receipts, and bounded retries. For a
retained alpha28 executable, set JUSTRUST_ACCEPT_DAEMON_IDENTITY=legacy-stat
explicitly to model its historical daemon handshake. Default remains SHA256.
"""
import hashlib
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
            attempts, charges, events, requests, admissions, event_paths = [], [], [], [], [], []
            started = int(time.time())
            class API(http.server.BaseHTTPRequestHandler):
                def log_message(self, *args):
                    pass
                def reply(self, data, status=200):
                    value = json.dumps(data).encode()
                    self.send_response(status)
                    self.send_header('Content-Length', str(len(value)))
                    self.end_headers()
                    self.wfile.write(value)
                def do_GET(self):
                    requests.append(self.path)
                    update = None if mode == 'generic' else {
                        'id': 'acceptance-update', 'reason': 'isolated update acceptance',
                        'state': 'failed' if mode == 'failed' else 'completed',
                        'started_at': started - 5,
                        'completed_at': started - 100 if mode == 'stale' else int(time.time()) + 1,
                        'interrupted_work_stopped': mode != 'failed',
                    }
                    self.reply({'update': update})
                def do_POST(self):
                    data = json.loads(self.rfile.read(int(self.headers.get('Content-Length', 0))) or '{}')
                    if self.path == '/v1/build/runs':
                        admissions.append(data)
                        if mode == 'waiting' and len(admissions) == 1:
                            self.reply({'admission': 'maintenance-v1',
                                        'error': {'code': 'host_updating'},
                                        'update': {'id': 'acceptance-update',
                                                   'reason': 'isolated update acceptance',
                                                   'state': 'announced',
                                                   'started_at': started - 5}}, status=409)
                            return
                        charges.append(data)
                        self.reply({'admission': 'maintenance-v1'})
                        return
                    elif self.path.endswith('/event'):
                        event_paths.append(self.path)
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
            # Default to current content identity. The explicit legacy mode is
            # only for testing already-running published clients during upgrades.
            # Freeze policy so host settings cannot launch a real daemon.
            identity_mode = os.environ.get('JUSTRUST_ACCEPT_DAEMON_IDENTITY', 'sha256')
            if identity_mode == 'sha256':
                binary_id = 'sha256-' + hashlib.sha256(Path(BIN).read_bytes()).hexdigest()
            elif identity_mode == 'legacy-stat':
                metadata = Path(BIN).stat()
                binary_id = f'{metadata.st_size}-{metadata.st_mtime_ns // 1_000_000_000}'
            else:
                raise ValueError('unknown acceptance daemon identity mode')
            build = binary_id + ':native=off:only=false:mirror-isolation=false'
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
                       JUSTRUST_HOSTED_NATIVE='off', JUSTRUST_HOSTED_NATIVE_ONLY='0',
                       JUSTRUST_HOSTED_MIRROR_ISOLATION='0',
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
            self.assertTrue(charges[0]['run_id'])
            self.assertTrue(attempts, result.stderr)
            for attempt in attempts:
                self.assertEqual(attempt['run_id'], charges[0]['run_id'])
            self.assertTrue(event_paths, result.stderr)
            self.assertEqual(set(event_paths), {f"/v1/build/runs/{charges[0]['run_id']}/event"})
            if mode in ('success', 'presync', 'waiting'):
                self.assertEqual(result.returncode, final_code, result.stderr)
                self.assertEqual(len(attempts), 1 if mode == 'waiting' else 2, result.stderr)
                if mode == 'waiting':
                    self.assertIn('waiting before admission', result.stderr)
                    self.assertEqual(len(admissions), 2)
                    self.assertEqual(admissions[0], admissions[1])
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
