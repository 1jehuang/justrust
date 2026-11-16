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


class ClientCIConcurrencyTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        blocks = re.findall(r"^concurrency:\n((?:[ \t]+[^\n]*\n)+)", cls.workflow, re.MULTILINE)
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
        cls.group = fields["group"]
        match = re.fullmatch(r"\$\{\{(.*?)\}\}", fields["cancel-in-progress"])
        if not match:
            raise AssertionError("cancel-in-progress must be an explicit expression")
        cls.cancel = match[1]

    def policy(self, ctx):
        return render(self.group, ctx).lower(), expression(self.cancel, ctx)

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
        for job in ("linux", "macos", "windows", "msrv"):
            self.assertRegex(self.workflow, rf"(?m)^  {job}:$")
        for check in ("cargo fmt --check", "cargo clippy --all-targets --locked",
                      "cargo test --locked", "cargo check --locked --all-targets",
                      "test_cross_test_bundle.py", "test_production_dogfood.py",
                      "test_toolchain_candidate.py", "test_release_order.py"):
            self.assertIn(check, self.workflow)


if __name__ == "__main__":
    unittest.main()
