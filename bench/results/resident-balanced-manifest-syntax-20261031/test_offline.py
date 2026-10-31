#!/usr/bin/env python3
"""Offline positives and coherently resealed semantic counterexamples, stdlib only."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('v82_validator', ROOT / 'validate.py')
validator = importlib.util.module_from_spec(spec)
spec.loader.exec_module(validator)


def emit(root, name, data):
    (root / name).write_text(json.dumps(data, indent=2, sort_keys=True) + '\n')


def reseal(root):
    # Deliberately update the attacker-editable outer seal. Rejections below
    # must come from semantic checks, not a stale generic checksum.
    emit(root, 'SHA256.json', {p.relative_to(root).as_posix(): {'sha256': hashlib.sha256(p.read_bytes()).hexdigest(), 'bytes': p.stat().st_size} for p in sorted(root.rglob('*')) if p.is_file() and p.name != 'SHA256.json'})


class Replay(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='v82-offline-', dir=os.environ.get('JCODE_SCRATCH_DIR'))
        self.root = Path(self.temp.name) / 'relocated'
        shutil.copytree(ROOT, self.root)
        self.data = json.loads((self.root / 'observations.json').read_text())

    def tearDown(self):
        self.temp.cleanup()

    def reject(self, pattern, historical=False):
        emit(self.root, 'observations.json', self.data)
        reseal(self.root)
        with self.assertRaisesRegex(ValueError, pattern):
            validator.validate(self.root, manifest=True, historical=historical)

    def test_positive_semantics(self):
        result = validator.validate(self.root, manifest=True)
        self.assertEqual((result['abba_calls'], result['timed_samples'], result['promotion']), (54, 20, 'REJECT'))

    def test_isolated_relocation_empty_home_path(self):
        completed = subprocess.run([sys.executable, '-I', '-S', '-B', str(self.root / 'offline_sandbox.py'), str(self.root)], env={'HOME': '', 'PATH': ''}, capture_output=True, text=True, check=True)
        receipt = json.loads(completed.stdout)
        self.assertEqual(len(receipt['denied_probes']), 6)
        self.assertEqual(receipt['isolated_replay']['promotion'], 'REJECT')

    def test_unknown_top_field(self):
        self.data['ambient_environment'] = {}
        self.reject('unexpected fields')

    def test_promotion_rewrite(self):
        self.data['contract']['promotion'] = 'ACCEPT'
        self.reject('rejection/scope')

    def test_noise_rewrite(self):
        self.data['abba']['rows'][0]['noise']['contamination'] = 'clean'
        self.reject('noise must remain unknown')

    def test_native_named_failure_coherent_journal(self):
        gate = self.data['gates'][2]
        rows = json.loads((self.root / gate['raw_journal']).read_text())
        for row in rows:
            if row['l'].startswith('test ') and ' ... ok' in row['l']:
                row['l'] = row['l'].replace(' ... ok', ' ... FAILED')
                break
        emit(self.root, gate['raw_journal'], rows)
        self.reject('native named test count/status')

    def test_native_inventory_missing(self):
        p = self.root / 'gates/all-source-before.sha256'
        p.write_text('\n'.join(p.read_text().splitlines()[1:]) + '\n')
        self.reject('actual native source count')

    def test_profile_trace_coherently_rewritten(self):
        row = self.data['profile']['rows'][0]
        event = next(e for e in row['events'] if e['name'] == 'resolver fetched manifest syntax')
        event['ts'] = 1
        emit(self.root, 'profile/0-edit.trace.json', {'traceEvents': row['events']})
        self.reject('subspan outside')

    def test_strict_diagnostic_removed(self):
        p = self.root / 'strict/on/03-type-error.stderr'
        p.write_text(p.read_text().replace('E0308', 'E9999'))
        self.reject('original strict oracle')

    def test_strict_parsed_and_raw_contract_coherently_weakened(self):
        row = self.data['strict'][0]['rows'][0]
        for suffix in ('stdout', 'stderr'):
            p = self.root / ('strict/off/00-cold-prepare.' + suffix)
            p.write_text(p.read_text().replace('144 filtered out', '143 filtered out'))
        row['parsed']['filtered'] = 143
        self.reject('original strict oracle|strict original reference tuple')

    def test_strict_multiple_modes_cannot_alias_controls(self):
        self.data['strict'][1]['startup_controls']['RA_JIT_PARALLEL_MANIFEST_PARSE'] = '0'
        self.reject('strict fixed startup')

    def test_strict_stale_nonce(self):
        self.data['strict'][0]['rows'][1]['nonce'] = self.data['strict'][0]['rows'][0]['nonce']
        self.reject('strict stale nonce')

    def test_abba_first_touch_dropped(self):
        del self.data['abba']['rows'][13]
        self.reject('54-call exact layout')

    def test_abba_start_tick_changed(self):
        row = self.data['abba']['rows'][13]
        stat = row['worker']['stat']
        prefix, tail = stat.split(') ', 1)
        fields = tail.split()
        fields[19] = str(int(fields[19]) + 1)
        row['worker']['stat'] = prefix + ') ' + ' '.join(fields)
        self.reject('PID/start/exe/socket/argv changed')

    def test_abba_bytes_changed_on_touch(self):
        self.data['abba']['rows'][13]['touches'][0]['after']['sha256'] = '0' * 64
        self.reject('touch changed bytes/size')

    def test_abba_time_changed_without_result(self):
        self.data['abba']['rows'][13]['elapsed_seconds'] *= 1.01
        self.reject('result/sample/median replay')

    def test_coherent_time_rewrite_still_not_historical(self):
        self.data['abba']['rows'][13]['elapsed_seconds'] *= 1.001
        values = {'off': [], 'on': []}
        for row in self.data['abba']['rows']:
            if row['phase'] == 'retouch':
                values[self.data['abba']['metadata']['order'][int(row['group'][-1])]].append(row['elapsed_seconds'])
        import statistics
        measurements = {arm: {'samples_seconds': samples, 'median_seconds': statistics.median(samples)} for arm, samples in values.items()}
        for arm in values:
            p = self.root / ('abba/' + arm + '-result.json')
            saved = json.loads(p.read_text())
            saved['measurements'] = measurements
            emit(self.root, p.relative_to(self.root).as_posix(), saved)
        self.reject('fixed historical observations', historical=True)

    def test_marker_removed(self):
        p = self.root / 'abba/call-025.stderr'
        p.write_text(p.read_text().replace(validator.MARKER, 'missing-parallel-marker'))
        self.reject('parallel|marker|activation')

    def test_duplicate_marker(self):
        p = self.root / 'abba/call-025.stderr'
        p.write_text(p.read_text() + validator.MARKER + '\n')
        self.reject('parallel|marker|activation')

    def test_source_patch_context_changed(self):
        p = self.root / 'source/originals/toolchain/resident/crates/blaze-manifest/src/manifest.rs'
        p.write_text(p.read_text().replace('pub fn read_manifest(', 'pub fn broken_manifest(', 1))
        self.reject('patch context/deletion|original/candidate patch join')

    def test_oracle_cannot_be_coherently_weakened(self):
        p = self.root / 'oracle/resident_state_matrix.py'
        p.write_text(p.read_text() + '\n# oracle tampered\n')
        self.reject('original oracle code changed')

    def test_private_path_rejected(self):
        (self.root / 'leak.txt').write_text('/' + 'home/private/secret')
        self.reject('private absolute path')

    def test_binary_rejected(self):
        (self.root / 'payload').write_bytes(b'\x7fELFexample')
        self.reject('binary payload')

    def test_restoration_cannot_claim_current_checkout(self):
        p = self.root / 'restoration/receipt.json'
        receipt = json.loads(p.read_text())
        receipt['scope'] = 'current native checkout'
        emit(self.root, p.relative_to(self.root).as_posix(), receipt)
        self.reject('restoration scope/controls')

    def test_restoration_original_sha_rewrite(self):
        p = self.root / 'restoration/receipt.json'
        receipt = json.loads(p.read_text())
        item = next(iter(receipt['files'].values()))
        item['preserved_live_original']['sha256'] = '0' * 64
        emit(self.root, p.relative_to(self.root).as_posix(), receipt)
        self.reject('restoration original SHA join')

    def test_formatting_failure_cannot_become_native_failure(self):
        p = self.root / 'formatting/receipt.json'
        receipt = json.loads(p.read_text())
        receipt['native_failure'] = True
        emit(self.root, p.relative_to(self.root).as_posix(), receipt)
        self.reject('formatting-only historical receipt')

    def test_adapter_cannot_mutate_frozen_source(self):
        p = self.root / 'drivers/suite-frozen-parallel-v81.py'
        p.write_text(p.read_text() + '\np.write_text("mutation")\n')
        self.reject('private adapter must not mutate')

    def test_adapter_cannot_drop_explicit_git_binding(self):
        p = self.root / 'drivers/suite-frozen-parallel-v81.py'
        p.write_text(p.read_text().replace("module.__file__=str(bench/'resident_suite.py')", 'module.__file__="unbound"'))
        self.reject('explicit pinned Git authority binding')

    def test_abba_command_cannot_alias_other_binary(self):
        self.data['abba']['rows'][13]['command'][0] = '[other]/engine'
        self.reject('command')

    def test_store_deletion_receipt_rejected(self):
        self.data['cleanup']['stores_deleted'] = True
        self.reject('cleanup scope/stores')


if __name__ == '__main__':
    unittest.main(verbosity=2)
