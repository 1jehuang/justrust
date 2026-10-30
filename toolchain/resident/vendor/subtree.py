#!/usr/bin/env python3
"""vendor/<name> is a squashed git subtree of a fork under indexable-inc; this moves code
between ra-jit, the fork and the fork's upstream.

    vendor/subtree.py push rust-analyzer [--rev REV] [--dry-run]
        send what REV (default HEAD) changed under vendor/rust-analyzer to the fork's branch
    vendor/subtree.py pull rust-analyzer [REF]
        take the fork's branch (or REF) into vendor/, as a squashed merge
    vendor/subtree.py upstream rust-analyzer [REF]
        merge upstream rust-analyzer's REF (default: its newest weekly release tag) into vendor/

`push` is what each fork runs every ten minutes (its own `sync from blaze` workflow, on its
`fork-sync` branch, with its own token). It makes one
fork commit for each commit on REV's first-parent line that changed the directory, with that
commit's tree under the directory, author, dates and message, so running it twice, or here and
in CI, makes the same commits. It starts after the newest commit whose directory is what the
fork's branch holds, and only ever fast-forwards the branch: if the fork holds something REV
never had, it stops and pushes nothing. A squashed merge on REV (from `pull` or `upstream`)
whose commit the fork does not have yet becomes a second parent of the fork commit, so an
upstream merge here is an upstream merge in the fork too.

`upstream` merges in this tree, not in the fork: the base is where the fork's history last met
upstream's, and conflicts are left in vendor/ as any `git merge` leaves them. Resolve, `git add`,
`git commit`; or `git merge --abort`.
"""
import argparse
import os
import re
import subprocess
import sys
from dataclasses import dataclass


@dataclass(frozen=True)
class Fork:
    url: str
    branch: str
    upstream: str | None


FORKS = {
    "rust-analyzer": Fork(
        "https://github.com/indexable-inc/rust-analyzer",
        "ra-jit",
        "https://github.com/rust-lang/rust-analyzer",
    ),
    # rustc's compiler/rustc_abi at the 1.99.0 tag; a newer one comes with a new toolchain
    # (docs/contributing.md), not from a merge.
    "rustc_abi": Fork("https://github.com/indexable-inc/rustc_abi", "main", None),
}

WEEKLY_TAG = re.compile(r"^\d{4}-\d{2}-\d{2}$")


def fail(msg: str) -> None:
    print(f"subtree.py: {msg}", file=sys.stderr)
    sys.exit(1)


def git(*args: str, input: str | None = None, env: dict[str, str] | None = None, check: bool = True) -> str:
    r = subprocess.run(
        ["git", *args],
        input=input,
        capture_output=True,
        text=True,
        env={**os.environ, **env} if env else None,
    )
    if check and r.returncode != 0:
        fail(f"git {' '.join(args)} exited {r.returncode}: {r.stderr.strip()[-2000:]}")
    return r.stdout.strip()


def has_commit(rev: str) -> bool:
    return subprocess.run(["git", "cat-file", "-e", f"{rev}^{{commit}}"], capture_output=True).returncode == 0


def is_ancestor(a: str, b: str) -> bool:
    return subprocess.run(["git", "merge-base", "--is-ancestor", a, b], capture_output=True).returncode == 0


def fetch(url: str, ref: str, into: str) -> str:
    """Fetches `ref` of `url` to the local ref `into` (refs/subtree/..., never pushed) and
    returns its commit. The ref keeps the objects from being collected."""
    git("fetch", "-q", "--no-tags", url, f"+{ref}:{into}")
    return git("rev-parse", f"{into}^{{commit}}")


def ensure_commit(sha: str, fork: Fork) -> None:
    """Makes sure `sha` (a split named by a squashed merge) is here, from the fork or upstream."""
    if has_commit(sha):
        return
    for url in filter(None, [fork.url, fork.upstream]):
        if subprocess.run(["git", "fetch", "-q", "--no-tags", url, sha], capture_output=True).returncode == 0 and has_commit(sha):
            return
    fail(f"{sha} is named by a squashed merge but neither {fork.url} nor {fork.upstream} has it")


