#!/usr/bin/env python3

from __future__ import annotations

import importlib.util
import json
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import threading
import unittest
import urllib.error
from unittest import mock


SCRIPT = Path(__file__).with_name("post_discord_release.py")
SPEC = importlib.util.spec_from_file_location("post_discord_release", SCRIPT)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class WebhookHandler(BaseHTTPRequestHandler):
    request_path = ""
    payload: dict[str, object] = {}
    response_status = 200
    existing_content = ""
    requests: list[tuple[str, str]] = []

    def do_GET(self) -> None:  # noqa: N802 - stdlib callback name
        type(self).requests.append(("GET", self.path))
        self.respond({"id": "123", "content": self.existing_content})

    def do_PATCH(self) -> None:  # noqa: N802 - stdlib callback name
        type(self).requests.append(("PATCH", self.path))
        type(self).request_path = self.path
        length = int(self.headers["Content-Length"])
        type(self).payload = json.loads(self.rfile.read(length))
        self.respond({"id": "123", "content": self.payload["content"]})

    def respond(self, payload: dict[str, object]) -> None:
        response = json.dumps(payload).encode()
        self.send_response(self.response_status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(response)))
        self.end_headers()
        self.wfile.write(response)

    def do_POST(self) -> None:  # noqa: N802 - stdlib callback name
        type(self).request_path = self.path
        length = int(self.headers["Content-Length"])
        type(self).payload = json.loads(self.rfile.read(length))
        response = json.dumps(
            {"id": "discord-message-123"}
            if self.response_status == 200
            else {"message": "rejected"}
        ).encode()
        self.send_response(self.response_status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(response)))
        self.end_headers()
        self.wfile.write(response)

    def log_message(self, _format: str, *_args: object) -> None:
        pass


