#!/usr/bin/env python3
"""Offline portability and tamper tests. Copies only this bundle into scratch."""
import json
import os
import shutil
import tempfile
import unittest
from pathlib import Path
import validate

ROOT = Path(__file__).resolve().parent


class EvidenceTests(unittest.TestCase):
    def setUp(self):
        scratch = os.environ.get('JCODE_SCRATCH_DIR')
        self.temp = tempfile.TemporaryDirectory(prefix='flat-hash-audit-', dir=scratch)
        self.root = Path(self.temp.name) / 'portable'
        shutil.copytree(ROOT, self.root, ignore=shutil.ignore_patterns('__pycache__'))

    def tearDown(self):
        self.temp.cleanup()

    def json(self, name):
        return json.loads((self.root / name).read_text())

    def save(self, name, data):
        (self.root / name).write_text(json.dumps(data, indent=2) + '\n')

    def reseal(self):
        # Update both ordinary byte seals so failures below require semantic
        # raw checks rather than merely noticing a stale top-level SHA.
        collection = self.json('collection.json')
        for row in collection['files']:
            row['sha256'] = validate.digest(self.root / row['artifact'])
        self.save('collection.json', collection)
        files = {str(p.relative_to(self.root)): validate.digest(p) for p in self.root.rglob('*')
                 if p.is_file() and p.name != 'manifest.json' and '__pycache__' not in p.parts}
        self.save('manifest.json', dict(files=files))

    def reject(self):
        self.reseal()
        with self.assertRaises(ValueError):
            validate.validate(self.root)

    def test_portable_without_original_paths(self):
        collection = self.json('collection.json')
        for row in collection['files']:
            row['origin'] = '/unavailable/original/' + row['artifact']
        self.save('collection.json', collection)
        self.reseal()
        self.assertEqual(validate.validate(self.root)['native_passed'], 351)

    def test_marker_removed_even_with_updated_stream_hash(self):
        name = 'interleaved/edit-0-old.stdout'
        path = self.root / name
        path.write_text(path.read_text().replace('JR_RUNTIME_EXECUTED_801', 'WRONG_MARKER'))
        state = self.json('interleaved/state.json')
        for row in state['rows']:
            if row['phase'] == 'edit' and row['replicate'] == 0 and row['stack'] == 'old':
                row['stdout_sha256'] = validate.digest(path)
        self.save('interleaved/state.json', state)
        self.reject()

    def test_native_passing_line_removed(self):
        name = 'journals/v23/20261030-123822295-3023351/output.jsonl'
        path = self.root / name
        lines = path.read_text().splitlines()
        at = next(i for i, line in enumerate(lines) if json.loads(line)['l'].startswith('test ')
                  and ' ... ok' in json.loads(line)['l'])
        del lines[at]
        path.write_text('\n'.join(lines) + '\n')
        self.reject()

    def test_dropped_transport_even_if_state_updated(self):
        path = self.root / 'profile-calibrated/1-new.worker-delta.log'
        path.write_text(path.read_text().replace('dropped=0', 'dropped=1'))
        state = self.json('profile-calibrated/state.json')
        for row in state['rows']:
            if row['replicate'] == 1 and row['stack'] == 'new':
                row['worker_delta_sha256'] = validate.digest(path)
                row['transport'][0][2] = 1
        self.save('profile-calibrated/state.json', state)
        self.reject()

    def test_missing_trace_span_even_if_state_updated(self):
        name = 'profile-calibrated/1-new.trace.json'
        trace = self.json(name)
        trace['traceEvents'] = [e for e in trace['traceEvents']
                                if e.get('name') != 'diagnostics.semantic_inclusive_asked']
        self.save(name, trace)
        state = self.json('profile-calibrated/state.json')
        for row in state['rows']:
            if row['replicate'] == 1 and row['stack'] == 'new':
                row['trace_sha256'] = validate.digest(self.root / name)
                row['events'] = [e for e in row['events'] if e['name'] != 'diagnostics.semantic_inclusive_asked']
        self.save('profile-calibrated/state.json', state)
        self.reject()

    def test_interleaved_order_changed(self):
        state = self.json('interleaved/state.json')
        state['rows'][15], state['rows'][16] = state['rows'][16], state['rows'][15]
        self.save('interleaved/state.json', state)
        self.reject()

    def test_failed_profile_reclassified_as_pass(self):
        state = self.json('profile-failed/state.json')
        state['status'] = 'passed'
        self.save('profile-failed/state.json', state)
        self.reject()

    def test_source_bytes_changed(self):
        path = self.root / 'build/v23/frozen-source/crates/ra-jit/src/diagnostic_hash.rs'
        path.write_text(path.read_text() + '\n// changed\n')
        self.reject()

    def test_unsealed_extra_artifact(self):
        (self.root / 'unsealed.txt').write_text('not in manifest')
        with self.assertRaisesRegex(ValueError, 'inventory'):
            validate.validate(self.root)


if __name__ == '__main__':
    unittest.main()
