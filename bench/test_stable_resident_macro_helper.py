#!/usr/bin/env python3
"""Synthetic protocol tests only. Real native acceptance is a separate root gate."""
import base64
import json
from pathlib import Path
import socket
import tempfile
import unittest
from unittest import mock

import stable_resident_macro_helper as helper

FAKE = r'''#!/usr/bin/env python3
import os, socket, struct, sys
try:
    fd = int(sys.argv[2])
    if fd < 3: raise ValueError()
    sock = socket.socket(fileno=fd)
except (ValueError, OSError):
    sys.stderr.buffer.write(b'jr-rustc proc-macro helper: invalid descriptor\n')
    sys.exit(1)
def exact(n):
    data = b''
    while len(data) < n:
        chunk = sock.recv(n-len(data))
        if not chunk: sys.exit(2)
        data += chunk
    return data
for seq in (1, 2, 3):
    magic, version, kind, reserved, n = struct.unpack('<8sHBBI', exact(16))
    payload = exact(n)
    if kind == 6: sys.exit(0)
    assert kind == 1 and struct.unpack('<Q', payload[:8])[0] == seq
    if seq < 3:
        assert payload[8] == 1
        reply = struct.pack('<QII', seq, 0, 1)
        kind = 4
    else:
        assert payload[8:] == b'\2' + struct.pack('<II', 0, 1) + b'\0'
        reply = b'unknown proc-macro library or client index'
        kind = 5
    sock.sendall(struct.pack('<8sHBBI', b'JRPMRPC\0', 1, kind, 0, len(reply)) + reply)
    if kind == 5:
        sys.stderr.buffer.write(b'jr-rustc proc-macro helper: unknown proc-macro library or client index\n')
        sys.exit(1)
'''


class HarnessTests(unittest.TestCase):
    def test_complete_capture_with_inherited_fd(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            compiler = root / 'fake-rustc'
            compiler.write_text(FAKE)
            compiler.chmod(0o700)
            library = root / 'macro.so'
            library.write_bytes(b'identity only, not loaded by synthetic helper')
            (root / 'out').mkdir()
            result = helper.capture(compiler, library, '__test_symbol', root / 'out', timeout=3)
            self.assertEqual(result['verdict'], 'passed')
            self.assertEqual(len(result['observations']), 9)
            loads = result['observations']['repeated-load']
            self.assertEqual([x['sequence'] for x in loads['loads']], [1, 2])
            self.assertEqual([x['library'] for x in loads['loads']], [0, 0])
            for name in ('authenticated-shutdown', 'repeated-load', 'invalid-client'):
                arm = result['observations'][name]
                for kind in ('stdout', 'stderr', 'sent', 'received'):
                    path = root / 'out' / name / (kind + '.bin')
                    self.assertEqual(base64.b64decode(arm[kind + '_b64']), path.read_bytes())
                    self.assertEqual(arm[kind + '_sha256'], helper.shared.digest(path))
            self.assertEqual(json.loads((root / 'out/receipt.json').read_text())['verdict'], 'passed')

    def test_failed_exchange_keeps_raw_evidence(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            compiler = root / 'bad-rustc'
            compiler.write_text(FAKE.replace("struct.pack('<QII', seq, 0, 1)", "struct.pack('<QII', 1, 0, 1)"))
            compiler.chmod(0o700)
            library = root / 'macro.so'
            library.write_bytes(b'x')
            (root / 'out').mkdir()
            with self.assertRaisesRegex(AssertionError, 'sequence'):
                helper.capture(compiler, library, 'symbol', root / 'out', timeout=3)
            report = json.loads((root / 'out/receipt.json').read_text())
            self.assertEqual(report['verdict'], 'failed')
            arm = report['observations']['repeated-load']
            self.assertGreater(len(base64.b64decode(arm['received_b64'])), 0)

    def test_frame_rejects_malformed_and_preserves_partial(self):
        for wire in (b'wrong!!!' + helper.frame(6)[8:], helper.frame(6)[:9]):
            left, right = socket.socketpair()
            try:
                right.sendall(wire)
                right.shutdown(socket.SHUT_WR)
                raw = bytearray()
                with self.assertRaises((ValueError, EOFError)):
                    helper.receive(left, raw)
                self.assertEqual(bytes(raw), wire)
            finally:
                left.close()
                right.close()

    def test_native_driver_identity_change_fails_acceptance(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            compiler = root / 'fake-rustc'
            compiler.write_text(FAKE)
            compiler.chmod(0o700)
            library = root / 'macro.so'
            library.write_bytes(b'synthetic fixture')
            output = root / 'out'
            output.mkdir()
            with mock.patch.object(helper.shared, 'identity', side_effect=[
                    {'drivers': [{'sha256': 'before'}]},
                    {'drivers': [{'sha256': 'changed'}]}]):
                with self.assertRaisesRegex(AssertionError, 'driver identity changed'):
                    helper.capture(compiler, library, 'symbol', output, timeout=3,
                                   native_identity=True)
            report = json.loads((output / 'receipt.json').read_text())
            self.assertEqual(report['verdict'], 'failed')
            self.assertNotEqual(report['compiler_before'], report['compiler_after'])

    def test_invalid_parameters(self):
        for timeout in (0, -1, float('nan'), float('inf')):
            with self.assertRaises(ValueError):
                helper.capture('missing', 'missing', 's', 'missing', timeout)
        for symbol in ('', 'bad\0symbol'):
            with self.assertRaises(ValueError):
                helper.encode_load('/macro.so', symbol)
        with self.assertRaises(ValueError):
            helper.frame(6, b'x')


if __name__ == '__main__':
    unittest.main()
