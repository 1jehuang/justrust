import unittest

from bench.proc_macro_profile import rank


def event(name, start, dur, macro=None, tid=1):
    return dict(ph='X', pid=42, tid=tid, ts=start, dur=dur, name=name,
                args={'arg0': macro} if macro else None)


class MacroProfileTests(unittest.TestCase):
    def test_self_time_subtracts_only_immediate_same_thread_children(self):
        macro = 'expand_proc_macro'
        events = [event('inner', 4, 1), event('bridge', 3, 5),
                  event(macro, 10, 2, 'B'), event(macro, 0, 15, 'A'),
                  event('other thread', 0, 14, tid=2), event(macro, 25, 0, 'A')]
        self.assertEqual(rank(events), [dict(macro='A', inclusive_us=15, self_us=8, calls=2),
                                        dict(macro='B', inclusive_us=2, self_us=2, calls=1)])

    def test_equal_rounded_intervals_use_completion_order(self):
        self.assertEqual(rank([event('child', 1, 3), event('expand_proc_macro', 1, 3, 'A')])
                         [0]['self_us'], 0)

    def test_missing_arguments_or_calls_is_not_evidence(self):
        for trace in ([], [event('expand_proc_macro', 0, 1)]):
            with self.assertRaises(ValueError):
                rank(trace)
