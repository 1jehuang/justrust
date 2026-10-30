"""Mock-only native suite tests. No compiler, engine or native workload invocation."""
import argparse
import contextlib
import copy
import io
import json
from pathlib import Path
import unittest
from unittest.mock import patch

import resident_suite as suite
import test_resident_engine_compare as fixture_module


class ResidentSuiteTests(unittest.TestCase):
    def setUp(self):
        self.fixture = fixture_module.HarnessTests()
        self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)
        self.args = self.fixture.args
        (self.args.toolchain / 'lib/rustlib/src/rust/library/lib.rs').write_text('// mock library source\n')
        self.assertEqual(self.fixture.run_harness(), 0)
        self.source = suite.harness.source_hashes(self.args.source)
        self.root = self.args.output

    def result(self):
        metadata, samples, receipts = suite.replay(self.root, self.source)
        return {'schema': suite.SCHEMA, 'artifacts_root': str(self.root),
                'artifacts_sha256': receipts, 'metadata': metadata,
                'measurements': samples, 'host': {'host': 'mock-host', 'cpu': ['mock-cpu'],
                    'ncpu': 2, 'machine': 'x86_64', 'kernel': 'mock', 'execution': 'local native direct'},
                'noise': 'unknown'}

    def rewrite(self, name, mutate):
        path = self.root / name
        obj = json.loads(path.read_text())
        mutate(obj)
        path.write_text(json.dumps(obj))

    def test_replay_exact_samples_and_receipts(self):
        result = self.result()
        self.assertEqual(result['measurements']['engine']['samples_seconds'], [0.01] * 5)
        self.assertEqual(len(result['artifacts_sha256']), 48)
        with patch.object(suite, 'pinned_hashes', return_value=self.source):
            suite.validate(result)

    def test_raw_marker_replayed_despite_accepted_boolean(self):
        path = self.root / 'engine-edit-0.stdout'
        path.write_text(path.read_text().replace('JR_RUNTIME_EXECUTED_1', 'JR_RUNTIME_EXECUTED_99'))
        with self.assertRaisesRegex(ValueError, 'marker'):
            self.result()

    def test_raw_failure_gate_replayed(self):
        for name, old, new in [('reference-type-error-0.stdout', 'E0308', 'E0000'),
                               ('engine-assert-failure-0.stdout', 'JR_RUNTIME_FAILURE', 'no marker'),
                               ('engine-type-recovery-0.stdout', '4 passed', '0 passed'),
                               ('engine-assert-recovery-0.stdout', suite.harness.NAMES[0], 'wrong::name')]:
            with self.subTest(name=name):
                path = self.root / name
                saved = path.read_bytes()
                path.write_text(saved.decode().replace(old, new))
                with self.assertRaises(ValueError):
                    self.result()
                path.write_bytes(saved)

    def test_missing_log_and_tampered_receipt(self):
        result = self.result()
        path = self.root / 'engine-edit-0.stderr'
        path.write_text('harmless changed bytes')
        with patch.object(suite, 'pinned_hashes', return_value=self.source):
            with self.assertRaisesRegex(ValueError, 'receipt'):
                suite.validate(result)
        path.unlink()
        with self.assertRaises(OSError):
            self.result()

    def test_invalid_rows_fail_closed(self):
        path = self.root / 'records.json'
        original = path.read_bytes()
        mutations = [lambda rows: rows.pop(),
                     lambda rows: rows.append(copy.deepcopy(rows[-1])),
                     lambda rows: rows[-1].update(timed_out=True),
                     lambda rows: rows[-1].update(error='spawn error'),
                     lambda rows: rows[-1].update(source_sha256='stale'),
                     lambda rows: rows[-1].update(command=['wrong']),
                     lambda rows: rows[-1].update(elapsed_seconds=float('nan')),
                     lambda rows: rows[-1].update(elapsed_seconds=-1)]
        for mutate in mutations:
            path.write_bytes(original)
            self.rewrite('records.json', mutate)
            with self.assertRaises(ValueError):
                self.result()
        path.write_bytes(original)

    def test_boolean_acceptance_and_summary_not_authority(self):
        self.rewrite('records.json', lambda rows: [r.update(accepted=False, gate_passed=False, parsed=None) for r in rows])
        self.rewrite('summary.json', lambda summary: summary.update(medians_seconds={'engine': 999}))
        self.assertEqual(self.result()['measurements']['engine']['median_seconds'], 0.01)

    def test_identity_restore_profile_seed_and_policy_rejected(self):
        path = self.root / 'metadata.json'
        original = path.read_bytes()
        mutations = [lambda m: m['source_after'].update({'extra': 'wrong'}),
                     lambda m: m.update(source_restored=False),
                     lambda m: m.update(toolchain_links_valid=False),
                     lambda m: m['tools_after']['engine'].update(sha256='changed'),
                     lambda m: m['toolchain_after']['files'].pop('bin/rustc'),
                     lambda m: m.update(store_scope='seeded'),
                     lambda m: m['environments']['engine'].update(BLAZE_OPT='1'),
                     lambda m: m['environments']['engine'].update(BLAZE_PROC_MACRO_SERVER='different')]
        for mutate in mutations:
            path.write_bytes(original)
            self.rewrite('metadata.json', mutate)
            with self.assertRaises(ValueError):
                self.result()
        path.write_bytes(original)

    def test_descriptive_compare_floor_and_candidate_changes(self):
        a = self.result()
        b = copy.deepcopy(a)
        b['metadata']['tools_before']['engine']['sha256'] = 'candidate changed'
        b['measurements']['engine'] = {'samples_seconds': [0.005] * 5, 'median_seconds': 0.005}
        out = io.StringIO()
        with patch.object(suite, 'validate', side_effect=[(a['metadata'], a['measurements']), (b['metadata'], b['measurements'])]), contextlib.redirect_stdout(out):
            suite.compare(a, b, floor=60)
        text = out.getvalue()
        self.assertIn('-50.00%', text)
        self.assertIn('does not exceed requested 60% floor', text)
        self.assertIn('descriptive/uncontrolled', text)
        self.assertIn('No significance or speedup verdict', text)
        self.assertIn('Reference-normalized', text)

    def test_compare_rejects_mixed_and_mismatched_identity(self):
        a = self.result()
        with self.assertRaisesRegex(ValueError, 'mix'):
            suite.compare(a, {'synth': {}})
        for field in ('host', 'reference', 'toolchain_before', 'profiles', 'proc_macro_policy'):
            b = copy.deepcopy(a)
            if field == 'host':
                b['host']['cpu'] = ['different']
            elif field == 'reference':
                b['metadata']['tools_before']['reference']['sha256'] = 'different'
            else:
                b['metadata'][field] = 'different'
            with patch.object(suite, 'validate', side_effect=[(a['metadata'], a['measurements']), (b['metadata'], b['measurements'])]):
                with self.assertRaises(ValueError):
                    suite.compare(a, b)

    def test_command_writes_distinct_suite_result(self):
        self.args.output = self.fixture.root / 'suite-run'
        with patch.object(suite.harness, 'pinned_source'), patch.object(suite.harness.runtime, 'run_process', side_effect=self.fixture.fake_run), patch.object(suite, 'pinned_hashes', return_value=self.source), patch.object(suite, 'host_identity', return_value={'host': 'mock'}), contextlib.redirect_stdout(io.StringIO()):
            suite.cmd_resident(self.args)
        result = json.loads((self.args.output / 'suite.json').read_text())
        self.assertEqual(result['schema'], suite.SCHEMA)
        self.assertEqual(result['noise'], 'unknown')
        self.assertEqual(len(result['measurements']['reference']['samples_seconds']), 5)

    def test_cli_only_five_samples_no_seed(self):
        parser = argparse.ArgumentParser()
        suite.add_parser(parser.add_subparsers())
        values = ['resident']
        for name in ('reference-engine', 'engine', 'source', 'output', 'toolchain'):
            values += ['--' + name, '/absolute/' + name]
        self.assertEqual(parser.parse_args(values).samples, 5)
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
            parser.parse_args(values + ['--samples', '4'])
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
            parser.parse_args(values + ['--seed', '1'])

    def test_suite_dispatch_before_standard_context(self):
        import suite as standard
        a = self.root / 'a.json'
        a.write_text(json.dumps({'schema': suite.SCHEMA}))
        with patch.object(suite, 'compare') as compare, patch('runtime_context.require_matching_context', side_effect=AssertionError('standard context called')):
            standard.cmd_compare(argparse.Namespace(a=a, b=a, floor=7))
        self.assertEqual(compare.call_args.args[2], 7)


if __name__ == '__main__':
    unittest.main()
