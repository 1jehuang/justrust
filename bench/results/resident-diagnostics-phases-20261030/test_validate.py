#!/usr/bin/env python3
"""Evidence-only validator controls. No Rust build, runtime suite, or live source."""
import json
import os
import shutil
import tempfile
import unittest
from pathlib import Path

import seal
import validate

HERE = Path(__file__).resolve().parent


class PortableEvidenceTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        scratch = Path(os.environ.get('JCODE_SCRATCH_DIR', str(Path.home() / '.jcode/scratch')))
        scratch.mkdir(parents=True, exist_ok=True)
        cls.temp = tempfile.TemporaryDirectory(prefix='diagnostics-portable-', dir=scratch)
        cls.root = Path(cls.temp.name) / 'relocated'
        shutil.copytree(HERE, cls.root, ignore=shutil.ignore_patterns('__pycache__'))
        cls.original = {}

    @classmethod
    def tearDownClass(cls):
        cls.temp.cleanup()

    def mutate_json(self, relative, change):
        path = self.root / relative
        self.original.setdefault(relative, path.read_bytes())
        value = json.loads(path.read_text())
        change(value)
        path.write_text(json.dumps(value, indent=2) + '\n')

    def retie_integrity(self):
        # A semantic control must fail even when an editor updates all file hashes.
        path = self.root / 'collection.json'
        self.original.setdefault('collection.json', path.read_bytes())
        value = json.loads(path.read_text())
        for item in value['files']:
            item['sha256'] = validate.digest(self.root / item['artifact'])
        path.write_text(json.dumps(value, indent=2) + '\n')
        seal.seal(self.root)

    def tearDown(self):
        for relative, data in self.original.items():
            (self.root / relative).write_bytes(data)
        self.original.clear()
        seal.seal(self.root)

    def test_portable_positive(self):
        result = validate.validate(self.root)
        self.assertEqual(result['verdict'], 'passed')
        self.assertEqual(result['native_passed'], 349)

    def test_missing_real_phase_rejected_after_hashes_retied(self):
        relative = 'profiles/v22/edit-one.trace.json'
        def remove_phase(trace):
            trace['traceEvents'] = [event for event in trace['traceEvents']
                                    if event.get('name') != 'diagnostics.filter_cache_update']
        self.mutate_json(relative, remove_phase)
        trace = json.loads((self.root / relative).read_text())
        def update_state(state):
            row = next(row for row in state['rows'] if row['mode'] == 'edit-one')
            row['trace'] = trace
            row['trace_sha256'] = validate.digest(self.root / relative)
        self.mutate_json('profiles/v22/state.json', update_state)
        self.retie_integrity()
        with self.assertRaisesRegex(ValueError, 'four actual phase events required'):
            validate.validate(self.root)

    def test_dropped_intervals_rejected_after_hashes_retied(self):
        state = json.loads((self.root / 'profiles/v22/state.json').read_text())
        artifact = state['worker_logs'][0]['artifact']
        relative = f'profiles/v22/{artifact}'
        path = self.root / relative
        self.original[relative] = path.read_bytes()
        path.write_text(path.read_text().replace('stale=94 dropped=0', 'stale=94 dropped=1'))
        self.mutate_json('profiles/v22/state.json', lambda state:
                         state['worker_logs'][0].update(sha256=validate.digest(path)))
        self.retie_integrity()
        with self.assertRaisesRegex(ValueError, 'actual transport receipt counts changed'):
            validate.validate(self.root)

    def test_raw_zero_test_pass_rejected_after_hashes_retied(self):
        relative = (f'journals/unit-v22-restored/runs/{validate.CORRECTED_RUN}/output.jsonl')
        path = self.root / relative
        self.original[relative] = path.read_bytes()
        path.write_text(path.read_text().replace('349 passed; 0 failed;', '0 passed; 0 failed;'))
        self.retie_integrity()
        with self.assertRaisesRegex(ValueError, 'corrected native full pass must have exactly349 tests'):
            validate.validate(self.root)


if __name__ == '__main__':
    unittest.main()
