#!/usr/bin/env python3
"""Portable positive and adversarial receipt tests. No Rust/native execution."""
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('typed_rejection_validate', ROOT / 'validate.py')
v = importlib.util.module_from_spec(spec)
spec.loader.exec_module(v)


class EvidenceTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='typed-rejection-audit-', dir=os.environ.get('JCODE_SCRATCH_DIR'))
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name) / 'moved-bundle'
        shutil.copytree(ROOT, self.root, ignore=shutil.ignore_patterns('__pycache__'))

    def read(self, name):
        return json.loads((self.root / name).read_text())

    def write(self, name, data):
        (self.root / name).write_text(json.dumps(data, indent=2) + '\n')
        self.rehash(name)

    def rehash(self, name):
        manifest = self.read('manifest.json')
        manifest['files_sha256'][name] = v.digest((self.root / name).read_bytes())
        (self.root / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')

    def reject(self, text=None):
        with self.assertRaises((ValueError, KeyError, OSError, TypeError, IndexError)) as caught:
            v.validate(self.root)
        if text:
            self.assertIn(text, str(caught.exception))

    def test_portable_copied_bundle_and_unrelated_cwd(self):
        result = v.validate(self.root)
        self.assertEqual(result['decision'], 'REJECTED_NOT_PROMOTED')
        self.assertEqual(result['metadata_parity_cases'], 6)
        self.assertEqual(len(result['recorder_ids']), 9)
        self.assertEqual(result['profile_rows'], 12)
        self.assertFalse(result['causal_savings_claim'])
        cli = subprocess.run([sys.executable, str(self.root / 'validate.py')], cwd=self.temp.name,
                             capture_output=True, text=True, check=False)
        self.assertEqual(cli.returncode, 0, cli.stderr)
        self.assertEqual(json.loads(cli.stdout), result)

    def test_source_hash_cannot_be_resealed_away(self):
        name = 'candidate/' + v.LIB
        with (self.root / name).open('ab') as output:
            output.write(b'\n// changed candidate\n')
        self.rehash(name)
        self.reject('exact source hash')

    def test_patch_must_reconstruct_exact_sources(self):
        name = 'candidate.patch'
        path = self.root / name
        path.write_text(path.read_text().replace('+pub fn programs_only', '+pub fn changed_name', 1))
        self.rehash(name)
        self.reject('patch does not reproduce')

    def test_nonzero_gate_even_when_rehashed(self):
        name = 'unit-v8/typed_programs.exit'
        (self.root / name).write_text('101\n')
        self.rehash(name)
        self.reject('native gate nonzero')

    def test_rejected_v7_must_remain_failed(self):
        name = 'build-v7/exit'
        (self.root / name).write_text('0\n')
        self.rehash(name)
        self.reject('v7 failed fixture relabeled')

    def test_failed_dispatch_cannot_be_called_passed(self):
        name = 'parity-failed-dispatch/state.json'
        state = self.read(name)
        state['status'] = 'passed'
        self.write(name, state)
        self.reject('initial dispatch failure')

    def test_metadata_stdout_byte_difference(self):
        name = 'parity-six-cases/full-new.stdout'
        with (self.root / name).open('ab') as output:
            output.write(b' ')
        self.rehash(name)
        self.reject('metadata stdout/stderr/status differ')

    def test_metadata_dispatch_not_optional(self):
        name = 'parity-six-cases/state.json'
        state = self.read(name)
        state['rows'][0]['command'].remove('cargo')
        self.write(name, state)
        self.reject('metadata dispatch/flags')

    def test_metadata_rejection_status(self):
        name = 'parity-six-cases/state.json'
        state = self.read(name)
        for row in state['rows']:
            if row['case'] == 'missing-feature':
                row['status'] = 0
        self.write(name, state)
        self.reject('metadata rejection status')

    def test_metadata_all_six_cases_required(self):
        name = 'parity-six-cases/state.json'
        state = self.read(name)
        state['rows'] = state['rows'][:-2]
        self.write(name, state)
        self.reject('six parity cases')

    def test_profile_all_warmups_and_edits_required(self):
        name = 'profile/state.json'
        state = self.read(name)
        state['rows'].pop(0)
        self.write(name, state)
        self.reject('12-row alternating order')

    def test_profile_raw_event_cannot_disagree_with_summary(self):
        name = 'profile/new-0.trace.json'
        trace = self.read(name)
        for event in trace['traceEvents']:
            if event.get('name') == 'client planning' and event.get('ph') == 'X':
                event['dur'] += 100
        self.write(name, trace)
        state = self.read('profile/state.json')
        next(row for row in state['rows'] if row['stack'] == 'new' and row['replicate'] == 0)['trace_sha256'] = v.digest((self.root / name).read_bytes())
        self.write('profile/state.json', state)
        self.reject('raw trace phase mismatch')

    def test_profile_restoration_required(self):
        name = 'profile/state.json'
        state = self.read(name)
        state['source_restored'] = False
        self.write(name, state)
        self.reject('profile restoration/identity')

    def test_profile_fresh_runtime_marker_required(self):
        name = 'profile/new-0.stdout'
        path = self.root / name
        path.write_text(path.read_text().replace('JR_RUNTIME_EXECUTED_501', 'STALE_RESULT'))
        self.rehash(name)
        self.reject('profile fresh edit marker')

    def test_recorder_export_id_must_match_raw(self):
        name = 'recorder-exports/native-v8-build/2026-10.jsonl'
        path = self.root / name
        rows = [json.loads(line) for line in path.read_text().splitlines()]
        rows[0]['id'] = 'invented'
        path.write_text(''.join(json.dumps(row) + '\n' for row in rows))
        self.rehash(name)
        self.reject('slim/raw recorder IDs differ')

    def test_recorder_export_counts_must_match_raw(self):
        name = 'recorder-exports/native-v8-build/2026-10.jsonl'
        path = self.root / name
        rows = [json.loads(line) for line in path.read_text().splitlines()]
        rows[1]['tests']['passed'] = 500
        path.write_text(''.join(json.dumps(row) + '\n' for row in rows))
        self.rehash(name)
        self.reject('slim/raw test counts')

    def test_suite_median_recomputed_from_raw_samples(self):
        name = 'rsa05/suite.json'
        suite = self.read(name)
        suite['measurements']['engine']['median_seconds'] = 0.0001
        self.write(name, suite)
        self.reject('median mismatch')

    def test_suite_runtime_marker_not_accepted_boolean_alone(self):
        name = 'rsa05/engine-edit-0.stdout'
        path = self.root / name
        path.write_text(path.read_text().replace('JR_RUNTIME_EXECUTED_1', 'STALE'))
        self.rehash(name)
        suite = self.read('rsa05/suite.json')
        suite['artifacts_sha256']['engine-edit-0.stdout'] = v.digest(path.read_bytes())
        self.write('rsa05/suite.json', suite)
        self.reject('fresh runtime marker')

    def test_missing_raw_artifact(self):
        (self.root / 'profile/old-0.trace.json').unlink()
        self.reject('inventory')

    def test_unindexed_file(self):
        (self.root / 'extra.txt').write_text('unindexed\n')
        self.reject('inventory')

    def test_no_binary_payload(self):
        name = 'extra.bin'
        (self.root / name).write_bytes(b'\x7fELFnot-a-real-executable')
        self.rehash(name)
        self.reject('binary/cache')

    def test_no_symlinks_or_path_escape(self):
        (self.root / 'escape').symlink_to(self.temp.name, target_is_directory=True)
        self.reject('symlink')

    def test_rejection_cannot_be_promoted(self):
        manifest = self.read('manifest.json')
        manifest['decision'] = 'PROMOTED'
        (self.root / 'manifest.json').write_text(json.dumps(manifest))
        self.reject('rejection/provenance')


if __name__ == '__main__':
    unittest.main(verbosity=2)
