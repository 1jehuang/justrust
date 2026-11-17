#!/usr/bin/env python3
"""Inherited Unix FD helper lifecycle acceptance. Uses prebuilt inputs, never builds.

Mock tests are protocol checks, not native acceptance or performance evidence.
The supplied declaration symbol and dylib must come from this exact toolchain.
"""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import socket
import struct
import subprocess
import time

import stable_resident as shared

MAX_FRAME = 64 * 1024 * 1024
HEADER = struct.Struct('<8sHBBI')
FLAG = '--jr-proc-macro-helper'
DIAGNOSTIC = b'unknown proc-macro library or client index'


def require(condition, message):
    if not condition:
        raise AssertionError(message)


def frame(kind, payload=b''):
    if kind not in range(1, 7) or len(payload) > MAX_FRAME or (kind == 6 and payload):
        raise ValueError('invalid frame kind or length')
    return HEADER.pack(b'JRPMRPC\0', 1, kind, 0, len(payload)) + payload


def encode_load(path, symbol):
    parts = [str(path).encode('utf-8'), symbol.encode('utf-8')]
    if any(not part or b'\0' in part for part in parts):
        raise ValueError('empty or NUL-containing path or symbol')
    payload = b'\1' + b''.join(struct.pack('<I', len(part)) + part for part in parts)
    if len(payload) > MAX_FRAME - 8:
        raise ValueError('Load exceeds session frame limit')
    return payload


def read_exact(stream, count, evidence):
    result = bytearray()
    while len(result) < count:
        chunk = stream.recv(count - len(result))
        if not chunk:
            raise EOFError('helper disconnected before complete frame')
        evidence.extend(chunk)
        result.extend(chunk)
    return bytes(result)


def receive(stream, evidence):
    magic, version, kind, reserved, length = HEADER.unpack(read_exact(stream, 16, evidence))
    if (magic != b'JRPMRPC\0' or version != 1 or reserved or kind not in range(1, 7)
            or length > MAX_FRAME or (kind == 6 and length)):
        raise ValueError('invalid helper frame header')
    return kind, read_exact(stream, length, evidence)


def environment():
    # Native helper must be a fresh non-resident process. Preserve loader paths.
    return {key: value for key, value in os.environ.items()
            if not key.startswith(('JUSTRUST_RUSTC_RESIDENT', 'JUSTRUST_CARGO_RESIDENT'))}


class Helper:
    def __init__(self, rustc, root, timeout, row):
        self.root, self.timeout, self.row = Path(root), timeout, row
        self.sent, self.received = bytearray(), bytearray()
        self.stream, child = socket.socketpair(socket.AF_UNIX, socket.SOCK_STREAM)
        self.stream.settimeout(timeout)
        self.stdout = (self.root / 'stdout').open('wb')
        self.stderr = (self.root / 'stderr').open('wb')
        self.process = None
        try:
            command = [str(rustc), FLAG, str(child.fileno())]
            self.process = subprocess.Popen(command, pass_fds=(child.fileno(),),
                stdin=subprocess.DEVNULL, stdout=self.stdout, stderr=self.stderr,
                env=environment(), cwd=root)
            row.update(command=command, pid=self.process.pid, cwd=str(root))
        except BaseException:
            self.stream.close()
            self.stdout.close()
            self.stderr.close()
            raise
        finally:
            child.close()

    def send(self, kind, payload=b''):
        wire = frame(kind, payload)
        self.sent.extend(wire)
        self.stream.sendall(wire)  # Never retry an Invoke after attempted transmission.

    def exchange(self, sequence, payload):
        self.send(1, struct.pack('<Q', sequence) + payload)
        kind, result = receive(self.stream, self.received)
        if kind == 4:
            require(len(result) >= 8 and struct.unpack('<Q', result[:8])[0] == sequence,
                    'wrong completion sequence')
            return kind, result[8:]
        return kind, result

    def finish(self):
        try:
            try:
                self.process.wait(timeout=self.timeout)
            except subprocess.TimeoutExpired:
                self.row['timed_out'] = True
                self.process.kill()
                self.process.wait()
        finally:
            self.stream.close()
            self.stdout.close()
            self.stderr.close()
            self.row['exit_status'] = self.process.returncode
            for name, data in [('sent', bytes(self.sent)), ('received', bytes(self.received)),
                               ('stdout', (self.root / 'stdout').read_bytes()),
                               ('stderr', (self.root / 'stderr').read_bytes())]:
                (self.root / (name + '.bin')).write_bytes(data)
                self.row[name + '_b64'] = shared.encoded(data)
                self.row[name + '_sha256'] = hashlib.sha256(data).hexdigest()
            (self.root / 'lifecycle.json').write_text(json.dumps(self.row, indent=2) + '\n')


