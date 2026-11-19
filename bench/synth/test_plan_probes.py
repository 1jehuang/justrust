"""Exercise the actual planner without builds, hosted access, or snapshots."""
import sys
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import suite

gen = suite._synth()[0].gen


class PlanProbeTests(unittest.TestCase):
    def exercise(self, kind, expected, calls, *, files=1, root=True, command='check'):
        base = Path('/fixture')
        targets = [{'kind': ['lib'], 'src_path': str(base / 'lib.rs')}] if root else []
        pkg = gen.Pkg('pkg', base, base / 'Cargo.toml', targets, True)
        ws = gen.Workspace('fixture', 'snap', base, base, {'pkg': pkg}, ['pkg'], ['pkg'])
        shape = dict(repo='fixture', pkg='pkg', flags='', command=command, change=kind,
                     files=files, crates=1, relation='target')
        with patch.object(gen, 'home_workspace', return_value=ws), patch.object(
                gen, 'probe', return_value={'module.rs': {'body': [7], 'test': []}}) as probe:
            plan = gen.plan(gen.Point(0, shape, 1), 42)
        self.assertEqual([e.label() for e in plan.edits], expected)
        self.assertEqual(plan.expect_fail, kind == 'type-error')
        self.assertEqual(plan.note, '')
        self.assertEqual(plan.cmd, (['test', '-p', 'pkg', '--', 'tests'] if command == 'test-filtered'
                                    else ['check', '-p', 'pkg']))
        self.assertEqual(probe.call_count, calls)

    def test_planner_discovery_counts(self):
        for kind, name in [('manifest', 'Cargo.toml'), ('new-file', 'lib.rs')]:
            with self.subTest(kind=kind):
                first = [f'{kind}:pkg:{name}']
                self.exercise(kind, first, 0)
                self.exercise(kind, first + ['body:pkg:module.rs:8'], 1, files=2)
                self.exercise(kind, first, 1, command='test-filtered')
        self.exercise('new-file', ['body:pkg:module.rs:8'], 1, root=False)
        self.exercise('item', ['item:pkg:module.rs'], 1)
        self.exercise('test', ['test:pkg:module.rs:8'], 2)
        self.exercise('type-error', ['type-error:pkg:module.rs:8'], 1)
