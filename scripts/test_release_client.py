#!/usr/bin/env python3
"""Offline tests for one-command immutable client release and activation."""
import importlib.util
from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest import mock

SPEC = importlib.util.spec_from_file_location("release_client", Path(__file__).with_name("release-client.py"))
release = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(release)
SHA = "a" * 40
VERSION = "v0.1.0-alpha.29"


class ReleaseClientTests(unittest.TestCase):
    def test_candidate_requires_exact_committed_version_and_stable_ancestry(self):
        with mock.patch.object(release, "run", side_effect=[SHA, "", '[package]\nversion="0.1.0-alpha.29"']) as run:
            self.assertEqual(release.candidate(VERSION, "HEAD"), SHA)
            self.assertIn(mock.call("git", "merge-base", "--is-ancestor", SHA, "origin/stable"), run.call_args_list)
            self.assertIn(mock.call("git", "show", SHA + ":Cargo.toml"), run.call_args_list)
        with mock.patch.object(release, "run", side_effect=[SHA, "", '[package]\nversion="0.1.0-alpha.28"']):
            with self.assertRaisesRegex(ValueError, "committed"):
                release.candidate(VERSION, "HEAD")
        for tag in ["stable", "v1;cmd", "--tag", "v1.0.0\n"]:
            with self.assertRaises(ValueError):
                release.candidate(tag, "HEAD")

    def test_remote_annotated_tag_resolves_to_commit(self):
        with mock.patch.object(release, "run", return_value=f"{'b' * 40}\trefs/tags/{VERSION}\n{SHA}\trefs/tags/{VERSION}^{{}}"):
            self.assertEqual(release.remote_tag(VERSION), SHA)

    def test_existing_remote_tag_is_never_moved_or_rebuilt(self):
        with mock.patch.object(release, "remote_tag", return_value=SHA), mock.patch.object(release, "run") as run:
            release.ensure_tag(VERSION, SHA)
            run.assert_not_called()
        with mock.patch.object(release, "remote_tag", return_value="b" * 40), mock.patch.object(release, "run") as run:
            with self.assertRaisesRegex(ValueError, "refusing to move"):
                release.ensure_tag(VERSION, SHA)
            run.assert_not_called()

    def test_new_tag_is_pushed_once_without_duplicate_workflow_dispatch(self):
        with mock.patch.object(release, "remote_tag", return_value=None), \
                mock.patch.object(release.subprocess, "run", return_value=SimpleNamespace(returncode=1)), \
                mock.patch.object(release, "run", side_effect=['', SHA, '']) as run:
            release.ensure_tag(VERSION, SHA)
            self.assertEqual(run.call_args_list, [mock.call("git", "tag", VERSION, SHA),
                                                 mock.call('git', 'rev-parse', '--verify', 'refs/tags/' + VERSION),
                                                 mock.call("git", "push", "origin", SHA + ":refs/tags/" + VERSION)])

    def test_conflicting_local_tag_fails_without_push(self):
        with mock.patch.object(release, "remote_tag", return_value=None), \
                mock.patch.object(release.subprocess, "run", return_value=SimpleNamespace(returncode=0, stdout="b" * 40)), \
                mock.patch.object(release, "run") as run:
            with self.assertRaisesRegex(ValueError, "local tag"):
                release.ensure_tag(VERSION, SHA)
            run.assert_called_once_with('git', 'rev-parse', '--verify', 'b' * 40 + '^{commit}')

    def test_annotated_tag_push_uses_verified_object_not_moving_local_tag(self):
        annotated = 'c' * 40
        with mock.patch.object(release, 'remote_tag', return_value=None), \
                mock.patch.object(release.subprocess, 'run', return_value=SimpleNamespace(returncode=0, stdout=annotated)), \
                mock.patch.object(release, 'run', side_effect=[SHA, '']) as run:
            release.ensure_tag(VERSION, SHA)
            self.assertEqual(run.call_args_list[-1], mock.call('git', 'push', 'origin', annotated + ':refs/tags/' + VERSION))

    def test_publication_does_not_wait_for_full_workflow_success(self):
        production = SimpleNamespace(target=mock.Mock(return_value="triple"), release_info=mock.Mock(return_value={}))
        with mock.patch.object(release, "remote_tag", return_value=SHA), \
                mock.patch.object(release, "release_runs") as runs:
            release.wait_for_publication(VERSION, SHA, 10, 1, production)
            runs.assert_not_called()
            production.release_info.assert_called_once_with(VERSION, "triple")

    def test_waits_for_production_selection_then_uses_same_tag(self):
        production = SimpleNamespace(target=lambda: "triple", release_info=mock.Mock(side_effect=[ValueError("old"), {}]))
        with mock.patch.object(release, "remote_tag", return_value=SHA), \
                mock.patch.object(release, "release_runs", return_value=[dict(status="in_progress")]):
            sleep = mock.Mock()
            release.wait_for_publication(VERSION, SHA, 10, 1, production, clock=lambda: 0, sleep=sleep)
            sleep.assert_called_once_with(1)
            self.assertEqual(production.release_info.call_count, 2)

    def test_failed_publication_and_timeout_never_activate(self):
        production = SimpleNamespace(target=lambda: "triple", release_info=mock.Mock(side_effect=ValueError("held")))
        with mock.patch.object(release, "remote_tag", return_value=SHA), \
                mock.patch.object(release, "release_runs", return_value=[dict(status="completed", conclusion="failure", url="run")]), \
                mock.patch.object(release, "platform_failed", return_value=True):
            with self.assertRaisesRegex(RuntimeError, "failed"):
                release.wait_for_publication(VERSION, SHA, 10, 1, production)
        with mock.patch.object(release, "remote_tag", return_value=SHA), \
                mock.patch.object(release, "release_runs", return_value=[]):
            clock = mock.Mock(side_effect=[0, 11])
            with self.assertRaisesRegex(TimeoutError, "held"):
                release.wait_for_publication(VERSION, SHA, 10, 1, production, clock=clock)

    def test_other_platform_failure_does_not_block_ready_local_platform(self):
        production = SimpleNamespace(target=lambda: "triple", release_info=mock.Mock(side_effect=[ValueError("cached old"), {}]))
        with mock.patch.object(release, "remote_tag", return_value=SHA), \
                mock.patch.object(release, "release_runs", return_value=[dict(status="completed", conclusion="failure", url="run")]), \
                mock.patch.object(release, "platform_failed", return_value=False):
            release.wait_for_publication(VERSION, SHA, 10, 1, production, clock=lambda: 0, sleep=mock.Mock())
            self.assertEqual(production.release_info.call_count, 2)

    def test_unavailable_workflow_status_keeps_waiting_for_publication(self):
        production = SimpleNamespace(target=lambda: "triple", release_info=mock.Mock(side_effect=[ValueError("old"), {}]))
        error = release.subprocess.CalledProcessError(1, "gh")
        with mock.patch.object(release, "remote_tag", return_value=SHA), \
                mock.patch.object(release, "release_runs", side_effect=error):
            sleep = mock.Mock()
            release.wait_for_publication(VERSION, SHA, 10, 1, production, clock=lambda: 0, sleep=sleep)
            sleep.assert_called_once_with(1)
            self.assertEqual(production.release_info.call_count, 2)
        # It still times out, and never reports publication, if nothing is published.
        production = SimpleNamespace(target=lambda: "triple", release_info=mock.Mock(side_effect=ValueError("held")))
        with mock.patch.object(release, "remote_tag", return_value=SHA), \
                mock.patch.object(release, "release_runs", side_effect=error):
            with self.assertRaisesRegex(TimeoutError, "held"):
                release.wait_for_publication(VERSION, SHA, 10, 1, production, clock=mock.Mock(side_effect=[0, 11]))

    def test_failure_check_is_scoped_to_preflight_and_local_target(self):
        import json
        jobs = [dict(name="preflight", status="completed", conclusion="success"),
                dict(name="build triple", status="completed", conclusion="success"),
                dict(name="build other", status="completed", conclusion="failure")]
        with mock.patch.object(release, "run", return_value=json.dumps(dict(jobs=jobs))):
            self.assertFalse(release.platform_failed(dict(databaseId=123), "triple"))
        for index in (0, 1):
            changed = [dict(job) for job in jobs]
            changed[index]["conclusion"] = "failure"
            with mock.patch.object(release, "run", return_value=json.dumps(dict(jobs=changed))):
                self.assertTrue(release.platform_failed(dict(databaseId=123), "triple"))

    def test_tag_drift_aborts_even_after_initial_push(self):
        production = SimpleNamespace(target=lambda: "triple", release_info=mock.Mock())
        with mock.patch.object(release, "remote_tag", return_value="b" * 40):
            with self.assertRaisesRegex(ValueError, "moved"):
                release.wait_for_publication(VERSION, SHA, 10, 1, production)
            production.release_info.assert_not_called()

    def test_transient_tag_lookup_requires_successful_reverification(self):
        production = SimpleNamespace(target=lambda: "triple", release_info=mock.Mock())
        error = release.subprocess.CalledProcessError(128, "git")
        with mock.patch.object(release, "remote_tag", side_effect=[error, SHA]), mock.patch.object(release, "release_runs") as runs:
            sleep = mock.Mock(side_effect=lambda _: production.release_info.assert_not_called())
            release.wait_for_publication(VERSION, SHA, 10, 1, production, clock=lambda: 0, sleep=sleep)
            sleep.assert_called_once_with(1)
            runs.assert_not_called()
            production.release_info.assert_called_once_with(VERSION, "triple")

    def test_dry_run_has_no_tag_or_activation_side_effects(self):
        with mock.patch.object(release, "candidate", return_value=SHA), \
                mock.patch.object(release, "ensure_tag") as tag, \
                mock.patch.object(release, "run") as run, \
                mock.patch.object(release, "production_module") as production:
            release.main(["--tag", VERSION])
            tag.assert_not_called()
            run.assert_not_called()
            production.assert_not_called()

    def test_execute_order_activates_only_after_production_selection_without_recompile(self):
        events = []
        production = SimpleNamespace(target=lambda: events.append("target"))
        def command(*args):
            events.append(args)
            if args[:3] == ("git", "remote", "get-url"):
                return "https://github.com/1jehuang/justrust.git"
            return ""
        with mock.patch.object(release, "candidate", return_value=SHA), \
                mock.patch.object(release, "ensure_tag", side_effect=lambda *a: events.append("tag")), \
                mock.patch.object(release, "wait_for_publication", side_effect=lambda *a: events.append("published")), \
                mock.patch.object(release, "production_module", return_value=production), \
                mock.patch.object(release, "run", side_effect=command):
            release.main(["--tag", VERSION, "--execute"])
        activation = (release.sys.executable, str(release.ROOT / "scripts/production-dogfood.py"), "--expect", VERSION, "--activate")
        self.assertLess(events.index("target"), events.index("tag"))
        self.assertLess(events.index("tag"), events.index("published"))
        self.assertLess(events.index("published"), events.index(activation))
        self.assertNotIn("cargo", str(events))

    def test_other_remote_is_refused_before_tag_push(self):
        with mock.patch.object(release, "candidate", return_value=SHA), \
                mock.patch.object(release, "run", return_value="https://github.com/other/fork.git"), \
                mock.patch.object(release, "ensure_tag") as tag:
            with self.assertRaisesRegex(ValueError, "official"):
                release.main(["--tag", VERSION, "--execute"])
            tag.assert_not_called()


if __name__ == "__main__":
    unittest.main()
