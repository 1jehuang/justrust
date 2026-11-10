#!/usr/bin/env python3
"""Real-binary remote status push acceptance, using isolated loopback services.

Run: python3 tests/remote-status-events.py [--binary target/release/justrust]
No account credentials, cloud traffic, builds, or third-party Python modules.
"""
import argparse
import base64
import hashlib
import http.server
import json
import os
from pathlib import Path
import queue
import select
import socket
import struct
import subprocess
import tempfile
import threading
import time


def wait_for(predicate, label, timeout=5):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if predicate():
            return
        time.sleep(0.02)
    raise AssertionError(f"timed out: {label}")


class Fleet(http.server.ThreadingHTTPServer):
    daemon_threads = True

    def __init__(self):
        super().__init__(("127.0.0.1", 0), Handler)
        self.lock = threading.Lock()
        self.requests = []
        self.connections = []
        self.used = 1
        self.state = "stopped"
        self.errors = queue.Queue()

    def snapshot(self):
        with self.lock:
            return len(self.requests), len(self.connections)

    def hint(self):
        with self.lock:
            peer = self.connections[-1]
        body = b'{"type":"status_changed"}'
        peer.sendall(bytes([0x81, len(body)]) + body)

    def disconnect(self):
        with self.lock:
            peer = self.connections[-1]
        peer.shutdown(socket.SHUT_RDWR)


class Handler(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *_):
        pass

    def do_GET(self):
        fleet = self.server
        if self.headers.get("Authorization") != "Bearer acceptance-only":
            fleet.errors.put("missing or wrong Authorization header")
            self.send_error(401)
            return
        if self.path == "/v1/build/events":
            key = self.headers.get("Sec-WebSocket-Key", "")
            digest = hashlib.sha1((key + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11").encode()).digest()
            self.send_response(101)
            self.send_header("Upgrade", "websocket")
            self.send_header("Connection", "Upgrade")
            self.send_header("Sec-WebSocket-Accept", base64.b64encode(digest).decode())
            self.end_headers()
            with fleet.lock:
                fleet.connections.append(self.connection)
            try:
                while self.connection.recv(1024):
                    pass
            except OSError:
                pass
            return
        with fleet.lock:
            fleet.requests.append(self.path)
            builds = {"included": 100, "included_used": fleet.used, "can_build": True}
            if self.path == "/v1/me":
                body = {"email": "acceptance@example.invalid", "build_hosts": True, "builds": builds}
            elif self.path == "/v1/build/host":
                body = {"state": fleet.state, "builds": builds, "credits": {}}
            else:
                fleet.errors.put(f"unexpected request: {self.path}")
                self.send_error(404)
                return
        raw = json.dumps(body).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(raw)))
        self.end_headers()
        self.wfile.write(raw)


