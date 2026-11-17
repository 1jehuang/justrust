"""Exact pinned author identity provisioning, using real Git objects and shell."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

import git_fixture_identity as fixture


class GitFixtureIdentityTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.env = {"PATH": os.environ["PATH"], "HOME": str(self.root),
                    "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": "/dev/null"}
        self.git("init", "-q", "--template=", str(self.root))
        self.original_config = (self.root / ".git" / "config").read_bytes()
        self.name = "Pinned Author ' $(touch must-not-exist)"
        self.email = "pinned-author@example.invalid"
        self.revision = self.commit(self.name, self.email)

    def git(self, *args, env=None):
        return subprocess.check_output(["git", "-C", str(self.root), *args],
                                       env=env or self.env, text=True).strip()

    def commit(self, name, email):
        env = dict(self.env, GIT_AUTHOR_NAME=name, GIT_AUTHOR_EMAIL=email,
                   GIT_COMMITTER_NAME=name, GIT_COMMITTER_EMAIL=email)
        self.git("-c", "commit.gpgsign=false", "commit", "--allow-empty", "-qm", "Fixture", env=env)
        return self.git("rev-parse", "HEAD")

    def test_exact_pinned_author_not_head_or_replacement(self):
        other = self.commit("Other Author", "other@example.invalid")
        self.git("replace", self.revision, other)
        expected = {f"GIT_{role}_{field}": value for role in ("AUTHOR", "COMMITTER")
                    for field, value in (("NAME", self.name), ("EMAIL", self.email))}
        self.assertEqual(fixture.pinned_author_env(self.root, self.revision), expected)
        self.assertFalse((self.root / ".gitconfig").exists())
        self.assertEqual((self.root / ".git" / "config").read_bytes(), self.original_config)

    def test_requires_exact_full_lowercase_commit_sha(self):
        for revision in ("HEAD", self.revision[:7], self.revision.upper(), "-n1", "", "g" * 40):
            with self.subTest(revision=revision), self.assertRaises(ValueError):
                fixture.pinned_author_env(self.root, revision)
        with self.assertRaises(subprocess.CalledProcessError):
            fixture.pinned_author_env(self.root, "0" * 40)
        tree = self.git("rev-parse", "HEAD^{tree}")
        with self.assertRaises((ValueError, subprocess.CalledProcessError)):
            fixture.pinned_author_env(self.root, tree)

    def test_cli_shell_quoting_roundtrip_has_receipt_and_no_global_writes(self):
        helper = Path(fixture.__file__).resolve()
        result = subprocess.run([sys.executable, str(helper), str(self.root), self.revision],
                                env=self.env, capture_output=True, text=True, check=True)
        self.assertIn(self.revision, result.stderr)
        exports = self.root / "exports.sh"
        exports.write_text(result.stdout)
        shell = subprocess.check_output(
            ["bash", "-c", 'source "$1"; printf "%s\\n" "$GIT_AUTHOR_NAME" "$GIT_AUTHOR_EMAIL" "$GIT_COMMITTER_NAME" "$GIT_COMMITTER_EMAIL"',
             "fixture", str(exports)], cwd=self.root, env=self.env, text=True)
        self.assertEqual(shell.splitlines(), [self.name, self.email, self.name, self.email])
        self.assertFalse((self.root / "must-not-exist").exists())
        self.assertFalse((self.root / ".gitconfig").exists())


if __name__ == "__main__":
    unittest.main()
