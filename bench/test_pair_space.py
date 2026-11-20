"""Synthetic storage admission. No compiler, target cleanup, or guest access."""
import contextlib
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest import mock

from synth import pair_space, run


class PairAdmission(unittest.TestCase):
    def test_shared_and_split_devices(self):
        budget = pair_space.PairSpace(10, {'a': 4, 'b': 7})
        paths = {'a': ['a-target', 'a-cache'], 'b': ['b-target', 'b-cache']}
        probe = {'a-target': (1, 22), 'a-cache': (1, 22), 'b-target': (1, 22), 'b-cache': (2, 18)}
        with mock.patch.object(pair_space, 'filesystem', side_effect=probe.__getitem__):
            budget.admit(paths, ['a', 'b'], 39)
            probe['b-cache'] = (2, 17)
            with self.assertRaisesRegex(pair_space.SpaceDeferred, 'filesystem 2'):
                budget.admit(paths, ['a', 'b'], 39)
            probe['b-cache'] = (2, 18)
            probe['a-target'] = (1, 18)
            budget.admit(paths, ['b'], 39)
            probe['a-cache'] = (1, 9)
            with self.assertRaises(pair_space.SpaceDeferred):
                budget.admit(paths, ['b'], 39)

    def test_unknown_or_zero_never_admitted(self):
        for floor, bounds in [(0, {'a': 1, 'b': 1}), (10, {'a': 1}), (10, {'a': 1, 'b': 0})]:
            with self.assertRaises(ValueError):
                pair_space.PairSpace(floor, bounds).validate(['a', 'b'])
        with tempfile.TemporaryDirectory() as tmp:
            parent = Path(tmp)
            self.assertEqual(pair_space.filesystem(parent / 'new' / 'target')[0], parent.stat().st_dev)
            self.assertEqual(list(parent.iterdir()), [])
            (parent / 'link').symlink_to(parent / 'absent')
            with self.assertRaises(pair_space.SpaceDeferred):
                pair_space.PairSpace(10, {'a': 1, 'b': 1}).admit(
                    {'a': [parent], 'b': [parent / 'link' / 'target']}, ['a', 'b'], 39)

    def test_point_39_is_deferred_before_either_prebuild_or_checkpoint_write(self):
        gib = 1024**3
        stacks = [run.versus.Stack(name, name, True, {}, private_cache=True) for name in ('015', '016')]
        point = SimpleNamespace(i=39)
        plan = SimpleNamespace(point=point, snap='desktop', cmd=['build', '-p', 'jcode-desktop-ui'])
        with tempfile.TemporaryDirectory() as tmp, contextlib.ExitStack() as ctx:
            partial = Path(tmp) / 'previous.json'
            partial.write_bytes(b'previous receipt')
            for name, value in [('_locked', None), ('sample', ([point], {})), ('plan', plan), ('restore_all', None)]:
                ctx.enter_context(mock.patch.object(run.gen, name, return_value=value))
            ctx.enter_context(mock.patch.object(run, 'checkpoint_identity', return_value={}))
            # Old 12 GiB entry floor would pass. Illustrative bounds, not inferred growth.
            ctx.enter_context(mock.patch.object(pair_space, 'filesystem', return_value=(1, 12*gib + 1)))
            measure = ctx.enter_context(mock.patch.object(run, 'run_point'))
            with self.assertRaisesRegex(pair_space.SpaceDeferred, 'point 39 deferred'):
                run.run(stacks, 1, 40, 1, 0, partial=partial,
                        pair_space=pair_space.PairSpace(11*gib, {'015': 2*gib, '016': 2*gib}))
            measure.assert_not_called()
            self.assertEqual(partial.read_bytes(), b'previous receipt')