def trees(revs: list[str], prefix: str) -> list[str | None]:
    """The tree at `prefix` in each of `revs` (None where there is none), in one git process."""
    out = git("cat-file", "--batch-check=%(objectname) %(objecttype)", input="".join(f"{r}:{prefix}\n" for r in revs))
    result: list[str | None] = []
    for line in out.splitlines():
        sha, _, kind = line.partition(" ")
        result.append(sha if kind == "tree" else None)
    if len(result) != len(revs):
        fail(f"cat-file answered {len(result)} lines for {len(revs)} commits")
    return result


def squash_splits(revs: str, prefix: str) -> list[str]:
    """The `git-subtree-split` commits of the squashed merges of `prefix` in `revs`, newest first."""
    log = git("log", "--no-show-signature", f"--grep=^git-subtree-dir: {prefix}/*$", "--format=%B%x00", revs)
    splits = []
    for body in log.split("\0"):
        m = re.search(r"^git-subtree-split: ([0-9a-f]{40})$", body, re.M)
        if m and m[1] not in splits:
            splits.append(m[1])
    return splits


def commit_env(rev: str) -> dict[str, str]:
    fields = git("show", "-s", "--format=%an%x00%ae%x00%aD%x00%cn%x00%ce%x00%cD", rev).split("\0")
    keys = ["GIT_AUTHOR_NAME", "GIT_AUTHOR_EMAIL", "GIT_AUTHOR_DATE", "GIT_COMMITTER_NAME", "GIT_COMMITTER_EMAIL", "GIT_COMMITTER_DATE"]
    return dict(zip(keys, fields))


def contains(base: str, fork_tip: str, tree: str) -> bool:
    """Whether `tree` already has every change the fork made from `base` to `fork_tip`: merging
    those changes into it leaves it as it is."""
    ours = git("commit-tree", tree, input="ra-jit\n")
    r = subprocess.run(
        ["git", "merge-tree", "--write-tree", f"--merge-base={base}", ours, fork_tip],
        capture_output=True,
        text=True,
    )
    return r.returncode == 0 and r.stdout.split("\n", 1)[0] == tree


def catch_up(name: str, fork: Fork, tip: str, line: list[str], line_trees: list[str | None]) -> tuple[int, int, str]:
    """Where to start when no commit on the line has the fork's tree: the fork was pushed from a
    branch that was merged with other changes, or ra-jit's history was rewritten since. Finds the
    newest fork commit whose tree the line has, and the first commit on the line after it that
    has everything the fork has since; the first fork commit made is that one, with a message
    that says so. Stops if no commit has it all: the fork holds something ra-jit does not."""
    on_line = {t: i for i, t in enumerate(line_trees) if t}
    ancestors = git("rev-list", "--first-parent", "--max-count=2000", tip).split()
    anchor = next(((a, on_line[t]) for a, t in zip(ancestors, trees(ancestors, "")) if t in on_line), None)
    if anchor is None:
        fail(f"{fork.url} {fork.branch} at {tip[:10]} shares no tree with ra-jit's vendor/{name}. Nothing was pushed.")
    base, base_index = anchor
    for j in range(base_index + 1, len(line)):
        if line_trees[j] and contains(base, tip, line_trees[j]):
            subjects = [
                git("show", "-s", "--format=%s", line[k])
                for k in range(base_index + 1, j + 1)
                if line_trees[k] and line_trees[k] != line_trees[k - 1]
            ]
            message = (
                f"Catch up with ra-jit at {line[j][:10]}\n\n"
                f"This branch had {git('rev-list', '--count', f'{base}..{tip}')} commits since {base[:10]} that "
                f"ra-jit has in other commits (pushed from a branch before its merge, or ra-jit's history "
                f"was rewritten since). This commit brings vendor/{name} to ra-jit's tree after:\n\n"
                + "".join(f"- {s}\n" for s in subjects)
            )
            return base_index, j, message
    fail(
        f"{fork.url} {fork.branch} at {tip[:10]} has changes since {base[:10]} that no commit of ra-jit has: "
        f"something was committed to the fork that ra-jit does not have. Bring it in with "
        f"`vendor/subtree.py pull {name}` (on a branch, merged into main as usual), then push again. "
        f"Nothing was pushed."
    )
    raise AssertionError


