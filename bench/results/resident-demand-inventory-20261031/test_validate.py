#!/usr/bin/env python3
"""Offline semantic negatives. Every mutation has a coherent outer reseal."""
import contextlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('inventory_validator', HERE / 'validate.py')
v = importlib.util.module_from_spec(spec)
spec.loader.exec_module(v)


def dump(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')


class Replay(unittest.TestCase):
    @contextlib.contextmanager
    def bundle(self):
        with tempfile.TemporaryDirectory(prefix='inventory-offline-', dir=os.environ.get('JCODE_SCRATCH_DIR')) as tmp:
            target = Path(tmp) / 'bundle'
            shutil.copytree(HERE, target, ignore=shutil.ignore_patterns('__pycache__'))
            yield target

    def mutate_json(self, root, name, mutate):
        p = root / name
        value = v.load(p)
        mutate(value)
        dump(p, value)

    def resign_runtime(self, root, name='rsa27'):
        directory = root / 'runtime' / name
        suite = v.load(directory / 'suite.json')
        suite['metadata'] = v.load(directory / 'metadata.json')
        for filename in suite['artifacts_sha256']:
            suite['artifacts_sha256'][filename] = v.sha((directory / filename).read_bytes())
        dump(directory / 'suite.json', suite)

    def reject(self, mutate, expected):
        with self.bundle() as root:
            mutate(root)
            self.resign_runtime(root)
            v.seal(root)
            # Ensure rejection is semantic, not the outer checksum inventory.
            v.manifest(root)
            with self.assertRaisesRegex(ValueError, expected):
                v.validate(root)

    def test_positive_no_processes(self):
        with mock.patch('subprocess.Popen', side_effect=AssertionError('native execution forbidden')), mock.patch('subprocess.check_output', side_effect=AssertionError('checkout forbidden')):
            result = v.validate(HERE)
        self.assertEqual(result['native_calls_during_replay'], 0)
        self.assertFalse(result['speed_claim'])
        self.assertFalse(result['zero_semantic_overhead_claim'])

    def test_relocated_empty_home_no_checkout(self):
        with self.bundle() as root:
            home = root.parent / 'empty-home'
            home.mkdir()
            script = '''import json, pathlib, sys
root = pathlib.Path(sys.argv[1]).resolve()
def guard(event, args):
    if event in ('subprocess.Popen', 'os.system', 'socket.connect'):
        raise AssertionError('external execution/network forbidden')
    if event == 'open' and isinstance(args[0], str):
        path = pathlib.Path(args[0])
        if path.is_absolute() and str(path).startswith('/home/') and not path.is_relative_to(root):
            raise AssertionError('original user path forbidden: ' + str(path))
sys.addaudithook(guard)
sys.path.insert(0, str(root))
import validate
print(json.dumps(validate.validate(root), sort_keys=True))
'''
            proc = subprocess.run([sys.executable, '-I', '-B', '-c', script, str(root)], cwd=root.parent,
                                  env={'HOME': str(home), 'PATH': '/nonexistent', 'PYTHONDONTWRITEBYTECODE': '1'},
                                  capture_output=True, text=True, timeout=30)
            self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
            result = json.loads(proc.stdout)
            self.assertEqual(result['status'], 'passed')
            self.assertEqual(list(home.iterdir()), [])

    def test_inventory_enabled_control_removed(self):
        self.reject(lambda r: self.mutate_json(r, 'probe/state.json', lambda s: s['rows'][1]['startup_controls'].pop('RA_JIT_DEMAND_INVENTORY')), 'explicit separate startup controls')

    def test_std_startup_control_removed(self):
        self.reject(lambda r: self.mutate_json(r, 'probe/state.json', lambda s: s['rows'][1]['startup_controls'].pop('RA_JIT_DEMAND_STD')), 'explicit separate startup controls')

    def test_inventory_control_not_exact(self):
        self.reject(lambda r: self.mutate_json(r, 'probe/state.json', lambda s: s['rows'][1]['startup_controls'].update(RA_JIT_DEMAND_INVENTORY='true')), 'invalid startup control')

    def test_coherently_wrong_positive_inventory(self):
        def change(root):
            self.mutate_json(root, 'probe/state.json', lambda s: s['rows'][1]['inventory_counts'][0].__setitem__(0, 6999))
            p = root / 'probe/on-worker-0.log'
            p.write_text(p.read_text().replace('6998 compiled roots', '6999 compiled roots'))
        self.reject(change, 'native inventory counts/full round')

    def test_coherently_nonempty_found_map(self):
        def change(root):
            self.mutate_json(root, 'probe/state.json', lambda s: s['rows'][1]['inventory_counts'][0].__setitem__(3, 1))
            p = root / 'probe/on-worker-0.log'
            p.write_text(p.read_text().replace('0 found instances', '1 found instances'))
        self.reject(change, 'native inventory counts/full round')

    def test_full_round_marker_removed(self):
        def change(root):
            p = root / 'probe/on-worker-0.log'
            p.write_text(p.read_text().replace('(full round)', '(patch delta)'))
        self.reject(change, 'native inventory counts/full round')

    def test_off_unexpected_inventory(self):
        def change(root):
            p = root / 'probe/off-worker-0.log'
            p.write_text(p.read_text() + '\ndemand inventory: unavailable\n')
        self.reject(change, 'off inventory absence')

    def test_cold_four_tests_missing(self):
        def change(root):
            p = root / 'probe/on.stdout'
            p.write_text(p.read_text().replace('4 passed;', '3 passed;'))
        self.reject(change, 'four actual tests')

    def test_probe_tool_coherently_changed(self):
        def change(root):
            self.mutate_json(root, 'probe/state.json', lambda s: s.update(tool_before='0' * 64, tool_after='0' * 64))
        self.reject(change, 'native tool identity unchanged')

    def test_tc_coherently_changed(self):
        def change(root):
            def edit(s):
                for key in ('toolchain_before', 'toolchain_after'):
                    s[key]['files']['bin/rustc']['sha256'] = '0' * 64
            self.mutate_json(root, 'probe/state.json', edit)
        self.reject(change, 'frozen TC commitment')

    def test_v35_compiler_failure_not_erased(self):
        self.reject(lambda r: self.mutate_json(r, 'journals/20261030-182508083-573757/summary.json', lambda s: s.update(exit=0)), 'journal status/count')

    def test_full_codegen_raw_journal_missing_name(self):
        def change(root):
            p = root / 'journals/20261030-182838650-595329/output.jsonl'
            lines = [json.loads(l) for l in p.read_text().splitlines()]
            for row in lines:
                if row['l'].startswith('test ') and '... ok' in row['l']:
                    row['l'] = row['l'].replace('... ok', '... ignored')
                    break
            p.write_text(''.join(json.dumps(l) + '\n' for l in lines))
        self.reject(change, 'full passing names')

    def test_source_after_line_missing(self):
        def change(root):
            p = root / 'gates/candidate-unit-v36/all-source-after.log'
            p.write_text('\n'.join(p.read_text().splitlines()[1:]) + '\n')
        self.reject(change, 'all-source before/after gate')

    def test_fixture_repair_not_substituted(self):
        def change(root):
            p = root / 'frozen/v36/codegen-mod.rs'
            p.write_text(p.read_text().replace('(1, local.as_u32())', '(0, local.as_u32())'))
        self.reject(change, 'v36 accepted byte binding')

    def test_accepted_patch_changed(self):
        def change(root):
            p = root / 'accepted/change.patch'
            p.write_text(p.read_text().replace('+    pub inventory: Option<CodegenInventory>,', '+    pub inventory: Option<CodegenUnit>,'))
        self.reject(change, 'gitshow full-byte replay')

    def test_runtime_missing_executed_marker_resigned_suite(self):
        def change(root):
            p = root / 'runtime/rsa27/engine-edit-4.stdout'
            p.write_text(p.read_text().replace('JR_RUNTIME_EXECUTED_5', 'NOT_EXECUTED_5'))
        self.reject(change, 'executed edit marker')

    def test_runtime_wrong_source_hash_resigned_suite(self):
        def change(root):
            self.mutate_json(root, 'runtime/rsa27/records.json', lambda rows: next(x for x in rows if x['phase'] == 'edit').update(source_sha256='0' * 64))
        self.reject(change, 'exact runtime source edit identity')

    def test_runtime_error_marker_missing_resigned_suite(self):
        def change(root):
            p = root / 'runtime/rsa27/engine-type-error-0.stderr'
            p.write_text(p.read_text().replace('JR_RUNTIME_TYPE_ERROR', 'MISSING_MARKER'))
        self.reject(change, 'actual type error')

    def test_runtime_recovery_failure_resigned_suite(self):
        def change(root):
            self.mutate_json(root, 'runtime/rsa27/records.json', lambda rows: next(x for x in rows if x['phase'] == 'type-recovery').update(status=101))
        self.reject(change, 'runtime success/recovery')

    def test_runtime_row_dropped_resigned_suite(self):
        self.reject(lambda r: self.mutate_json(r, 'runtime/rsa27/records.json', lambda rows: rows.pop()), 'all 22 runtime rows')

    def test_unknown_environment_rejected(self):
        self.reject(lambda r: self.mutate_json(r, 'probe/state.json', lambda s: s['rows'][0]['environment'].update(UNEXPECTED_PRIVATE_TOKEN='not-a-secret')), 'unexpected environment key')

    def test_no_zero_noise_upgrade(self):
        self.reject(lambda r: self.mutate_json(r, 'probe/state.json', lambda s: s['rows'][0]['noise'].update(contamination='quiet')), 'noise must remain unknown')

    def test_733_root_log_not_reduced_to_summary(self):
        self.reject(lambda r: (r / 'outer/32891831af.output').write_text('OK\n'), '733 Python root raw receipt')


if __name__ == '__main__':
    unittest.main(verbosity=2)
