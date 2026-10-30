#!/usr/bin/env python3
"""Mutation tests for offline audit only. Never compiles or executes resident engines."""
import json
import os
from pathlib import Path
import shutil
import tempfile
import unittest

import validate as audit

ROOT = Path(__file__).resolve().parent


class Mutation:
    def __init__(self):
        self.changed = {}
        self.e = audit.Evidence(ROOT, self.changed)

    def text(self, path, transform):
        self.changed[path] = transform(self.e.text(path)).encode()

    def obj(self, path, transform):
        data = self.e.obj(path)
        transform(data)
        self.changed[path] = (json.dumps(data, indent=2) + '\n').encode()

    def seal(self):
        """Rehash mutations so tests reach semantic checks, not only integrity checks."""
        for name in audit.SESSIONS:
            base = f'sessions/{name}/'
            if not any(path.startswith(base) for path in self.changed):
                continue
            suite = self.e.obj(base + 'suite.json')
            suite['metadata'] = self.e.obj(base + 'metadata.json')
            for path in suite['artifacts_sha256']:
                suite['artifacts_sha256'][path] = audit.sha(self.e.read(base + path))
            self.changed[base + 'suite.json'] = (json.dumps(suite, indent=2) + '\n').encode()
        manifest = self.e.obj('manifest.json')
        for path in list(manifest['sha256']):
            if path in self.changed:
                manifest['sha256'][path] = audit.sha(self.e.read(path))
        self.changed['manifest.json'] = json.dumps(manifest).encode()
        return self.e


