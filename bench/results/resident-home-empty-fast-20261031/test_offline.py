#!/usr/bin/env python3
"""Offline replay controls only. No Rust/native builds, tests, daemons, or toolchain probes."""
import copy
import json
import os
import re
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

sys.dont_write_bytecode = True
import validate

ROOT = Path(__file__).resolve().parent


def resign(root):
    inventory = {str(p.relative_to(root)): validate.sha(p.read_bytes()) for p in root.rglob('*')
                 if p.is_file() and p.name != 'SHA256.json' and '__pycache__' not in p.parts}
    (root / 'SHA256.json').write_text(json.dumps(inventory, sort_keys=True))


class EvidenceControls(unittest.TestCase):
    def setUp(self):
        self.data = json.loads((ROOT / 'observations.json').read_text())

    def test_real_selected_observations(self):
        validate.semantic(self.data)
        result = validate.validate_package(ROOT, manifest=True)
        self.assertEqual(result['strict_rows'], 28)
        self.assertEqual(result['native'], [56, 364])
        self.assertEqual(result['fixtures'], 3)

    def test_semantic_mutations_independent_of_hash_commitment(self):
        changes = [
            ('promote', lambda d: d.update(promotion=True)),
            ('speed', lambda d: d.update(speed_claim=True)),
            ('causal', lambda d: d.update(causal_regression_claim=True)),
            ('noise', lambda d: d.update(noise='quiet')),
            ('case coverage', lambda d: d['cases'].pop('off-repeat')),
            ('phase coverage', lambda d: d['cases']['on']['rows'].pop()),
            ('on disabled', lambda d: d['cases']['on']['controls'].update(RA_JIT_HOME_EMPTY_FAST='0')),
            ('not exact flag', lambda d: d['cases']['on']['controls'].update(RA_JIT_HOME_EMPTY_FAST='true')),
            ('on not entered', lambda d: d['cases']['on']['rows'][0].update(marker_lines=[])),
            ('off entered', lambda d: d['cases']['off']['rows'][0].update(marker_lines=[validate.MARKER])),
            ('binary', lambda d: d['cases']['on'].update(tool_after='0' * 64)),
            ('source changed', lambda d: d['cases']['on'].update(source_after='0' * 64)),
            ('toolchain changed', lambda d: d['cases']['on'].update(toolchain_after='0' * 64)),
            ('nonce replay', lambda d: d['cases']['on']['rows'][1].update(nonce=d['cases']['on']['rows'][0]['nonce'])),
            ('success status', lambda d: d['cases']['on']['rows'][0].update(status=101)),
            ('type status', lambda d: d['cases']['on']['rows'][3].update(status=0)),
            ('assert status', lambda d: d['cases']['on']['rows'][5].update(status=0)),
            ('timeout', lambda d: d['cases']['on']['rows'][0].update(timed_out=True)),
            ('named tests', lambda d: d['cases']['on']['rows'][0]['parsed']['tests'][0].__setitem__(0, 'fake')),
            ('filtered', lambda d: d['cases']['on']['rows'][0]['parsed'].update(filtered=143)),
            ('raw result', lambda d: d['cases']['on']['rows'][0].update(stdout='')),
            ('type diagnostic', lambda d: d['cases']['on']['rows'][3].update(diagnostic_lines=[])),
            ('assert diagnostic', lambda d: d['cases']['on']['rows'][5].update(diagnostic_lines=[])),
            ('env injection', lambda d: d['cases']['on']['rows'][0]['safe_environment'].update(SECRET='not-public')),
            ('omissions', lambda d: d['cases']['on']['rows'][0].update(omitted_environment_entries=0)),
            ('source coverage', lambda d: d['cases']['on']['rows'][0]['input_inventory'].pop('Cargo.toml')),
            ('status receipt', lambda d: d['cases']['on']['rows'][0]['status_receipt'].update(status=101)),
            ('provider attribution', lambda d: d['root_analysis'].update(marker_scope='cache hits proven')),
            ('root speed', lambda d: d['root_analysis'].update(speed_claim=True)),
            ('native gate', lambda d: d['native']['check'].update(exit=101)),
            ('native summary', lambda d: d['native']['codegen-native'].update(summaries=[])),
            ('fixture control marker', lambda d: d['fixture_controls']['on'].update(marker_count=0)),
            ('fixture control erased', lambda d: d['fixture_controls'].pop('no-home')),
            ('launch control', lambda d: d['fixture_launch_witness']['cases'][0].update(RA_JIT_HOME_EMPTY_FAST='1')),
            ('launch provenance', lambda d: d['fixture_launch_witness'].update(scope='contemporaneous worker receipt')),
            ('native source count', lambda d: d['native'].update(source_count=3756)),
            ('native lock check', lambda d: d['native'].update(lock_check_ok=False)),
            ('lock edges', lambda d: d['lock_integration'].update(new_packages=1)),
            ('restore', lambda d: d['restoration'].update(source_head_diff_empty=False)),
            ('stop fail', lambda d: d['cleanup']['rows'][0].update(status=101)),
            ('store deletion', lambda d: d['cleanup'].update(stores_deleted=True)),
            ('arithmetic', lambda d: d['root_analysis']['comparisons'][0]['rows'][0].update(percent_on_vs_off=0)),
        ]
        for label, mutate in changes:
            with self.subTest(label=label):
                data = copy.deepcopy(self.data)
                mutate(data)
                with self.assertRaises((ValueError, KeyError, TypeError)):
                    validate.semantic(data)
        self.assertEqual(len(changes), 42)

    def test_coherent_raw_and_parsed_wrong_named_test(self):
        data = copy.deepcopy(self.data)
        row = data['cases']['on']['rows'][0]
        old, new = validate.NAMES[0], 'summary::tests::different_test'
        row['parsed']['tests'][0][0] = new
        row['stdout'] = row['stdout'].replace(old, new)
        # Coherently changing both raw output and parsed receipt must still fail the
        # independent exact named-test oracle, not just a checksum or parser mismatch.
        with self.assertRaisesRegex(ValueError, 'exact named results'):
            validate.semantic(data)

    def test_actual_three_fixture_results_and_four_controls(self):
        lines = json.loads((ROOT / 'native/codegen-results.json').read_text())
        names = [line.split(' ...')[0].removeprefix('test ') for line in lines[:-1] if ' ... ok ' in line]
        self.assertEqual(len(names), 56)
        self.assertEqual({n for n in names if n.startswith('home::empty_fast_tests::v68_real_database_')},
                         {'home::empty_fast_tests::' + n for n in validate.FIXTURES})

    def test_original_oracle_reparse_preserves_tuple_equality(self):
        parse, contract, oracle = validate.original_oracles(ROOT)
        row = self.data['cases']['on']['rows'][0]
        parsed = parse(row['stdout'])
        self.assertEqual(validate.canonical(parsed), validate.canonical(row['parsed']))
        self.assertIsInstance(parsed['tests'][0], tuple)
        self.assertIsInstance(row['parsed']['tests'][0], list)
        expected = contract(parsed, row['phase'])
        # The original oracle still rejects JSON lists if passed without reparsing.
        self.assertNotEqual(oracle(row, row['stdout'], expected, row['phase'], row['nonce'], 0), [])
        self.assertEqual(oracle(dict(row, parsed=parsed), row['stdout'], expected, row['phase'], row['nonce'], 0), [])
        self.assertEqual(validate.replay_original_oracles(ROOT, self.data), 28)

    def test_exact_patch_context_rejects_mutation(self):
        old = (ROOT / 'frozen/baseline/home.rs').read_text()
        patch = (ROOT / 'frozen/change-home.rs.patch').read_text()
        with self.assertRaisesRegex(ValueError, 'patch exact context'):
            validate.replay_patch(old.replace('impl Homes {', 'impl Changed {', 1), patch)

    def test_privacy_allowlists(self):
        text = (ROOT / 'observations.json').read_text()
        for forbidden in ('/home/', 'Authorization', 'API_KEY', 'RUSTUP_HOME', 'XDG_CACHE_HOME'):
            self.assertNotIn(forbidden, text)
        for case in self.data['cases'].values():
            for row in case['rows']:
                self.assertEqual(set(row['safe_environment']), set(validate.ENV))
        self.assertNotIn('environment', self.data['cleanup']['rows'][0])
        for p in ROOT.rglob('*'):
            if p.is_file() and '__pycache__' not in p.parts:
                self.assertIsNone(re.search(rb'/home/[A-Za-z0-9_.-]+/', p.read_bytes()), str(p.relative_to(ROOT)))

    def test_empty_home_path_isolated_relocation_and_resigned_negatives(self):
        with tempfile.TemporaryDirectory(prefix='home-empty-fast-offline-', dir=os.environ.get('JCODE_SCRATCH_DIR')) as scratch:
            scratch = Path(scratch)
            relocated = scratch / 'unrelated' / 'bundle'
            shutil.copytree(ROOT, relocated, ignore=shutil.ignore_patterns('__pycache__'))
            home = scratch / 'empty-home'
            home.mkdir()
            env = {'HOME': str(home), 'PATH': '', 'PYTHONDONTWRITEBYTECODE': '1'}
            command = [sys.executable, '-I', '-B', str(relocated / 'validate.py'), '--manifest']
            result = subprocess.run(command, cwd=scratch, env=env, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(json.loads(result.stdout)['native_jobs_executed'], 0)
            self.assertEqual(list(home.iterdir()), [])
            # Resign all mutable hashes after coherently falsifying the native journal.
            journal = relocated / 'native/codegen-results.json'
            original = journal.read_bytes()
            lines = json.loads(original)
            lines[0] = lines[0].replace(' ... ok ', ' ... FAILED ')
            journal.write_text(json.dumps(lines))
            resign(relocated)
            failed = subprocess.run(command, cwd=scratch, env=env, capture_output=True, text=True)
            self.assertNotEqual(failed.returncode, 0)
            self.assertIn('full native named results', failed.stderr)
            journal.write_bytes(original)
            # Coherent historical time rewrite, including matching root output and pair
            # arithmetic, passes the semantic arithmetic but cannot rewrite fixed history.
            data = copy.deepcopy(self.data)
            row = data['cases']['off']['rows'][0]
            row['elapsed_seconds'] *= 2
            pair = data['root_analysis']['comparisons'][0]['rows'][0]
            pair['off_seconds'] = row['elapsed_seconds']
            pair['percent_on_vs_off'] = 100 * (pair['on_seconds'] / pair['off_seconds'] - 1)
            validate.semantic(data)
            (relocated / 'observations.json').write_text(json.dumps(data))
            (relocated / 'analysis/root-output.json').write_text(json.dumps(data['root_analysis']))
            resign(relocated)
            failed = subprocess.run(command, cwd=scratch, env=env, capture_output=True, text=True)
            self.assertNotEqual(failed.returncode, 0)
            self.assertIn('fixed historical observation commitment', failed.stderr)


if __name__ == '__main__':
    unittest.main(verbosity=2)
