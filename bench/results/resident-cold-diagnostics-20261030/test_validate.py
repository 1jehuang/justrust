#!/usr/bin/env python3
"""Offline exact-v30 replay controls. Never starts a native engine or build."""
import importlib.util
import json
import os
import re
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('cold_diagnostics_validator', HERE / 'validate.py')
v = importlib.util.module_from_spec(spec)
spec.loader.exec_module(v)


class EvidenceTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='cold-diagnostic-evidence-', dir=os.environ.get('JCODE_SCRATCH_DIR', str(Path.home() / '.jcode/scratch')))
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name) / 'relocated'
        shutil.copytree(HERE, self.root, ignore=shutil.ignore_patterns('__pycache__'))
        self.b = v.base(self.root)

    def update(self, relative, data):
        path = self.root / relative
        path.write_bytes(data)
        evidence = self.b.load(self.root / 'evidence.json')
        if relative.startswith('runs/'):
            parts = relative.split('/')
            run, name = parts[1], '/'.join(parts[2:])
            transform_path = self.root / 'runs' / run / 'transformation.json'
            transform = self.b.load(transform_path)
            if name in transform['derivative_sha256']:
                transform['derivative_sha256'][name] = v.sha(data)
                if not name.endswith('.sanitized.json'):
                    transform['private_original_sha256'][name] = v.sha(data)
                    evidence['private_commitments'][run][name] = v.sha(data)
                transform_path.write_text(json.dumps(transform, indent=2) + '\n')
        elif relative in evidence['receipts']:
            evidence['receipts'][relative]['retained_sha256'] = v.sha(data)
        (self.root / 'evidence.json').write_text(json.dumps(evidence, indent=2) + '\n')
        self.b.seal(self.root)

    def edit_json(self, relative, change):
        value = self.b.load(self.root / relative)
        change(value)
        self.update(relative, (json.dumps(value, indent=2) + '\n').encode())

    def rejected(self, pattern):
        self.b.manifest(self.root)
        with self.assertRaisesRegex(ValueError, pattern):
            v.validate(self.root)

    def test_relocation_empty_home_no_originals_or_native(self):
        with patch.object(subprocess, 'Popen', side_effect=AssertionError('native/process invocation forbidden')):
            result = v.validate(self.root)
        self.assertEqual(result['runs']['p']['accepted'], 7)
        self.assertTrue(result['runs']['p']['candidate_all_four_accepted'])
        self.assertEqual(result['native_gates']['tests'], 359)
        home = Path(self.temp.name) / 'empty-home'
        home.mkdir()
        process = subprocess.run([sys.executable, str(self.root / 'validate.py')], cwd=home,
            env=dict(os.environ, HOME=str(home), PYTHONPATH='', PYTHONDONTWRITEBYTECODE='1'),
            capture_output=True, text=True, timeout=30)
        self.assertEqual(process.returncode, 0, process.stderr)
        self.assertEqual(json.loads(process.stdout)['status'], 'PASS')

    def test_baseline_initial_cold_rejection_mandatory(self):
        self.edit_json('runs/p/records.sanitized.json', lambda rows: rows[0].update(accepted=True, failures=[]))
        self.rejected('oracle drift|acceptance vector')

    def test_candidate_error_not_relabelled_recovery(self):
        self.edit_json('runs/p/records.sanitized.json', lambda rows: rows[4].update(phase='type-recovery'))
        self.rejected('schedule')

    def test_candidate_current_nonce_cannot_replay(self):
        self.edit_json('runs/p/records.sanitized.json', lambda rows: rows[4].update(nonce=rows[6]['nonce']))
        self.rejected('mutation inputs|nonce')

    def test_recovery_counts_not_just_exit_zero(self):
        self.edit_json('runs/p/records.sanitized.json', lambda rows: rows[5]['parsed'].update(passed=3))
        self.rejected('raw parser result')

    def test_candidate_diagnostic_raw_nonce_mutation_with_coherent_receipts(self):
        rows = self.b.load(self.root / 'runs/p/records.sanitized.json')
        row = rows[4]
        for name, receipt in row['raw'].items():
            relative = 'runs/p/' + str(Path(receipt['path']).relative_to('/home/jeremy/p'))
            data = (self.root / relative).read_bytes()
            if row['nonce'].encode() not in data:
                continue
            data = data.replace(row['nonce'].encode(), b'0' * 32)
            self.update(relative, data)
            receipt['sha256'], receipt['bytes'] = v.sha(data), len(data)
        self.update('runs/p/records.sanitized.json', (json.dumps(rows, indent=2) + '\n').encode())
        self.rejected('oracle drift')

    def test_path_limit_failures_never_admitted(self):
        self.edit_json('runs/rstp03/records.sanitized.json', lambda rows: rows[5].update(accepted=True, failures=[]))
        self.rejected('oracle drift|path-limit')

    def test_source_manifest_restoration_false(self):
        self.edit_json('runs/p/metadata.sanitized.json', lambda meta: meta.update(source_restored=False))
        self.rejected('restoration')

    def test_candidate_tool_identity_drift(self):
        self.edit_json('runs/p/metadata.sanitized.json', lambda meta: meta['tools_after']['engine'].update(sha256='0' * 64))
        self.rejected('tool identity')

    def test_source_recipe_drift(self):
        self.edit_json('runs/p/records.sanitized.json', lambda rows: rows[4]['before_phase_inputs'].update({'Cargo.toml': '0' * 64}))
        self.rejected('mutation inputs')

    def test_environment_secret_rejected(self):
        self.edit_json('runs/p/records.sanitized.json', lambda rows: rows[4]['environment'].update(API_TOKEN='synthetic-not-a-real-secret'))
        self.rejected('environment credential')

    def test_native_full359_exit_cannot_be_relabelled(self):
        self.update('journals/unit/full-native.exit', b'1\n')
        self.rejected('full359 gate')

    def test_journal_command_and_status_identity(self):
        self.edit_json('journals/unit/20261030-163303318-1833909/summary.json', lambda summary: summary.update(exit=1))
        self.rejected('journal command/status')

    def test_full359_raw_ignore_drift_rejected(self):
        relative = 'journals/unit/20261030-163303318-1833909/output.jsonl'
        lines = [json.loads(line) for line in (self.root / relative).read_text().splitlines()]
        found = False
        for line in lines:
            if line['l'].startswith('test ') and re.search(r' \.\.\. ok(?: <[0-9.]+s>)?\s*$', line['l']):
                line['l'] = line['l'].replace('... ok', '... ignored', 1)
                found = True
                break
        self.assertTrue(found)
        self.update(relative, ''.join(json.dumps(line) + '\n' for line in lines).encode())
        self.rejected('full359 exact raw')

    def test_patch_exact_context_not_fuzzy(self):
        relative = 'proof/cold-diagnostics.patch'
        data = (self.root / relative).read_bytes().replace(b'@@ -9634', b'@@ -9633', 1)
        self.update(relative, data)
        self.rejected('patch')

    def test_candidate_source_not_repaired_after_reseal(self):
        self.update('proof/main.rs.frozen', (self.root / 'proof/main.rs.original').read_bytes())
        self.rejected('source identities')

    def test_runner_unchanged_between_both_output_roots(self):
        relative = 'provenance/matrix-typeerror-v30-probe.py'
        self.update(relative, (self.root / relative).read_bytes() + b'\n# synthetic mutation\n')
        self.rejected('same exact runner')


if __name__ == '__main__':
    unittest.main()