class AuditTests(unittest.TestCase):
    def reject(self, change, expected, seal=True):
        mutation = Mutation()
        change(mutation)
        evidence = mutation.seal() if seal else mutation.e
        with self.assertRaisesRegex(audit.InvalidEvidence, expected):
            audit.validate(evidence)

    def test_original_evidence_passes(self):
        result = audit.validate(audit.Evidence(ROOT))
        self.assertEqual(result['decision'], 'REJECTED')
        self.assertEqual(result['native_tests_passed'], 347)
        self.assertGreater(result['comparisons']['rsb03/rsa09']['reference_normalized_percent'], 12)

    def test_relocated_bundle_is_portable_without_historical_paths(self):
        scratch = Path(os.environ.get('JCODE_SCRATCH_DIR', Path.home() / '.jcode/scratch'))
        scratch.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(prefix='root-classification-audit-', dir=scratch) as temporary:
            relocated = Path(temporary) / 'evidence'
            shutil.copytree(ROOT, relocated, ignore=shutil.ignore_patterns('__pycache__'))
            self.assertEqual(audit.validate(audit.Evidence(relocated))['decision'], 'REJECTED')

    def test_unsealed_corruption_fails_integrity(self):
        self.reject(lambda m: m.text('provenance.json', lambda t: t + ' '), 'checksum mismatch', seal=False)

    def test_v12_body_provider_reexecution_is_not_accepted(self):
        self.reject(lambda m: m.text('native/candidate-unit-v12/resident_root_classification.log',
                    lambda t: t.replace('revision 2: [export, root, main] = [2, 2, 1]',
                                        'revision 2: [export, root, main] = [2, 4, 1]')),
                    'v12 exact provider counts')

    def test_v11_selective_reuse_cannot_be_retconned(self):
        self.reject(lambda m: m.text('native/candidate-unit-v11/resident_root_classification.log',
                    lambda t: t.replace('[2, 4, 1]', '[2, 2, 1]')), 'v11 exact provider counts')

    def test_compile_failure_must_remain_failure(self):
        self.reject(lambda m: m.text('native/candidate-build-v9/check-exit', lambda t: '0\n'), 'v9 compile failure lost')

    def test_compile_errors_must_be_original(self):
        self.reject(lambda m: m.text('native/candidate-build-v9/check.log', lambda t: t.replace('E0277', 'removed')), 'return-mode errors lost')

    def test_original_v10_failed_counts_preserved(self):
        self.reject(lambda m: m.text('native/candidate-unit-v10/resident_root_classification.log',
                    lambda t: t.replace('left: 7', 'left: 5')), 'actual failed counts lost')

    def test_full_native_exit_must_pass(self):
        self.reject(lambda m: m.text('native/candidate-unit-v12/full-native.exit', lambda t: '101\n'), 'full native suite failed')

    def test_full_native_all_individual_raw_tests_required(self):
        path = 'native/candidate-unit-v12/justrust-home/runs/20261030-091611542-269328/output.jsonl'
        def remove_one(text):
            rows = text.splitlines(keepends=True)
            for n, row in enumerate(rows):
                if json.loads(row)['l'].startswith('test ') and ' ... ok' in row:
                    del rows[n]
                    break
            return ''.join(rows)
        self.reject(lambda m: m.text(path, remove_one), 'raw347 individual')

    def test_source_build_receipt_binds_original_hash(self):
        self.reject(lambda m: m.text('native/candidate-build-v12/source-before.sha256',
                    lambda t: t.replace(audit.SOURCE_HASHES[12], '0' * 64)), 'v12 build/source mismatch')

    def test_old_source_receipts_not_reconstructed(self):
        self.reject(lambda m: m.obj('provenance.json', lambda d: d.update(source_reconstruction_claim=True)), 'source reconstruction')

    def test_older_full_source_remains_unknown(self):
        self.reject(lambda m: m.obj('provenance.json', lambda d: d['older_full_source'].update(v10='reconstructed')), 'availability retconned')

    def test_no_promotion_of_nonwin(self):
        self.reject(lambda m: m.obj('provenance.json', lambda d: d.update(promotion=True)), 'negative decision changed')

    def test_wrong_final_pair_rejected(self):
        self.reject(lambda m: m.obj('provenance.json', lambda d: d.update(final_pair=['rsb02', 'rsa08'])), 'comparison provenance')

    def test_no_sample_exclusion(self):
        self.reject(lambda m: m.obj('sessions/rsa09/summary.json', lambda d: d.update(excluded=['engine-edit-0'])), 'excluded sample')

    def test_fabricated_median_rejected(self):
        self.reject(lambda m: m.obj('sessions/rsa09/suite.json', lambda d: d['measurements']['engine'].update(median_seconds=0.01)), 'median not computed')

    def test_missing_edit_record_rejected(self):
        self.reject(lambda m: m.obj('sessions/rsa09/records.json', lambda d: d.pop()), 'runtime records missing')

    def test_runtime_edit_marker_required(self):
        self.reject(lambda m: m.text('sessions/rsa09/engine-edit-4.stdout', lambda t: t.replace('JR_RUNTIME_EXECUTED_5', 'NO_MARKER')), 'runtime edit marker missing')

    def test_type_error_requires_actual_diagnostic(self):
        self.reject(lambda m: m.text('sessions/rsa09/engine-type-error-0.stderr', lambda t: t.replace('JR_RUNTIME_TYPE_ERROR', 'NO_MARKER')), 'real type diagnostic')

    def test_assert_failure_requires_actual_runtime_marker(self):
        self.reject(lambda m: m.text('sessions/rsa09/engine-assert-failure-0.stderr', lambda t: t.replace('JR_RUNTIME_FAILURE', 'NO_MARKER')), 'real runtime assertion')

    def test_recovery_requires_actual_test_names(self):
        self.reject(lambda m: m.text('sessions/rsa09/engine-type-recovery-0.stdout', lambda t: t.replace('parses_libtest_results', 'other_test')), 'raw runtime test name')

    def test_binary_identity_binds_build_to_measurement(self):
        def mutate(m):
            def change(d):
                for side in ['tools_before', 'tools_after']:
                    d[side]['engine']['sha256'] = '0' * 64
            m.obj('sessions/rsa09/metadata.json', change)
        self.reject(mutate, 'timed engine identity differs')

    def test_cross_session_toolchain_identity_required(self):
        def mutate(m):
            def change(d):
                for side in ['toolchain_before', 'toolchain_after']:
                    d[side]['path'] += '-different'
            m.obj('sessions/rsa09/metadata.json', change)
        self.reject(mutate, 'cross-session source/toolchain mismatch')

    def test_noise_unknown_cannot_be_presented_as_quiet(self):
        self.reject(lambda m: m.obj('sessions/rsa09/suite.json', lambda d: d.update(noise='quiet')), 'noise uncertainty')

    def test_matched_edits_require_same_source_hash(self):
        def mutate(m):
            def change(rows):
                next(row for row in rows if row['stack'] == 'engine' and row['phase'] == 'edit')['source_sha256'] = '0' * 64
            m.obj('sessions/rsa09/records.json', change)
        self.reject(mutate, 'engine/reference source edit mismatch')

    def test_negative_duration_rejected(self):
        def mutate(m):
            def change(rows):
                next(row for row in rows if row['stack'] == 'engine' and row['phase'] == 'edit')['elapsed_seconds'] = -1
            m.obj('sessions/rsa09/records.json', change)
        self.reject(mutate, 'invalid sample duration')

    def test_patch_replays_exactly_against_archived_baseline(self):
        mutation = Mutation()
        mutation.text('candidate-v12/candidate.patch', lambda t: t.replace('+fn exported_function', '+fn fabricated_function'))
        with self.assertRaisesRegex(audit.InvalidEvidence, 'does not reconstruct frozen candidate'):
            audit.verify_patch(mutation.e)

    def test_native_direct_contract_cannot_be_replaced_by_wrapper(self):
        def mutate(m):
            def change(rows):
                next(row for row in rows if row['stack'] == 'engine' and row['phase'] == 'edit')['command'].insert(1, 'cargo')
            m.obj('sessions/rsa09/records.json', change)
        self.reject(mutate, 'actual direct native runtime contract changed')

    def test_negative_performance_decision_requires_actual_regression(self):
        def mutate(m):
            base = 'sessions/rsa09/'
            def records(rows):
                for row in rows:
                    if row['stack'] == 'engine' and row['phase'] == 'edit':
                        row['elapsed_seconds'] = 0.05
            m.obj(base + 'records.json', records)
            m.obj(base + 'suite.json', lambda d: d['measurements']['engine'].update(samples_seconds=[0.05] * 5, median_seconds=0.05))
            m.obj(base + 'summary.json', lambda d: d['medians_seconds'].update(engine=0.05))
            baseline = m.e.obj('sessions/rsb03/summary.json')['medians_seconds']
            reference = m.e.obj(base + 'summary.json')['medians_seconds']['reference']
            raw = (0.05 / baseline['engine'] - 1) * 100
            normalized = ((0.05 / reference) / (baseline['engine'] / baseline['reference']) - 1) * 100
            m.text('provenance/suite-v12-repeat-compare.log', lambda t: t.replace('+11.53%', f'{raw:+.2f}%').replace('+12.63%', f'{normalized:+.2f}%'))
        self.reject(mutate, 'final replication no longer supports rejection')

    def test_manifest_cannot_drop_original_artifact(self):
        self.reject(lambda m: m.obj('manifest.json', lambda d: d['sha256'].pop('provenance/root-classification-source-ready-v9.txt')), 'inventory omits or adds')

    def test_restored_head_bytes_are_bound_to_receipt(self):
        self.reject(lambda m: m.obj('restoration.json', lambda d: d['restored_sha256'].update({'toolchain/resident/crates/ra-jit/src/main.rs': '0' * 64})), 'restored source not byte-exact')

    def test_failed_mechanism_must_block_timing(self):
        self.reject(lambda m: m.text('provenance/suite-after-v10.exit', lambda t: '0\n'), 'did not block v10 timing')

    def test_original_compare_output_must_match_all_sample_math(self):
        self.reject(lambda m: m.text('provenance/suite-v12-repeat-compare.log', lambda t: t.replace('+12.63%', '-12.63%')), 'original suite comparison disagrees')

    def test_unsafe_paths_rejected(self):
        with self.assertRaisesRegex(audit.InvalidEvidence, 'unsafe artifact path'):
            audit.Evidence(ROOT).read('../outside')


if __name__ == '__main__':
    unittest.main(verbosity=2)
