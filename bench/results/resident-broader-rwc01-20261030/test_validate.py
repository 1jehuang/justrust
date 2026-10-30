#!/usr/bin/env python3
"""Offline semantic rejection tests with coherent re-sealing, plus relocation."""
import copy
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
spec = importlib.util.spec_from_file_location('broader_validator', HERE / 'validate.py')
validator = importlib.util.module_from_spec(spec)
spec.loader.exec_module(validator)
collect_spec = importlib.util.spec_from_file_location('broader_collector', HERE / 'collect.py')
collector = importlib.util.module_from_spec(collect_spec)
collect_spec.loader.exec_module(collector)


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')


def resign(root):
    """Update derivative/raw hashes and outer seal so rejection is semantic."""
    for run in ('rwc01', 'rwc02'):
        folder = root / 'runs' / run
        receipt = validator.load(folder / 'transformation.json')
        for name in receipt['derivative_sha256']:
            digest = validator.sha((folder / name).read_bytes())
            receipt['derivative_sha256'][name] = digest
            if name not in ('metadata.sanitized.json', 'records.sanitized.json'):
                receipt['private_original_sha256'][name] = digest
        write_json(folder / 'transformation.json', receipt)
    validator.seal(root)


class EvidenceTests(unittest.TestCase):
    def clone(self):
        temp = tempfile.TemporaryDirectory(prefix='rwc-offline-')
        self.addCleanup(temp.cleanup)
        root = Path(temp.name) / 'relocated'
        shutil.copytree(HERE, root, ignore=shutil.ignore_patterns('__pycache__', '*.pyc'))
        return root

    def reject(self, change, message):
        root = self.clone()
        change(root)
        resign(root)
        # A checksum-only validator would now accept. The semantic gate must fail.
        validator.manifest(root)
        with self.assertRaisesRegex(ValueError, message):
            validator.validate(root)

    def row_change(self, callback, phase='edit', workload='summary-test'):
        def change(root):
            path = root / 'runs/rwc01/records.sanitized.json'
            rows = validator.load(path)
            row = next(r for r in rows if r['stack'] == 'engine'
                       and r['phase'] == phase and r['workload'] == workload)
            callback(root, row)
            write_json(path, rows)
        return change

    def meta_change(self, callback):
        def change(root):
            path = root / 'runs/rwc01/metadata.sanitized.json'
            meta = validator.load(path)
            callback(meta)
            write_json(path, meta)
        return change

    def test_baseline_offline_does_not_spawn_or_read_absolute_paths(self):
        real_open = Path.open
        def only_retained_files(path, *args, **kwargs):
            self.assertTrue(path.absolute().is_relative_to(HERE), 'read outside relocated evidence')
            return real_open(path, *args, **kwargs)
        with patch.object(subprocess, 'Popen', side_effect=AssertionError('native execution forbidden')), patch.object(
                Path, 'open', only_retained_files):
            result = validator.validate(HERE)
        self.assertEqual(result['status'], 'PASS')
        self.assertEqual(result['total_rows'], 164)
        self.assertEqual(result['total_timed_edits'], 80)
        self.assertEqual(result['raw_streams'], 328)

    def test_relocated_cli_without_checkout_or_home(self):
        root = self.clone()
        with tempfile.TemporaryDirectory(prefix='rwc-cwd-') as cwd:
            env = dict(os.environ, HOME=cwd, PYTHONPATH='', PYTHONDONTWRITEBYTECODE='1')
            result = subprocess.run([sys.executable, str(root / 'validate.py')], cwd=cwd,
                                    env=env, capture_output=True, text=True, timeout=30)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads(result.stdout)['status'], 'PASS')

    def test_stale_missing_duplicate_markers(self):
        for replacement in ('', 'JR_WORKLOAD_EXECUTED_summary_test_999',
                            'JR_WORKLOAD_EXECUTED_summary_test_1 JR_WORKLOAD_EXECUTED_summary_test_1'):
            def alter(root, row):
                path = root / 'runs/rwc01' / (Path(row['raw_prefix']).name + '.stdout')
                path.write_text(path.read_text().replace('JR_WORKLOAD_EXECUTED_summary_test_1', replacement))
                row['parsed'] = validator.parse_tests(path.read_text())
            self.reject(self.row_change(alter), 'executed unique edit marker')

    def test_coherent_wrong_test_name_and_filtered_count(self):
        def alter(root, row):
            path = root / 'runs/rwc01' / (Path(row['raw_prefix']).name + '.stdout')
            path.write_text(path.read_text().replace('parses_libtest_results', 'invented_test')
                            .replace('144 filtered', '143 filtered'))
            row['parsed'] = validator.parse_tests(path.read_text())
        self.reject(self.row_change(alter), 'exact native test names/counts')

    def test_type_error_must_be_real(self):
        def alter(root, row):
            path = root / 'runs/rwc01' / (Path(row['raw_prefix']).name + '.stderr')
            path.write_text(path.read_text().replace('E0308', 'E9999'))
        self.reject(self.row_change(alter, phase='type-error'), 'deliberate type error')

    def test_assertion_oracle_must_be_present(self):
        def alter(root, row):
            path = root / 'runs/rwc01' / (Path(row['raw_prefix']).name + '.stderr')
            path.write_text(path.read_text().replace('JR_WORKLOAD_FAILURE', 'REMOVED_ORACLE'))
        self.reject(self.row_change(alter, phase='assert-failure'), 'assertion failure oracle')

    def test_recovery_status_is_not_a_compile_failure(self):
        def alter(root, row):
            row['status'] = 101
        self.reject(self.row_change(alter, phase='type-recovery'), 'process exit status')

    def test_timeout_and_invalid_duration_and_rejected_marker(self):
        for key, value in (('timed_out', True), ('elapsed_seconds', float('nan')),
                           ('elapsed_seconds', -1), ('accepted', False)):
            def alter(root, row):
                row[key] = value
            self.reject(self.row_change(alter), 'process error/timeout/duration|executed unique edit marker')

    def test_coherent_edit_input_hash_still_requires_exact_bytes(self):
        def alter(root, row):
            row['input_sha256'] = row['source_sha256'] = '0' * 64
        self.reject(self.row_change(alter, workload='shim-function'), 'edited byte identity')

    def test_coherent_identity_mutations(self):
        def tools(meta):
            meta['tools_before']['engine']['sha256'] = '0' * 64
            meta['tools_after'] = copy.deepcopy(meta['tools_before'])
        def toolchain(meta):
            meta['toolchain_before']['path'] = '/invented/toolchain'
            meta['toolchain_after'] = copy.deepcopy(meta['toolchain_before'])
        def source(meta):
            meta['source_before']['src/shim.rs'] = '0' * 64
            meta['source_after'] = copy.deepcopy(meta['source_before'])
        for callback, message in ((tools, 'engine identity'), (toolchain, 'toolchain identity'),
                                   (source, 'source identity')):
            self.reject(self.meta_change(callback), message)

    def test_metadata_and_process_environment_secrets_rejected(self):
        self.reject(self.meta_change(lambda m: m['environments']['engine'].update(
            API_TOKEN='synthetic-never-real')), 'environment policy')
        self.reject(self.row_change(lambda root, row: row['environment'].update(
            API_TOKEN='synthetic-never-real')), 'process environment drift/secret')

    def test_shared_caches_rejected(self):
        self.reject(self.meta_change(lambda m: m['environments']['engine'].update(
            CARGO_TARGET_DIR=m['environments']['reference']['CARGO_TARGET_DIR'])), 'private cache roots')

    def test_coherent_backup_mutation_rejected(self):
        def change(root):
            path = root / 'runs/rwc01/metadata.sanitized.json'
            meta = validator.load(path)
            backup = meta['backups']['src/shim.rs']
            local = root / 'runs/rwc01' / Path(backup['path']).name
            local.write_bytes(local.read_bytes() + b'\n// synthetic mutation\n')
            backup['sha256'] = validator.sha(local.read_bytes())
            write_json(path, meta)
        self.reject(change, 'backup/source mismatch')

    def test_coherent_schedule_mutation_rejected(self):
        def change(root):
            path = root / 'runs/rwc01/records.sanitized.json'
            rows = validator.load(path)
            rows[2], rows[3] = rows[3], rows[2]
            write_json(path, rows)
        self.reject(change, 'invocation/gate/order schedule')

    def test_coherent_median_mutation_rejected(self):
        def change(root):
            path = root / 'runs/rwc01/summary.json'
            summary = validator.load(path)
            summary['medians_seconds']['summary-test']['engine'] = 0.00001
            write_json(path, summary)
        self.reject(change, 'median/excluded')

    def test_missing_raw_and_extra_source_input_rejected(self):
        root = self.clone()
        (root / 'runs/rwc01/engine-summary-test-edit-0.stdout').unlink()
        validator.seal(root)
        with self.assertRaises((ValueError, FileNotFoundError)):
            validator.validate(root)
        self.reject(lambda root: (root / 'input/source63/unexpected.txt').write_text('extra'),
                    'unexpected source63 input')

    def test_regression_log_cannot_be_replaced_by_claimed_pass(self):
        def change(root):
            path = root / 'root/bench-broader-v28-regression.log'
            path.write_text(path.read_text().replace('Ran 710 tests', 'Ran 709 tests'))
        self.reject(change, 'root 710-test regression')

    def test_historical_harness_cannot_be_resigned(self):
        self.reject(lambda root: (root / 'provenance/0dba7c33e/resident_workload_compare.py').write_text(
            '# synthetic different harness\n'), 'historical harness/helper bytes')

    def test_paths_and_symlinks_rejected(self):
        root = self.clone()
        with (root / 'SHA256SUMS').open('a') as stream:
            stream.write('0' * 64 + '  ../escape\n')
        with self.assertRaisesRegex(ValueError, 'unsafe evidence path'):
            validator.validate(root)
        root = self.clone()
        (root / 'escaping-link').symlink_to('/etc/passwd')
        with self.assertRaisesRegex(ValueError, 'symlink evidence'):
            validator.seal(root)

    def test_exporter_api_token_omission_and_originals_unchanged(self):
        with tempfile.TemporaryDirectory(prefix='rwc-sanitize-') as temp:
            root = Path(temp)
            original = root / 'private'
            original.mkdir()
            private_meta = {'environments': {'engine': dict(API_TOKEN='NEVER_PUBLISH_SENTINEL',
                                                            BLAZE_OPT='0')}}
            private_rows = [{'environment': dict(API_TOKEN='NEVER_PUBLISH_SENTINEL', BLAZE_OPT='0')}]
            write_json(original / 'metadata.json', private_meta)
            write_json(original / 'records.json', private_rows)
            write_json(original / 'summary.json', {})
            (original / 'engine-edit.stdout').write_text('ordinary safe raw output')
            before = {p.name: p.read_bytes() for p in original.iterdir()}
            collector.export_run(original, root / 'public')
            after = {p.name: p.read_bytes() for p in original.iterdir()}
            self.assertEqual(before, after)
            for path in (root / 'public').iterdir():
                self.assertNotIn(b'API_TOKEN', path.read_bytes())
                self.assertNotIn(b'NEVER_PUBLISH_SENTINEL', path.read_bytes())
            receipt = validator.load(root / 'public/transformation.json')
            self.assertEqual(receipt['omitted'], dict(metadata_environment_entries=1,
                                                       process_environment_entries=1))
            self.assertEqual(receipt['private_original_sha256']['metadata.json'],
                             validator.sha(before['metadata.json']))


if __name__ == '__main__':
    unittest.main()