def cmd_push(name: str, fork: Fork, rev: str, dry_run: bool) -> None:
    prefix = f"vendor/{name}"
    rev = git("rev-parse", f"{rev}^{{commit}}")
    tip = fetch(fork.url, f"refs/heads/{fork.branch}", f"refs/subtree/{name}/fork")
    tip_tree = git("rev-parse", f"{tip}^{{tree}}")

    line = git("rev-list", "--first-parent", "--reverse", rev).split()
    line_trees = trees(line, prefix)

    # Where the fork is: the newest commit on the line whose directory is the fork's tree.
    start = next((i + 1 for i in reversed(range(len(line))) if line_trees[i] == tip_tree), None)
    first_message, since = None, None
    if start is None:
        since, start, first_message = catch_up(name, fork, tip, line, line_trees)

    parent, prev_tree, made = tip, tip_tree, []
    for i in range(start, len(line)):
        tree = line_trees[i]
        if tree is None or tree == prev_tree:
            continue
        c = line[i]
        parents = [parent]
        prev = line[since] if since is not None and not made else line[i - 1] if i else None
        for split in reversed(squash_splits(f"{prev}..{c}" if prev else c, prefix)):
            ensure_commit(split, fork)
            if not any(is_ancestor(split, p) for p in parents):
                parents.append(split)
        message = first_message if not made and first_message else git("show", "-s", "--format=%B", c).rstrip() + "\n"
        message += f"\nindexable-inc/ra-jit@{c}\n"
        args = ["commit-tree", tree]
        for p in parents:
            args += ["-p", p]
        parent = git(*args, input=message, env=commit_env(c))
        prev_tree = tree
        made.append((parent, c))

    if not made:
        print(f"{name}: {fork.branch} at {tip[:10]} is up to date with {rev[:10]}")
        return
    for new, c in made:
        print(f"{name}: {new[:10]} from {c[:10]} {git('show', '-s', '--format=%s', c)[:100]}")
    if dry_run:
        print(f"{name}: would push {len(made)} commits, {fork.branch} {tip[:10]}..{parent[:10]} (dry run)")
        return
    # No `+`: the fork rejects anything but a fast-forward, even if it moved since the fetch.
    git("push", "-q", fork.url, f"{parent}:refs/heads/{fork.branch}")
    git("update-ref", f"refs/subtree/{name}/fork", parent)
    print(f"{name}: pushed {len(made)} commits, {fork.branch} {tip[:10]}..{parent[:10]}")


def cmd_pull(name: str, fork: Fork, ref: str | None) -> None:
    os.execvp("git", ["git", "subtree", "pull", "--prefix", f"vendor/{name}", fork.url, ref or fork.branch, "--squash"])


def replace_tree(root: str, path: str, tree: str) -> str:
    """`root` with the tree at `path` replaced by `tree`."""
    head, _, rest = path.partition("/")
    entries = [e for e in git("ls-tree", "-z", root).split("\0") if e]
    out, found = [], False
    for e in entries:
        meta, _, entry = e.partition("\t")
        if entry == head:
            found = True
            child = replace_tree(meta.split()[2], rest, tree) if rest else tree
            e = f"040000 tree {child}\t{head}"
        out.append(e)
    if not found:
        fail(f"{head} is not in tree {root}")
    return git("mktree", "-z", input="\0".join(out) + "\0")


def latest_weekly_tag(url: str) -> str:
    tags = [l.split("refs/tags/")[1] for l in git("ls-remote", "--tags", url).splitlines() if "refs/tags/" in l]
    weekly = sorted(t for t in tags if WEEKLY_TAG.match(t))
    if not weekly:
        fail(f"{url} has no weekly release tag (YYYY-MM-DD)")
    return weekly[-1]


