#!/usr/bin/env python3
"""Spike: drive rust-analyzer over LSP on the pinned Desktop snapshot.

Measures: time until the workspace is loaded and the edited file analyzed,
then per-edit latency from didChange to native diagnostics for the file.
Not a benchmark of record. Usage: bench/ra_spike.py RA_BINARY [n]
"""
import json
import os
import subprocess
import sys
import threading
import time
from pathlib import Path

RA = sys.argv[1]
N = int(sys.argv[2]) if len(sys.argv) > 2 else 3
ROOT = next((Path.home() / ".justrust/bench/work/desktop").glob("*")) / "jcode-desktop"
FILE = ROOT / "crates/jcode-desktop-ui/src/fps_counter.rs"
URI = FILE.as_uri()

p = subprocess.Popen([RA], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=open("/tmp/ra-spike.log", "w"),
                     env={**os.environ, "RA_LOG": "warn"})
lock = threading.Lock()
msgs = []
cv = threading.Condition(lock)
t0 = time.monotonic()


def send(obj):
    b = json.dumps(obj).encode()
    p.stdin.write(b"Content-Length: %d\r\n\r\n" % len(b) + b)
    p.stdin.flush()


def reader():
    f = p.stdout
    while True:
        h = {}
        while True:
            line = f.readline()
            if not line:
                return
            line = line.strip()
            if not line:
                break
            k, v = line.decode().split(":", 1)
            h[k.lower()] = v.strip()
        body = json.loads(f.read(int(h["content-length"])))
        with cv:
            msgs.append((time.monotonic(), body))
            cv.notify_all()
        if body.get("method") in ("workspace/configuration",):
            send({"jsonrpc": "2.0", "id": body["id"], "result": [CFG]})
        elif "id" in body and "method" in body:
            send({"jsonrpc": "2.0", "id": body["id"], "result": None})


CFG = {
    "checkOnSave": False,
    "cargo": {"targetDir": True, "buildScripts": {"enable": True}},
    "procMacro": {"enable": True},
    "diagnostics": {"enable": True, "experimental": {"enable": False}},
    "cachePriming": {"enable": True},
}
threading.Thread(target=reader, daemon=True).start()
send({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
    "processId": os.getpid(), "rootUri": ROOT.as_uri(),
    "capabilities": {"textDocument": {"diagnostic": {"dynamicRegistration": False}, "publishDiagnostics": {}},
                     "window": {"workDoneProgress": True},
                     "experimental": {"serverStatusNotification": True}},
    "initializationOptions": CFG}})


def wait(pred, timeout=600):
    end = time.monotonic() + timeout
    with cv:
        while True:
            for i, (t, m) in enumerate(msgs):
                if pred(m):
                    del msgs[: i + 1]
                    return t, m
            left = end - time.monotonic()
            if left <= 0:
                raise TimeoutError
            cv.wait(left)


wait(lambda m: m.get("id") == 1)
send({"jsonrpc": "2.0", "method": "initialized", "params": {}})
text = FILE.read_text()
send({"jsonrpc": "2.0", "method": "textDocument/didOpen", "params": {"textDocument": {
    "uri": URI, "languageId": "rust", "version": 1, "text": text}}})
t, _ = wait(lambda m: m.get("method") == "experimental/serverStatus" and m["params"].get("quiescent"))
print(f"quiescent after {t - t0:.1f}s", flush=True)
rid = 100


def pull():
    global rid
    rid += 1
    my = rid
    s = time.monotonic()
    send({"jsonrpc": "2.0", "id": my, "method": "textDocument/diagnostic",
          "params": {"textDocument": {"uri": URI}}})
    t, m = wait(lambda m: m.get("id") == my)
    return t - s, m.get("result", {}).get("items", [])


d, items = pull()
print(f"first pull {d*1000:.0f}ms, {len(items)} items", flush=True)
ver = 1
for i in range(N):
    for bad in (True, False):
        ver += 1
        new = text.replace("    ) -> String {\n        if self",
                           f'    ) -> String {{\n        let _: u32 = "type error {i}";\n        if self') if bad else text
        s = time.monotonic()
        send({"jsonrpc": "2.0", "method": "textDocument/didChange", "params": {
            "textDocument": {"uri": URI, "version": ver}, "contentChanges": [{"text": new}]}})
        d, items = pull()
        errs = [x for x in items if x.get("severity") == 1]
        print(f"edit {i} bad={bad}: {(time.monotonic()-s)*1000:.0f}ms, {len(errs)} errors "
              f"{[(x.get('code'), x['message'][:60], x['range']['start']['line']) for x in errs]}", flush=True)
p.kill()
