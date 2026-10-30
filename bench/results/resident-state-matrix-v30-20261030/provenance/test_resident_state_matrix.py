#!/usr/bin/env python3
"""Synthetic mocks only. No engine, compiler, build or native workload is executed."""
import argparse
from contextlib import ExitStack
import copy
import json
import os
from pathlib import Path
import subprocess
import tempfile
import tomllib
import unittest
from unittest.mock import patch

import resident_state_matrix as matrix


def parsed(names=None, ignored=False):
    tests = [(name, 'ok') for name in (names or matrix.shared.NAMES)]
    if ignored:
        tests += [('split_index::tests::attach_cost', 'ignored')]
    return dict(tests=sorted(tests), passed=len(tests) - int(ignored), failed=0,
                ignored=int(ignored), filtered=0 if ignored else 144)


def full_contract():
    names = matrix.shared.NAMES + ['shim::tests::describes_cargo_rustc_args',
                                   'shim::tests::parses_time_passes_json']
    names += [f'synthetic::case_{i:03}' for i in range(141)]
    return parsed(names, ignored=True)


def output(contract):
    lines = [f'test {name} ... {status}' for name, status in contract['tests']]
    state = 'FAILED' if contract['failed'] else 'ok'
    lines.append(f"test result: {state}. {contract['passed']} passed; {contract['failed']} failed; "
                 f"{contract['ignored']} ignored; 0 measured; {contract['filtered']} filtered out")
    return '\n'.join(lines) + '\n'


class Fixture:
    def __init__(self, root):
        self.root = root
        self.source = root / 'source63'
        (self.source / 'src').mkdir(parents=True)
        self.originals = {
            'src/summary.rs': b'fn shares_overlapping_time() { assert!(true); }\n',
            'src/shim.rs': (b'fn describes_cargo_rustc_args() { assert!(true); }\n'
                           b'fn parse_pass(json: &[u8]) -> Option<Pass> { None }\n'),
            'Cargo.toml': b'[package]\nname = "synthetic"\nversion = "0.0.0"\n',
        }
        for name, data in self.originals.items():
            (self.source / name).write_bytes(data)
        self.tools = {}
        for stack in matrix.STACKS:
            tool = root / stack
            tool.write_bytes(b'\x7fELFsynthetic-never-executed')
            tool.chmod(0o700)
            self.tools[stack] = tool
        self.tc = root / 'tc'
        self.tc.mkdir()
        self.args = argparse.Namespace(source=self.source, output=root / 'o',
            toolchain=self.tc, reference_engine=self.tools['reference'],
            engine=self.tools['engine'], proc_macro_server=None, samples=5, timeout=2)
        self.calls = []
        self.fault = None

    def runner(self, command, cwd, env, prefix, timeout):
        stack, phase = prefix.name.split('-', 1)
        rep = int(prefix.parent.name[1:])
        self.calls.append(dict(stack=stack, phase=phase, rep=rep, command=command,
            nonce=env.get('JR_MATRIX_NONCE'), inputs=matrix.shared.source_hashes(cwd),
            mtimes={n: (cwd / n).stat().st_mtime_ns for n in matrix.FILES}))
        row = dict(status=0, error=None, timed_out=False, elapsed_seconds=rep + 0.25,
                   command=list(map(str, command)), environment={'SECRET_TOKEN': 'not-a-secret-fixture'},
                   noise={'contamination': 'unknown'})
        if phase == 'version':
            text = matrix.shared.REFERENCE_VERSION + '\n' if stack == 'reference' else 'jr-engine synthetic\n'
        elif phase == 'type-error':
            row['status'] = 101
            text = 'error[E0308]: JR_MATRIX_TYPE_' + env['JR_MATRIX_NONCE'] + '\n'
        else:
            contract = parsed()
            if phase == 'ordinary-shim-edit':
                contract = parsed(['shim::tests::describes_cargo_rustc_args',
                                   'shim::tests::parses_time_passes_json'])
            elif phase == 'all-lib':
                contract = full_contract()
            elif phase == 'assert-failure':
                row['status'] = 101
                contract['tests'] = [(n, 'FAILED' if n == matrix.shared.EDIT_TEST else s)
                                     for n, s in contract['tests']]
                contract.update(passed=3, failed=1)
            text = ('JR_MATRIX_RUNTIME_' + env['JR_MATRIX_NONCE'] + '\n') * (2 if phase == 'all-lib' else 1)
            if phase == 'summary-testbody-edit':
                text += f'JR_RUNTIME_EXECUTED_{rep + 1}\n'
            elif phase == 'ordinary-shim-edit':
                text += f'JR_WORKLOAD_EXECUTED_shim_function_{rep + 1}\n' * 2
            elif phase == 'assert-failure':
                text += 'JR_MATRIX_FAILURE_' + env['JR_MATRIX_NONCE'] + '\n'
            text += output(contract)
        if self.fault:
            row, text = self.fault(stack, phase, rep, row, text)
        for suffix, data in (('stdout', text), ('stderr', '')):
            with prefix.with_suffix('.' + suffix).open('x') as stream:
                stream.write(data)
        return row

    def run(self):
        with ExitStack() as patches:
            pinned = patches.enter_context(patch.object(matrix.shared, 'pinned_source'))
            patches.enter_context(patch.object(matrix.shared, 'toolchain_identity',
                return_value={'synthetic': 'immutable'}))
            patches.enter_context(patch.object(matrix.runtime, 'run_process', side_effect=self.runner))
            result = matrix.execute(self.args)
            pinned.assert_called_once_with(self.source)
        return result

    def read(self, filename):
        return json.loads((self.args.output / filename).read_text())


class MatrixTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='mx-', dir=os.environ.get('JCODE_SCRATCH_DIR'))
        self.addCleanup(self.temp.cleanup)
        self.fixture = Fixture(Path(self.temp.name))

    def test_complete_synthetic_sessions_rotate_reuse_and_restore(self):
        f = self.fixture
        self.assertEqual(f.run(), 0)
        meta, rows, summary = f.read('metadata.json'), f.read('records.json'), f.read('summary.json')
        self.assertEqual(meta['status'], 'complete')
        self.assertTrue(meta['source_restored'])
        self.assertEqual(meta['planned_version_calls'], 10)
        self.assertEqual(meta['planned_workload_calls'], 140)
        self.assertEqual(len(rows), len(matrix.PHASES) * 5 * 2)
        self.assertTrue(all(r['accepted'] for r in rows))
        self.assertTrue(all(r['environment'].keys() <= matrix.workload.ENV_RECEIPT_ALLOWLIST for r in rows))
        self.assertTrue(all('SECRET_TOKEN' not in v['process']['environment'] for v in meta['versions'].values()))
        for phase in matrix.PHASES:
            self.assertEqual(meta['counts'][phase], 10)
            self.assertEqual(summary['states'][phase]['engine']['median_seconds'], 2.25)
        for rep in range(5):
            calls = [c for c in f.calls if c['rep'] == rep and c['phase'] == 'cold-prepare']
            self.assertEqual([c['stack'] for c in calls], matrix.runtime.order(matrix.STACKS, rep))
            for stack in matrix.STACKS:
                noops = [c for c in f.calls if c['rep'] == rep and c['stack'] == stack
                         and c['phase'] in ('cold-prepare', 'repeated-noop-1', 'repeated-noop-2')]
                self.assertEqual(len({json.dumps(c['inputs'], sort_keys=True) for c in noops}), 1)
                self.assertEqual(len({json.dumps(c['mtimes'], sort_keys=True) for c in noops}), 1)
                self.assertEqual(len({c['nonce'] for c in noops}), 3)
                phase_rows = {r['phase']: r for r in rows if r['replicate'] == rep and r['stack'] == stack}
                for phase in ('repeated-noop-1', 'repeated-noop-2'):
                    self.assertEqual(phase_rows[phase]['changed_inputs_since_stack_phase'], [])
                self.assertEqual(phase_rows['ordinary-shim-edit']['changed_inputs_since_stack_phase'], ['src/shim.rs'])
                self.assertEqual(phase_rows['signature-invalidation']['changed_inputs_since_stack_phase'], ['src/summary.rs'])
                self.assertEqual(phase_rows['manifest-feature-add']['changed_inputs_since_stack_phase'], ['Cargo.toml'])
        for name, original in f.originals.items():
            self.assertEqual((f.source / name).read_bytes(), original)
        self.assertEqual(len({r['cache_scope'] for r in rows}), 10)
        self.assertEqual(meta['contracts']['0:ordinary-shim-edit']['ordinary_invocations'], 2)
        self.assertEqual(meta['contracts']['0:all-lib']['expected']['ignored'], 1)
        self.assertEqual(len(meta['contracts']['0:all-lib']['expected']['tests']), 148)
        for r in rows:
            for receipt in r['raw'].values():
                self.assertEqual(matrix.runtime.base.digest(Path(receipt['path'])), receipt['sha256'])

    def test_stale_noop_output_is_excluded_not_silently_dropped(self):
        f = self.fixture
        def fault(stack, phase, rep, row, text):
            if stack == 'engine' and phase == 'repeated-noop-1':
                text = text.replace('JR_MATRIX_RUNTIME_' + f.calls[-1]['nonce'], 'JR_MATRIX_RUNTIME_stale')
            return row, text
        f.fault = fault
        self.assertEqual(f.run(), 1)
        state = f.read('summary.json')['states']['repeated-noop-1']['engine']
        self.assertTrue(state['excluded'])
        self.assertEqual(state['observed_samples'], 5)
        self.assertIsNone(state['median_seconds'])
        self.assertIn('fresh invocation marker/count mismatch', state['failures'][0]['reasons'])
        self.assertFalse(f.read('summary.json')['states']['type-recovery']['engine']['excluded'])

    def test_timeout_retains_raw_and_explicit_unattempted_phases(self):
        f = self.fixture
        def fault(stack, phase, rep, row, text):
            if stack == 'engine' and phase == 'ordinary-shim-edit' and rep == 0:
                row.update(timed_out=True, status=-9)
            return row, text
        f.fault = fault
        self.assertEqual(f.run(), 1)
        rows = f.read('records.json')
        timed = next(r for r in rows if r['stack'] == 'engine' and r['phase'] == 'ordinary-shim-edit' and r['replicate'] == 0)
        self.assertTrue(timed['timed_out'])
        self.assertTrue(Path(timed['raw']['stdout']['path']).exists())
        later = next(r for r in rows if r['stack'] == 'engine' and r['phase'] == 'signature-base' and r['replicate'] == 0)
        self.assertIn('previous timeout', later['failures'][0])
        self.assertEqual(f.read('summary.json')['states']['signature-base']['engine']['observed_samples'], 5)

    def test_exception_restores_manifest_and_both_sources_and_excludes_all(self):
        f = self.fixture
        def fault(stack, phase, rep, row, text):
            if phase == 'manifest-feature-add':
                self.assertIn(b'[features]', (f.source / 'Cargo.toml').read_bytes())
                raise RuntimeError('synthetic interruption')
            return row, text
        f.fault = fault
        with self.assertRaisesRegex(RuntimeError, 'synthetic interruption'):
            f.run()
        for name, original in f.originals.items():
            self.assertEqual((f.source / name).read_bytes(), original)
        summary = f.read('summary.json')
        self.assertTrue(f.read('metadata.json')['source_restored'])
        self.assertFalse(summary['identities_unchanged'])
        self.assertTrue(all(s['excluded'] for p in summary['states'].values() for s in p.values()))

    def test_changed_tool_identity_excludes_every_median(self):
        f = self.fixture
        def fault(stack, phase, rep, row, text):
            if phase == 'all-lib' and rep == 4 and stack == 'engine':
                f.tools['engine'].write_bytes(b'\x7fELFchanged-fixture')
            return row, text
        f.fault = fault
        self.assertEqual(f.run(), 1)
        self.assertFalse(f.read('summary.json')['identities_unchanged'])
        self.assertTrue(all(s['median_seconds'] is None for p in f.read('summary.json')['states'].values() for s in p.values()))

    def test_mutated_raw_receipt_excludes_every_median(self):
        f = self.fixture
        def fault(stack, phase, rep, row, text):
            if phase == 'all-lib' and rep == 4 and stack == 'engine':
                artifact = f.args.output / 's0/reference-cold-prepare.stdout'
                artifact.write_text(artifact.read_text() + 'synthetic late writer\n')
            return row, text
        f.fault = fault
        self.assertEqual(f.run(), 1)
        self.assertFalse(f.read('metadata.json')['raw_receipts_unchanged'])
        self.assertFalse(f.read('summary.json')['identities_unchanged'])

    def test_reference_runtime_failure_blocks_candidate_contract(self):
        f = self.fixture
        def fault(stack, phase, rep, row, text):
            if stack == 'reference' and phase == 'all-lib':
                text = text.replace('JR_MATRIX_RUNTIME_' + f.calls[-1]['nonce'], 'JR_MATRIX_RUNTIME_stale')
            return row, text
        f.fault = fault
        self.assertEqual(f.run(), 1)
        states = f.read('summary.json')['states']['all-lib']
        self.assertTrue(all(s['excluded'] for s in states.values()))
        self.assertIn('reference state oracle unavailable/failed', states['engine']['failures'][0]['reasons'])

    def test_candidate_full_lib_name_drift_rejected(self):
        f = self.fixture
        def fault(stack, phase, rep, row, text):
            if stack == 'engine' and phase == 'all-lib':
                text = text.replace('synthetic::case_000', 'synthetic::replacement')
            return row, text
        f.fault = fault
        self.assertEqual(f.run(), 1)
        states = f.read('summary.json')['states']
        self.assertTrue(states['all-lib']['engine']['excluded'])
        self.assertFalse(states['all-lib']['reference']['excluded'])

    def test_full_lib_reference_admits_only_exact_pinned_ignore(self):
        expected = full_contract()
        self.assertEqual(matrix.contract(expected, 'all-lib'), expected)
        drift = copy.deepcopy(expected)
        drift['tests'] = [(n.replace('attach_cost', 'other_ignore'), s) for n, s in drift['tests']]
        self.assertIsNone(matrix.contract(drift, 'all-lib'))
        drift = copy.deepcopy(expected)
        drift.update(passed=146, ignored=2)
        self.assertIsNone(matrix.contract(drift, 'all-lib'))

    def test_oversized_raw_fails_closed_without_truncating_receipt(self):
        prefix = self.fixture.root / 'bounded'
        prefix.with_suffix('.stdout').write_bytes(b'x' * 100)
        prefix.with_suffix('.stderr').write_bytes(b'')
        with patch.object(matrix, 'MAX_RAW_BYTES', 16):
            raw, text, error = matrix.captured(prefix)
        self.assertEqual(text, '')
        self.assertIn('bounded parser limit', error)
        self.assertEqual(raw['stdout']['bytes'], 100)
        self.assertEqual(prefix.with_suffix('.stdout').read_bytes(), b'x' * 100)
        row = dict(input_unchanged=True, status=0, error=None, timed_out=False,
                   parsed=parsed(), raw_error=error)
        self.assertIn(error, matrix.oracle(row, 'JR_MATRIX_RUNTIME_n', parsed(), 'repeated-noop-1', 'n', 0))

    def test_pinned_gitshow_recipes_and_manifest_semantics(self):
        repo = Path(__file__).resolve().parent.parent
        originals = {name: subprocess.check_output(['git', 'show', matrix.shared.PIN + ':' + name], cwd=repo)
                     for name in matrix.FILES}
        base = matrix.baseline(originals)
        self.assertEqual(base['src/summary.rs'].count(matrix.PROBE), 1)
        self.assertEqual(base['src/shim.rs'].count(matrix.PROBE), 1)
        for phase in matrix.PHASES:
            state = matrix.inputs(originals, phase, 0, 'pinnedmock')
            self.assertEqual(set(state), set(matrix.FILES))
        state = matrix.inputs(originals, 'manifest-feature-add', 0, 'pinnedmock')
        before = tomllib.loads(originals['Cargo.toml'].decode())
        after = tomllib.loads(state['Cargo.toml'].decode())
        self.assertEqual(after.pop('features'), {'jr_matrix_unused': []})
        self.assertEqual(after, before)
        declared = subprocess.check_output(['git', 'show', matrix.shared.PIN + ':src/split_index.rs'], cwd=repo)
        self.assertIn(b'#[ignore]\n    fn attach_cost()', declared)

    def test_imported_recipes_and_signature_manifest_transitions(self):
        original = self.fixture.originals
        base = matrix.baseline(original)
        edit = matrix.inputs(original, 'summary-testbody-edit', 2, 'n')
        self.assertEqual(edit[matrix.FILES[0]], matrix.runtime.edit_bytes(base[matrix.FILES[0]], 2))
        ordinary = matrix.inputs(original, 'ordinary-shim-edit', 2, 'n')
        self.assertEqual(ordinary[matrix.FILES[1]], matrix.workload.edited(base[matrix.FILES[1]], 'shim-function', 'edit', 2))
        signature = matrix.inputs(original, 'signature-invalidation', 0, 'n')
        self.assertIn(matrix.SIGNATURE_CHANGED, signature[matrix.FILES[0]])
        self.assertNotIn(matrix.SIGNATURE, signature[matrix.FILES[0]])
        manifest = matrix.inputs(original, 'manifest-feature-add', 0, 'n')
        self.assertEqual(manifest[matrix.FILES[0]], signature[matrix.FILES[0]])
        self.assertEqual(manifest[matrix.FILES[1]], signature[matrix.FILES[1]])
        self.assertEqual(manifest['Cargo.toml'], original['Cargo.toml'] + b'\n[features]\njr_matrix_unused = []\n')
        failing = matrix.inputs(original, 'assert-failure', 0, 'fresh')[matrix.FILES[0]]
        self.assertLess(failing.index(matrix.PROBE), failing.index(b'JR_MATRIX_FAILURE_fresh'))

    def test_freshness_marker_duplicates_and_wrong_ordinary_counts_fail(self):
        row = dict(input_unchanged=True, status=0, error=None, timed_out=False, parsed=parsed())
        for text in ('', 'JR_MATRIX_RUNTIME_old', 'JR_MATRIX_RUNTIME_n-stale', 'JR_MATRIX_RUNTIME_n\nJR_MATRIX_RUNTIME_n'):
            self.assertTrue(matrix.oracle(row, text, parsed(), 'repeated-noop-1', 'n', 0))
        self.assertFalse(matrix.oracle(row, 'JR_MATRIX_RUNTIME_n', parsed(), 'repeated-noop-1', 'n', 0))
        text = 'JR_MATRIX_RUNTIME_n\nJR_WORKLOAD_EXECUTED_shim_function_1\n'
        self.assertIn('ordinary function invocation count not reference-validated',
                      matrix.oracle(row, text, parsed(), 'ordinary-shim-edit', 'n', 0, 2))

    def test_type_error_requires_current_nonce_no_test_execution(self):
        row = dict(input_unchanged=True, status=101, error=None, timed_out=False, parsed=None)
        self.assertFalse(matrix.oracle(row, 'E0308 JR_MATRIX_TYPE_fresh', None, 'type-error', 'fresh', 0))
        self.assertTrue(matrix.oracle(row, 'E0308 JR_MATRIX_TYPE_stale', None, 'type-error', 'fresh', 0))
        self.assertTrue(matrix.oracle(row, 'E0308 JR_MATRIX_TYPE_fresh JR_MATRIX_RUNTIME_fresh', None, 'type-error', 'fresh', 0))

    def test_rejects_unbounded_timeout_and_insufficient_samples_before_processes(self):
        for samples, timeout in ((4, 2), (31, 2), (5, float('nan')), (5, float('inf')), (5, 0), (5, 3601)):
            self.fixture.args.samples, self.fixture.args.timeout = samples, timeout
            with self.assertRaises(ValueError):
                self.fixture.run()
            self.assertFalse(self.fixture.calls)

    def test_receipt_allowlist_never_logs_credentials(self):
        env = dict(HOME='/synthetic', CARGO_HOME='/synthetic/cargo',
                   AWS_SECRET_ACCESS_KEY='fixture', GITHUB_TOKEN='fixture',
                   SECRET_TOKEN='fixture', JR_MATRIX_NONCE='fixture')
        self.assertEqual(matrix.workload.receipt_environment(env),
                         {'HOME': '/synthetic', 'CARGO_HOME': '/synthetic/cargo'})


if __name__ == '__main__':
    unittest.main()
