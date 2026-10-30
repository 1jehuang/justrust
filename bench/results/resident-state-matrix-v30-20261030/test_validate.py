#!/usr/bin/env python3
"""Offline relocation and re-signed semantic mutation tests. No native execution."""
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('rsm02_evidence_validator', HERE / 'validate.py')
v = importlib.util.module_from_spec(spec)
spec.loader.exec_module(v)


class EvidenceTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='matrix-evidence-', dir=os.environ.get('JCODE_SCRATCH_DIR', str(Path.home() / '.jcode/scratch')))
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name) / 'relocated'
        shutil.copytree(HERE, self.root, ignore=shutil.ignore_patterns('__pycache__'))

    def edit_json(self, relative, change):
        path = self.root / relative
        data = v.base(self.root).load(path)
        change(data)
        path.write_text(json.dumps(data, indent=2) + '\n')
        if relative.startswith('runs/'):
            run = relative.split('/')[1]
            transform_path = self.root / 'runs' / run / 'transformation.json'
            transform = v.base(self.root).load(transform_path)
            name = path.name
            if name in transform['derivative_sha256']:
                transform['derivative_sha256'][name] = v.sha(path.read_bytes())
                transform_path.write_text(json.dumps(transform, indent=2) + '\n')
        elif relative.startswith('root/'):
            evidence_path = self.root / 'evidence.json'
            evidence = v.base(self.root).load(evidence_path)
            evidence['root_receipts'][relative[5:]]['retained_sha256'] = v.sha(path.read_bytes())
            evidence_path.write_text(json.dumps(evidence, indent=2) + '\n')
        v.base(self.root).seal(self.root)

    def rejected(self, pattern):
        # Hash inventory is coherent. The rejection must be semantic, not a stale seal.
        v.base(self.root).manifest(self.root)
        with self.assertRaisesRegex(ValueError, pattern):
            v.validate(self.root)

    def test_relocated_no_original_paths_or_processes(self):
        with patch.object(subprocess, 'Popen', side_effect=AssertionError('native process forbidden')):
            result = v.validate(self.root)
        self.assertEqual(result['original_matrix']['rejected'], 10)
        self.assertEqual(result['intrinsic_derivative']['intrinsic_accepted'], 5)
        self.assertEqual(result['intrinsic_derivative']['original_cross_engine_admitted'], 0)
        home = Path(self.temp.name) / 'empty-home'
        home.mkdir()
        env = dict(os.environ, HOME=str(home), PYTHONPATH='', PYTHONDONTWRITEBYTECODE='1')
        process = subprocess.run([sys.executable, str(self.root / 'validate.py')], cwd=home,
                                 env=env, capture_output=True, text=True, timeout=30)
        self.assertEqual(process.returncode, 0, process.stderr)
        self.assertEqual(json.loads(process.stdout)['status'], 'PASS')

    def test_matrix_negative_cannot_be_admitted(self):
        self.edit_json('runs/rsm02/records.sanitized.json', lambda rows: next(r for r in rows if r['phase'] == 'type-error').update(accepted=True, failures=[]))
        self.rejected('oracle|rejected|acceptance|failures')

    def test_matrix_elapsed_cannot_drift_from_summary(self):
        self.edit_json('runs/rsm02/records.sanitized.json', lambda rows: [r.update(elapsed_seconds=r['elapsed_seconds'] + 7) for r in rows if r['phase'] == 'cold-prepare'])
        self.rejected('summary|median')

    def test_nonce_replay_rejected(self):
        self.edit_json('runs/rsm02/records.sanitized.json', lambda rows: rows[1].update(nonce=rows[0]['nonce']))
        self.rejected('nonce|source|input')

    def test_restoration_false_rejected(self):
        self.edit_json('runs/rsm02/metadata.sanitized.json', lambda meta: meta.update(source_restored=False))
        self.rejected('restoration')

    def test_credential_not_permitted_even_with_new_seal(self):
        self.edit_json('runs/rsm02/records.sanitized.json', lambda rows: rows[0]['environment'].update(API_TOKEN='synthetic-not-a-real-secret'))
        self.rejected('environment credential')

    def test_frozen_helper_tampering_rejected_after_reseal(self):
        path = self.root / 'provenance/resident_state_matrix.py'
        path.write_text(path.read_text() + '\n# synthetic mutation\n')
        v.base(self.root).seal(self.root)
        self.rejected('provenance|helper')

    def test_source_tampering_rejected_after_reseal(self):
        path = self.root / 'input/source63/Cargo.toml'
        path.write_bytes(path.read_bytes() + b'\n# synthetic mutation\n')
        v.base(self.root).seal(self.root)
        self.rejected('source63 blob')

    def test_symlink_rejected(self):
        (self.root / 'link').symlink_to('/etc/passwd')
        with self.assertRaisesRegex(ValueError, 'symlink'):
            v.base(self.root).seal(self.root)

    def test_candidate_cross_engine_exclusion_cannot_be_promoted(self):
        self.edit_json('runs/rsm02/records.sanitized.json', lambda rows: next(r for r in rows if r['phase'] == 'type-error' and r['stack'] == 'engine').update(accepted=True, failures=[]))
        self.rejected('candidate oracle admission drift')

    def test_intrinsic_derivative_never_promotes_cross_engine(self):
        self.edit_json('intrinsic-typeerror.json', lambda derivative: derivative['cases'][0].update(original_cross_engine_accepted=True))
        self.rejected('intrinsic derivative')

    def test_intrinsic_derivative_nonce_must_match_raw(self):
        self.edit_json('intrinsic-typeerror.json', lambda derivative: derivative['cases'][0].update(nonce='0' * 32))
        self.rejected('intrinsic derivative')

    def test_typeerror_median_remains_excluded(self):
        self.edit_json('runs/rsm02/summary.sanitized.json', lambda summary: summary['states']['type-error']['engine'].update(excluded=False, median_seconds=0.001))
        self.rejected('median|summary')


if __name__ == '__main__':
    unittest.main()
