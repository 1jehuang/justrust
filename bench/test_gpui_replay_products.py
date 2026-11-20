import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent / 'patches/gpui-test-wrapper'))
from replay_products import finish_successful_trial


class ReplayProducts(unittest.TestCase):
    def test_only_named_products_from_this_trial(self):
        with tempfile.TemporaryDirectory() as root:
            trial = Path(root) / 'trial'
            trial.mkdir()
            products = {'libreplay.so', 'libheld.rmeta', 'libnegative.rmeta', 'libedge.rmeta'}
            evidence = {'held.rs', 'control.diag', 'candidate.diag', 'held.log', 'other.rmeta'}
            for name in products | evidence:
                (trial / name).touch()
            sibling = Path(root) / 'libreplay.so'
            sibling.touch()
            finish_successful_trial(trial, keep=True)
            self.assertEqual({p.name for p in trial.iterdir()}, products | evidence)
            finish_successful_trial(trial, keep=False)
            self.assertEqual({p.name for p in trial.iterdir()}, evidence)
            self.assertTrue(sibling.exists())
