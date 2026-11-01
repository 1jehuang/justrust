#!/usr/bin/env python3
"""Actual archived-output tests only. Never invokes Rust or network services."""
import sys
sys.dont_write_bytecode = True
import gzip
import importlib.util
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

HERE = Path(__file__).absolute().parent
spec = importlib.util.spec_from_file_location('archive_validator', HERE / 'validate.py')
v = importlib.util.module_from_spec(spec)
spec.loader.exec_module(v)


class ArchiveTests(unittest.TestCase):
    STATUS = 'INTEGRITY-PASS-CONFOUNDED'
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='three-stack-archive-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name) / 'relocated'
        shutil.copytree(HERE, self.root, ignore=shutil.ignore_patterns('__pycache__'))

    def entries(self):
        return [json.loads(line) for line in gzip.decompress((self.root / 'journal.jsonl.gz').read_bytes()).splitlines()]

    def reseal(self, entries):
        def repair(value):
            if isinstance(value, dict):
                if set(value) == {'file', 'sha256'}:
                    value['sha256'] = v.sha((self.root / value['file']).read_bytes())
                else:
                    for x in value.values():
                        repair(x)
            elif isinstance(value, list):
                for x in value:
                    repair(x)
        repair(entries)
        previous, lines = None, []
        for e in entries:
            e.pop('sha256', None)
            e['previous'] = previous
            e['sha256'] = v.sha(v.canonical(e))
            previous = e['sha256']
            lines.append(v.canonical(e))
        raw = b'\n'.join(lines) + b'\n'
        (self.root / 'journal.jsonl.gz').write_bytes(gzip.compress(raw, mtime=0))
        projection = json.loads((self.root / 'PROJECTION.json').read_text())
        projection['derivative_journal_uncompressed_sha256'] = v.sha(raw)
        (self.root / 'PROJECTION.json').write_bytes(v.canonical(projection) + b'\n')

    def outcome(self, entries, phase='body-edit', stack='cargo'):
        return next(e['data'] for e in entries if e['kind'] == 'outcome'
                    and e['data']['phase'] == phase and e['data']['stack'] == stack)

    def test_actual_integrity_and_explicit_confounded_status(self):
        result = v.validate(self.root)
        self.assertEqual(result['actual_timed_calls'], 126)
        self.assertEqual(result['shipping_local_receipts'], 42)
        self.assertEqual(result['fresh_execution_nonces'], 54)
        self.assertFalse(result['review']['fair_edit_to_result'])
        self.assertIn('CONFOUNDED', result['status'])

    def test_empty_home_path_relocation_readonly_native_network_denied(self):
        # The copied archive is unrelated to the original absolute /RUN placeholder.
        result = subprocess.run([sys.executable, '-B', str(self.root / 'validate.py'), '--archive', str(self.root), '--run', 'initial'],
            env={'HOME': '', 'PATH': '', 'PYTHONDONTWRITEBYTECODE': '1', 'LC_ALL': 'C.UTF-8'},
            cwd=self.temp.name, capture_output=True, text=True, check=False)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(self.STATUS, result.stdout)

    def test_native_network_write_denials_are_active(self):
        code = '''import importlib.util, subprocess, socket, pathlib
spec=importlib.util.spec_from_file_location('v', 'validate.py')
v=importlib.util.module_from_spec(spec)
spec.loader.exec_module(v)
v.deny_side_effects()
operations=[lambda: subprocess.run(['/nonexistent-native']), lambda: socket.socket(), lambda: open('forbidden-write', 'w')]
for operation in operations:
    try:
        operation()
    except PermissionError:
        continue
    raise AssertionError('side effect was not denied')
print('all three denied')
'''
        result = subprocess.run([sys.executable, '-B', '-c', code], cwd=self.root,
            env={'HOME': '', 'PATH': '', 'PYTHONDONTWRITEBYTECODE': '1'}, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse((self.root / 'forbidden-write').exists())

    def test_raw_tamper_rejected_by_fixed_commitment(self):
        (self.root / 'r0/cargo/cold.stdout').write_text('fake')
        with self.assertRaises(ValueError):
            v.validate(self.root)

    def test_stale_execution_raw_nonce_rejected_even_resealed(self):
        entries = self.entries()
        row = self.outcome(entries)
        path = self.root / row['raw']['stdout']['file']
        path.write_text(path.read_text().replace('JR_RUNTIME_EXECUTED_' + str(row['nonce']), 'JR_RUNTIME_EXECUTED_1'))
        self.reseal(entries)
        with self.assertRaises(ValueError):
            v.semantics(self.root)

    def test_status_101_required_even_resealed(self):
        entries = self.entries()
        for e in entries:
            if e['kind'] in ('process', 'outcome') and e['data']['phase'] == 'type-error':
                e['data']['status'] = 0
        self.reseal(entries)
        with self.assertRaises(ValueError):
            v.semantics(self.root)

    def test_named_tests_and_counters_not_exitcode_only(self):
        entries = self.entries()
        row = self.outcome(entries, 'cold')
        path = self.root / row['raw']['stdout']['file']
        path.write_text(path.read_text().replace('summary::tests::shares_overlapping_time', 'stale::missing_test'))
        self.reseal(entries)
        with self.assertRaises(ValueError):
            v.semantics(self.root)

    def test_duplicate_nonce_rejected(self):
        entries = self.entries()
        rows = [e['data'] for e in entries if e['kind'] == 'outcome' and e['data']['nonce'] is not None]
        old, new = rows[1]['nonce'], rows[0]['nonce']
        for e in entries:
            if e['kind'] in ('intent', 'process', 'outcome') and e['data'].get('nonce') == old:
                e['data']['nonce'] = new
        self.reseal(entries)
        with self.assertRaises(ValueError):
            v.semantics(self.root)

    def test_remote_compiler_receipt_rejected(self):
        entries = self.entries()
        row = self.outcome(entries, 'cold', 'justrust')
        path = self.root / row['recorded']['meta']['file']
        meta = json.loads(path.read_text())
        meta['place'] = 'hosted'
        meta['real_rustc'] = '/WRONG/rustc'
        path.write_bytes(v.canonical(meta))
        self.reseal(entries)
        with self.assertRaises(ValueError):
            v.semantics(self.root)

    def test_tool_identity_rejected(self):
        entries = self.entries()
        entries[0]['data']['identities']['tools']['cargo']['sha256'] = '0' * 64
        self.reseal(entries)
        with self.assertRaises(ValueError):
            v.semantics(self.root)

    def test_shared_cache_rejected(self):
        entries = self.entries()
        states = [e['data'] for e in entries if e['kind'] == 'state']
        states[1]['environment']['CARGO_TARGET_DIR'] = states[0]['environment']['CARGO_TARGET_DIR']
        self.reseal(entries)
        with self.assertRaises(ValueError):
            v.semantics(self.root)

    def test_order_and_missing_calls_rejected(self):
        entries = self.entries()
        i = next(i for i, e in enumerate(entries) if e['kind'] == 'process')
        entries[i], entries[i + 1] = entries[i + 1], entries[i]
        self.reseal(entries)
        with self.assertRaises(ValueError):
            v.semantics(self.root)

    def test_coherent_timing_history_rewrite_rejected(self):
        entries = self.entries()
        for e in entries:
            if e['kind'] in ('process', 'outcome'):
                e['data']['elapsed_seconds'] *= 2
        self.reseal(entries)
        summary = json.loads((self.root / 'summary.json').read_text())
        for phase in summary['statistics'].values():
            for stack in ('cargo', 'justrust', 'blaze'):
                for key in ('median', 'min', 'max'):
                    phase[stack][key] *= 2
                phase[stack]['samples_seconds'] = [x * 2 for x in phase[stack]['samples_seconds']]
        (self.root / 'summary.json').write_bytes(v.canonical(summary))
        # Semantic outcomes and recomputed ratios alone cannot authenticate time.
        self.assertEqual(v.semantics(self.root)['actual_timed_calls'], 126)
        commitment = json.loads((self.root / 'COMMITMENT.json').read_text())
        for name in commitment['files']:
            commitment['files'][name] = v.sha((self.root / name).read_bytes())
        (self.root / 'COMMITMENT.json').write_bytes(v.canonical(commitment) + b'\n')
        with self.assertRaisesRegex(ValueError, 'fixed derivative commitment changed'):
            v.validate(self.root)

    def test_confounded_review_cannot_be_silently_removed(self):
        (self.root / 'REVIEW.json').write_text('{}')
        with self.assertRaises(ValueError):
            v.validate(self.root)

    def test_no_private_path_leak_in_public_derivative(self):
        for path in self.root.rglob('*'):
            if path.is_file() and path.name not in ('test_archive.py',):
                raw = gzip.decompress(path.read_bytes()) if path.name == 'journal.jsonl.gz' else path.read_bytes()
                self.assertNotIn(b'/home/' + b'jeremy', raw, str(path.relative_to(self.root)))


class PrimaryArchiveTests(ArchiveTests):
    STATUS = 'INTEGRITY-AND-SEMANTICS-PASS-CORRECTED-PRIMARY'

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='three-stack-primary-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name) / 'relocated'
        shutil.copytree(HERE / 'primary-v91', self.root, ignore=shutil.ignore_patterns('__pycache__'))
        shutil.copyfile(HERE / 'validate.py', self.root / 'validate.py')

    def test_actual_integrity_and_explicit_confounded_status(self):
        result = v.validate(self.root)
        self.assertEqual(result['actual_timed_calls'], 126)
        self.assertEqual(result['shipping_local_receipts'], 42)
        self.assertEqual(result['speculative_background_recordings'], 0)
        self.assertEqual(result['justrust_recordings_per_round'], [7] * 6)
        self.assertEqual(result['status'], self.STATUS)

    def test_confounded_review_cannot_be_silently_removed(self):
        # The corrected inventory is also fixed by its derivative commitment.
        proof = json.loads((self.root / 'recording-inventory.json').read_text())
        proof[0]['count'] = 8
        (self.root / 'recording-inventory.json').write_bytes(v.canonical(proof))
        with self.assertRaises(ValueError):
            v.validate(self.root)


if __name__ == '__main__':
    unittest.main()
