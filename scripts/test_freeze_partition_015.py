import unittest

from freeze_partition_015 import freeze, replace_one


class FreezeTests(unittest.TestCase):
    def test_pinned_transplant(self):
        receipt = freeze()
        self.assertEqual(receipt['tree'], 'c371bc11e2f8c92985a494842a4857fd6b7c843a')
        self.assertEqual(receipt['sha256'], '0716ad7c0e8b0ea259978ea80e40b6df49fd938b9bcc0c72ca6d247af4a6e542')
        self.assertEqual(receipt['cargo_tree'], '05548a5ca378f808b9a5901d3e219ed5620f011a')

    def test_ambiguous_or_missing_context_fails_closed(self):
        for text in ('', 'xx'):
            with self.assertRaises(ValueError):
                replace_one(text, 'x', 'y')
