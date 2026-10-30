#!/usr/bin/env python3
"""Offline mutation rejection and relocation tests. Never runs native/compiler commands."""
import contextlib
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

import validate as v
from raw_checks import Evidence, InvalidEvidence, sha

ROOT = Path(__file__).resolve().parent


def encode(obj):
    return (json.dumps(obj, indent=2) + '\n').encode()


class ReplayTests(unittest.TestCase):
    def mutate(self, path, change):
        obj = json.loads((ROOT / path).read_text())
        change(obj)
        return {path: encode(obj)}

    def rehash_session(self, name, changes):
        """Update suite hashes so mutations must fail semantic checks, not just hashing."""
        prefix = f'sessions/{name}/'
        suite = json.loads((ROOT / prefix / 'suite.json').read_text())
        for path, contents in changes.items():
            if path.startswith(prefix):
                suite['artifacts_sha256'][path.removeprefix(prefix)] = sha(contents)
                if path == prefix + 'metadata.json':
                    suite['metadata'] = json.loads(contents)
        return dict(changes, **{prefix + 'suite.json': encode(suite)})

    def reject(self, check, changes, pattern=None):
        with self.assertRaisesRegex(InvalidEvidence, pattern or '.'):
            check(Evidence(ROOT, changes))

    def suite_check(self, e):
        return v.timing(e, 'v17', 'rsb06', 'rsa12', 16, 17)

    def test_complete_bundle(self):
        report = v.audit(Evidence(ROOT))
        self.assertEqual(report['audit'], 'PASS')
        self.assertEqual(len(report['native']['full351_names']), 351)
        self.assertEqual(len(report['timings']), 6)
        self.assertTrue(all(row['actual_rows'] == 12 for row in report['traces'].values()))

    def test_missing_artifact(self):
        self.reject(v.inventory, {'traces/v16/old-0.trace.json': None}, 'missing artifact')

    def test_checksum_tamper(self):
        self.reject(v.inventory, {'source-ready/v17/main.source-ready.rs': b'bad'}, 'checksum')

    def test_unsafe_path(self):
        with self.assertRaises(InvalidEvidence): Evidence(ROOT).read('../external')

    def test_source_provider_tamper(self):
        path = 'source-ready/v17/main.source-ready.rs'
        data = (ROOT / path).read_bytes().replace(b'count.set(count.get() + 1)', b'count.set(0)', 1)
        self.assertNotEqual(data, (ROOT / path).read_bytes())
        self.reject(v.sources, {path:data}, 'full source')

    def test_v19_quality_attribute_removed(self):
        path = 'source-ready/v19/main.source-ready.rs'
        data = (ROOT / path).read_bytes().replace(b'#[cfg(test)]\nfn exported(', b'fn exported(', 1)
        self.reject(v.sources, {path:data}, 'full source')

    def test_patch_context_tamper(self):
        patch = '--- old\n+++ new\n@@ -1 +1 @@\n-wrong\n+new\n'
        with self.assertRaisesRegex(InvalidEvidence, 'context'):
            v.replay_patch('old\n', patch)

    def test_native_positive_count(self):
        paths = json.loads((ROOT / 'manifest.json').read_text())['sha256']
        path = next(p for p in paths if p.startswith('native/candidate-unit-v17/') and p.endswith('/summary.json') and json.loads((ROOT/p).read_text())['tests']['passed'] == 4)
        changes = self.mutate(path, lambda x: x['tests'].__setitem__('passed', 0))
        self.reject(v.native, changes, 'summary differs')

    def test_native_wrong_filter(self):
        paths = json.loads((ROOT / 'manifest.json').read_text())['sha256']
        path = next(p for p in paths if p.startswith('native/candidate-unit-v17/') and p.endswith('/summary.json') and json.loads((ROOT/p).read_text())['tests']['passed'] == 4)
        changes = self.mutate(path, lambda x: x['args'].__setitem__(-2, 'not_the_fixture'))
        self.reject(v.native, changes, 'args mismatch')

    def test_native_missing_raw_journal(self):
        paths = json.loads((ROOT / 'manifest.json').read_text())['sha256']
        path = next(p for p in paths if p.startswith('native/candidate-unit-v17/') and p.endswith('/output.jsonl'))
        self.reject(v.native, {path:None}, 'missing artifact')

    def test_native_exit_failure(self):
        self.reject(v.native, {'native/candidate-unit-v17/full-native.exit': b'101\n'}, 'exit receipt')

    def test_runtime_marker_stale(self):
        path = 'sessions/rsa12/engine-edit-0.stdout'
        data = (ROOT/path).read_bytes().replace(b'JR_RUNTIME_EXECUTED_1', b'JR_RUNTIME_EXECUTED_99')
        self.reject(self.suite_check, self.rehash_session('rsa12', {path:data}), 'runtime marker')

    def test_runtime_type_error_gate_erased(self):
        path = 'sessions/rsa12/records.json'
        def change(rows):
            next(r for r in rows if r['stack']=='engine' and r['phase']=='type-error')['status'] = 0
        self.reject(self.suite_check, self.rehash_session('rsa12', self.mutate(path, change)), 'type error gate')

    def test_runtime_assertion_gate_erased(self):
        path = 'sessions/rsa12/records.json'
        def change(rows):
            next(r for r in rows if r['stack']=='engine' and r['phase']=='assert-failure')['gate_passed'] = False
        self.reject(self.suite_check, self.rehash_session('rsa12', self.mutate(path, change)), 'assertion failure')

    def test_raw_timing_summary_tamper(self):
        path = 'sessions/rsa12/records.json'
        def change(rows):
            for row in rows:
                if row['stack']=='engine' and row['phase']=='edit': row['elapsed_seconds'] = 0.00001
        self.reject(self.suite_check, self.rehash_session('rsa12', self.mutate(path, change)), 'timing summary')

    def test_noise_guarantee_invented(self):
        path = 'sessions/rsa12/records.json'
        def change(rows): rows[0]['noise']['contamination'] = 'quiet'
        self.reject(self.suite_check, self.rehash_session('rsa12', self.mutate(path, change)), 'noise guarantee')

    def test_identity_mismatch(self):
        path = 'sessions/rsa12/metadata.json'
        def change(x):
            x['tools_before']['engine']['sha256'] = '0'*64
            x['tools_after']['engine']['sha256'] = '0'*64
        self.reject(self.suite_check, self.rehash_session('rsa12', self.mutate(path, change)), 'executable hash')

    def test_missing_trace_row(self):
        changes = self.mutate('traces/v17/state.json', lambda x: x['rows'].pop())
        self.reject(lambda e:v.traces(e,17), changes, '12 actual')

    def test_trace_phase_summary_tamper(self):
        changes = self.mutate('traces/v17/phase-summary.json', lambda x:x['medians']['new']['worker roots'].__setitem__('median_elapsed_us', 1))
        self.reject(lambda e:v.traces(e,17), changes, 'phase-summary')

    def test_trace_identity_mismatch(self):
        changes = self.mutate('traces/v17/state.json', lambda x:x['rows'][0].__setitem__('tool_sha256','0'*64))
        self.reject(lambda e:v.traces(e,17), changes, 'runtime gate')

    def test_trace_marker_stale(self):
        path = 'traces/v17/new-0.stdout'
        data = (ROOT/path).read_bytes().replace(b'JR_RUNTIME_EXECUTED_501',b'JR_RUNTIME_EXECUTED_999')
        self.reject(lambda e:v.traces(e,17), {path:data}, 'marker stale')

    def test_promotion_overclaim(self):
        changes = self.mutate('provenance.json', lambda x:x.__setitem__('promotion',True))
        self.reject(lambda e:v.audit(e,verify_inventory=False), changes, 'overclaim')

    def test_cleanup_namespace_escaped(self):
        changes = self.mutate('provenance/resident-private-cleanup-before-v19-20261030.json', lambda x:x['rows'][0]['namespace'].__setitem__('BLAZE_HOME','/other/user/data'))
        self.reject(v.cleanup, changes, 'namespace escaped')

    def test_cleanup_missing_receipt(self):
        changes = self.mutate('provenance/resident-private-cleanup-before-v19-20261030.json', lambda x:x['rows'].pop())
        self.reject(v.cleanup, changes, 'missing/duplicate namespace')

    def warm_check(self, e):
        return v.warm_checks.replay(e, v.ENGINE_HASHES)

    def test_warm_missing_row(self):
        self.reject(self.warm_check, self.mutate('observations/interleaved-v19/state.json', lambda x:x['rows'].pop()), '141 rows')

    def test_warm_order(self):
        def change(x): x['rows'][15],x['rows'][16] = x['rows'][16],x['rows'][15]
        self.reject(self.warm_check, self.mutate('observations/interleaved-v19/state.json',change), 'ordering')

    def test_warm_median(self):
        self.reject(self.warm_check, self.mutate('observations/interleaved-v19/state.json', lambda x:x['medians_seconds'].__setitem__('new',0.01)), 'median')

    def test_warm_source(self):
        self.reject(self.warm_check, self.mutate('observations/interleaved-v19/state.json', lambda x:x['source_after'].__setitem__('Cargo.toml','0'*64)), 'source63')

    def test_warm_identity(self):
        self.reject(self.warm_check, self.mutate('observations/interleaved-v19/state.json', lambda x:x['configs']['new'].__setitem__('sha256','0'*64)), 'identity')

    def test_warm_type_gate(self):
        self.reject(self.warm_check, self.mutate('observations/interleaved-v19/state.json', lambda x:x['rows'][1].__setitem__('status',0)), 'type-error')

    def test_warm_marker_even_after_rehash(self):
        path = 'observations/interleaved-v19/edit-0-new.stdout'
        contents = (ROOT/path).read_bytes().replace(b'JR_RUNTIME_EXECUTED_801',b'JR_RUNTIME_EXECUTED_999')
        def change(x):
            next(r for r in x['rows'] if r['stack']=='new' and r['phase']=='edit' and r['replicate']==0)['stdout_sha256'] = sha(contents)
        changes = self.mutate('observations/interleaved-v19/state.json',change)
        changes[path] = contents
        self.reject(self.warm_check, changes, 'marker')

    def test_portable_relocation(self):
        base = os.environ.get('JCODE_SCRATCH_DIR')
        with tempfile.TemporaryDirectory(prefix='root-snapshot-offline-', dir=base) as folder:
            target = Path(folder)/'relocated'
            shutil.copytree(ROOT,target,ignore=shutil.ignore_patterns('__pycache__'))
            result = subprocess.run([sys.executable,'-B',str(target/'validate.py'),str(target)],cwd=folder,
                                    capture_output=True,text=True,timeout=120)
            self.assertEqual(result.returncode,0,result.stdout+result.stderr)
            self.assertEqual(json.loads(result.stdout)['audit'],'PASS')


if __name__ == '__main__':
    unittest.main(verbosity=2)
