"""Real Git regression for Desktop 7d38fb7's caller-identity version fixtures."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import runtime_context as runtime


class DesktopVersionRuntimeIdentityTests(unittest.TestCase):
    def test_missing_caller_identity_still_fails_with_real_git_diagnostic(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            home = root / "home"
            home.mkdir()
            archive = root / "archive"
            archive.mkdir()
            env = {"PATH": os.environ["PATH"], "HOME": str(home)}
            with patch.dict(os.environ, {runtime.SELECTOR: runtime.POLICY}, clear=True):
                with runtime.runtime_context(env, archive) as child:
                    self.assertFalse(any(key.startswith(("GIT_AUTHOR_", "GIT_COMMITTER_"))
                                         for key in child), "runtime must not invent an identity")
                    fixture = root / "fixture"
                    fixture.mkdir()
                    subprocess.run(["git", "init", "-q", str(fixture)], env=child,
                                   check=True, capture_output=True)
                    # Avoid host system identity guesses, without providing identity.
                    subprocess.run(["git", "-C", str(fixture), "config", "user.useConfigOnly", "true"],
                                   env=child, check=True, capture_output=True)
                    result = subprocess.run(["git", "-C", str(fixture), "-c", "commit.gpgsign=false",
                                             "commit", "--allow-empty", "-qm", "Version fixture"],
                                            env=child, capture_output=True, text=True)
                    self.assertEqual(result.returncode, 128, result.stderr)
                    self.assertIn("identity unknown", result.stderr.lower())
                    self.assertIn("fatal:", result.stderr)

    def test_fixture_commits_and_annotated_tags_keep_callers_configured_identity(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            home = root / "caller-home"
            home.mkdir()
            archive = root / "archive"
            archive.mkdir()
            # Intentionally no inherited Git environment or host global config.
            # A fixture-specific identity is configured only in this test's HOME.
            env = {"PATH": os.environ["PATH"], "HOME": str(home),
                   "GIT_CONFIG_NOSYSTEM": "1"}
            config = home / ".gitconfig"
            config.write_text('[user]\n\tname = Desktop Fixture Caller\n'
                              '\temail = desktop-fixture@example.invalid\n'
                              '[credential]\n\thelper = !false\n')
            original = config.read_bytes()

            def git(child, repo, *args):
                result = subprocess.run(["git", "-C", str(repo), *args],
                                        env=child, text=True, capture_output=True)
                self.assertEqual(result.returncode, 0,
                                 f"git {args!r}: {result.stderr}")
                return result.stdout.strip()

            with patch.dict(os.environ, {runtime.SELECTOR: runtime.POLICY}, clear=True):
                with runtime.runtime_context(env, archive) as child:
                    private_home = Path(child["HOME"])
                    self.assertNotEqual(private_home, home)
                    fixture = root / "fixture"
                    fixture.mkdir()
                    git(child, fixture, "init", "-q", "--initial-branch=main")
                    git(child, fixture, "config", "commit.gpgsign", "false")
                    git(child, fixture, "config", "tag.gpgsign", "false")
                    git(child, fixture, "commit", "--allow-empty", "-qm", "Version fixture")
                    git(child, fixture, "tag", "-am", "Base", "v0.1.0")
                    git(child, fixture, "commit", "--allow-empty", "-qm", "Version fixture next")
                    self.assertEqual(git(child, fixture, "rev-list", "--count", "v0.1.0..HEAD"), "1")
                    self.assertEqual(git(child, fixture, "show", "-s", "--format=%an <%ae>|%cn <%ce>"),
                                     "Desktop Fixture Caller <desktop-fixture@example.invalid>|"
                                     "Desktop Fixture Caller <desktop-fixture@example.invalid>")
                    self.assertEqual(git(child, fixture, "for-each-ref", "--format=%(taggername) <%(taggeremail:trim)>",
                                         "refs/tags/v0.1.0"),
                                     "Desktop Fixture Caller <desktop-fixture@example.invalid>")
                    result = subprocess.run(["git", "config", "--global", "--get", "credential.helper"],
                                            env=child, capture_output=True, text=True)
                    self.assertEqual(result.returncode, 1, "caller credentials must not be copied")
                self.assertFalse(private_home.exists())
            self.assertEqual(config.read_bytes(), original)
            self.assertFalse((archive / ".git").exists())


if __name__ == "__main__":
    unittest.main()
