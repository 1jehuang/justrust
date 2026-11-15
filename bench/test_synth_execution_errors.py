"""Missing remote provenance is not evidence of a local fallback."""
import contextlib
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import suite

runner, _ = suite._synth()


class ExecutionErrorTests(unittest.TestCase):
    def test_unverified_remote_rejects_both_admission_failure_and_local_success(self):
        for exit_code in (0, 1):
            with self.subTest(exit=exit_code), contextlib.ExitStack() as ctx:
                plan = SimpleNamespace(snap='fixture', pre=['test'], cmd=['test'],
                                       prev=None, expect_fail=False)
                pre = {'exit': 0, 'wall': 1.0, 'log': '/unmeasured'}
                measured = {'exit': exit_code, 'wall': 0.1, 'log': '/retained',
                            'remote_unverified': True, 'tests_failed': 0,
                            'compile_failed': False}
                ctx.enter_context(patch.object(runner.gen, '_locked'))
                ctx.enter_context(patch.object(runner.gen, 'restore_all'))
                ctx.enter_context(patch.object(runner.gen, 'apply', return_value=[]))
                restored = ctx.enter_context(patch.object(runner.gen, 'restore'))
                ctx.enter_context(patch.object(runner.os, 'sync'))
                ctx.enter_context(patch.object(runner, '_run', side_effect=[pre, measured]))
                keep = ctx.enter_context(patch.object(runner, '_keep_log'))
                result = runner.run_point(SimpleNamespace(), plan, 0)
                self.assertEqual(result['wall'], 0.1)
                self.assertIn('forced remote execution was not verified', result['error'])
                self.assertIn('/retained', result['error'])
                self.assertNotIn('fell back', result['error'])
                restored.assert_called_once_with(plan, [])
                keep.assert_called_once_with(pre)


if __name__ == '__main__':
    unittest.main()
