#!/usr/bin/env python3
"""Post one GitHub release announcement to a Discord webhook.

The release workflow publishes releases with GitHub's built-in GITHUB_TOKEN.
GitHub deliberately suppresses most follow-up events created by that token, so
the notifier also listens for successful tag-driven Release workflow runs.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import sys
import urllib.error
import urllib.parse
import urllib.request
from typing import Any


DISCORD_LIMIT = 2_000
MARKER_PREFIX = "justrust-discord-announced"
MESSAGE_MARKER_PREFIX = "justrust-discord-message"
MAX_HIGHLIGHTS = 5
PLATFORM_BLOCK = re.compile(
    r"\n?<!-- jcode-platform-availability:start -->.*?"
    r"<!-- jcode-platform-availability:end -->\n?",
    flags=re.DOTALL,
)


def announcement_marker(tag: str) -> str:
    return f"<!-- {MARKER_PREFIX}:{tag} -->"


def already_announced(body: str, tag: str) -> bool:
    return announcement_marker(tag) in body


def release_notes_for_discord(body: str) -> str:
    body = PLATFORM_BLOCK.sub("\n", body)
    body = re.sub(r"<!--.*?-->", "", body, flags=re.DOTALL)
    body = re.sub(
        r"(?im)^\s*(?:\*\*)?Full Changelog(?:\*\*)?:[^\n]*$", "", body
    )
    # Prefer curated highlights over installation instructions and a full commit dump.
    heading = re.search(r"(?im)^(#{1,6})\s+Highlights\s*:?\s*$", body)
    if heading:
        end = re.search(rf"(?m)^#{{1,{len(heading[1])}}}\s+", body[heading.end():])
        body = body[heading.start():heading.end() + end.start()] if end else body[heading.start():]
    return re.sub(r"\n{3,}", "\n\n", body).strip()


def compare_base(*, repository: str, tag: str, release: dict[str, Any], token: str) -> str:
    link = re.search(
        rf"https://github\.com/{re.escape(repository)}/compare/([^\s<>\)]+)",
        release.get("body") or "",
    )
    if link:
        base, separator, head = urllib.parse.unquote(link[1]).rpartition("...")
        if separator and base and head == tag:
            return base
        raise RuntimeError("Release changelog does not compare against the requested tag")
    # Older hand-authored releases may lack a comparison link. Never compare to
    # today's latest tag when refreshing an older announcement.
    releases = github_request(
        f"https://api.github.com/repos/{repository}/releases?per_page=100", token=token
    )
    published = release.get("published_at")
    previous = [
        item for item in releases
        if not item.get("draft") and item.get("tag_name") != tag
        and published and item.get("published_at") and item["published_at"] < published
    ]
    if not previous:
        raise RuntimeError("No previous release found; add a Highlights section to this release")
    return max(previous, key=lambda item: item["published_at"])["tag_name"]


def highlight_subject(message: str) -> str | None:
    subject = message.splitlines()[0].strip() if message else ""
    conventional = re.match(r"^([a-z]+)(?:\([^)]*\))?!?:\s*(.+)$", subject)
    if conventional:
        if conventional[1] in {"docs", "test", "tests", "bench", "chore", "ci", "release", "build"}:
            return None
        subject = conventional[2]
    if not subject or re.search(
        r"(?i)\b(?:fixture|acceptance|benchmark|doctest|evidence)\b|^Merge\b", subject
    ):
        return None
    subject = subject[0].upper() + subject[1:]
    return subject if len(subject) <= 240 else subject[:239].rstrip() + "…"


def is_client_change(files: list[dict[str, Any]]) -> bool:
    def shipped(path: str) -> bool:
        return (
            path.startswith("src/") and path.endswith(".rs")
            or path in {"install.sh", "install.ps1", ".github/workflows/release.yml"}
            or path.startswith("scripts/install")
        )
    return any(
        shipped(item.get("filename", "")) or shipped(item.get("previous_filename", ""))
        for item in files
    )


def build_release_highlights(
    *, repository: str, tag: str, release: dict[str, Any], token: str
) -> str:
    base = compare_base(repository=repository, tag=tag, release=release, token=token)
    comparison = urllib.parse.quote(f"{base}...{tag}", safe="")
    endpoint = f"https://api.github.com/repos/{repository}/compare/{comparison}"
    first = github_request(f"{endpoint}?per_page=100&page=1", token=token)
    count = first.get("total_commits", 0)
    if first.get("status") not in {"ahead", "identical"} or not 0 <= count <= 500:
        raise RuntimeError("Unsupported release comparison; add an explicit Highlights section")
    commits = list(first.get("commits", []))
    for page in range(2, (count + 99) // 100 + 1):
        commits.extend(github_request(f"{endpoint}?per_page=100&page={page}", token=token)["commits"])
    if len(commits) != count:
        raise RuntimeError("Incomplete release comparison; refusing to invent release highlights")
    highlights: list[str] = []
    for commit in reversed(commits):
        subject = highlight_subject(commit.get("commit", {}).get("message", ""))
        if not subject or subject in highlights:
            continue
        sha = urllib.parse.quote(commit["sha"], safe="")
        detail = github_request(
            f"https://api.github.com/repos/{repository}/commits/{sha}", token=token
        )
        # This repository also contains unpublished compiler experiments. Those
        # are not shipping client features merely because their commits share a tag.
        if not is_client_change(detail.get("files", [])):
            continue
        highlights.append(subject)
        if len(highlights) == MAX_HIGHLIGHTS:
            break
    if not highlights:
        raise RuntimeError("No client highlights found; add a Highlights section before announcing")
    return "## Highlights\n\n" + "\n".join(f"- {item}" for item in highlights)


def release_message(*, repository: str, tag: str, release: dict[str, Any], token: str) -> str:
    body = release.get("body") or ""
    notes = release_notes_for_discord(body)
    substantive = [
        line for line in notes.splitlines()
        if line.strip() and not line.lstrip().startswith("#")
        and not re.fullmatch(r"\s*<?https?://\S+>?\s*", line)
    ]
    if not substantive:
        body = build_release_highlights(repository=repository, tag=tag, release=release, token=token)
    return format_message(
        tag=tag, name=release.get("name") or tag, body=body, url=release["html_url"]
    )


def format_message(*, tag: str, name: str, body: str, url: str) -> str:
    title = f"## justrust {tag}" + (f" | {name}" if name and name != tag else "")
    notes = release_notes_for_discord(body)
    message = title if not notes else f"{title}\n{notes}"
    suffix = f"\n… (full notes: <{url}>)"
    if len(message) > DISCORD_LIMIT:
        message = message[: DISCORD_LIMIT - len(suffix)] + suffix
    else:
        link = f"\n<{url}>"
        message = message[: DISCORD_LIMIT - len(link)] + link
    return message


def github_request(
    url: str,
    *,
    token: str,
    method: str = "GET",
    payload: dict[str, Any] | None = None,
) -> Any:
    data = None if payload is None else json.dumps(payload).encode("utf-8")
    request = urllib.request.Request(
        url,
        data=data,
        method=method,
        headers={
            "Accept": "application/vnd.github+json",
            "Authorization": f"Bearer {token}",
            "Content-Type": "application/json",
            "User-Agent": "justrust-release-bot/1.0",
            "X-GitHub-Api-Version": "2022-11-28",
        },
    )
    with urllib.request.urlopen(request, timeout=30) as response:
        return json.load(response)


def fetch_release(*, repository: str, tag: str, token: str) -> dict[str, Any]:
    quoted_tag = urllib.parse.quote(tag, safe="")
    return github_request(
        f"https://api.github.com/repos/{repository}/releases/tags/{quoted_tag}",
        token=token,
    )


def discord_webhook_url(webhook_url: str) -> str:
    parts = urllib.parse.urlsplit(webhook_url)
    query = urllib.parse.parse_qsl(parts.query, keep_blank_values=True)
    if not any(key == "wait" for key, _ in query):
        query.append(("wait", "true"))
    return urllib.parse.urlunsplit(
        (
            parts.scheme,
            parts.netloc,
            parts.path,
            urllib.parse.urlencode(query),
            parts.fragment,
        )
    )


def post_to_discord(*, webhook_url: str, content: str) -> dict[str, Any]:
    request = urllib.request.Request(
        discord_webhook_url(webhook_url),
        data=json.dumps(
            {
                "content": content,
                # Release notes are generated from commit subjects. Never let
                # them turn @everyone or user-looking text into real mentions.
                "allowed_mentions": {"parse": []},
            }
        ).encode("utf-8"),
        headers={
            "Content-Type": "application/json",
            "User-Agent": "justrust-release-bot/1.0",
        },
    )
    with urllib.request.urlopen(request, timeout=30) as response:
        result = json.load(response)
    if not result.get("id"):
        raise RuntimeError("Discord accepted the webhook but returned no message id")
    return result


def edit_discord_message(
    *, webhook_url: str, message_id: str, content: str, tag: str, release_url: str
) -> dict[str, Any]:
    if not re.fullmatch(r"[0-9]{1,22}", message_id):
        raise RuntimeError("Discord message id must be a numeric snowflake")
    parts = urllib.parse.urlsplit(webhook_url)
    query = [(key, value) for key, value in urllib.parse.parse_qsl(parts.query) if key != "wait"]
    url = urllib.parse.urlunsplit((
        parts.scheme, parts.netloc, parts.path.rstrip("/") + f"/messages/{message_id}",
        urllib.parse.urlencode(query), "",
    ))
    headers = {"Content-Type": "application/json", "User-Agent": "justrust-release-bot/1.0"}
    with urllib.request.urlopen(urllib.request.Request(url, headers=headers), timeout=30) as response:
        existing = json.load(response)
    previous = existing.get("content") or ""
    title = previous.splitlines()[0] if previous else ""
    expected = f"## justrust {tag}"
    if (
        str(existing.get("id")) != message_id
        or not (title == expected or title.startswith(expected + " | "))
        or release_url not in re.findall(r"https?://[^\s<>\)]+", previous)
    ):
        raise RuntimeError("Refusing to edit a Discord message belonging to another release")
    request = urllib.request.Request(
        url, method="PATCH", headers=headers,
        data=json.dumps({"content": content, "allowed_mentions": {"parse": []}}).encode("utf-8"),
    )
    with urllib.request.urlopen(request, timeout=30) as response:
        result = json.load(response)
    if str(result.get("id")) != message_id:
        raise RuntimeError("Discord did not confirm the requested message edit")
    return result


def mark_release_announced(
    *, repository: str, release: dict[str, Any], tag: str, token: str,
    message_id: str | None = None,
) -> None:
    body = release.get("body") or ""
    marker = announcement_marker(tag)
    message_marker = f"<!-- {MESSAGE_MARKER_PREFIX}:{tag}:{message_id} -->" if message_id else ""
    if marker in body and (not message_marker or message_marker in body):
        return
    updated_body = body.rstrip()
    if marker not in body:
        updated_body += f"\n\n{marker}"
    if message_marker and message_marker not in body:
        updated_body += f"\n{message_marker}"
    updated_body += "\n"
    github_request(
        f"https://api.github.com/repos/{repository}/releases/{release['id']}",
        token=token,
        method="PATCH",
        payload={"body": updated_body},
    )


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tag", default=os.environ.get("GITHUB_REF_NAME"))
    parser.add_argument("--repository", default=os.environ.get("GITHUB_REPOSITORY"))
    parser.add_argument("--edit-message-id", default=os.environ.get("DISCORD_RELEASE_MESSAGE_ID"))
    parser.add_argument("--preview", action="store_true", help="Print the announcement without posting or marking it")
    return parser.parse_args()


def required(value: str | None, description: str) -> str:
    if value:
        return value
    raise SystemExit(f"Missing {description}")


def announce_release(
    *, repository: str, tag: str, token: str, webhook_url: str, message_id: str | None = None
) -> str | None:
    release = fetch_release(repository=repository, tag=tag, token=token)
    if release.get("draft"):
        raise RuntimeError(f"Refusing to announce draft release {tag}")

    body = release.get("body") or ""
    if already_announced(body, tag) and not message_id:
        print(f"Discord announcement for {tag} is already recorded; skipping")
        return None

    content = release_message(repository=repository, tag=tag, release=release, token=token)
    editing = bool(message_id)
    if message_id:
        message = edit_discord_message(
            webhook_url=webhook_url, message_id=message_id, content=content,
            tag=tag, release_url=release["html_url"],
        )
    else:
        message = post_to_discord(webhook_url=webhook_url, content=content)
    message_id = str(message["id"])
    print(f"{'Updated' if editing else 'Posted'} {tag} on Discord as message {message_id}")

    # Re-fetch immediately before patching so an edit made while the Discord
    # request was in flight is not overwritten with the initially fetched body.
    try:
        latest_release = fetch_release(repository=repository, tag=tag, token=token)
        mark_release_announced(
            repository=repository, release=latest_release, tag=tag, token=token, message_id=message_id
        )
    except Exception as error:  # noqa: BLE001 - best-effort after public side effect
        # A marker write failure must not turn a successful public post into a
        # failed workflow that an operator is likely to rerun and duplicate.
        print(
            f"::warning::Discord post succeeded, but the release marker failed: {error}",
            file=sys.stderr,
        )
    return message_id


def main() -> int:
    args = parse_args()
    tag = required(args.tag, "release tag (--tag or GITHUB_REF_NAME)")
    repository = required(
        args.repository, "GitHub repository (--repository or GITHUB_REPOSITORY)"
    )
    token = required(
        os.environ.get("GH_TOKEN") or os.environ.get("GITHUB_TOKEN"), "GH_TOKEN"
    )
    try:
        if args.preview:
            release = fetch_release(repository=repository, tag=tag, token=token)
            print(release_message(repository=repository, tag=tag, release=release, token=token))
            return 0
        webhook_url = required(os.environ.get("DISCORD_RELEASE_WEBHOOK"), "DISCORD_RELEASE_WEBHOOK")
        announce_release(
            repository=repository,
            tag=tag,
            token=token,
            webhook_url=webhook_url,
            message_id=args.edit_message_id or None,
        )
        return 0
    except urllib.error.HTTPError as error:
        detail = error.read().decode("utf-8", errors="replace")
        print(f"HTTP {error.code} while announcing {tag}: {detail}", file=sys.stderr)
        return 1
    except (RuntimeError, urllib.error.URLError) as error:
        print(f"Could not announce {tag}: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
