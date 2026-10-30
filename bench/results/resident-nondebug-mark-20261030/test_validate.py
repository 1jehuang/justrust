#!/usr/bin/env python3
"""Offline semantic mutations with re-signed inventories. Never run native workloads."""
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
spec = importlib.util.spec_from_file_location('nondebug_audit', ROOT / 'validate.py')
audit = importlib.util.module_from_spec(spec)
spec.loader.exec_module(audit)


def encoded(obj):
    return (json.dumps(obj, indent=2) + '\n').encode()


class RejectionControls(unittest.TestCase):
    def signed(self, edits):
        """Retie every changed outer checksum, not the immutable semantic identities."""
        edits = dict(edits)
        for prefix in ('source-ready/v29/', 'source-ready/v29b/', audit.FIXTURE):
            inventory = audit.hash_lines(audit.Evidence(ROOT).text(prefix + 'SHA256SUMS'))
            touched = False
            for path in inventory:
                if prefix + path in edits:
                    inventory[path] = audit.sha(edits[prefix + path])
                    touched = True
            if touched:
                edits[prefix + 'SHA256SUMS'] = ''.join(
                    f'{digest}  {path}\n' for path, digest in inventory.items()).encode()
        manifest = audit.Evidence(ROOT).obj('manifest.json')
        for name, data in edits.items():
            if name != 'manifest.json' and name in manifest['sha256']:
                manifest['sha256'][name] = audit.sha(data) if data is not None else '0' * 64
        edits['manifest.json'] = encoded(manifest)
        return audit.Evidence(ROOT, edits)

    def session(self, updates, name='rsa26'):
        """Retie original session hierarchy including metadata and nested suite copies."""
        prefix = f'sessions/{name}/'
        suite = audit.Evidence(ROOT).obj(prefix + 'suite.json')
        updates = dict(updates)
        if 'metadata.json' in updates:
            suite['metadata'] = json.loads(updates['metadata.json'])
        for file, data in updates.items():
            suite['artifacts_sha256'][file] = audit.sha(data)
        updates['suite.json'] = encoded(suite)
        return {prefix + file: data for file, data in updates.items()}

    def reject(self, edits, message):
        with self.assertRaisesRegex(audit.InvalidEvidence, message):
            audit.validate(self.signed(edits))

    def test_positive_offline_replay(self):
        result = audit.validate(audit.Evidence(ROOT))
        self.assertEqual(result['decision'], 'REJECTED')
        self.assertEqual(result['native_test_counts'], [4, 50, 359])
        self.assertFalse(result['promotion'])

    def test_plain_checksum_corruption(self):
        e = audit.Evidence(ROOT, {'decision.json': b'{}'})
        with self.assertRaisesRegex(audit.InvalidEvidence, 'checksum mismatch'):
            audit.validate(e)

    def test_resigned_promotion_is_rejected(self):
        decision = audit.Evidence(ROOT).obj('decision.json')
        decision['promotion'] = True
        self.reject({'decision.json': encoded(decision)}, 'rejection/no-promotion')

    def test_resigned_source_readiness_cannot_claim_native_proof(self):
        path = audit.FIXTURE + 'ready-v29b.json'
        ready = audit.Evidence(ROOT).obj(path)
        ready['provider_avoidance_proven'] = True
        self.reject({path: encoded(ready)}, 'readiness retconned')

    def test_resigned_candidate_source_change_is_rejected(self):
        path = audit.FIXTURE + 'patch.rs.candidate'
        original = audit.Evidence(ROOT).read(path)
        changed = original.replace(b'assert_eq!(take_provider_counts(), (0, 0)',
                                   b'assert_eq!(take_provider_counts(), (0, 1)', 1)
        self.assertNotEqual(changed, original)
        ready_path = audit.FIXTURE + 'ready-v29b.json'
        ready = audit.Evidence(ROOT).obj(ready_path)
        ready['candidate_patch_sha256'] = audit.sha(changed)
        self.reject({path: changed, ready_path: encoded(ready)}, 'immutable candidate source')

    def test_resigned_patch_reconstruction_change(self):
        path = audit.FIXTURE + 'patch.diff'
        raw = audit.Evidence(ROOT).read(path)
        changed = raw.replace(b'+        root_mark(db, root, debug)', b'+        root_mark(db, root, false)', 1)
        self.assertNotEqual(raw, changed)
        self.reject({path: changed}, 'reconstruction')

    def test_resigned_original_lock_identity(self):
        path = 'provenance/original-Cargo.lock'
        self.reject({path: audit.Evidence(ROOT).read(path) + b'\n'}, 'immutable original lock')

    def test_resigned_native_test_rename_keeps_count_but_fails_names(self):
        run = next(iter(audit.NATIVE))
        path = f'native/unit/raw-runs/{run}/output.jsonl'
        data = audit.Evidence(ROOT).read(path)
        changed = data.replace(b'nondebug_marks_execute_cold_and_reuse_both_debug_keys_in_one_revision', b'nondebug_fake_passing_test')
        self.assertNotEqual(changed, data)
        self.reject({path: changed}, 'individual test names/counts')

    def test_resigned_native_raw_status(self):
        run = next(iter(audit.NATIVE))
        path = f'native/unit/raw-runs/{run}/summary.json'
        obj = audit.Evidence(ROOT).obj(path)
        obj['exit'] = 101
        self.reject({path: encoded(obj)}, 'native raw run identity/status')

    def test_resigned_empty_native_filter(self):
        run = next(iter(audit.NATIVE))
        changes = {}
        for filename in ('meta.json', 'summary.json'):
            path = f'native/unit/raw-runs/{run}/{filename}'
            obj = audit.Evidence(ROOT).obj(path)
            obj['args'][obj['args'].index('nondebug_')] = 'resident_no_matching_tests'
            changes[path] = encoded(obj)
        self.reject(changes, 'native test filter')

    def test_resigned_missing_frozen_source_check(self):
        path = 'native/unit/all-source-after.log'
        self.reject({path: audit.Evidence(ROOT).read(path).split(b'\n', 1)[1]}, 'source changed during')

    def test_resigned_runtime_marker_loss(self):
        data = audit.Evidence(ROOT).read('sessions/rsa26/engine-edit-0.stdout')
        changes = self.session({'engine-edit-0.stdout': data.replace(b'JR_RUNTIME_EXECUTED_1', b'NO_RUNTIME_MARKER')})
        self.reject(changes, 'actual runtime edit marker')

    def test_resigned_type_diagnostic_loss(self):
        data = audit.Evidence(ROOT).read('sessions/rsa26/engine-type-error-0.stderr')
        self.reject(self.session({'engine-type-error-0.stderr': data.replace(b'error[E0308]', b'error[E9999]')}), 'type-error gate')

    def test_resigned_assertion_marker_loss(self):
        data = audit.Evidence(ROOT).read('sessions/rsa26/engine-assert-failure-0.stderr')
        self.reject(self.session({'engine-assert-failure-0.stderr': data.replace(b'JR_RUNTIME_FAILURE', b'OTHER_RUNTIME_FAILURE')}), 'assertion negative gate')

    def test_resigned_missing_recovery_phase(self):
        rows = audit.Evidence(ROOT).obj('sessions/rsa26/records.json')
        rows = [r for r in rows if not (r['stack'] == 'engine' and r['phase'] == 'type-recovery')]
        self.reject(self.session({'records.json': encoded(rows)}), 'runtime test/phase count')

    def test_resigned_source_identity_all_copies(self):
        meta = audit.Evidence(ROOT).obj('sessions/rsa26/metadata.json')
        for field in ('source_before', 'source_after'):
            meta[field]['src/summary.rs'] = '0' * 64
        self.reject(self.session({'metadata.json': encoded(meta)}), 'immutable source identity')

    def test_resigned_toolchain_identity_all_copies(self):
        meta = audit.Evidence(ROOT).obj('sessions/rsa26/metadata.json')
        for field in ('toolchain_before', 'toolchain_after'):
            meta[field]['files']['bin/rustc']['sha256'] = '0' * 64
        self.reject(self.session({'metadata.json': encoded(meta)}), 'immutable toolchain identity')

    def test_resigned_binary_identity_all_copies(self):
        meta = audit.Evidence(ROOT).obj('sessions/rsa26/metadata.json')
        for field in ('tools_before', 'tools_after'):
            meta[field]['engine']['sha256'] = '0' * 64
        self.reject(self.session({'metadata.json': encoded(meta)}), 'immutable binary identity')

    def test_resigned_profile_change(self):
        meta = audit.Evidence(ROOT).obj('sessions/rsa26/metadata.json')
        meta['environments']['engine']['CARGO_PROFILE_TEST_DEBUG'] = '2'
        self.reject(self.session({'metadata.json': encoded(meta)}), 'runtime profile')

    def test_resigned_changed_median(self):
        summary = audit.Evidence(ROOT).obj('sessions/rsa26/summary.json')
        summary['medians_seconds']['engine'] = .001
        self.reject(self.session({'summary.json': encoded(summary)}), 'all-sample median')

    def test_resigned_sample_exclusion(self):
        summary = audit.Evidence(ROOT).obj('sessions/rsa26/summary.json')
        summary['excluded'] = [0]
        self.reject(self.session({'summary.json': encoded(summary)}), 'sample exclusion')

    def test_resigned_restoration_claim_change(self):
        obj = audit.Evidence(ROOT).obj('restoration.json')
        obj['scoped_git_diff_head_clean'] = False
        self.reject({'restoration.json': encoded(obj)}, 'restoration not completed')

    def test_detached_relocation_empty_path(self):
        scratch = os.environ.get('JCODE_SCRATCH_DIR')
        self.assertTrue(scratch, 'explicit scratch required for relocation, never /tmp')
        with tempfile.TemporaryDirectory(prefix='nondebug-mark-offline-', dir=scratch) as temporary:
            destination = Path(temporary) / 'relocated-evidence'
            shutil.copytree(ROOT, destination, ignore=shutil.ignore_patterns('__pycache__'))
            env = {'PATH': '', 'HOME': str(Path(temporary) / 'empty-home')}
            result = subprocess.run([sys.executable, '-I', str(destination / 'validate.py')],
                                    cwd=temporary, env=env, text=True, capture_output=True, timeout=60)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertEqual(json.loads(result.stdout)['decision'], 'REJECTED')


if __name__ == '__main__':
    unittest.main(verbosity=2)
