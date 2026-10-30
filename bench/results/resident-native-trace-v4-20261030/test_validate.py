#!/usr/bin/env python3
"""Offline mutation tests. Never launch the resident engine or modify live evidence."""
from contextlib import redirect_stdout
import io
import json
from pathlib import Path
import shutil
import tempfile
import unittest
from unittest.mock import patch

import validate


class TraceValidation(unittest.TestCase):
    def fixture(self):
        return {'traceEvents': [{'ph': 'X', 'name': 'client command', 'ts': 1, 'dur': 9}],
                'raJit': {'total_us': 9, 'breakdown': [['client command', 9]]}}

    def test_valid_trace(self):
        self.assertEqual(validate.validate_trace(self.fixture()), {'client command'})

    def test_reject_empty_trace(self):
        data = self.fixture()
        data['traceEvents'] = []
        with self.assertRaises(ValueError):
            validate.validate_trace(data)

    def test_reject_negative_duration(self):
        data = self.fixture()
        data['traceEvents'][0]['dur'] = -1
        with self.assertRaises(ValueError):
            validate.validate_trace(data)

    def test_reject_missing_owner(self):
        data = self.fixture()
        data['traceEvents'][0]['name'] = 'worker check'
        with self.assertRaises(ValueError):
            validate.validate_trace(data)

    def test_reject_double_counted_breakdown(self):
        data = self.fixture()
        data['raJit']['breakdown'].append(['overlap', 1])
        with self.assertRaises(ValueError):
            validate.validate_trace(data)


class EvidenceMutation(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name) / 'evidence'
        shutil.copytree(validate.ROOT, self.root, ignore=shutil.ignore_patterns('__pycache__'))

    def check(self):
        with patch.object(validate, 'ROOT', self.root), redirect_stdout(io.StringIO()):
            validate.main()

    def mutate_json(self, relative, change):
        path = self.root / relative
        data = json.loads(path.read_text())
        change(data)
        path.write_text(json.dumps(data))

    def test_retained_evidence_passes(self):
        self.check()

    def test_reject_status_changed(self):
        self.mutate_json('acceptance.json', lambda data: data['rows'][0].update(status=101))
        with self.assertRaises(ValueError):
            self.check()

    def test_reject_worker_changed(self):
        self.mutate_json('acceptance.json', lambda data: data['rows'][0].update(worker_identity_unchanged=False))
        with self.assertRaises(ValueError):
            self.check()

    def test_reject_trace_changed(self):
        with (self.root / 'trace-only.json').open('a') as stream:
            stream.write(' ')
        with self.assertRaises(ValueError):
            self.check()

    def test_reject_artifact_inode_changed(self):
        def change(data):
            row = next(row for row in data['rows'] if row['mode'] == 'alias')
            next(iter(row['artifacts'].values()))['inode'] += 1
        self.mutate_json('root-off-on-off/state.json', change)
        with self.assertRaises(ValueError):
            self.check()

    def test_reject_hidden_compile_diagnostic(self):
        (self.root / 'root-errors-recovery/type-error.stderr').write_text('')
        with self.assertRaises(ValueError):
            self.check()

    def test_reject_lost_negative_control(self):
        self.mutate_json('old-missing-trace-control/state.json', lambda data: data.update(status='passed'))
        with self.assertRaises(ValueError):
            self.check()


if __name__ == '__main__':
    unittest.main()
