#!/usr/bin/env python3
"""Offline controls only. No compiler, engine, benchmark, subprocess or network."""
import importlib.util
import builtins
import io
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import tempfile
import unittest
from unittest.mock import patch

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('declarations_audit', HERE / 'validate.py')
v = importlib.util.module_from_spec(spec)
spec.loader.exec_module(v)


class Controls(unittest.TestCase):
    def test_relocated_and_resigned_corruption(self):
        with tempfile.TemporaryDirectory(prefix='declarations-audit-') as tmp:
            root = Path(tmp) / 'relocated'
            shutil.copytree(HERE, root, ignore=shutil.ignore_patterns('__pycache__'))
            def forbidden(*args, **kwargs):
                raise AssertionError('native/process/network operation forbidden')
            original_open, original_io_open = builtins.open, io.open
            checkout = HERE.parents[2]
            def guarded(opener):
                def checked(file, *args, **kwargs):
                    if isinstance(file, (str, bytes, os.PathLike)):
                        path = Path(os.fsdecode(file)).resolve()
                        if path == checkout or checkout in path.parents:
                            raise AssertionError('original checkout read forbidden')
                    return opener(file, *args, **kwargs)
                return checked
            with patch.object(builtins, 'open', guarded(original_open)), patch.object(io, 'open', guarded(original_io_open)), patch.object(subprocess, 'Popen', forbidden), patch.object(socket, 'socket', forbidden), patch.dict(os.environ, {'HOME': str(Path(tmp)/'empty-home'), 'PATH': '/nonexistent'}, clear=True):
                self.assertEqual(v.validate(root)['runtime_rows'], 42)
                controls = [
                    ('startup flag', lambda s: s['startup_controls'].update(RA_JIT_DECLARATIONS_CACHE='0')),
                    ('missing transition', lambda s: s['rows'].pop()),
                    ('source identity', lambda s: s['source_after'].update({'Cargo.toml':'0'*64})),
                    ('tool identity', lambda s: s.update(tool_after='0'*64)),
                    ('toolchain identity', lambda s: s['toolchain_after'].update({'extra':'changed'})),
                    ('input recipe', lambda s: s['rows'][0]['inputs'].update({'Cargo.toml':'0'*64})),
                    ('parsed actual names', lambda s: s['rows'][0]['parsed']['tests'].pop()),
                    ('secret environment field', lambda s: s['rows'][0]['environment'].update(UNSUPPORTED_SECRET='synthetic-control')),
                    ('noise claim', lambda s: s['rows'][0]['noise'].update(contamination='clean')),
                    ('elapsed validity', lambda s: s['rows'][0].update(elapsed_seconds=-1)),
                    ('process status', lambda s: s['rows'][0].update(status=101)),
                    ('restoration', lambda s: s.update(unchanged=False)),
                ]
                state_path = root / 'runtime/v41-on/state.json'
                original = state_path.read_bytes()
                for label, mutate in controls:
                    with self.subTest(control=label):
                        state = json.loads(original)
                        mutate(state)
                        state_path.write_text(json.dumps(state))
                        v.seal(root)
                        with self.assertRaises(ValueError):
                            v.validate(root)
                        state_path.write_bytes(original)
                # Coherently rewrite raw receipts and the outer transport seal.
                for label, phase, old in [('invocation marker','cold-prepare','JR_MATRIX_RUNTIME_'), ('type diagnostic','type-error','E0308'), ('assertion marker','assert-failure','JR_MATRIX_FAILURE_')]:
                    with self.subTest(control=label):
                        index = v.PHASES.index(phase)
                        state = json.loads(original)
                        changed = []
                        for stream in ('stdout','stderr'):
                            path = root / f'runtime/v41-on/{index:02}-{phase}.{stream}'
                            data = path.read_bytes()
                            replacement = data.replace(old.encode(), b'REMOVED_CONTROL_')
                            changed.append((path,data))
                            path.write_bytes(replacement)
                            state['rows'][index]['raw'][stream].update(bytes=len(replacement), sha256=v.sha(replacement))
                        state_path.write_text(json.dumps(state))
                        v.seal(root)
                        with self.assertRaises(ValueError):
                            v.validate(root)
                        for path,data in changed:
                            path.write_bytes(data)
                        state_path.write_bytes(original)
                # Native journal semantics and positive counter are not inferred from absence.
                c = v.load(root/'commitments.json')
                resident = next(g for g in c['gates'] if g['tests']['passed']==369)
                paths = [(root/'journals'/resident['run']/'output.jsonl', b'declarations_round_trip_remaps_fresh_database_and_preserves_native_walk'), (root/'runtime/v41-on/00-cold-prepare-worker-0.log', b'native_hit=1')]
                for path,needle in paths:
                    with self.subTest(control=str(path.relative_to(root))):
                        data=path.read_bytes()
                        self.assertIn(needle,data)
                        path.write_bytes(data.replace(needle,b'REMOVED_CONTROL'))
                        v.seal(root)
                        with self.assertRaises(ValueError):
                            v.validate(root)
                        path.write_bytes(data)
                v.seal(root)
                self.assertEqual(v.validate(root)['decision'], 'REJECT/no promotion')
        print('PASS relocation, empty HOME/PATH, forbidden subprocess/network, 17 resigned semantic controls, final positive replay')


if __name__ == '__main__':
    unittest.main(verbosity=2)
