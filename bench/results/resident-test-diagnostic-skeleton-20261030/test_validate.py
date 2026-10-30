#!/usr/bin/env python3
"""Offline negative controls on private copies, plus isolated relocated Python replay."""
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

from validate import digest, seal, validate

HERE = Path(__file__).resolve().parent


class EvidenceControls(unittest.TestCase):
    def setUp(self):
        scratch = Path(os.environ.get('JCODE_SCRATCH_DIR', str(Path.home() / '.jcode/scratch')))
        scratch.mkdir(parents=True, exist_ok=True)
        self.temp = tempfile.TemporaryDirectory(prefix='test-skeleton-v28-offline-', dir=scratch)
        self.base = Path(self.temp.name)
        self.root = self.base / 'relocated-bundle'
        shutil.copytree(HERE, self.root, ignore=shutil.ignore_patterns('__pycache__'))

    def tearDown(self):
        self.temp.cleanup()

    def change(self, name, callback, reseal=True):
        path = self.root / name
        value = json.loads(path.read_text())
        callback(value)
        path.write_text(json.dumps(value) + '\n')
        if reseal:
            seal(self.root)

    def rejected(self, reason):
        with self.assertRaisesRegex((ValueError, OSError, KeyError), reason):
            validate(self.root)

    def test_positive_complete_frozen_bundle(self):
        result = validate(self.root)
        self.assertEqual((result['native_tests'], result['suite_rows'], result['warm_rows'], result['profile_rows']), (359, 88, 282, 12))

    def test_relocation_without_repo_or_original_scratch_access(self):
        empty = self.base / 'empty-working-directory'
        empty.mkdir()
        result = subprocess.run([sys.executable, '-I', '-B', str(self.root / 'validate.py')],
                                cwd=empty, env={'PATH': '/nonexistent', 'HOME': str(empty)},
                                capture_output=True, text=True, timeout=30)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(json.loads(result.stdout)['profile_rows'], 12)

    def test_checksum_corruption(self):
        (self.root / 'unit/v28/full-native.exit').write_text('1\n')
        self.rejected('hash mismatch')

    def test_rehashed_failed_gate(self):
        (self.root / 'unit/v28/full-native.exit').write_text('1\n')
        seal(self.root)
        self.rejected('failed native gate')

    def test_rehashed_extra_payload_not_manifested(self):
        (self.root / 'invented.txt').write_text('extra')
        self.rejected('coverage')

    def test_missing_profile_payload(self):
        (self.root / 'profile/2-new.worker-delta.log').unlink()
        seal(self.root)
        self.rejected('No such file')

    def test_symlink_escape(self):
        (self.root / 'escape').symlink_to('/etc/passwd')
        self.rejected('symlink')

    def test_manifest_path_escape(self):
        with (self.root / 'SHA256SUMS').open('a') as stream:
            stream.write('0' * 64 + '  ../outside\n')
        self.rejected('unsafe')

    def test_compiled_source_identity(self):
        (self.root / 'build/v28/frozen-source/crates/ra-jit/src/outline.rs').write_text('not the compiler input')
        seal(self.root)
        self.rejected('compiled source mismatch')

    def test_rehashed_native_count(self):
        self.change('unit/v28/raw-runs/20261030-143103071-1087/summary.json',
                    lambda s: s['tests'].update(passed=358))
        self.rejected('native gate count')

    def test_rehashed_native_raw_result(self):
        path = self.root / 'unit/v28/raw-runs/20261030-143103071-1087/output.jsonl'
        path.write_text(path.read_text().replace('359 passed', '358 passed'))
        seal(self.root)
        self.rejected('raw native verdict')

    def test_rehashed_v27_failure_erasure(self):
        self.change('unit/v27/raw-runs/20261030-142404178-4153131/summary.json',
                    lambda s: s.update(exit=0))
        self.rejected('v27 failure lost')

    def test_rehashed_regression_gate(self):
        path = self.root / 'root/bench-v28-regression.log'
        path.write_text(path.read_text().replace('Ran 700 tests', 'Ran 699 tests'))
        seal(self.root)
        self.rejected('regression gate')

    def test_rehashed_suite_source_change(self):
        self.change('suites/rsa24/metadata.json', lambda s: s['source_after'].update({'Cargo.toml': '0' * 64}))
        self.rejected('suite identity')

    def test_rehashed_suite_row_loss(self):
        self.change('suites/rsa24/records.json', lambda s: s.pop())
        # Refresh the suite's inner artifact hash so this is a structural test.
        self.change('suites/rsa24/suite.json', lambda s: s['artifacts_sha256'].update(
            {'records.json': digest(self.root / 'suites/rsa24/records.json')}))
        self.rejected('suite row coverage')

    def test_rehashed_suite_edit_status(self):
        self.change('suites/rsa24/records.json', lambda s: s[-1].update(status=101))
        self.change('suites/rsa24/suite.json', lambda s: s['artifacts_sha256'].update(
            {'records.json': digest(self.root / 'suites/rsa24/records.json')}))
        self.rejected('runtime status')

    def test_rehashed_warm_row_loss(self):
        self.change('warm/forward/state.json', lambda s: s['rows'].pop())
        self.rejected('warm row coverage')

    def test_rehashed_warm_median(self):
        self.change('warm/reverse/state.json', lambda s: s['medians_seconds'].update(new=0.001))
        self.rejected('warm median')

    def test_rehashed_warm_runtime_marker_loss(self):
        path = self.root / 'warm/forward/edit-0-new.stdout'
        path.write_text(path.read_text().replace('JR_RUNTIME_EXECUTED_2801', 'REMOVED_MARKER'))
        self.change('warm/forward/state.json', lambda s: next(r for r in s['rows'] if
                    r['stack'] == 'new' and r['phase'] == 'edit' and r['replicate'] == 0).update(stdout_sha256=digest(path)))
        self.rejected('raw runtime result|marker')

    def test_rehashed_profile_row_loss(self):
        self.change('profile/state.json', lambda s: s['rows'].pop())
        self.rejected('profile admission')

    def test_rehashed_profile_tool_change(self):
        self.change('profile/state.json', lambda s: s['tools_after'].update(new='0' * 64))
        self.rejected('profile tool identity')

    def test_rehashed_invented_inventory(self):
        self.change('profile/state.json', lambda s: s['rows'][3]['collector_inventory'][0].update(calls=99999))
        self.rejected('raw inventory')

    def test_rehashed_coherent_wrong_selected_count(self):
        path = self.root / 'profile/2-new.worker-delta.log'
        path.write_text(path.read_text().replace('label=hir.collect.selected_body_inclusive calls=1 ',
                                                 'label=hir.collect.selected_body_inclusive calls=2 '))
        def change(state):
            row = state['rows'][5]
            row['worker_delta_sha256'] = digest(path)
            for counter in row['collector_inventory']:
                if counter['label'] == 'hir.collect.selected_body_inclusive':
                    counter['calls'] = 2
        self.change('profile/state.json', change)
        self.rejected('selected body proof')

    def test_rehashed_coherent_transport_drops(self):
        path = self.root / 'profile/2-new.worker-delta.log'
        path.write_text(path.read_text().replace('dropped=0 cap=4000', 'dropped=1 cap=4000'))
        def change(state):
            row = state['rows'][5]
            row['worker_delta_sha256'] = digest(path)
            for counter in row['transport']:
                counter[2] = 1
        self.change('profile/state.json', change)
        self.rejected('dropped profile spans')

    def test_rehashed_profile_synthetic_events(self):
        path = self.root / 'profile/2-new.trace.json'
        data = json.loads(path.read_text())
        data['traceEvents'].append(dict(name='hir.collect.invented', ph='X', dur=1))
        path.write_text(json.dumps(data))
        self.change('profile/state.json', lambda s: s['rows'][5].update(trace_sha256=digest(path)))
        self.rejected('synthetic collector')

    def test_rehashed_wrong_exact_marker(self):
        path = self.root / 'warm/forward/edit-0-new.stdout'
        path.write_text(path.read_text().replace('JR_RUNTIME_EXECUTED_2801', 'JR_RUNTIME_EXECUTED_9999'))
        self.change('warm/forward/state.json', lambda s: next(r for r in s['rows'] if
                    r['stack'] == 'new' and r['phase'] == 'edit' and r['replicate'] == 0).update(stdout_sha256=digest(path)))
        self.rejected('runtime edit marker mismatch')

    def test_rehashed_wrong_failing_name(self):
        path = self.root / 'warm/forward/assert-failure-0-new.stdout'
        before = path.read_text()
        first = 'summary::tests::shares_overlapping_time'
        second = 'summary::tests::parses_libtest_results'
        path.write_text(before.replace(first, 'TEMP_TEST_NAME').replace(second, first).replace('TEMP_TEST_NAME', second))
        def change(state):
            row = next(r for r in state['rows'] if r['stack'] == 'new' and r['phase'] == 'assert-failure')
            row['stdout_sha256'] = digest(path)
            for test in row['parsed']['tests']:
                if test[0] == first:
                    test[1] = 'ok'
                elif test[0] == second:
                    test[1] = 'FAILED'
        self.change('warm/forward/state.json', change)
        self.rejected('wrong failing test')

    def test_rehashed_wrong_counterbalanced_order(self):
        def change(state):
            state['rows'][15], state['rows'][16] = state['rows'][16], state['rows'][15]
        self.change('warm/forward/state.json', change)
        self.rejected('warm exact phase/permutation order')

    def test_rehashed_duplicate_phase(self):
        self.change('warm/forward/state.json', lambda s: s['rows'][2].update(phase='prepare'))
        self.rejected('warm exact phase/permutation order')

    def test_rehashed_coherent_missing_semantic_phase(self):
        path = self.root / 'profile/2-new.trace.json'
        data = json.loads(path.read_text())
        data['traceEvents'] = [e for e in data['traceEvents'] if e.get('name') != 'diagnostics.semantic.syntax_walk']
        path.write_text(json.dumps(data))
        def change(state):
            row = state['rows'][5]
            row['events'] = [e for e in row['events'] if e.get('name') != 'diagnostics.semantic.syntax_walk']
            row['trace_sha256'] = digest(path)
        self.change('profile/state.json', change)
        self.rejected('positive phase missing')

    def test_rehashed_coherent_wrong_semantic_thread(self):
        path = self.root / 'profile/2-new.trace.json'
        data = json.loads(path.read_text())
        for event in data['traceEvents']:
            if event.get('name') == 'diagnostics.semantic.collect':
                event['tid'] = -999
        path.write_text(json.dumps(data))
        def change(state):
            row = state['rows'][5]
            for event in row['events']:
                if event.get('name') == 'diagnostics.semantic.collect':
                    event['tid'] = -999
            row['trace_sha256'] = digest(path)
        self.change('profile/state.json', change)
        self.rejected('semantic thread containment')


if __name__ == '__main__':
    unittest.main(verbosity=2)
