"""Pure archive tests. No uploads, credentials, or native builds."""
import base64
import hashlib
import json
import os
import subprocess
import sys
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import record_archive as archive


class ArchiveTests(unittest.TestCase):
    def setUp(self):
        self.original = b'  {"receipt": [1, 2], "unicode": "\xc3\xa9"}\r\n\x00'
        self.digest = hashlib.sha256(self.original).hexdigest()
        self.envelope = archive.encode(self.original)

    def test_roundtrip_preserves_all_bytes_not_json_normalization(self):
        self.assertEqual(archive.decode(self.envelope, self.digest), self.original)
        self.assertEqual(self.envelope, archive.encode(self.original))
        self.assertEqual(set(self.envelope), archive.FIELDS)

    def test_empty_roundtrip(self):
        raw = b""
        self.assertEqual(archive.decode(archive.encode(raw), hashlib.sha256(raw).hexdigest()), raw)

    def test_large_receipt_with_existing_backend_cap(self):
        raw = b'{"unchanged":1}\n' * 200000
        envelope = archive.encode(raw)
        self.assertGreater(len(raw), 1_980_000)
        self.assertLess(len(archive._json(envelope)), 1_980_000)
        self.assertEqual(archive.decode(envelope, hashlib.sha256(raw).hexdigest()), raw)

    def test_invalid_schema_and_bounds(self):
        changes = [{"schema": "v2"}, {"encoding": "identity"},
                   {"extra": 1}, {"original_bytes": True}, {"original_bytes": -1},
                   {"original_bytes": archive.MAX_ORIGINAL_BYTES + 1},
                   {"compressed_bytes": archive.MAX_COMPRESSED_BYTES + 1},
                   {"compressed_bytes": 0}, {"payload": "!"},
                   {"original_sha256": "0" * 64}, {"original_sha256": self.digest.upper()}]
        for change in changes:
            with self.subTest(change=change), self.assertRaises(ValueError):
                archive.decode({**self.envelope, **change}, self.digest)

    def test_pinned_digest_is_mandatory(self):
        for digest in (None, "", "0" * 64):
            with self.subTest(digest=digest), self.assertRaises(ValueError):
                archive.decode(self.envelope, digest)

    def test_corruption_truncation_trailing_and_concatenated_members(self):
        compressed = base64.b64decode(self.envelope["payload"])
        for raw in (compressed[:-1], compressed + b"x", compressed * 2,
                    compressed[:-8] + b"\0" * 8):
            envelope = {**self.envelope, "payload": base64.b64encode(raw).decode(),
                        "compressed_bytes": len(raw)}
            with self.subTest(raw=raw), self.assertRaises(ValueError):
                archive.decode(envelope, self.digest)

    def test_decompression_bomb_cannot_exceed_declared_size(self):
        envelope = archive.encode(b"a" * 1000000)
        envelope["original_bytes"] = 1
        with self.assertRaises(ValueError):
            archive.decode(envelope, envelope["original_sha256"])

    def test_hash_checks_reconstructed_content(self):
        envelope = {**self.envelope, "original_sha256": "0" * 64}
        with self.assertRaisesRegex(ValueError, "reconstructed SHA256"):
            archive.decode(envelope, "0" * 64)

    def test_encoding_and_envelope_bounds(self):
        with patch.object(archive, "MAX_ORIGINAL_BYTES", 1), self.assertRaises(ValueError):
            archive.encode(self.original)
        with patch.object(archive, "MAX_COMPRESSED_BYTES", 100), self.assertRaises(ValueError):
            archive.encode(os.urandom(1000))
        with patch.object(archive, "MAX_ENVELOPE_BYTES", 10), self.assertRaises(ValueError):
            archive.encode(self.original)
        with patch.object(archive, "MAX_ENVELOPE_BYTES", 10), self.assertRaises(ValueError):
            archive.decode(self.envelope, self.digest)

    def test_authenticated_fetch_identity_and_duplicate_fields(self):
        good = {"kind": "bench", "id": "receipt", "data": self.envelope}
        with patch.object(archive, "_command", return_value=json.dumps(good).encode()) as command:
            self.assertEqual(archive.fetch("receipt", self.digest, "client"), self.original)
            command.assert_called_once_with("client", ["show", "bench", "receipt"], archive.MAX_RESPONSE_BYTES)
        for raw in (json.dumps({**good, "id": "other"}).encode(),
                    b'{"id":"a","id":"b"}'):
            with patch.object(archive, "_command", return_value=raw), self.assertRaises(ValueError):
                archive.fetch("receipt", self.digest)

    def test_upload_requires_authenticated_readback_and_preserves_source(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "original.json"
            source.write_bytes(self.original)
            stored = {}

            def command(binary, args, limit):
                if args[0] == "put":
                    stored["data"] = json.loads(Path(args[-1]).read_bytes())
                    return b"queued for retry"
                return json.dumps({"kind": "bench", "id": "receipt", **stored}).encode()

            with patch.object(archive, "_command", side_effect=command) as called:
                result = archive.upload(source, "receipt")
            self.assertEqual(called.call_count, 2)
            self.assertTrue(result["verified_authenticated_reconstruction"])
            self.assertEqual(result["original_sha256"], self.digest)
            self.assertEqual(source.read_bytes(), self.original)
            with patch.object(archive, "_command", return_value=b"{}"), self.assertRaises(ValueError):
                archive.upload(source, "receipt")

    def test_output_is_exclusive_and_only_after_verification(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "receipt"
            output.write_bytes(b"keep")
            with patch.object(archive, "fetch", return_value=self.original), self.assertRaises(FileExistsError):
                archive.main(["get", "receipt", str(output), "--sha256", self.digest])
            self.assertEqual(output.read_bytes(), b"keep")
            output.unlink()
            with patch.object(archive, "fetch", side_effect=ValueError("bad hash")), self.assertRaises(ValueError):
                archive.main(["get", "receipt", str(output), "--sha256", self.digest])
            self.assertFalse(output.exists())

    def test_command_output_bound_and_exit_status_without_network(self):
        real_popen = subprocess.Popen
        for script, limit, expected in (("print('ok')", 3, b"ok\n"),
                                        ("print('too long')", 3, None),
                                        ("raise SystemExit(2)", 100, None)):
            def spawn(*args, **kwargs):
                return real_popen([sys.executable, "-c", script], **kwargs)
            with self.subTest(script=script), patch.object(archive.subprocess, "Popen", side_effect=spawn):
                if expected is None:
                    with self.assertRaises(ValueError):
                        archive._command("unused", [], limit)
                else:
                    self.assertEqual(archive._command("unused", [], limit), expected)

    def test_command_timeout_kills_child_without_network(self):
        real_popen = subprocess.Popen
        children = []

        def spawn(*args, **kwargs):
            child = real_popen([sys.executable, "-c", "import time; time.sleep(30)"], **kwargs)
            children.append(child)
            return child

        with patch.object(archive.subprocess, "Popen", side_effect=spawn), \
                patch.object(archive.selectors, "DefaultSelector") as selector:
            selector.return_value.__enter__.return_value.select.return_value = []
            with self.assertRaisesRegex(ValueError, "timeout"):
                archive._command("unused", [], 100)
        self.assertIsNotNone(children[0].poll())

    def test_bounded_input_and_invalid_ids(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "input"
            path.write_bytes(b"abc")
            with self.assertRaises(ValueError):
                archive.read_bounded(path, 2)
        for record_id in ("../receipt", "-invalid", "a" * 129):
            with self.subTest(record_id=record_id), self.assertRaises(ValueError):
                archive.fetch(record_id, self.digest)


if __name__ == "__main__":
    unittest.main()
