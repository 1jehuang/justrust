import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location("dogfood", Path(__file__).with_name("production-dogfood.py"))
M = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(M)
TRIPLE = "x86_64-unknown-linux-musl"
TAG = "v0.1.0-alpha.24"
URL = f"{M.ASSETS}{TAG}/justrust-{TRIPLE}.tar.gz"
INFO = dict(tag=TAG, version=TAG[1:], enabled=True, url=URL, sha256_url=URL + ".sha256")


class ProductionDogfoodTests(unittest.TestCase):
    def test_service_selection_must_match_and_be_enabled(self):
        for overrides in ({}, {"enabled": False}, {"tag": "v0.1.0-alpha.23"},
                          {"url": "https://attacker.invalid/binary"}, {"version": "wrong"}):
            with self.subTest(overrides=overrides), patch.object(M, "fetch", return_value=json.dumps(INFO | overrides).encode()):
                if overrides:
                    with self.assertRaises(ValueError):
                        M.release_info(TAG, TRIPLE)
                else:
                    self.assertEqual(M.release_info(TAG, TRIPLE), INFO)

    def archive(self, kind=tarfile.REGTYPE, duplicate=False):
        out = io.BytesIO()
        with tarfile.open(fileobj=out, mode="w:gz") as bundle:
            for _ in range(2 if duplicate else 1):
                member = tarfile.TarInfo(f"justrust-{TRIPLE}/justrust")
                member.type = kind
                member.size = 6 if kind == tarfile.REGTYPE else 0
                member.linkname = "/etc/passwd" if kind == tarfile.SYMTYPE else ""
                bundle.addfile(member, io.BytesIO(b"binary") if member.size else None)
        return out.getvalue()

    def test_archive_checksum_and_member_validation(self):
        for kind, duplicate in ((tarfile.REGTYPE, False), (tarfile.SYMTYPE, False), (tarfile.REGTYPE, True)):
            data = self.archive(kind, duplicate)
            checksum = hashlib.sha256(data).hexdigest().encode()
            with patch.object(M, "fetch", side_effect=[checksum, data]):
                if kind == tarfile.REGTYPE and not duplicate:
                    self.assertEqual(M.verified_binary(INFO, TRIPLE), b"binary")
                else:
                    with self.assertRaises(ValueError):
                        M.verified_binary(INFO, TRIPLE)
        with patch.object(M, "fetch", side_effect=[b"0" * 64, self.archive()]):
            with self.assertRaisesRegex(ValueError, "checksum mismatch"):
                M.verified_binary(INFO, TRIPLE)

    def test_source_build_rejected_even_with_matching_version(self):
        with patch.object(M, "run", side_effect=[f"justrust {TAG[1:]}", "(built from source)"]):
            with self.assertRaisesRegex(ValueError, "release build"):
                M.validate_release(Path("/binary"), TAG[1:])

    def test_only_successful_hosted_receipt_accepted(self):
        row = dict(id="run-1", cwd="/smoke", args=["test", "--remote"], remote="hosted", exit=0)
        self.assertEqual(M.validate_run(json.dumps(row), Path("/smoke")), "run-1")
        for overrides in ({"remote": None}, {"exit": 1}, {"cwd": "/other"}, {"args": ["check"]}):
            with self.assertRaises(ValueError):
                M.validate_run(json.dumps(row | overrides), Path("/smoke"))

    def test_development_overrides_not_inherited(self):
        with patch.dict(os.environ, {"JUSTRUST_HOSTED_DEV": "1", "JUSTRUST_UPDATE_URL": "bad",
                                    "JUSTRUST_HOSTED_SESSION": "0", "JCODE_API_KEY": "test"}, clear=True):
            env = M.production_env()
        self.assertNotIn("JUSTRUST_HOSTED_DEV", env)
        self.assertNotIn("JUSTRUST_UPDATE_URL", env)
        self.assertEqual(env["JUSTRUST_HOSTED_SESSION"], "1")
        self.assertEqual(env["JCODE_API_KEY"], "test")

    def test_activation_preserves_backup_and_replaces_selected_path(self):
        with tempfile.TemporaryDirectory() as directory, patch.dict(os.environ, {"JUSTRUST_HOME": directory}):
            binary = Path(directory) / "justrust"
            binary.write_bytes(b"old")
            binary.chmod(0o755)
            backup = M.activate(binary, b"release")
            self.assertEqual(backup.read_bytes(), b"old")
            self.assertEqual(binary.read_bytes(), b"release")
            self.assertTrue(os.access(binary, os.X_OK))


if __name__ == "__main__":
    unittest.main()
