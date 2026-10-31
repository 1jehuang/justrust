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
spec = importlib.util.spec_from_file_location('rsm05_evidence_validator', HERE / 'validate.py')
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
        elif relative.startswith(('probes/', 'journals/')):
            evidence_path = self.root / 'evidence.json'
            evidence = v.base(self.root).load(evidence_path)
            evidence['receipts'][relative]['retained_sha256'] = v.sha(path.read_bytes())
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
        self.assertEqual(result['matrices']['rsm05']['rejected'], 10)
        self.assertEqual(result['intrinsic_only']['rsm05']['accepted'], 5)
        self.assertEqual(result['intrinsic_only']['rsm05']['original_cross_engine_admitted'], 0)
        home = Path(self.temp.name) / 'empty-home'
        home.mkdir()
        env = dict(os.environ, HOME=str(home), PYTHONPATH='', PYTHONDONTWRITEBYTECODE='1')
        process = subprocess.run([sys.executable, str(self.root / 'validate.py')], cwd=home,
                                 env=env, capture_output=True, text=True, timeout=30)
        self.assertEqual(process.returncode, 0, process.stderr)
        self.assertEqual(json.loads(process.stdout)['status'], 'PASS')

    def test_matrix_negative_cannot_be_admitted(self):
        self.edit_json('runs/rsm05/records.sanitized.json', lambda rows: next(r for r in rows if r['phase'] == 'type-error').update(accepted=True, failures=[]))
        self.rejected('oracle|rejected|acceptance|failures')

    def test_matrix_elapsed_cannot_drift_from_summary(self):
        self.edit_json('runs/rsm05/records.sanitized.json', lambda rows: [r.update(elapsed_seconds=r['elapsed_seconds'] + 7) for r in rows if r['phase'] == 'cold-prepare'])
        self.rejected('summary|median')

    def test_nonce_replay_rejected(self):
        self.edit_json('runs/rsm05/records.sanitized.json', lambda rows: rows[1].update(nonce=rows[0]['nonce']))
        self.rejected('nonce|source|input')

    def test_restoration_false_rejected(self):
        self.edit_json('runs/rsm05/metadata.sanitized.json', lambda meta: meta.update(source_restored=False))
        self.rejected('restoration')

    def test_credential_not_permitted_even_with_new_seal(self):
        self.edit_json('runs/rsm05/records.sanitized.json', lambda rows: rows[0]['environment'].update(API_TOKEN='synthetic-not-a-real-secret'))
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
        self.edit_json('runs/rsm05/records.sanitized.json', lambda rows: next(r for r in rows if r['phase'] == 'type-error' and r['stack'] == 'engine').update(accepted=True, failures=[]))
        self.rejected('candidate oracle admission drift')

    def test_intrinsic_derivative_never_promotes_cross_engine(self):
        self.edit_json('intrinsic-typeerror.json', lambda derivative: derivative['rsm05']['cases'][0].update(original_cross_engine_accepted=True))
        self.rejected('intrinsic derivative')

    def test_intrinsic_derivative_nonce_must_match_raw(self):
        self.edit_json('intrinsic-typeerror.json', lambda derivative: derivative['rsm05']['cases'][0].update(nonce='0' * 32))
        self.rejected('intrinsic derivative')

    def test_typeerror_median_remains_excluded(self):
        self.edit_json('runs/rsm05/summary.sanitized.json', lambda summary: summary['states']['type-error']['engine'].update(excluded=False, median_seconds=0.001))
        self.rejected('median|summary')


    def test_reference_demand_flag_forbidden(self):
        self.edit_json('runs/rsm05/records.sanitized.json', lambda rows: rows[0]['environment'].update(RA_JIT_DEMAND_STD='1'))
        self.rejected('environment drift')

    def test_engine_demand_flag_required(self):
        self.edit_json('runs/rsm05/records.sanitized.json', lambda rows: next(r for r in rows if r['stack'] == 'engine')['environment'].pop('RA_JIT_DEMAND_STD'))
        self.rejected('environment drift')

    def test_default_control_engine_flag_forbidden(self):
        self.edit_json('runs/rsm08/records.sanitized.json', lambda rows: next(r for r in rows if r['stack'] == 'engine')['environment'].update(RA_JIT_DEMAND_STD='1'))
        self.rejected('environment drift')

    def test_default_control_intrinsic_not_promoted(self):
        self.edit_json('intrinsic-typeerror.json', lambda derivative: derivative['rsm08']['cases'][0].update(original_cross_engine_accepted=True))
        self.rejected('intrinsic derivative')

    def test_metadata_optin_cannot_be_default(self):
        self.edit_json('runs/rsm05/metadata.sanitized.json', lambda meta: meta.update(engine_demand_std=False))
        self.rejected('startup control')

    def test_legacy_failure_not_promoted(self):
        self.edit_json('probes/v31/state.sanitized.json', lambda state: state.update(status='passed'))
        self.rejected('failed opt-in prototype')

    def test_legacy_control_receipt_not_fabricated(self):
        self.edit_json('probes/v33/state.sanitized.json', lambda state: state['rows'][1]['environment'].update(RA_JIT_DEMAND_STD='1'))
        self.rejected('serialized-control absence')

    def test_aborted_finally_not_fabricated(self):
        self.edit_json('runs/rsm03/metadata.sanitized.json', lambda meta: meta.update(source_restored=True))
        self.rejected('fabricated aborted')

    def test_native_full359_failure_rejected(self):
        path = next((self.root / 'journals/v33/unit').glob('*/summary.json'))
        self.edit_json(str(path.relative_to(self.root)), lambda summary: summary.update(exit=1))
        self.rejected('native journal command/status')

    def test_source_patch_tamper_rejected(self):
        path = self.root / 'proof/v33/source.patch'
        path.write_text(path.read_text().replace('demand_std_enabled()', 'disabled_std_enabled()', 1))
        evidence_path = self.root / 'evidence.json'
        evidence = v.base(self.root).load(evidence_path)
        evidence['receipts']['proof/v33/source.patch'].update(retained_sha256=v.sha(path.read_bytes()), original_sha256=v.sha(path.read_bytes()))
        evidence_path.write_text(json.dumps(evidence, indent=2) + '\n')
        v.base(self.root).seal(self.root)
        self.rejected('source patch replay|patch old context')

    def test_timeout_finally_not_fabricated(self):
        self.edit_json('runs/rsm07/metadata.sanitized.json', lambda meta: meta.update(source_restored=True))
        self.rejected('fabricated timeout')

    def test_unsupported_control_not_promoted(self):
        self.edit_json('runs/rsm06/records.sanitized.json', lambda rows: rows[0].update(accepted=True))
        self.rejected('unsupported control')

    def test_stop_outcomes_are_not_universal_success(self):
        relative = 'cleanup/receipt.json'
        path = self.root / relative
        data = v.base(self.root).load(path)
        next(row for row in data['rows'] if row['status'] == 1)['status'] = 0
        path.write_text(json.dumps(data, indent=2) + '\n')
        evidence_path = self.root / 'evidence.json'
        evidence = v.base(self.root).load(evidence_path)
        evidence['receipts'][relative]['retained_sha256'] = v.sha(path.read_bytes())
        evidence_path.write_text(json.dumps(evidence, indent=2) + '\n')
        v.base(self.root).seal(self.root)
        self.rejected('scoped native stop outcomes')

    def test_mixed_shape_cannot_hide_credential(self):
        self.edit_json('runs/rsm05/records.sanitized.json', lambda rows: rows[0]['environment'].update(API_TOKEN='synthetic-not-a-real-secret', OTHER=1))
        self.rejected('environment receipt shape')

    def test_resigned_raw_stale_error_nonce_not_accepted(self):
        rows_path = self.root / 'runs/rsm05/records.sanitized.json'
        rows = v.base(self.root).load(rows_path)
        row = next(row for row in rows if row['phase'] == 'type-error' and row['stack'] == 'engine')
        found = False
        transform_path = self.root / 'runs/rsm05/transformation.json'
        transform = v.base(self.root).load(transform_path)
        for receipt in row['raw'].values():
            relative = Path(receipt['path']).relative_to('/home/jeremy/rsm05')
            path = self.root / 'runs/rsm05' / relative
            raw = path.read_bytes()
            if row['nonce'].encode() not in raw:
                continue
            found = True
            raw = raw.replace(row['nonce'].encode(), b'0' * 32)
            path.write_bytes(raw)
            receipt.update(sha256=v.sha(raw), bytes=len(raw))
            transform['private_original_sha256'][str(relative)] = v.sha(raw)
            transform['derivative_sha256'][str(relative)] = v.sha(raw)
        self.assertTrue(found)
        rows_path.write_text(json.dumps(rows, indent=2) + '\n')
        transform['derivative_sha256'][rows_path.name] = v.sha(rows_path.read_bytes())
        transform_path.write_text(json.dumps(transform, indent=2) + '\n')
        evidence_path = self.root / 'evidence.json'
        evidence = v.base(self.root).load(evidence_path)
        evidence['private_commitments']['rsm05'] = transform['private_original_sha256']
        evidence_path.write_text(json.dumps(evidence, indent=2) + '\n')
        v.base(self.root).seal(self.root)
        self.rejected('candidate oracle admission')

    def test_native_full359_raw_name_status_required(self):
        path = next((self.root / 'journals/v33/unit').glob('*/output.jsonl'))
        rows = [json.loads(line) for line in path.read_text().splitlines()]
        changed = False
        for row in rows:
            if row.get('l', '').startswith('test ') and ' ... ok' in row['l']:
                row['l'] = row['l'].replace(' ... ok', ' ... FAILED', 1)
                changed = True
                break
        self.assertTrue(changed)
        path.write_text(''.join(json.dumps(row) + '\n' for row in rows))
        evidence_path = self.root / 'evidence.json'
        evidence = v.base(self.root).load(evidence_path)
        evidence['receipts'][str(path.relative_to(self.root))]['retained_sha256'] = v.sha(path.read_bytes())
        evidence_path.write_text(json.dumps(evidence, indent=2) + '\n')
        v.base(self.root).seal(self.root)
        self.rejected('full359 exact raw')


if __name__ == '__main__':
    unittest.main()