def capture(rustc, macro_library, symbol, root, timeout=30, native_identity=False):
    if not math.isfinite(timeout) or timeout <= 0:
        raise ValueError('timeout must be finite and positive')
    rustc, macro_library, root = map(lambda path: Path(path).resolve(),
                                    (rustc, macro_library, root))
    report = dict(schema='stable-resident-macro-helper/v1', verdict='incomplete',
                  claim='Helper load lifecycle correctness only, no expansion or speed claim.',
                  observations={}, symbol=symbol)
    started = time.monotonic()
    try:
        if native_identity:
            report['compiler_before'] = shared.identity(rustc)
        report['rustc_sha256_before'] = shared.digest(rustc)
        report['macro_library'] = str(macro_library)
        report['macro_library_sha256_before'] = shared.digest(macro_library)
        load = encode_load(macro_library, symbol)
        for descriptor in ('-1', '0', '1', '2', '2147483647', 'not-an-fd'):
            row = shared.run([rustc, FLAG, descriptor], timeout, cwd=root, env=environment())
            report['observations']['invalid-fd-' + descriptor] = row
            require(row['status'] == 1 and not row.get('timed_out'), 'invalid FD not rejected')
            require(b'jr-rustc proc-macro helper:' in __import__('base64').b64decode(row['stderr_b64']),
                    'missing invalid FD diagnostic')
        for name in ('authenticated-shutdown', 'repeated-load', 'invalid-client'):
            directory = root / name
            directory.mkdir(mode=0o700)
            row = report['observations'][name] = {}
            worker = Helper(rustc, directory, timeout, row)
            try:
                if name == 'authenticated-shutdown':
                    worker.send(6)
                else:
                    responses = row['loads'] = []
                    for sequence in (1, 2):
                        kind, result = worker.exchange(sequence, load)
                        require(kind == 4 and len(result) == 8, 'Load did not complete exactly')
                        index, count = struct.unpack('<II', result)
                        responses.append(dict(sequence=sequence, library=index, clients=count))
                        require(index == 0 and 0 < count <= 65536, 'invalid loaded library/count')
                    require(responses[0]['clients'] == responses[1]['clients'], 'repeated Load changed clients')
                    if name == 'repeated-load':
                        worker.send(6)
                    else:
                        invalid = b'\2' + struct.pack('<II', 0, count) + b'\0'
                        kind, result = worker.exchange(3, invalid)
                        row['failure_b64'] = shared.encoded(result)
                        require(kind == 5 and DIAGNOSTIC in result, 'invalid client did not fail')
                        # No replay or second Invoke. Failure must independently terminate.
                worker.process.wait(timeout=timeout)
                require(worker.stream.recv(1) == b'', 'bytes followed terminal exit')
            finally:
                worker.finish()
            require(not row.get('timed_out'), 'helper did not exit')
            require(row['exit_status'] == (1 if name == 'invalid-client' else 0),
                    'unexpected helper exit status')
        if native_identity:
            report['compiler_after'] = shared.identity(rustc)
            require(report['compiler_before'] == report['compiler_after'],
                    'native compiler/driver identity changed during acceptance')
        report['rustc_sha256_after'] = shared.digest(rustc)
        report['macro_library_sha256_after'] = shared.digest(macro_library)
        for key in ('rustc', 'macro_library'):
            require(report[key + '_sha256_before'] == report[key + '_sha256_after'],
                    key + ' changed during acceptance')
        report['verdict'] = 'passed'
        return report
    except BaseException as error:
        report.update(verdict='failed', error=f'{type(error).__name__}: {error}')
        raise
    finally:
        report['elapsed_seconds'] = time.monotonic() - started
        (root / 'receipt.json').write_text(json.dumps(report, indent=2) + '\n')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--rustc', required=True, type=Path)
    parser.add_argument('--macro-library', required=True, type=Path)
    parser.add_argument('--symbol', required=True)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--timeout', type=float, default=30)
    args = parser.parse_args()
    if not math.isfinite(args.timeout) or args.timeout <= 0:
        parser.error('timeout must be finite and positive')
    os.umask(0o077)
    args.output.mkdir(mode=0o700, parents=True, exist_ok=False)
    print('Private raw evidence: ' + str(args.output.resolve()), flush=True)
    try:
        report = capture(args.rustc, args.macro_library, args.symbol, args.output, args.timeout,
                         native_identity=True)
        print(json.dumps(dict(verdict=report['verdict'], claim=report['claim'])))
    finally:
        receipt = args.output.resolve() / 'receipt.json'
        if receipt.exists():
            try:
                result = subprocess.run(['justrust', 'records', 'put', 'bench',
                    args.output.resolve().name, str(receipt)], capture_output=True, text=True)
                print(result.stdout, end='')
                if result.returncode:
                    print('Record upload failed, private receipt retained: ' + result.stderr)
            except OSError as error:
                print('Record upload failed, private receipt retained: ' + str(error))


if __name__ == '__main__':
    main()
