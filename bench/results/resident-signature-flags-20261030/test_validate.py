#!/usr/bin/env python3
"""Offline positive replay, relocation, and semantic mutation tests. No native tools."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

import validate as v

ROOT = Path(__file__).resolve().parent
FULL = 'native/candidate-unit-v15/justrust-home/runs/20261030-100536218-780696/'
FIXTURE = 'native/candidate-unit-v15/justrust-home/runs/20261030-100010641-602393/'
V14 = 'native/candidate-build-v14/justrust-home/runs/20261030-095427948-579324/'
V13 = 'native/candidate-unit-v13/justrust-home/runs/20261030-095015494-562655/'


class EvidenceTests(unittest.TestCase):
    def setUp(self):
        self.changes = {}
        self.e = v.Evidence(ROOT, self.changes)
        # Evidence accepts an explicit dictionary without copying it.
        self.e.overrides = self.changes

    def text(self, path, change):
        self.changes[path] = change(self.e.text(path)).encode()

    def obj(self, path, change):
        data = self.e.obj(path)
        change(data)
        self.changes[path] = (json.dumps(data, indent=2) + '\n').encode()

    def journal(self, path, change):
        data = self.e.journal(path)
        change(data)
        self.changes[path] = ''.join(json.dumps(r) + '\n' for r in data).encode()

    def reseal_session(self, name='rsa10'):
        """Reseal redundant JSON receipts so mutations reach independent semantic checks."""
        prefix = f'sessions/{name}/'
        suite = self.e.obj(prefix + 'suite.json')
        suite['metadata'] = self.e.obj(prefix + 'metadata.json')
        for path in suite['artifacts_sha256']:
            suite['artifacts_sha256'][path] = v.sha(self.e.read(prefix + path))
        self.changes[prefix + 'suite.json'] = (json.dumps(suite) + '\n').encode()

    def reject(self, expression):
        # Deliberately bypass the outer inventory. A checksum-only audit is not
        # enough: these tests require the inner independent semantic guard too.
        with self.assertRaisesRegex(v.InvalidEvidence, expression):
            v.audit(self.e, verify_inventory=False)

    def test_01_original_inventory_and_replay(self):
        report = v.audit(self.e)
        self.assertEqual(report['artifact_count'], 266)
        self.assertEqual(report['native']['slim_exports'], 11)
        self.assertEqual(len(report['native']['all347_raw_names']), 347)
        self.assertEqual(report['native']['fixture_comparisons'], 96)
        self.assertFalse(report['promotion'])
        self.assertGreater(report['runtime']['reference_normalized_delta_percent'], 0)

    def test_02_portable_copy_without_git_or_original_paths(self):
        with tempfile.TemporaryDirectory(dir=os.environ.get('JCODE_SCRATCH_DIR')) as directory:
            target = Path(directory) / 'detached-evidence'
            shutil.copytree(ROOT, target, ignore=shutil.ignore_patterns('__pycache__'))
            environment = dict(os.environ, PATH='', PYTHONDONTWRITEBYTECODE='1')
            result = subprocess.run([sys.executable, str(target / 'validate.py'), str(target)],
                                    cwd=directory, env=environment, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(json.loads(result.stdout)['decision'], 'NOT_A_PERFORMANCE_WIN')

    def test_03_missing_raw_journal(self):
        self.changes[FULL + 'output.jsonl'] = None
        self.reject('missing artifact')

    def test_04_inventory_detects_tampering(self):
        self.changes[FULL + 'output.jsonl'] = b'changed'
        with self.assertRaisesRegex(v.InvalidEvidence, 'checksum mismatch'):
            v.inventory(self.e)

    def test_05_promotion_forbidden(self):
        self.obj('provenance.json', lambda d: d.update(promotion=True))
        self.reject('promotion/decision')

    def test_06_decision_cannot_claim_win(self):
        self.obj('provenance.json', lambda d: d.update(decision='WIN'))
        self.reject('promotion/decision')

    def test_07_wrong_frozen_source(self):
        path = 'native/candidate-build-v15/frozen-source/' + v.SIG.removeprefix('toolchain/resident/')
        self.text(path, lambda s: s.replace('1 << 15', '1 << 0'))
        self.reject('frozen build source')

    def test_08_wrong_ready_source(self):
        self.text('source-ready/v15/' + v.MAIN, lambda s: s.replace('12 * 8', '12 * 7'))
        self.reject('source ready snapshot')

    def test_09_patch_context_rejected(self):
        self.text('source-ready/v15/source.patch', lambda s: s.replace('@@ -38,7', '@@ -39,7', 1))
        self.reject('patch context')

    def test_10_patch_missing_file_rejected(self):
        self.text('source-ready/v15/source.patch', lambda s: s[:s.index('diff --git a/toolchain/resident/crates/ra-jit')])
        self.reject('missing patch file')

    def test_11_changed_source_after(self):
        self.text('native/candidate-unit-v15/source-after.log', lambda s: s.replace(': OK', ': FAILED', 1))
        self.reject('source changed')

    def test_12_full_gate_source_changed(self):
        self.text('native/candidate-unit-v15/full-native-source-after.log', lambda s: s + 'changed\n')
        self.reject('full347')

    def test_13_export_id_binding(self):
        self.journal('exports/native-v15-unit/2026-10.jsonl', lambda rows: rows[0].update(id='different'))
        self.reject('11 exact exports')

    def test_14_export_pass_count_binding(self):
        self.journal('exports/native-v15-unit/2026-10.jsonl', lambda rows: rows[0]['tests'].update(passed=0))
        self.reject('test count mismatch')

    def test_15_export_wall_binding(self):
        self.journal('exports/native-v15-unit/2026-10.jsonl', lambda rows: rows[0].update(wall=999))
        self.reject('timing mismatch')

    def test_16_native_meta_filter_binding(self):
        self.obj(FIXTURE + 'meta.json', lambda d: d['args'].remove('resident_signature_flags'))
        self.reject('command/filter')

    def test_17_native_summary_id_binding(self):
        self.obj(FIXTURE + 'summary.json', lambda d: d.update(id='different'))
        self.reject('ID mismatch')

    def test_18_v13_failure_cannot_be_erased(self):
        self.obj(V13 + 'summary.json', lambda d: d.update(exit=0))
        self.reject('exit failure erased')

    def test_19_v14_failure_cannot_be_erased(self):
        self.obj(V14 + 'summary.json', lambda d: d.update(exit=0))
        self.reject('exit failure erased')

    def test_20_original_workspace_diagnostic_required(self):
        self.text(V13 + 'output.jsonl', lambda s: s.replace('requires dev-dependencies and is not a member of the workspace', 'unrelated failure'))
        self.reject('original native failure')

    def test_21_original_e0502_required(self):
        self.text(V14 + 'output.jsonl', lambda s: s.replace('error[E0502]', 'error[OTHER]'))
        self.reject('original native failure')

    def test_22_filtered_fixture_must_run(self):
        self.text(FIXTURE + 'output.jsonl', lambda s: s.replace('test tests::resident_signature_flags_match_direct_attributes_across_revisions ... ok', 'not a passing test'))
        self.reject('raw test names')

    def test_23_all347_raw_names_pinned(self):
        self.text(FULL + 'output.jsonl', lambda s: s.replace('tests::resident_signature_flags_match_direct_attributes_across_revisions', 'tests::an_unrelated_test'))
        self.reject('full347 raw names')

    def test_24_full347_duplicate_names(self):
        self.text(FULL + 'output.jsonl', lambda s: s.replace('answers::own::tests::arguments_read', 'answers::cargo::tests::questions_read'))
        self.reject('duplicate raw native')

    def test_25_zero_test_admission_rejected(self):
        self.text(FIXTURE + 'output.jsonl', lambda s: s.replace('running 1 test', 'running 0 tests'))
        self.reject('positive test admission')

    def test_26_blocked_v14_runtime_preserved(self):
        self.changes['provenance/suite-v14.exit'] = b'0\n'
        self.reject('blocked runtime')

    def test_27_suite_missing_phase(self):
        self.obj('sessions/rsa10/records.json', lambda d: d.pop())
        self.reseal_session()
        self.reject('incomplete runtime')

    def test_28_runtime_error_status(self):
        self.obj('sessions/rsa10/records.json', lambda d: d[2].update(status=0))
        self.reseal_session()
        self.reject('actual type error')

    def test_29_runtime_error_diagnostic(self):
        self.text('sessions/rsa10/engine-type-error-0.stderr', lambda s: s.replace('E0308', 'OTHER'))
        self.reseal_session()
        self.reject('actual type error')

    def test_30_runtime_assertion_diagnostic(self):
        self.text('sessions/rsa10/engine-assert-failure-0.stderr', lambda s: s.replace('JR_RUNTIME_FAILURE', 'unrelated failure'))
        self.reseal_session()
        self.reject('assertion failure')

    def test_31_runtime_marker_stale(self):
        self.text('sessions/rsa10/engine-edit-0.stdout', lambda s: s.replace('JR_RUNTIME_EXECUTED_1', 'JR_RUNTIME_EXECUTED_9'))
        self.reseal_session()
        self.reject('runtime marker')

    def test_32_runtime_marker_duplicate(self):
        self.text('sessions/rsa10/engine-edit-0.stderr', lambda s: s + '\nJR_RUNTIME_EXECUTED_1\n')
        self.reseal_session()
        self.reject('runtime marker')

    def test_33_runtime_name_parity(self):
        self.text('sessions/rsa10/engine-edit-0.stdout', lambda s: s.replace('summary::tests::parses_libtest_results', 'summary::tests::different_test'))
        self.reseal_session()
        self.reject('correctness parity')

    def test_34_workload_edit_hash(self):
        self.obj('sessions/rsa10/records.json', lambda d: d[2].update(source_sha256='0' * 64))
        self.reseal_session()
        self.reject('stale workload edit')

    def test_35_timeout_cannot_be_accepted(self):
        self.obj('sessions/rsa10/records.json', lambda d: d[-1].update(timed_out=True))
        self.reseal_session()
        self.reject('process error/timeout')

    def test_36_nan_timing_rejected(self):
        self.obj('sessions/rsa10/records.json', lambda d: d[-1].update(elapsed_seconds=float('nan')))
        self.reseal_session()
        self.reject('invalid raw timing')

    def test_37_suite_sample_summary_rejected(self):
        self.obj('sessions/rsa10/suite.json', lambda d: d['measurements']['engine'].update(median_seconds=0.001))
        self.reject('timing summary')

    def test_38_engine_binary_identity_pinned(self):
        def change(d):
            for key in ['tools_before', 'tools_after']:
                d[key]['engine']['sha256'] = '0' * 64
        self.obj('sessions/rsa10/metadata.json', change)
        self.reseal_session()
        self.reject('exact executable hash')

    def test_39_toolchain_identity_pinned(self):
        def change(d):
            for key in ['toolchain_before', 'toolchain_after']:
                d[key]['files']['bin/rustc']['sha256'] = '0' * 64
        self.obj('sessions/rsa10/metadata.json', change)
        self.reseal_session()
        self.reject('full toolchain identity')

    def test_40_source63_identity_pinned(self):
        def change(d):
            for key in ['source_before', 'source_after']:
                d[key]['Cargo.toml'] = '0' * 64
        self.obj('sessions/rsa10/metadata.json', change)
        self.reseal_session()
        self.reject('source63')

    def test_41_remote_policy_rejected(self):
        self.obj('sessions/rsa10/metadata.json', lambda d: d['environments']['engine'].update(BLAZE_REMOTE='on'))
        self.reseal_session()
        self.reject('remote/proc-macro')

    def test_42_shared_target_rejected(self):
        self.obj('sessions/rsa10/metadata.json', lambda d: d['environments']['engine'].update(CARGO_TARGET_DIR='/shared/target'))
        self.reseal_session()
        self.reject('stores not private')

    def test_43_host_mismatch(self):
        self.obj('sessions/rsa10/suite.json', lambda d: d['host'].update(cpu=['different CPU']))
        self.reject('host/cpu')

    def test_44_comparison_reproducible(self):
        self.text('provenance/suite-v15-compare.log', lambda s: s.replace('+2.21%', '-20.00%'))
        self.reject('comparison not reproducible')

    def test_46_inconsistent_raw_footer_verdict(self):
        self.text(FULL + 'output.jsonl', lambda s: s.replace('test result: ok.', 'test result: FAILED.'))
        self.reject('raw libtest verdict')

    def test_47_effective_per_command_backend(self):
        self.obj('sessions/rsa10/records.json', lambda d: d[-1]['environment'].update(BLAZE_BACKEND='llvm'))
        self.reseal_session()
        self.reject('effective per-command')

    def test_45_safe_relative_artifact_paths(self):
        with self.assertRaisesRegex(v.InvalidEvidence, 'unsafe artifact'):
            self.e.read('../secret')


if __name__ == '__main__':
    unittest.main(verbosity=2)