class Daemon:
    def __init__(self, path):
        self.path = path
        self.pushed = 0
        self.sock = socket.socket(socket.AF_UNIX)
        self.sock.bind(str(path))
        self.sock.listen()
        self.sock.settimeout(0.1)
        self.stop = threading.Event()
        self.thread = threading.Thread(target=self.serve, daemon=True)
        self.thread.start()

    @staticmethod
    def read_exact(peer, size):
        data = b""
        while len(data) < size:
            part = peer.recv(size - len(data))
            if not part:
                raise EOFError()
            data += part
        return data

    def serve(self):
        while not self.stop.is_set():
            try:
                peer, _ = self.sock.accept()
            except socket.timeout:
                continue
            except OSError:
                break
            with peer:
                peer.settimeout(1)
                try:
                    length = struct.unpack(">I", self.read_exact(peer, 4))[0]
                    request = json.loads(self.read_exact(peer, length))
                    length = struct.unpack(">I", self.read_exact(peer, 4))[0]
                    self.read_exact(peer, length)
                    assert request["t"] == "Ping", request
                    body = json.dumps({"t": "Pong", "roots": ["/acceptance"],
                                       "machine": "acceptance", "pushed": self.pushed,
                                       "connected": True, "build": "acceptance",
                                       "active": 0, "skipped": 0}).encode()
                    peer.sendall(struct.pack(">I", len(body)) + body + struct.pack(">I", 0))
                except (EOFError, OSError):
                    pass

    def close(self):
        self.stop.set()
        self.sock.close()
        self.thread.join(timeout=2)
        self.path.unlink(missing_ok=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=Path("target/release/justrust"))
    parser.add_argument("--idle-seconds", type=float, default=32,
                        help="idle window, default crosses the 30-second hosted cache TTL")
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    fleet = Fleet()
    thread = threading.Thread(target=fleet.serve_forever, daemon=True)
    thread.start()
    proc = None
    daemon = None
    with tempfile.TemporaryDirectory(prefix="justrust-status-events-") as temporary:
        root = Path(temporary)
        remote = root / "justrust" / "remote"
        remote.mkdir(parents=True)
        (remote / "backend.json").write_text('{"kind":"hosted"}')
        env = os.environ.copy()
        # Never inherit credentials, SSH state, or endpoint overrides.
        for key in list(env):
            if key.startswith(("JUSTRUST_", "JCODE_")):
                env.pop(key)
        base = f"http://127.0.0.1:{fleet.server_port}/v1"
        env.update(HOME=str(root), XDG_CONFIG_HOME=str(root / "config"),
                   JUSTRUST_HOME=str(root / "justrust"), JCODE_HOME=str(root / "jcode"),
                   JCODE_API_KEY="acceptance-only", JCODE_API_BASE=base,
                   JUSTRUST_BUILD_BASE=base, JUSTRUST_RECORDS="off",
                   NO_PROXY="127.0.0.1,localhost", no_proxy="127.0.0.1,localhost")
        output = []
        output_lock = threading.Lock()

        def has(text):
            with output_lock:
                return any(text in row.get("text", "") or text in row.get("tooltip", "") for row in output)

        try:
            proc = subprocess.Popen([str(binary), "remote", "status", "--waybar", "--watch", "1"],
                                    env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                    text=True)

            reader_stop = threading.Event()

            def collect():
                pending = b""
                fd = proc.stdout.fileno()
                while not reader_stop.is_set():
                    if not select.select([fd], [], [], 0.1)[0]:
                        continue
                    chunk = os.read(fd, 65536)
                    if not chunk:
                        return
                    pending += chunk
                    while b"\n" in pending:
                        line, pending = pending.split(b"\n", 1)
                        with output_lock:
                            output.append(json.loads(line))

            reader = threading.Thread(target=collect, daemon=True)
            reader.start()
            wait_for(lambda: fleet.snapshot()[1] >= 1 and has("99b"), "initial authenticated snapshot")
            time.sleep(0.5)  # Drain the initial ONLINE refresh debounce.
            before = fleet.snapshot()[0]
            time.sleep(max(3.2, args.idle_seconds))
            assert fleet.snapshot()[0] == before, "idle subscription polled hosted API"
            print(f"PASS idle: no API polling for {max(3.2, args.idle_seconds):g}s")

            with fleet.lock:
                fleet.used = 7
            started = time.monotonic()
            fleet.hint()
            wait_for(lambda: has("93b"), "push invalidates fresh status cache", timeout=2)
            print(f"PASS hosted change: {time.monotonic() - started:.3f}s")

            connections = fleet.snapshot()[1]
            with fleet.lock:
                fleet.used = 12
            fleet.disconnect()
            wait_for(lambda: fleet.snapshot()[1] > connections and has("88b"),
                     "reconnect takes fresh snapshot without a change hint", timeout=5)
            print("PASS reconnect: fresh snapshot without server replay")

            local_requests = fleet.snapshot()[0]
            daemon = Daemon(remote / "daemon.sock")
            wait_for(lambda: has("sync 1r 0↑"), "daemon socket creation")
            started = time.monotonic()
            daemon.pushed = 42
            (remote / "daemon-status").write_text("1\n")
            wait_for(lambda: has("sync 1r 42↑"), "daemon CLOSE_WRITE hint", timeout=2)
            print(f"PASS local daemon hint: {time.monotonic() - started:.3f}s")
            daemon.close()
            daemon = None
            wait_for(lambda: "sync" not in output[-1]["text"], "daemon socket deletion")
            assert fleet.snapshot()[0] == local_requests, "local hints refreshed hosted APIs"
            print("PASS daemon socket lifecycle")

            assert fleet.errors.empty(), list(fleet.errors.queue)
            # Closing the reader must terminate even when no further status changes.
            reader_stop.set()
            reader.join(timeout=2)
            assert not reader.is_alive(), "output reader did not stop"
            proc.stdout.close()
            proc.wait(timeout=3)
            assert proc.returncode == 0, proc.stderr.read()
            print("PASS orphan exit: idle watcher stops when stdout closes")
        finally:
            if daemon:
                daemon.close()
            if proc and proc.poll() is None:
                proc.terminate()
                try:
                    proc.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    proc.kill()
                    proc.wait()
            fleet.shutdown()
            fleet.server_close()


if __name__ == "__main__":
    main()
