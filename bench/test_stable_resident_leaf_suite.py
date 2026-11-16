#!/usr/bin/env python3
"""Pure Python unit tests. No compiler execution or native receipts produced."""
import argparse
import copy
import io
from pathlib import Path
import unittest
from unittest.mock import patch
from contextlib import redirect_stdout

import stable_resident_leaf_suite as suite


class FixtureTests(unittest.TestCase):
    def test_pinned_three_files_crossfile_edits(self):
        old, _ = suite.fixture(128, 0)
        self.assertEqual(set(old), {'fixture.rs', 'module_a.rs', 'module_b.rs'})
        for revision in range(1, 64):
            new, expected = suite.fixture(128, revision)
            self.assertEqual(len(expected), 256)
            self.assertEqual([n for n in new if old[n] != new[n]],
                             ['module_b.rs' if revision % 2 else 'module_a.rs'])
            self.assertEqual(old['fixture.rs'], new['fixture.rs'])
            old = new
        self.assertEqual(suite.fixture(128, 7), suite.fixture(128, 7))

    def test_lifetime_bounds(self):
        for values in ((0, 2, 1), (1, 64, 1), (1, 1, 1), (1, 2, 0)):
            with self.assertRaises(ValueError):
                suite.workload(*values)
        with self.assertRaises(ValueError):
            suite.fixture(1, 64)

    def test_counterbalance(self):
        orders = [suite.order(i) for i in range(3)]
        for position in range(3):
            self.assertEqual({row[position] for row in orders}, set(suite.ARMS))
        self.assertEqual(suite.order(0), suite.order(3))

    def test_oracle_all_exports_and_negative_corruptions(self):
        _, expected = suite.fixture(8, 5)
        data = '\n'.join(f'define i{v["width"]} @{n}() {{\n ret i{v["width"]} {v["value"]}\n}}'
                         for n, v in expected.items()).encode()
        self.assertEqual(suite.oracle(data, expected), expected)
        variants = [data.replace(b'ret i8 9', b'ret i8 99', 1), data + data,
                    data.replace(b'leaf_b_0007', b'leaf_b_9999'),
                    data.replace(b'ret i8', b'call void @bad()\n ret i8', 1),
                    data.replace(b'ret i64', b'ret i32', 1), b'']
        for corrupt in variants:
            self.assertNotEqual(corrupt, data)
            with self.assertRaises(ValueError):
                suite.oracle(corrupt, expected)

    def test_exact_identity_refusal_without_execution(self):
        with self.assertRaisesRegex(ValueError, 'exact frozen1527 compiler'):
            suite.capture(Path('/usr/bin/rustc'), Path('/never-created'))
        identity = dict(path=str(suite.COMPILER.resolve()), sha256=suite.BINARY_SHA,
                        drivers=[dict(sha256=suite.DRIVER_SHA)])
        suite.check_identity(identity)
        guest = copy.deepcopy(identity)
        guest['path'] = '/home/ubuntu/private-experiment/bin/rustc'
        suite.check_identity(guest)
        guest['sha256'] = '0' * 64
        with self.assertRaisesRegex(ValueError, 'compiler bytes'):
            suite.check_identity(guest)
        identity['drivers'][0]['sha256'] = '0' * 64
        with self.assertRaisesRegex(ValueError, 'driver SHA'):
            suite.check_identity(identity)

    def test_execution_roots_allowed_semantic_changes_refused(self):
        execution = suite.execution_identity()
        first = dict(workspace='/evidence/before/workspace', execution=execution,
                     actual_execution=dict(argv=execution['argv'], cwd='/evidence/before/workspace'))
        second = copy.deepcopy(first)
        second['workspace'] = second['actual_execution']['cwd'] = '/evidence/after/workspace'
        suite.validate_execution(first)
        suite.validate_execution(second)
        self.assertEqual(first['execution'], second['execution'])
        for mutate in ('argv', 'cwd', 'rootnames', 'environment_sha256'):
            bad = copy.deepcopy(first)
            if mutate == 'argv':
                bad['actual_execution']['argv'].append('-Copt-level=3')
            elif mutate == 'cwd':
                bad['actual_execution']['cwd'] = '/different'
            elif mutate == 'rootnames':
                bad['execution']['rootnames'] = ['other.rs']
            else:
                bad['execution']['environment_sha256'] = {}
            with self.assertRaises(ValueError):
                suite.validate_execution(bad)

    def test_parser_api(self):
        parser = argparse.ArgumentParser()
        suite.add_parser(parser.add_subparsers())
        args = parser.parse_args(['stable-native-leaf', '--compiler', str(suite.COMPILER), '--output', '/unused'])
        self.assertEqual(args.items, 128)
        self.assertIs(args.fn, suite.run)


class AggregateTests(unittest.TestCase):
    def rows(self):
        # Synthetic timing-only inputs, never serialized as native evidence.
        return [dict(arm=arm, warmup=warm, validated=True,
                     record=dict(status=0, elapsed_seconds=value))
                for arm in suite.ARMS for warm, value in ((True, 100), (False, 1), (False, 3))]

    def test_warmup_separate(self):
        result = suite.aggregate(self.rows())
        self.assertEqual(result['leaf']['median_seconds'], 2)
        self.assertEqual(result['leaf']['n'], 2)

    def test_errors_unvalidated_missing_and_invalid_time_refused(self):
        for field, value in (('status', 1), ('elapsed_seconds', float('nan')),
                             ('elapsed_seconds', 0), ('elapsed_seconds', True)):
            rows = self.rows()
            rows[1]['record'][field] = value
            with self.assertRaises(ValueError):
                suite.aggregate(rows)
        rows = self.rows()
        rows[1]['validated'] = False
        with self.assertRaises(ValueError):
            suite.aggregate(rows)
        with self.assertRaises(ValueError):
            suite.aggregate([])

    def test_failed_receipt_refused(self):
        for report in ({}, dict(schema=suite.SCHEMA, verdict='failed'), dict(schema='other', verdict='passed')):
            with self.assertRaises(ValueError):
                suite.validate(report)

    def test_strict_comparison_mismatches_and_arm_ratios(self):
        # Mock validation isolates comparison policy without fabricating native receipts.
        original = {key: key for key in ('identity_before', 'workload', 'workload_sha256', 'host', 'execution')}
        stats = {arm: dict(median_seconds=value) for arm, value in zip(suite.ARMS, (4, 2, 1))}
        with patch.object(suite, 'validate', return_value=stats):
            for key in original:
                changed = copy.deepcopy(original)
                changed[key] = 'different'
                with self.assertRaisesRegex(ValueError, key):
                    suite.compare(original, changed, 10)
            output = io.StringIO()
            with redirect_stdout(output):
                suite.compare(original, original, 10)
            self.assertIn('leaf/control=0.500000', output.getvalue())
            self.assertIn('leaf/fresh=0.250000', output.getvalue())
            self.assertIn('NOT broadgate or realproject', output.getvalue())
            for floor in (-1, float('nan'), float('inf')):
                with self.assertRaises(ValueError):
                    suite.compare(original, original, floor)


if __name__ == '__main__':
    unittest.main()
