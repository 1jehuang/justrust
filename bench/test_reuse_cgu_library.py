import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('held_lib', Path(__file__).with_name('reuse-cgu-library.py'))
lib = importlib.util.module_from_spec(spec)
spec.loader.exec_module(lib)


class HeldLibrary(unittest.TestCase):
    def test_fixture_is_body_only_and_probe_observes_both_states(self):
        source = b'fn body() {\n' + lib.ANCHOR + b'("a", "b")\n}'
        test = b'    assert!(contents.contains("[menubar]"));'
        with patch.object(subprocess, 'check_output', side_effect=[source, test]) as show:
            original, changed, probe = lib.fixture(Path('/unused'))
        self.assertEqual(show.call_args_list[0].args[0][-1], f'{lib.REV}:{lib.SRC}')
        self.assertEqual(original, source)
        self.assertEqual(len(changed.splitlines()), len(original.splitlines()))
        self.assertIn(lib.MARK, changed)
        self.assertIn(lib.MARK, probe)
        self.assertIn(b'CGU_EDITED', probe)
        with patch.object(subprocess, 'check_output', side_effect=[source + source, test]):
            with self.assertRaisesRegex(ValueError, 'anchor'):
                lib.fixture(Path('/unused'))

    def test_exposure_not_unit_wall(self):
        u = dict(start=1, end=11, rmeta_secs=3)
        units = [u, dict(start=0, end=5), dict(start=6, end=8), dict(start=7, end=10)]
        self.assertEqual(lib.exposed(u, units), 2)
        self.assertEqual(lib.exposed(u, [u]), 7)

    def test_journal_refuses_wrong_toolchain_and_noise(self):
        with tempfile.TemporaryDirectory() as tree:
            root = Path(tree)
            record = root / 'runs/held'
            record.mkdir(parents=True)
            (record / 'units.jsonl').write_text('{}\n')
            env = {'JUSTRUST_HOME': tree}
            out = subprocess.CompletedProcess([], 0, stdout='`justrust show held`')
            with patch.object(lib.suite, 'foreign_load', return_value=(0, 0)), patch.object(subprocess, 'run', return_value=out):
                for pin, foreign, error in [('wrong', 0, 'compiler'), ('pin', 1, 'concurrent'), ('pin', 0, '')]:
                    summary = {'toolchain': pin, 'exit': 0, 'resources': {'max_foreign_rustc': foreign, 'avg_other_cores': 0}}
                    (record / 'summary.json').write_text(json.dumps(summary))
                    if error:
                        with self.assertRaisesRegex(ValueError, error):
                            lib.read_trial(root, env, [], 'pin')
                    else:
                        self.assertEqual(lib.read_trial(root, env, [], 'pin')[:2], (summary, [{}]))
