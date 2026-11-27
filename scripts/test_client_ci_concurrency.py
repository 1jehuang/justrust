#!/usr/bin/env python3
"""Offline, stdlib-only checks of the client CI concurrency policy.

Read the actual workflow scalars and interpret only their small expression subset.
This is not a general YAML or Actions interpreter. Unknown syntax fails closed so
policy changes cannot silently bypass the behavioral regression cases.
"""
import ast
from pathlib import Path
import re
import unittest

ROOT = Path(__file__).resolve().parents[1]


def expression(source, context):
    """Evaluate strings, context lookups, ==, && and || with operand returns."""
    tree = ast.parse(source.replace("&&", " and ").replace("||", " or ").strip(), mode="eval")

    def visit(node):
        if isinstance(node, ast.Constant) and isinstance(node.value, str):
            return node.value
        if isinstance(node, ast.Name) and node.id == "github":
            return context
        if isinstance(node, ast.Attribute):
            return visit(node.value).get(node.attr, {})
        if isinstance(node, ast.Compare) and len(node.ops) == 1 and isinstance(node.ops[0], ast.Eq):
            return visit(node.left) == visit(node.comparators[0])
        if isinstance(node, ast.BoolOp) and isinstance(node.op, (ast.And, ast.Or)):
            result = visit(node.values[0])
            for value in node.values[1:]:
                if isinstance(node.op, ast.And) and not result:
                    break
                if isinstance(node.op, ast.Or) and result:
                    break
                result = visit(value)
            return result
        raise AssertionError(f"unsupported concurrency expression: {ast.dump(node)}")

    return visit(tree.body)


def render(source, context):
    return re.sub(r"\$\{\{(.*?)\}\}", lambda match: str(expression(match[1], context)), source)


def context(event="push", ref="refs/heads/stable", run=100, pr=None, workflow="CI", **extra):
    return dict(event_name=event, ref=ref, run_id=run, workflow=workflow,
                event={"pull_request": {"number": pr}}, **extra)


def policy_fields(workflow):
    """The one workflow-level concurrency group and cancel expression."""
    blocks = re.findall(r"^concurrency:\n((?:[ \t]+[^\n]*\n)+)", workflow, re.MULTILINE)
    if len(blocks) != 1:
        raise AssertionError("expected one workflow-level concurrency block")
    fields = {}
    for line in blocks[0].splitlines():
        match = re.fullmatch(r"  (group|cancel-in-progress): (.+)", line)
        if not match or match[1] in fields:
            raise AssertionError(f"unexpected concurrency field: {line}")
        fields[match[1]] = match[2]
    if set(fields) != {"group", "cancel-in-progress"}:
        raise AssertionError("missing concurrency policy field")
    match = re.fullmatch(r"\$\{\{(.*?)\}\}", fields["cancel-in-progress"])
    if not match:
        raise AssertionError("cancel-in-progress must be an explicit expression")
    return fields["group"], match[1]


class ClientCIConcurrencyTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        cls.group, cls.cancel = policy_fields(cls.workflow)
        cls.macos = (ROOT / ".github/workflows/ci-macos.yml").read_text()
        cls.macos_group, cls.macos_cancel = policy_fields(cls.macos)

    def policy(self, ctx):
        return render(self.group, ctx).lower(), expression(self.cancel, ctx)

    def macos_policy(self, ctx):
        return render(self.macos_group, ctx).lower(), expression(self.macos_cancel, ctx)

    def test_macos_stable_runs_finish_and_only_the_pending_one_is_replaced(self):
        # macOS runners queue far longer than the gap between stable pushes. A
        # shared group without cancellation lets the running one report and
        # keeps a single pending run, which the next push replaces.
        first, cancel = self.macos_policy(context(workflow="CI macOS"))
        second, next_cancel = self.macos_policy(context(run=101, workflow="CI macOS"))
        self.assertEqual(first, second)
        self.assertIs(cancel, False)
        self.assertIs(next_cancel, False)
        self.assertNotEqual(first, self.policy(context())[0])
        self.assertNotEqual(first, self.policy(context(workflow="CI macOS"))[0])

    def test_macos_pr_updates_supersede_and_tags_stay_independent(self):
        first, cancel = self.macos_policy(context("pull_request", "refs/pull/12/merge", pr=12, workflow="CI macOS"))
        second, _ = self.macos_policy(context("pull_request", "refs/pull/12/merge", run=101, pr=12, workflow="CI macOS"))
        other, _ = self.macos_policy(context("pull_request", "refs/pull/13/merge", pr=13, workflow="CI macOS"))
        self.assertEqual(first, second)
        self.assertNotEqual(first, other)
        self.assertIs(cancel, True)
        groups = []
        for ref, run in (("refs/tags/v1.0.0", 100), ("refs/tags/v1.0.0", 101), ("refs/tags/v1.0.1", 102)):
            group, cancel = self.macos_policy(context(ref=ref, run=run, workflow="CI macOS"))
            groups.append(group)
            self.assertIs(cancel, False)
        self.assertEqual(len(set(groups)), len(groups))

    def test_macos_workflow_keeps_the_ci_triggers_environment_and_checks(self):
        for trigger in ('    tags: ["v*"]', "    branches: [stable]", "  pull_request:\n"):
            self.assertIn(trigger, self.macos)
        self.assertRegex(self.macos, r"(?m)^  macos:$")
        self.assertNotRegex(self.workflow, r"(?m)^  macos:$")
        self.assertNotIn("macos-", self.workflow)
        self.assertIn("os: [macos-latest, macos-15-intel]", self.macos)
        env = re.search(r"^env:\n((?:[ \t]+[^\n]*\n)+)", self.workflow, re.MULTILINE)
        self.assertIn(env[0], self.macos)
        for check in ("cargo clippy --all-targets --locked", "cargo test --locked",
                      "ci/check-test-bundle.sh", "Depcache across checkouts",
                      "macOS integration paths", "Release build runs"):
            self.assertIn(check, self.macos)

    def test_stable_pushes_supersede_each_other(self):
        first, cancel = self.policy(context())
        second, next_cancel = self.policy(context(run=101))
        self.assertEqual(first, second)
        self.assertIs(cancel, True)
        self.assertIs(next_cancel, True)

    def test_same_pr_updates_supersede_each_other(self):
        # Use PR number even if the event's ref changes (for example on close).
        first, cancel = self.policy(context("pull_request", "refs/pull/12/merge", pr=12))
        second, next_cancel = self.policy(context("pull_request", "refs/heads/stable", run=101, pr=12))
        self.assertEqual(first, second)
        self.assertIs(cancel, True)
        self.assertIs(next_cancel, True)

    def test_same_named_branches_from_different_contributors_are_independent(self):
        groups = []
        for pr in (12, 13):
            group, cancel = self.policy(context("pull_request", f"refs/pull/{pr}/merge",
                                               pr=pr, head_ref="fix", actor=f"user-{pr}"))
            groups.append(group)
            self.assertIs(cancel, True)
        self.assertNotEqual(*groups)
        self.assertNotIn(self.policy(context())[0], groups)

    def test_tag_runs_never_replace_running_or_pending_ci(self):
        groups = []
        for ref, run in (("refs/tags/v1.0.0", 100), ("refs/tags/v1.0.0", 101),
                         ("refs/tags/v1.0.1", 102), ("refs/tags/stable", 103)):
            group, cancel = self.policy(context(ref=ref, run=run))
            groups.append(group)
            self.assertIs(cancel, False)
        self.assertEqual(len(set(groups)), len(groups))
        self.assertNotIn(self.policy(context())[0], groups)

    def test_other_refs_and_event_types_remain_independent(self):
        for event, ref in (("push", "refs/heads/main"), ("push", "refs/heads/topic"),
                           ("workflow_dispatch", "refs/heads/stable"),
                           ("schedule", "refs/heads/stable"),
                           ("pull_request_target", "refs/heads/stable")):
            with self.subTest(event=event, ref=ref):
                first, cancel = self.policy(context(event, ref, run=100))
                second, next_cancel = self.policy(context(event, ref, run=101))
                self.assertNotEqual(first, second)
                self.assertIs(cancel, False)
                self.assertIs(next_cancel, False)

    def test_workflow_namespace_prevents_release_workflow_collisions(self):
        self.assertTrue(self.group.startswith("client-ci-"))
        for event, ref, pr in (("push", "refs/heads/stable", None),
                               ("push", "refs/tags/v1.0.0", None),
                               ("pull_request", "refs/pull/12/merge", 12)):
            ci, _ = self.policy(context(event, ref, pr=pr))
            release, _ = self.policy(context(event, ref, pr=pr, workflow="Release"))
            self.assertNotEqual(ci, release)

    def test_ci_runs_policy_test_and_retains_broad_checks(self):
        self.assertIn("python3 -m unittest discover -s scripts -p test_client_ci_concurrency.py -v",
                      self.workflow)
        self.assertIn('    tags: ["v*"]', self.workflow)
        self.assertIn("    branches: [stable]", self.workflow)
        self.assertIn("  pull_request:\n", self.workflow)
        for job in ("linux", "windows", "msrv"):
            self.assertRegex(self.workflow, rf"(?m)^  {job}:$")
        for check in ("cargo fmt --check", "cargo clippy --all-targets --locked",
                      "cargo test --locked", "cargo check --locked --all-targets",
                      "test_cross_test_bundle.py", "test_production_dogfood.py",
                      "test_toolchain_candidate.py", "test_release_order.py"):
            self.assertIn(check, self.workflow)


if __name__ == "__main__":
    unittest.main()
