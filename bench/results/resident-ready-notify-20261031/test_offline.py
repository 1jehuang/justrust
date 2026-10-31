#!/usr/bin/env python3
"""Offline controls only. Uses Python subprocesses, never resident/native tools."""
import copy
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

sys.dont_write_bytecode = True
import validate

ROOT = Path(__file__).resolve().parent


class EvidenceControls(unittest.TestCase):
    def setUp(self):
        self.data = json.loads((ROOT / 'observations.json').read_text())

    def test_real_selected_observations(self):
        validate.semantic(self.data)
        self.assertEqual(validate.validate_package(ROOT)['strict_rows'], 28)

    def test_semantic_mutations(self):
        # Call the semantic oracle directly: these failures do not depend on checksum
        # mismatch. The separate commitment also guards coherent replacement of history.
        mutations = [
            ('promote', lambda d: d.update(promotion=True)),
            ('speed', lambda d: d.update(speed_claim=True)),
            ('causal regression', lambda d: d.update(causal_regression_claim=True)),
            ('noise', lambda d: d.update(noise='quiet')),
            ('case missing', lambda d: d['cases'].pop('off-repeat')),
            ('row missing', lambda d: d['cases']['on']['rows'].pop()),
            ('on disabled', lambda d: d['cases']['on']['controls'].update(RA_JIT_READY_NOTIFY='0')),
            ('wrong exact value', lambda d: d['cases']['on']['controls'].update(RA_JIT_READY_NOTIFY='true')),
            ('on unexercised', lambda d: d['cases']['on']['rows'][0].update(marker_lines=[])),
            ('off marker', lambda d: d['cases']['off']['rows'][0].update(marker_lines=[validate.MARKER])),
            ('tool identity', lambda d: d['cases']['on'].update(tool_after='0' * 64)),
            ('source changed', lambda d: d['cases']['on'].update(source_after='0' * 64)),
            ('toolchain changed', lambda d: d['cases']['on'].update(toolchain_after='0' * 64)),
            ('nonce replay', lambda d: d['cases']['on']['rows'][1].update(nonce=d['cases']['on']['rows'][0]['nonce'])),
            ('success exit', lambda d: d['cases']['on']['rows'][0].update(status=101)),
            ('type exit', lambda d: d['cases']['on']['rows'][3].update(status=0)),
            ('assert exit', lambda d: d['cases']['on']['rows'][5].update(status=0)),
            ('timeout', lambda d: d['cases']['on']['rows'][0].update(timed_out=True)),
            ('wrong test', lambda d: d['cases']['on']['rows'][0]['parsed']['tests'][0].__setitem__(0, 'fake')),
            ('filtered count', lambda d: d['cases']['on']['rows'][0]['parsed'].update(filtered=143)),
            ('raw result', lambda d: d['cases']['on']['rows'][0].update(stdout='')),
            ('type diagnostic', lambda d: d['cases']['on']['rows'][3].update(diagnostic_lines=[])),
            ('assert diagnostic', lambda d: d['cases']['on']['rows'][5].update(diagnostic_lines=[])),
            ('env injection', lambda d: d['cases']['on']['rows'][0]['safe_environment'].update(SECRET='not-public')),
            ('omissions', lambda d: d['cases']['on']['rows'][0].update(omitted_environment_entries=0)),
            ('history rewrite', lambda d: d['analysis_attempts'][0].update(outcome='passed')),
            ('invent stderr', lambda d: d['analysis_attempts'][0].update(stderr_retained=True)),
            ('gate fail', lambda d: d['native']['check'].update(exit=101)),
            ('fixture missing', lambda d: d['native']['fixtures'].pop()),
            ('fixture failed', lambda d: d['native']['fixtures'].__setitem__(0, d['native']['fixtures'][0].replace('ok <', 'FAILED <'))),
            ('source count', lambda d: d['native'].update(source_check_count=3756)),
            ('lock changed', lambda d: d['native'].update(lock_check_ok=False)),
            ('regression receipt', lambda d: d['native'].update(benchmark_regression_lines=[])),
            ('stop failure', lambda d: d['cleanup']['candidate']['rows'][0].update(status=101)),
            ('store deletion', lambda d: d['cleanup']['candidate'].update(stores_deleted=True)),
            ('restoration', lambda d: d['restoration'].update(head_diff_empty=False)),
            ('pair arithmetic', lambda d: d['root_validation']['pairs'][0]['percent_change'].update({'cold-prepare': 0})),
        ]
        for label, mutate in mutations:
            with self.subTest(label=label):
                data = copy.deepcopy(self.data)
                mutate(data)
                with self.assertRaises((ValueError, KeyError, TypeError)):
                    validate.semantic(data)
        self.assertEqual(len(mutations), 37)

    def test_patch_context_mutation(self):
        original = (ROOT / 'frozen/daemon.rs.original').read_text()
        patch = (ROOT / 'frozen/candidate.patch').read_text()
        with self.assertRaises(ValueError):
            validate.replay_patch(original.replace('Arc, Mutex, MutexGuard, PoisonError,', 'Arc, Changed, MutexGuard, PoisonError,', 1), patch)

    def test_privacy_selection(self):
        raw = (ROOT / 'observations.json').read_text()
        for forbidden in ('/home/', 'Authorization', 'API_KEY', 'RUSTUP_HOME', 'XDG_CACHE_HOME'):
            self.assertNotIn(forbidden, raw)
        for case in self.data['cases'].values():
            for row in case['rows']:
                self.assertEqual(set(row['safe_environment']), set(validate.ENV))
        self.assertNotIn('environment', self.data['cleanup']['candidate']['rows'][0])

    def test_relocation_and_rehashed_history_mutation(self):
        with tempfile.TemporaryDirectory(prefix='ready-notify-offline-',
                                         dir=os.environ.get('JCODE_SCRATCH_DIR')) as scratch:
            scratch = Path(scratch)
            relocated = scratch / 'unrelated' / 'bundle'
            shutil.copytree(ROOT, relocated, ignore=shutil.ignore_patterns('__pycache__'))
            env = {'HOME': str(scratch / 'empty-home'), 'PATH': '', 'PYTHONDONTWRITEBYTECODE': '1'}
            command = [sys.executable, '-I', '-B', str(relocated / 'validate.py'), '--manifest']
            passed = subprocess.run(command, cwd=scratch, env=env, capture_output=True, text=True)
            self.assertEqual(passed.returncode, 0, passed.stderr)
            self.assertEqual(json.loads(passed.stdout)['native_jobs_executed'], 0)
            # Coherently change a historical value and re-sign the mutable inventory.
            data = json.loads((relocated / 'observations.json').read_text())
            data['restoration']['time_utc'] = '2000-01-01T00:00:00Z'
            (relocated / 'observations.json').write_text(json.dumps(data))
            inventory = {str(p.relative_to(relocated)): validate.sha(p.read_bytes())
                         for p in relocated.rglob('*') if p.is_file() and p.name != 'SHA256.json'}
            (relocated / 'SHA256.json').write_text(json.dumps(inventory))
            failed = subprocess.run(command, cwd=scratch, env=env, capture_output=True, text=True)
            self.assertNotEqual(failed.returncode, 0)
            self.assertIn('fixed historical observation commitment', failed.stderr)


if __name__ == '__main__':
    unittest.main(verbosity=2)