def cmd_upstream(name: str, fork: Fork, ref: str | None) -> None:
    if fork.upstream is None:
        fail(f"vendor/{name} has no upstream to merge from")
    prefix = f"vendor/{name}"
    top = git("rev-parse", "--show-toplevel")
    os.chdir(top)
    gitdir = git("rev-parse", "--absolute-git-dir")
    if os.path.exists(os.path.join(gitdir, "MERGE_HEAD")):
        fail("a merge is in progress; finish it or `git merge --abort` first")
    if git("status", "--porcelain", "--untracked-files=no"):
        fail("the tree has changes; commit or stash them first")

    ref = ref or latest_weekly_tag(fork.upstream)
    remote_ref = ref if ref.startswith("refs/") or re.fullmatch(r"[0-9a-f]{40}", ref) else (
        f"refs/tags/{ref}" if WEEKLY_TAG.match(ref) else f"refs/heads/{ref}"
    )
    upstream = fetch(fork.upstream, remote_ref, f"refs/subtree/{name}/upstream")
    tip = fetch(fork.url, f"refs/heads/{fork.branch}", f"refs/subtree/{name}/fork")
    splits = squash_splits("HEAD", prefix)
    for s in splits:
        ensure_commit(s, fork)
    # The newest commit of upstream's that the fork's history (as pushed, and as squashed here
    # since) already has.
    base = git("merge-base", upstream, tip, *splits, check=False)
    if not base:
        fail(f"{fork.upstream} {ref} shares no history with {fork.url} {fork.branch}")
    if base == upstream:
        print(f"{prefix} already has {ref} ({upstream[:10]})")
        return

    head = git("rev-parse", "HEAD")
    head_tree = git("rev-parse", "HEAD^{tree}")
    up_tree = git("rev-parse", f"{upstream}^{{tree}}")
    base_side = git("commit-tree", replace_tree(head_tree, prefix, git("rev-parse", f"{base}^{{tree}}")), input="merge base\n")
    their_side = git("commit-tree", replace_tree(head_tree, prefix, up_tree), input=f"{name} {ref}\n")
    # What `git subtree` writes for a squashed merge: `push` reads its git-subtree-split and
    # makes the upstream commit a parent of the fork's commit.
    squash = git(
        "commit-tree", up_tree,
        input=f"Squashed '{prefix}/' content from {name} {ref}\n\ngit-subtree-dir: {prefix}\ngit-subtree-split: {upstream}\n",
    )
    count = git("rev-list", "--count", f"{base}..{upstream}")
    message = f"{name}: merge upstream {ref} ({upstream[:10]})\n\n{count} upstream commits since {base[:10]}, merged into {prefix}.\n"

    labels = {f"GITHEAD_{their_side}": f"{name} {ref}", f"GITHEAD_{base_side}": f"{name} {base[:10]}"}
    r = subprocess.run(["git", "merge-recursive", base_side, "--", "HEAD", their_side], env={**os.environ, **labels})
    if r.returncode not in (0, 1):
        fail(f"git merge-recursive exited {r.returncode}")
    with open(os.path.join(gitdir, "ORIG_HEAD"), "w") as f:
        f.write(head + "\n")
    with open(os.path.join(gitdir, "MERGE_HEAD"), "w") as f:
        f.write(squash + "\n")
    with open(os.path.join(gitdir, "MERGE_MSG"), "w") as f:
        f.write(message)

    conflicts = git("diff", "--name-only", "--diff-filter=U").splitlines()
    if not conflicts:
        git("commit", "-q", "--no-edit")
        print(f"merged {name} {ref} ({upstream[:10]}, {count} commits since {base[:10]}) into {prefix}: {git('rev-parse', '--short', 'HEAD')}")
        return
    print(f"\n{len(conflicts)} files conflict merging {name} {ref} ({upstream[:10]}, {count} commits since {base[:10]}):")
    for p in conflicts:
        print(f"    {p}")
    print("\nResolve them, `git add` them and `git commit`; or `git merge --abort`.")
    sys.exit(1)


def main() -> None:
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = p.add_subparsers(dest="op", required=True)
    push = sub.add_parser("push", help="send vendor/<name> commits to the fork")
    push.add_argument("name", choices=FORKS)
    push.add_argument("--rev", default="HEAD")
    push.add_argument("--dry-run", action="store_true", help="make the commits, push nothing")
    pull = sub.add_parser("pull", help="take the fork's branch into vendor/<name>")
    pull.add_argument("name", choices=FORKS)
    pull.add_argument("ref", nargs="?")
    up = sub.add_parser("upstream", help="merge an upstream release into vendor/<name>")
    up.add_argument("name", choices=FORKS)
    up.add_argument("ref", nargs="?", help="tag, branch or commit (default: newest weekly tag)")
    a = p.parse_args()

    os.chdir(git("rev-parse", "--show-toplevel"))
    fork = FORKS[a.name]
    if a.op == "push":
        cmd_push(a.name, fork, a.rev, a.dry_run)
    elif a.op == "pull":
        cmd_pull(a.name, fork, a.ref)
    else:
        cmd_upstream(a.name, fork, a.ref)


if __name__ == "__main__":
    main()
