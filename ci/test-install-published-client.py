#!/usr/bin/env python3
"""Offline stdlib tests for the pinned published bootstrap."""
import hashlib
import importlib.util
import io
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("installer", Path(__file__).with_name("install-published-client.py"))
installer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(installer)
TAG = "v0.1.0-alpha.28"
NAME = f"justrust-{installer.TRIPLE}/justrust"
REAL_FETCH = installer.fetch


def archive(entries=None):
    output = io.BytesIO()
    with tarfile.open(fileobj=output, mode="w:gz") as bundle:
        for name, kind, mode, data in entries or [(NAME, tarfile.REGTYPE, 0o755, b"executable")]:
            member = tarfile.TarInfo(name)
            member.type = kind
            member.mode = mode
            member.linkname = "/bin/sh" if kind in (tarfile.SYMTYPE, tarfile.LNKTYPE) else ""
            member.size = len(data) if kind == tarfile.REGTYPE else 0
            bundle.addfile(member, io.BytesIO(data))
    return output.getvalue()


class InstallerTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.path = self.root / "github-path"
        self.path.write_text("previous\n")
        self.env = patch.dict(os.environ, RUNNER_TEMP=str(self.root), GITHUB_PATH=str(self.path))
        self.env.start()
        self.addCleanup(self.env.stop)
        for key, value in (("system", "Linux"), ("machine", "x86_64")):
            mock = patch.object(installer.platform, key, return_value=value)
            mock.start()
            self.addCleanup(mock.stop)
        data = archive()
        self.network = patch.object(installer, "fetch", return_value=data).start()
        self.addCleanup(patch.stopall)
        self.digest = hashlib.sha256(data).hexdigest()

    def test_success_registers_only_after_validation(self):
        destination = self.root / "install"
        def validate(binary, tag):
            self.assertEqual(self.path.read_text(), "previous\n")
            self.assertEqual(binary.read_bytes(), b"executable")
            self.assertEqual(binary.stat().st_mode & 0o777, 0o700)
            self.assertEqual(tag, TAG)
        with patch.object(installer, "validate", side_effect=validate):
            binary = installer.install(TAG, self.digest, destination, True)
        self.assertEqual(binary, destination / "justrust")
        self.assertEqual(self.path.read_text(), f"previous\n{destination}\n")
        self.network.assert_called_once_with(f"{installer.ASSETS}{TAG}/justrust-{installer.TRIPLE}.tar.gz")

    def test_checksum_mismatch_never_executes_or_registers(self):
        with patch.object(installer, "validate") as validate:
            with self.assertRaises(ValueError):
                installer.install(TAG, "0" * 64, self.root / "install", True)
            validate.assert_not_called()
        self.assertEqual(self.path.read_text(), "previous\n")
        self.assertFalse((self.root / "install").exists())

    def test_validation_failure_cleans_and_preserves_path(self):
        with patch.object(installer, "validate", side_effect=ValueError("bad release")):
            with self.assertRaises(ValueError):
                installer.install(TAG, self.digest, self.root / "install", True)
        self.assertFalse((self.root / "install").exists())
        self.assertEqual(self.path.read_text(), "previous\n")

    def test_bad_pins(self):
        for tag in ("main", "v1.2.3/../x", "v1.2.3;echo bad", "v1.2.3\n", "v1.2.3-..", "--help"):
            with self.subTest(tag=tag), self.assertRaises(ValueError):
                installer.install(tag, self.digest)
        for digest in ("", "a" * 63, "g" * 64, "a" * 64 + "\n", "https://example.com/checksum"):
            with self.subTest(digest=digest), self.assertRaises(ValueError):
                installer.install(TAG, digest)
        self.network.assert_not_called()

    def test_unsupported_platform(self):
        for system, machine in (("Darwin", "x86_64"), ("Linux", "aarch64")):
            with patch.object(installer.platform, "system", return_value=system), patch.object(installer.platform, "machine", return_value=machine):
                with self.assertRaises(ValueError):
                    installer.install(TAG, self.digest)
        self.network.assert_not_called()

    def test_bad_output_locations_and_missing_environment(self):
        link = self.root / "link"
        link.symlink_to(self.root / "absent")
        for destination in (self.root, self.root.parent / "outside", link, self.root / "line\nbreak"):
            with self.subTest(destination=destination), self.assertRaises(ValueError):
                installer.install(TAG, self.digest, destination)
        with patch.dict(os.environ, {"RUNNER_TEMP": ""}):
            with self.assertRaises(ValueError):
                installer.install(TAG, self.digest)
        with patch.dict(os.environ, {"GITHUB_PATH": ""}):
            with self.assertRaises(ValueError):
                installer.install(TAG, self.digest, github_path=True)
        self.network.assert_not_called()

    def test_archive_attacks(self):
        good = (NAME, tarfile.REGTYPE, 0o755, b"binary")
        cases = [[(name, kind, mode, data)] for name, kind, mode, data in (
            (NAME, tarfile.SYMTYPE, 0o755, b""),
            (NAME, tarfile.LNKTYPE, 0o755, b""),
            ("../justrust", tarfile.REGTYPE, 0o755, b"bad"),
            ("/tmp/justrust", tarfile.REGTYPE, 0o755, b"bad"),
            (NAME, tarfile.REGTYPE, 0o644, b"bad"),
            (NAME, tarfile.REGTYPE, 0o755, b""),
        )]
        cases += [[good, good], [good, ("../evil", tarfile.SYMTYPE, 0o755, b"")]]
        for entries in cases:
            data = archive(entries)
            with self.subTest(entries=entries), self.assertRaises(ValueError):
                installer.executable(data, hashlib.sha256(data).hexdigest())

    def test_official_documentation_members_are_not_extracted(self):
        data = archive([(f"justrust-{installer.TRIPLE}", tarfile.DIRTYPE, 0o755, b""),
                        (NAME, tarfile.REGTYPE, 0o755, b"executable"),
                        (f"justrust-{installer.TRIPLE}/README.md", tarfile.REGTYPE, 0o644, b"readme"),
                        (f"justrust-{installer.TRIPLE}/LICENSE", tarfile.REGTYPE, 0o644, b"license")])
        self.assertEqual(installer.executable(data, hashlib.sha256(data).hexdigest()), b"executable")

    def test_archive_size_bound(self):
        data = archive()
        with patch.object(installer, "LIMIT", 4), self.assertRaises(ValueError):
            installer.executable(data, hashlib.sha256(data).hexdigest())

    def test_version_and_release_marker(self):
        for outputs in (("justrust wrong",), (f"justrust {TAG[1:]}", "development build")):
            with patch.object(installer.subprocess, "run", side_effect=[subprocess.CompletedProcess([], 0, text) for text in outputs]):
                with self.assertRaises(ValueError):
                    installer.validate(Path("/fake/justrust"), TAG)
        with patch.object(installer.subprocess, "run", side_effect=[subprocess.CompletedProcess([], 0, text) for text in (f"justrust {TAG[1:]}", "status (release build)")]) as run:
            installer.validate(Path("/fake/justrust"), TAG)
            self.assertEqual(run.call_args.kwargs["timeout"], 30)
            self.assertEqual(run.call_args.kwargs["env"]["JUSTRUST_NO_UPDATE"], "1")

    def test_production_release_takes_only_the_tag_from_the_service(self):
        checksum = f"{'AB' * 32}  justrust-{installer.TRIPLE}.tar.gz\n".encode()
        selected = b'{"version":"1.2.3","tag":"v1.2.3","enabled":true,"url":"https://evil.example/x","sha256_url":"https://evil.example/y"}'
        self.network.side_effect = [selected, checksum]
        self.assertEqual(installer.production_release(), ("v1.2.3", "ab" * 32))
        self.assertEqual([call.args[0] for call in self.network.call_args_list], [
            f"{installer.PRODUCTION}?target={installer.TRIPLE}",
            f"{installer.ASSETS}v1.2.3/justrust-{installer.TRIPLE}.tar.gz.sha256"])

    def test_production_release_rejects_bad_selections_and_retries_outages(self):
        for selected in (b"[]", b"not json", b'{"version":"1.2.3","tag":"v1.2.3","enabled":false}',
                         b'{"version":"1.2.4","tag":"v1.2.3","enabled":true}',
                         b'{"version":"1.2.3-rc.1","tag":"v1.2.3-rc.1","enabled":true}',
                         b'{"version":"../x","tag":"v../x","enabled":true}'):
            self.network.reset_mock()
            self.network.side_effect = None
            self.network.return_value = selected
            with self.subTest(selected=selected), self.assertRaises(ValueError):
                installer.production_release(attempts=1)
            self.network.assert_called_once()
        self.network.reset_mock()
        self.network.side_effect = [b'{"version":"1.2.3","tag":"v1.2.3","enabled":true}', b"<html>not a checksum</html>"]
        with self.assertRaises(ValueError):
            installer.production_release(attempts=1)
        self.network.side_effect = [OSError("dns"), b'{"version":"1.2.3","tag":"v1.2.3","enabled":true}', b"cd" * 32]
        with patch.object(installer.time, "sleep") as sleep:
            self.assertEqual(installer.production_release(delay=7), ("v1.2.3", "cd" * 32))
        sleep.assert_called_once_with(7)

    def test_redirect_rejects_http(self):
        with self.assertRaises(ValueError):
            installer.HTTPSRedirect().redirect_request(None, None, 302, "", {}, "http://example.com/asset")

    def test_fetch_size_and_scheme(self):
        for url, data in (("http://example.com", b"data"), ("https://example.com", b"12345")):
            with patch.object(installer.urllib.request, "build_opener") as opener, patch.object(installer, "LIMIT", 4):
                response = opener.return_value.open.return_value.__enter__.return_value
                response.geturl.return_value = url
                response.read.return_value = data
                with self.assertRaises(ValueError):
                    REAL_FETCH("https://github.com/official")


if __name__ == "__main__":
    unittest.main()
