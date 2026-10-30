#!/usr/bin/env python3
"""Copied-bundle replay and semantic mutations, never native workloads."""
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import unittest
import validate

class AuditMutations(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temp = tempfile.TemporaryDirectory(dir=os.environ.get('JCODE_SCRATCH_DIR'))
        cls.root = Path(cls.temp.name) / 'standalone'
        shutil.copytree(validate.ROOT, cls.root, ignore=shutil.ignore_patterns('__pycache__'))

    @classmethod
    def tearDownClass(cls):
        cls.temp.cleanup()

    def mutate(self, name, change, message):
        path = self.root / name
        original = path.read_bytes()
        # Refresh native artifact receipt hashes to prove semantic rejection,
        # rather than merely proving a mutated byte changes a checksum.
        session = name.split('/')[0]
        suite_path = self.root / session / 'suite.json' if session in validate.ENGINES else None
        suite_original = suite_path.read_bytes() if suite_path else None
        try:
            path.write_bytes(change(original))
            if suite_path and name != session + '/suite.json':
                suite = json.loads(suite_original)
                rel = name[len(session)+1:]
                if rel in suite['artifacts_sha256']:
                    suite['artifacts_sha256'][rel] = hashlib.sha256(path.read_bytes()).hexdigest()
                if rel == 'metadata.json':
                    suite['metadata'] = json.loads(path.read_bytes())
                suite_path.write_text(json.dumps(suite))
            with self.assertRaisesRegex(ValueError, message):
                validate.audit(self.root, integrity=False)
        finally:
            path.write_bytes(original)
            if suite_path:
                suite_path.write_bytes(suite_original)

    def json_mutate(self, name, change, message):
        def apply(data):
            obj = json.loads(data)
            change(obj)
            return json.dumps(obj).encode()
        self.mutate(name, apply, message)

    def test_standalone_native_replay(self):
        report = validate.audit(self.root)
        self.assertEqual(report['accepted_runtime_edits'], 20)
        self.assertIn('-8.61%', report['native_comparison'])
        self.assertIn('relative delta +2.57%', report['native_comparison'])
        self.assertIn('No significance or speedup verdict', report['native_comparison'])
        detached = subprocess.run([sys.executable, '-B', str(self.root / 'validate.py')], cwd=self.root, env=dict(os.environ, HOME=str(self.root), PATH=str(self.root / 'no-tools'), PYTHONDONTWRITEBYTECODE='1'), capture_output=True, text=True)
        self.assertEqual(detached.returncode, 0, detached.stderr)
        self.assertEqual(json.loads(detached.stdout)['accepted_runtime_edits'], 20)

    def test_all_twenty_runtime_markers(self):
        for session in validate.ENGINES:
            for stack in ('reference', 'engine'):
                for rep in range(5):
                    token = f'JR_RUNTIME_EXECUTED_{rep+1}'.encode()
                    for replacement in (b'', b'JR_RUNTIME_EXECUTED_99', token + b' ' + token):
                        with self.subTest(session=session, stack=stack, rep=rep, replacement=replacement):
                            self.mutate(f'{session}/{stack}-edit-{rep}.stdout', lambda data, token=token, replacement=replacement: data.replace(token, replacement), 'runtime marker')

    def test_raw_names_counts_failure_and_recovery(self):
        for session in validate.ENGINES:
            for stack in ('reference', 'engine'):
                self.mutate(f'{session}/{stack}-edit-0.stdout', lambda d: d.replace(b'parses_libtest_results', b'wrong_name'), 'libtest parity')
                self.mutate(f'{session}/{stack}-edit-0.stdout', lambda d: d.replace(b'4 passed', b'5 passed'), 'libtest parity')
                self.mutate(f'{session}/{stack}-type-error-0.stderr', lambda d: d.replace(b'E0308', b'E9999'), 'real type failure')
                self.mutate(f'{session}/{stack}-assert-failure-0.stderr', lambda d: d.replace(b'JR_RUNTIME_FAILURE', b'no failure'), 'assertion failure')
                for phase in ('type-recovery', 'assert-recovery'):
                    self.mutate(f'{session}/{stack}-{phase}-0.stdout', lambda d: d.replace(b'4 passed', b'0 passed'), 'libtest parity')
                self.json_mutate(f'{session}/records.json', lambda rows, stack=stack: next(r for r in rows if r['stack'] == stack and r['phase'] == 'edit').update(accepted=False), 'accepted runtime edit')

    def test_native_suite_receipt_and_comparison_contract(self):
        self.json_mutate('rsa01/suite.json', lambda d: d.update(schema='synthetic/v1'), 'mix native')
        self.json_mutate('rsa01/suite.json', lambda d: d['host'].update(cpu=['other CPU']), 'host/cpu/execution')
        self.json_mutate('rsa01/suite.json', lambda d: d.update(noise='quiet'), 'noise scope')
        self.json_mutate('rsa01/suite.json', lambda d: d['measurements']['engine'].update(median_seconds=0.0001), 'sample receipts')
        self.json_mutate('rsb01/suite.json', lambda d: d['artifacts_sha256'].update({'records.json': '0'*64}), 'raw artifact receipts')
        self.json_mutate('rsa01/summary.json', lambda d: d['medians_seconds'].update(engine=0.0001), 'median replay')
        self.json_mutate('rsa01/records.json', lambda d: d.pop(), 'missing/duplicate/out-of-order')
        self.json_mutate('rsa01/records.json', lambda d: d[0].update(elapsed_seconds=float('nan')), 'invalid timing')

    def test_immutable_identities_and_no_seed(self):
        self.mutate('source/src/summary.rs', lambda d: d+b'\n', 'pristine Git source')
        self.json_mutate('rsa01/metadata.json', lambda d: d['tools_after']['engine'].update(sha256='0'*64), 'tools changed')
        self.json_mutate('rsb01/metadata.json', lambda d: d['toolchain_after']['files']['bin/rustc'].update(sha256='0'*64), 'toolchain changed')
        self.json_mutate('rsa01/metadata.json', lambda d: d.update(store_scope='seeded'), 'seeded or unknown')
        self.json_mutate('rsa01/records.json', lambda d: d[0]['command'].append('seed'), 'command identity')
        self.mutate('provenance/channel-manifest.toml', lambda d: d+b'\n', 'retained toolchain manifest')

    def unit_base(self, filt):
        text = (self.root / f'build/candidate-unit-v4/{filt}.log').read_text()
        run_id = re.search(r'justrust log (\d{8}-\d{9}-\d+)', text).group(1)
        return f'build/candidate-unit-v4/runs/{run_id}/'

    def test_v3_rejection_and_v4_actual_units(self):
        self.mutate('build/candidate-unit-v3/exit', lambda d: b'0\n', 'v3 rejection')
        self.mutate('build/candidate-unit-v3/graph.log', lambda d: d.replace(b'E0603', b'E9999'), 'fixture compile errors')
        for filt in ('resident_graph_reuse', 'the_roots_of_a_listed_crate', 'telemetry'):
            self.mutate(f'build/candidate-unit-v4/{filt}.exit', lambda d: b'101\n', 'unit filter exit')
            base = self.unit_base(filt)
            self.json_mutate(base+'meta.json', lambda d: d['args'].append('--ignored'), 'unit filter command')
            self.json_mutate(base+'summary.json', lambda d: d.update(exit=101), 'actual unit summary')
            self.mutate(base+'output.jsonl', lambda d: d.replace(b'0 failed', b'1 failed'), 'unit test counts')
        self.mutate(self.unit_base('resident_graph_reuse')+'output.jsonl', lambda d: d.replace(b'tests::resident_graph_reuse_tracks_nested_items_and_macro_edits ... ok', b'tests::different_test ... ok'), 'graph fixture name')
        self.mutate(self.unit_base('telemetry')+'output.jsonl', lambda d: d.replace(b'cargo::telemetry_tests::disabled_phase_needs_no_clock_or_span ... ok', b'cargo::telemetry_tests::weakened_test ... ok'), 'telemetry fixture names')

    def test_build_sources_and_binary_receipts(self):
        self.mutate('build/candidate-build-v4/compile-exit', lambda d: b'1\n', 'v4 compile exit')
        self.mutate('build/candidate-build-v4/engine-identity.sha256', lambda d: b'wrong\n', 'built tool hash')
        self.mutate('build/candidate-build-v4/source-before.sha256', lambda d: b'0'*64 + d[64:], 'pre-build source hash')
        self.mutate('build/candidate-unit-v4/source-after.log', lambda d: d.replace(b': OK', b': FAILED'), 'post-unit source receipt')

    def test_seal_rejects_extra_file(self):
        path = self.root/'unexpected.txt'
        try:
            path.write_text('not allowlisted')
            with self.assertRaisesRegex(ValueError, 'integrity seal'):
                validate.audit(self.root)
        finally:
            path.unlink()

if __name__ == '__main__':
    unittest.main()
