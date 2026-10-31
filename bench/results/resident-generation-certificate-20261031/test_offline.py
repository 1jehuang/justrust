#!/usr/bin/env python3
"""Offline semantic, adversarial mutation, export-privacy and relocation tests."""
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

sys.dont_write_bytecode = True
HERE = Path(__file__).resolve().parent


def module(name):
    spec = importlib.util.spec_from_file_location(name, HERE / (name + '.py'))
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


validator = module('validate')
exporter = module('export_private')
DATA = json.loads((HERE / 'observations.json').read_text())


def row(data, index):
    return data['groups']['whole-generation-v64-on-seven']['rows'][index]


def fallback(data):
    return data['groups']['manifest-cfg-reload-v64']['rows'][7]


class OfflineEvidenceTests(unittest.TestCase):
    def test_actual_observations(self):
        self.assertEqual(validator.validate(DATA), {'correctness_rows': 35, 'same_binary_seven_state_rows': 14, 'decision': 'rejected-restored', 'speedup_claim': False})

    def test_adversarial_mutations_rejected(self):
        mutations = {
            'test-name': lambda d: row(d, 1).__setitem__('stdout', row(d, 1)['stdout'].replace('parses_libtest_results', 'different_test')),
            'passed-count': lambda d: row(d, 1).__setitem__('stdout', row(d, 1)['stdout'].replace('4 passed', '3 passed')),
            'ignored-count': lambda d: row(d, 1).__setitem__('stdout', row(d, 1)['stdout'].replace('0 ignored', '1 ignored')),
            'filtered-count': lambda d: row(d, 1).__setitem__('stdout', row(d, 1)['stdout'].replace('144 filtered', '143 filtered')),
            'missing-runtime-nonce': lambda d: row(d, 1).__setitem__('stdout', row(d, 1)['stdout'].replace('JR_MATRIX_RUNTIME_' + row(d, 1)['nonce'], 'JR_MATRIX_RUNTIME_' + '0' * 32)),
            'duplicate-test': lambda d: row(d, 1).__setitem__('stdout', row(d, 1)['stdout'] + 'test summary::tests::parses_libtest_results ... ok\n'),
            'type-exit': lambda d: row(d, 3).__setitem__('status', 0),
            'missing-type-literal': lambda d: row(d, 3).__setitem__('stderr', ''),
            'assert-exit': lambda d: row(d, 5).__setitem__('status', 0),
            'assert-diagnostic': lambda d: row(d, 5).__setitem__('stderr', ''),
            'timeout': lambda d: row(d, 1).__setitem__('timed_out', True),
            'transport': lambda d: row(d, 1).__setitem__('transport_error', True),
            'changed-binary': lambda d: d['groups']['whole-generation-v64-off-seven'].__setitem__('engine_sha256', '0' * 64),
            'changed-retained-pid': lambda d: row(d, 2).__setitem__('worker_pids', [999]),
            'missing-success-marker': lambda d: row(d, 2).__setitem__('daemon_events', [e for e in row(d, 2)['daemon_events'] if not e.startswith('whole-generation retained-load success')]),
            'wrong-success-pid': lambda d: row(d, 2).__setitem__('daemon_events', [e.replace('pid=151379', 'pid=999') for e in row(d, 2)['daemon_events']]),
            'fallback-missing': lambda d: fallback(d).__setitem__('daemon_events', [e for e in fallback(d)['daemon_events'] if not e.startswith('retained load failed')]),
            'fallback-twice': lambda d: fallback(d)['daemon_events'].extend([e for e in fallback(d)['daemon_events'] if e.startswith('retained load failed')]),
            'cold-start-twice': lambda d: fallback(d)['daemon_events'].append('worker-start pid=999'),
            'native-value': lambda d: d['groups']['native-link-reload-v64-on']['rows'][1].__setitem__('stdout', 'NATIVE_' + d['groups']['native-link-reload-v64-on']['rows'][1]['nonce'] + '_11\n'),
            'compiler-failure-softening': lambda d: d['compilers'][0].__setitem__('check_exit', 0),
            'native-count-softening': lambda d: d['units'][-1].__setitem__('summaries', [[374, 0, 0, 0, 1]]),
            'unknown-environment-field': lambda d: row(d, 1).__setitem__('AWS_SECRET_ACCESS_KEY', 'not-an-actual-secret'),
            'absolute-private-path': lambda d: d['restoration']['files'][0].__setitem__('path', '/home/private/input.rs'),
            'quiet-machine-claim': lambda d: row(d, 1)['noise'].__setitem__('contamination', 'quiet'),
            'promotion': lambda d: d.__setitem__('decision', 'promoted'),
            'nonfinite-time': lambda d: row(d, 1).__setitem__('elapsed_seconds', float('nan')),
            'phase-missing': lambda d: d['groups']['whole-generation-v64-on-seven']['rows'].pop(),
        }
        for name, mutate in mutations.items():
            with self.subTest(mutation=name):
                changed = copy.deepcopy(DATA)
                mutate(changed)
                with self.assertRaises(ValueError):
                    validator.validate(changed)
        self.assertEqual(len(mutations), 28)

    def test_exporter_drops_unrecognized_metadata_lines(self):
        injected = '\n'.join(['AWS_SECRET_ACCESS_KEY=not-an-actual-secret', 'PATH=/private/tools', 'error: could not compile `justrust` SECRET=hidden', 'generation-certificate result=unknown stage=plan SECRET=hidden'])
        self.assertEqual(exporter.selected(injected), '')
        self.assertEqual(exporter.daemon(injected), [])
        self.assertEqual(exporter.selected('NATIVE_' + '1' * 32 + '_11\n'), 'NATIVE_' + '1' * 32 + '_11\n')

    def test_relocation_and_package_hashes(self):
        scratch = os.environ.get('JCODE_SCRATCH_DIR', str(Path.home() / '.jcode/scratch'))
        with tempfile.TemporaryDirectory(prefix='generation-evidence-relocated-', dir=scratch) as directory:
            dest = Path(directory) / 'unrelated-name'
            dest.mkdir()
            names = [line.split('  ', 1)[1] for line in (HERE / 'SHA256SUMS').read_text().splitlines()]
            for name in names + ['SHA256SUMS']:
                shutil.copyfile(HERE / name, dest / name)
            result = subprocess.run([sys.executable, str(dest / 'validate.py'), '--manifest'], cwd=directory, capture_output=True, text=True, check=True)
            self.assertEqual(json.loads(result.stdout)['correctness_rows'], 35)
            observations = dest / 'observations.json'
            observations.write_text(observations.read_text() + '\n')
            with self.assertRaises(ValueError):
                validator.verify_manifest(dest)


if __name__ == '__main__':
    unittest.main(verbosity=2)