class DiscordReleaseTests(unittest.TestCase):
    def setUp(self) -> None:
        WebhookHandler.response_status = 200
        WebhookHandler.requests = []
        WebhookHandler.payload = {}

    def test_notes_strip_all_comments_and_full_changelog_lines(self) -> None:
        body = """<!-- private marker -->
- Faster hosted checks.
<!-- multiline
private metadata -->
**Full Changelog**: https://github.com/owner/repo/compare/v1...v2
"""
        self.assertEqual(
            MODULE.release_notes_for_discord(body), "- Faster hosted checks."
        )

    def test_notes_prefer_highlights_over_generated_sections(self) -> None:
        body = """Generated introduction.
## Highlights

- Better diagnostics.

## What's Changed
- Internal maintenance.

**Full Changelog**: https://github.com/owner/repo/compare/v1...v2
"""
        notes = MODULE.release_notes_for_discord(body)
        self.assertIn("Better diagnostics.", notes)
        self.assertNotIn("Generated introduction", notes)
        self.assertNotIn("Internal maintenance", notes)
        self.assertNotIn("Full Changelog", notes)

    def test_metadata_only_notes_are_empty(self) -> None:
        self.assertEqual(MODULE.release_notes_for_discord(
            "<!-- justrust-discord-announced:v2 -->\n"
            "**Full Changelog**: https://github.com/owner/repo/compare/v1...v2"
        ), "")

    def test_edit_message_option_defaults_from_environment_and_cli_overrides(self) -> None:
        with mock.patch.dict("os.environ", {"DISCORD_RELEASE_MESSAGE_ID": "123"}):
            with mock.patch("sys.argv", [str(SCRIPT)]):
                self.assertEqual(MODULE.parse_args().edit_message_id, "123")
            with mock.patch("sys.argv", [str(SCRIPT), "--edit-message-id", "456"]):
                self.assertEqual(MODULE.parse_args().edit_message_id, "456")

    def test_highlights_use_exact_release_range_and_client_files(self) -> None:
        subjects = [
            ("compiler", "Improve compiler scheduling", "toolchain/rustc/compiler/lib.rs"),
            ("bench", "Improve edit-loop timing", "bench/suite.py"),
            ("docs", "Explain hosted builds", "docs/remote.md"),
            ("tests", "Cover retry failures", "tests/retry.rs"),
            ("release", "release: v2", "Cargo.toml"),
            ("client", "fix: preserve diagnostics on reconnect\n\nDetails", "src/remote.rs"),
        ]
        commits = [{"sha": sha, "commit": {"message": subject}} for sha, subject, _ in subjects]
        paths = {sha: path for sha, _, path in subjects}

        def respond(url: str, **kwargs: object) -> dict[str, object]:
            self.assertEqual(kwargs, {"token": "token"})
            if "/compare/" in url:
                self.assertEqual(url, "https://api.github.com/repos/owner/repo/compare/v1...v2?per_page=100&page=1")
                return {"status": "ahead", "total_commits": len(commits), "commits": commits}
            self.assertIn("/commits/", url)
            return {"files": [{"filename": paths[url.rsplit("/", 1)[1]]}]}

        with mock.patch.object(MODULE, "github_request", side_effect=respond) as request:
            notes = MODULE.build_release_highlights(
                repository="owner/repo", tag="v2", token="token",
                release={"body": "**Full Changelog**: https://github.com/owner/repo/compare/v1...v2"},
            )
        self.assertEqual(notes, "## Highlights\n\n- Preserve diagnostics on reconnect")
        self.assertFalse(any("/releases" in call.args[0] for call in request.call_args_list))

    def test_highlights_limit_and_deduplicate_subjects(self) -> None:
        commits = [{"sha": str(index), "commit": {"message": f"fix: improvement {index // 2}"}} for index in range(14)]
        with mock.patch.object(MODULE, "github_request", side_effect=[
            {"status": "ahead", "total_commits": len(commits), "commits": commits},
            *[{"files": [{"filename": "src/main.rs"}]} for _ in range(5)],
        ]):
            notes = MODULE.build_release_highlights(
                repository="owner/repo", tag="v2", token="token",
                release={"body": "**Full Changelog**: https://github.com/owner/repo/compare/v1...v2"},
            )
        bullets = [line for line in notes.splitlines() if line.startswith("- ")]
        self.assertEqual(len(bullets), 5)
        self.assertEqual(len(set(bullets)), 5)

    def test_compare_fallback_selects_previous_published_release_not_latest(self) -> None:
        releases = [
            {"tag_name": "v3", "published_at": "2026-03-01T00:00:00Z"},
            {"tag_name": "v2", "published_at": "2026-02-01T00:00:00Z"},
            {"tag_name": "v1-draft", "published_at": "2026-01-31T00:00:00Z", "draft": True},
            {"tag_name": "v0", "published_at": "2025-12-01T00:00:00Z"},
            {"tag_name": "v1", "published_at": "2026-01-01T00:00:00Z"},
        ]
        with mock.patch.object(MODULE, "github_request", return_value=releases) as request:
            base = MODULE.compare_base(
                repository="owner/repo", tag="v2", token="token",
                release={"body": "", "published_at": "2026-02-01T00:00:00Z"},
            )
        self.assertEqual(base, "v1")
        request.assert_called_once_with("https://api.github.com/repos/owner/repo/releases?per_page=100", token="token")

    def test_compare_rejects_wrong_head_and_ignores_other_repository(self) -> None:
        with mock.patch.object(MODULE, "github_request") as request:
            with self.assertRaises(RuntimeError):
                MODULE.compare_base(
                    repository="owner/repo", tag="v2", token="token",
                    release={"body": "**Full Changelog**: https://github.com/owner/repo/compare/v1...v3"},
                )
        request.assert_not_called()
        with mock.patch.object(MODULE, "github_request", return_value=[
            {"tag_name": "v1", "published_at": "2026-01-01T00:00:00Z"},
        ]) as request:
            self.assertEqual(MODULE.compare_base(
                repository="owner/repo", tag="v2", token="token",
                release={"body": "**Full Changelog**: https://github.com/other/repo/compare/evil...v2", "published_at": "2026-02-01T00:00:00Z"},
            ), "v1")
        request.assert_called_once()

    def test_incomplete_comparison_refuses_highlights(self) -> None:
        with mock.patch.object(MODULE, "github_request", return_value={
            "status": "ahead", "total_commits": 2,
            "commits": [{"sha": "one", "commit": {"message": "Fix reconnect"}}],
        }), self.assertRaisesRegex(RuntimeError, "Incomplete"):
            MODULE.build_release_highlights(
                repository="owner/repo", tag="v2", token="token",
                release={"body": "**Full Changelog**: https://github.com/owner/repo/compare/v1...v2"},
            )

    def test_highlights_read_compare_pages_and_renamed_client_files(self) -> None:
        skipped = [{"sha": str(index), "commit": {"message": "test: maintain fixtures"}} for index in range(100)]
        with mock.patch.object(MODULE, "github_request", side_effect=[
            {"status": "ahead", "total_commits": 101, "commits": skipped},
            {"commits": [{"sha": "client", "commit": {"message": "fix: retain useful error context"}}]},
            {"files": [{"filename": "retired/client.rs", "previous_filename": "src/client.rs"}]},
        ]) as request:
            notes = MODULE.build_release_highlights(
                repository="owner/repo", tag="v2", token="token",
                release={"body": "**Full Changelog**: https://github.com/owner/repo/compare/v1...v2"},
            )
        self.assertEqual(notes, "## Highlights\n\n- Retain useful error context")
        self.assertEqual(request.call_count, 3)
        self.assertTrue(request.call_args_list[1].args[0].endswith("?per_page=100&page=2"))

    def test_announce_uses_fallback_for_sanitized_empty_notes(self) -> None:
        release = {"id": 2, "body": "<!-- metadata -->\n**Full Changelog**: https://github.com/owner/repo/compare/v1...v2", "html_url": "https://example.test/v2"}
        with (
            mock.patch.object(MODULE, "fetch_release", return_value=release),
            mock.patch.object(MODULE, "build_release_highlights", return_value="## Highlights\n\n- Faster retries.") as highlights,
            mock.patch.object(MODULE, "post_to_discord", return_value={"id": "123"}) as post,
            mock.patch.object(MODULE, "mark_release_announced"),
        ):
            self.assertEqual(MODULE.announce_release(repository="owner/repo", tag="v2", token="token", webhook_url="https://discord.test/webhook"), "123")
        highlights.assert_called_once_with(repository="owner/repo", tag="v2", release=release, token="token")
        self.assertIn("- Faster retries.", post.call_args.kwargs["content"])
        self.assertNotIn("metadata", post.call_args.kwargs["content"])

    def test_curated_notes_do_not_fetch_commit_highlights(self) -> None:
        with mock.patch.object(MODULE, "build_release_highlights") as highlights:
            message = MODULE.release_message(repository="owner/repo", tag="v2", token="token", release={"body": "## Highlights\n\n- Better checks.", "html_url": "https://example.test/v2"})
        highlights.assert_not_called()
        self.assertIn("- Better checks.", message)

    def test_no_client_highlights_refuses_bare_link_announcement(self) -> None:
        release = {"id": 2, "body": "**Full Changelog**: https://github.com/owner/repo/compare/v1...v2", "html_url": "https://example.test/v2"}
        with (
            mock.patch.object(MODULE, "fetch_release", return_value=release),
            mock.patch.object(MODULE, "github_request", return_value={"status": "identical", "total_commits": 0, "commits": []}),
            mock.patch.object(MODULE, "post_to_discord") as post,
            mock.patch.object(MODULE, "mark_release_announced") as mark,
            self.assertRaisesRegex(RuntimeError, "No client highlights"),
        ):
            MODULE.announce_release(repository="owner/repo", tag="v2", token="token", webhook_url="https://discord.test/webhook")
        post.assert_not_called()
        mark.assert_not_called()

    def test_edit_existing_marker_preserves_concurrent_body_and_records_id(self) -> None:
        release = {"id": 2, "body": "Useful notes.\n<!-- justrust-discord-announced:v2 -->", "html_url": "https://example.test/v2"}
        latest = {**release, "body": release["body"] + "\nConcurrent edit."}
        with (
            mock.patch.object(MODULE, "fetch_release", side_effect=[release, latest]),
            mock.patch.object(MODULE, "edit_discord_message", return_value={"id": "123"}) as edit,
            mock.patch.object(MODULE, "post_to_discord") as post,
            mock.patch.object(MODULE, "github_request") as request,
        ):
            self.assertEqual(MODULE.announce_release(repository="owner/repo", tag="v2", token="token", webhook_url="https://discord.test/webhook", message_id="123"), "123")
        post.assert_not_called()
        edit.assert_called_once()
        updated = request.call_args.kwargs["payload"]["body"]
        self.assertIn("Concurrent edit.", updated)
        self.assertIn("<!-- justrust-discord-message:v2:123 -->", updated)
        self.assertEqual(updated.count("<!-- justrust-discord-announced:v2 -->"), 1)

    def test_edit_verifies_existing_message_and_disables_mentions(self) -> None:
        url = "https://github.com/owner/repo/releases/tag/v2"
        WebhookHandler.existing_content = f"## justrust v2\n<{url}>"
        server = ThreadingHTTPServer(("127.0.0.1", 0), WebhookHandler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            result = MODULE.edit_discord_message(
                webhook_url=f"http://127.0.0.1:{server.server_port}/webhooks/7/token?thread_id=9&wait=true",
                message_id="123", content="Updated @everyone", tag="v2", release_url=url,
            )
        finally:
            server.shutdown()
            thread.join()
            server.server_close()
        self.assertEqual(result["id"], "123")
        self.assertEqual([method for method, _ in WebhookHandler.requests], ["GET", "PATCH"])
        for _, path in WebhookHandler.requests:
            self.assertTrue(path.startswith("/webhooks/7/token/messages/123?"), path)
            self.assertIn("thread_id=9", path)
            self.assertNotIn("wait=", path)
        self.assertEqual(WebhookHandler.payload, {
            "content": "Updated @everyone", "allowed_mentions": {"parse": []},
        })

    def test_edit_rejects_non_numeric_message_id_before_network(self) -> None:
        with mock.patch.object(MODULE.urllib.request, "urlopen") as request:
            with self.assertRaisesRegex(RuntimeError, "message id"):
                MODULE.edit_discord_message(
                    webhook_url="https://discord.test/webhooks/7/token",
                    message_id="../other", content="Updated", tag="v2",
                    release_url="https://example.test/v2",
                )
        request.assert_not_called()

    def test_edit_refuses_other_product_tag_or_release_url(self) -> None:
        url = "https://github.com/owner/repo/releases/tag/v2"
        for content in (
            f"## another-product v2\n<{url}>",
            f"## justrust v20\n<{url}>",
            "## justrust v2\n<https://github.com/owner/other/releases/tag/v2>",
            "## justrust v2\n<https://github.com/owner/repo/releases/tag/v20>",
        ):
            with self.subTest(content=content):
                WebhookHandler.requests = []
                WebhookHandler.existing_content = content
                server = ThreadingHTTPServer(("127.0.0.1", 0), WebhookHandler)
                thread = threading.Thread(target=server.serve_forever, daemon=True)
                thread.start()
                try:
                    with self.assertRaises(RuntimeError):
                        MODULE.edit_discord_message(
                            webhook_url=f"http://127.0.0.1:{server.server_port}/webhooks/7/token",
                            message_id="123", content="New notes", tag="v2", release_url=url,
                        )
                finally:
                    server.shutdown()
                    thread.join()
                    server.server_close()
                self.assertEqual([method for method, _ in WebhookHandler.requests], ["GET"])

    def test_edit_does_not_patch_when_existing_message_fetch_fails(self) -> None:
        WebhookHandler.response_status = 404
        server = ThreadingHTTPServer(("127.0.0.1", 0), WebhookHandler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            with self.assertRaises(urllib.error.HTTPError) as raised:
                MODULE.edit_discord_message(
                    webhook_url=f"http://127.0.0.1:{server.server_port}/webhooks/7/token",
                    message_id="123", content="New notes", tag="v2",
                    release_url="https://github.com/owner/repo/releases/tag/v2",
                )
            raised.exception.close()
        finally:
            server.shutdown()
            thread.join()
            server.server_close()
        self.assertEqual([method for method, _ in WebhookHandler.requests], ["GET"])

    def test_empty_notes_include_product_and_link(self) -> None:
        self.assertEqual(
            MODULE.format_message(tag="v1", name="v1", body="", url="https://example.test/v1"),
            "## justrust v1\n<https://example.test/v1>",
        )

    def test_boundary_notes_fit_with_link(self) -> None:
        url = "https://example.test/v1"
        for length in (1950, 1970, 1980, 2000):
            message = MODULE.format_message(tag="v1", name="v1", body="x" * length, url=url)
            self.assertLessEqual(len(message), MODULE.DISCORD_LIMIT)
            self.assertTrue(message.endswith(f"<{url}>") or message.endswith(f"<{url}>)"))

    def test_refetch_failure_after_post_is_not_a_failed_announcement(self) -> None:
        release = {
            "id": 1, "body": "notes", "html_url": "https://example.test/v1",
        }
        with (
            mock.patch.object(MODULE, "fetch_release", side_effect=[release, RuntimeError("offline")]),
            mock.patch.object(MODULE, "post_to_discord", return_value={"id": "message-1"}),
            mock.patch("sys.stderr") as stderr,
        ):
            result = MODULE.announce_release(
                repository="owner/repo", tag="v1", token="token", webhook_url="https://discord.test/webhook",
            )
        self.assertEqual(result, "message-1")
        self.assertTrue(stderr.write.called)

    def test_formats_release_and_removes_platform_availability(self) -> None:
        body = """## Highlights

Fast startup.

<!-- jcode-platform-availability:start -->
## Platform availability
- Linux: available
<!-- jcode-platform-availability:end -->
"""
        message = MODULE.format_message(
            tag="v0.52.0",
            name="v0.52.0",
            body=body,
            url="https://example.test/releases/v0.52.0",
        )
        self.assertEqual(
            message,
            "## justrust v0.52.0\n## Highlights\n\nFast startup."
            "\n<https://example.test/releases/v0.52.0>",
        )

    def test_truncates_to_discord_limit_and_keeps_release_link(self) -> None:
        url = "https://example.test/releases/v0.52.0"
        message = MODULE.format_message(
            tag="v0.52.0", name="A large release", body="x" * 3_000, url=url
        )
        self.assertLessEqual(len(message), MODULE.DISCORD_LIMIT)
        self.assertTrue(message.endswith(f"… (full notes: <{url}>)"))

    def test_marker_is_tag_specific(self) -> None:
        body = "notes\n\n<!-- justrust-discord-announced:v0.52.0 -->\n"
        self.assertTrue(MODULE.already_announced(body, "v0.52.0"))
        self.assertFalse(MODULE.already_announced(body, "v0.53.0"))

    def test_posts_with_wait_and_disables_mentions(self) -> None:
        server = ThreadingHTTPServer(("127.0.0.1", 0), WebhookHandler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            result = MODULE.post_to_discord(
                webhook_url=f"http://127.0.0.1:{server.server_port}/webhook?thread_id=1",
                content="Release @everyone",
            )
        finally:
            server.shutdown()
            thread.join()
            server.server_close()

        self.assertEqual(result["id"], "discord-message-123")
        self.assertIn("thread_id=1", WebhookHandler.request_path)
        self.assertIn("wait=true", WebhookHandler.request_path)
        self.assertEqual(
            WebhookHandler.payload,
            {"content": "Release @everyone", "allowed_mentions": {"parse": []}},
        )

    def test_rejected_webhook_raises_http_error(self) -> None:
        WebhookHandler.response_status = 400
        server = ThreadingHTTPServer(("127.0.0.1", 0), WebhookHandler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            with self.assertRaisesRegex(
                urllib.error.HTTPError, "HTTP Error 400"
            ) as raised:
                MODULE.post_to_discord(
                    webhook_url=f"http://127.0.0.1:{server.server_port}/webhook",
                    content="Release",
                )
            raised.exception.close()
        finally:
            server.shutdown()
            thread.join()
            server.server_close()

    def test_announce_skips_an_existing_marker(self) -> None:
        release = {
            "id": 52,
            "name": "v0.52.0",
            "body": "notes\n<!-- justrust-discord-announced:v0.52.0 -->",
            "html_url": "https://example.test/v0.52.0",
            "draft": False,
        }
        with (
            mock.patch.object(MODULE, "fetch_release", return_value=release),
            mock.patch.object(MODULE, "post_to_discord") as post,
        ):
            result = MODULE.announce_release(
                repository="owner/repo",
                tag="v0.52.0",
                token="token",
                webhook_url="https://discord.test/webhook",
            )
        self.assertIsNone(result)
        post.assert_not_called()

    def test_announce_rejects_draft_release(self) -> None:
        release = {
            "id": 53,
            "name": "v0.53.0",
            "body": "notes",
            "html_url": "https://example.test/v0.53.0",
            "draft": True,
        }
        with (
            mock.patch.object(MODULE, "fetch_release", return_value=release),
            mock.patch.object(MODULE, "post_to_discord") as post,
            self.assertRaisesRegex(RuntimeError, "draft release"),
        ):
            MODULE.announce_release(
                repository="owner/repo",
                tag="v0.53.0",
                token="token",
                webhook_url="https://discord.test/webhook",
            )
        post.assert_not_called()

    def test_announce_refetches_before_writing_marker(self) -> None:
        initial = {
            "id": 54,
            "name": "v0.54.0",
            "body": "initial notes",
            "html_url": "https://example.test/v0.54.0",
            "draft": False,
        }
        latest = {**initial, "body": "notes edited during post"}
        with (
            mock.patch.object(MODULE, "fetch_release", side_effect=[initial, latest]),
            mock.patch.object(
                MODULE, "post_to_discord", return_value={"id": "message-54"}
            ),
            mock.patch.object(MODULE, "mark_release_announced") as mark,
        ):
            result = MODULE.announce_release(
                repository="owner/repo",
                tag="v0.54.0",
                token="token",
                webhook_url="https://discord.test/webhook",
            )
        self.assertEqual(result, "message-54")
        mark.assert_called_once_with(
            repository="owner/repo",
            release=latest,
            tag="v0.54.0",
            token="token",
            message_id="message-54",
        )


if __name__ == "__main__":
    unittest.main()
