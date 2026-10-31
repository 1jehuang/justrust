#!/usr/bin/env python3
"""Relocation and coherently re-sealed semantic controls, strictly offline."""
import builtins
import importlib.util
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
spec = importlib.util.spec_from_file_location('cache_attribution_audit',HERE/'validate.py')
v = importlib.util.module_from_spec(spec)
spec.loader.exec_module(v)


class Controls(unittest.TestCase):
    def test_relocated_empty_environment_and_resigned_negatives(self):
        count = 0
        with tempfile.TemporaryDirectory(prefix='cache-attribution-audit-',dir=os.environ.get('JCODE_SCRATCH_DIR')) as tmp:
            root = Path(tmp)/'relocated'
            shutil.copytree(HERE,root,ignore=shutil.ignore_patterns('__pycache__'))
            original_open,original_io_open = builtins.open,io.open
            checkout = HERE.parents[2]
            def forbidden(*args,**kwargs):
                raise AssertionError('native/process/network operation forbidden')
            def guarded(opener):
                def checked(file,*args,**kwargs):
                    if isinstance(file,(str,bytes,os.PathLike)):
                        path = Path(os.fsdecode(file)).resolve()
                        if path == root or root in path.parents:
                            return opener(file,*args,**kwargs)
                        if path == checkout or checkout in path.parents or Path('/home/jeremy') in path.parents:
                            raise AssertionError('original checkout/home inputs forbidden')
                    return opener(file,*args,**kwargs)
                return checked
            with patch.object(builtins,'open',guarded(original_open)),patch.object(io,'open',guarded(original_io_open)),patch.object(subprocess,'Popen',forbidden),patch.object(socket,'socket',forbidden),patch.dict(os.environ,{'HOME':str(Path(tmp)/'empty-home'),'PATH':''},clear=True):
                self.assertEqual(v.validate(root)['runtime_rows'],14)
                state_path = root/'runtime/on/state.json'
                original = state_path.read_bytes()
                def reject(changes):
                    nonlocal count
                    originals = [(p,p.read_bytes()) for p,_ in changes]
                    try:
                        for p,data in changes:
                            p.write_bytes(data)
                        v.seal(root)
                        with self.assertRaises(ValueError):
                            v.validate(root)
                        count += 1
                    finally:
                        for p,data in originals:
                            p.write_bytes(data)
                controls = [
                    ('startup flag',lambda s:s['startup_controls'].update(RA_JIT_CACHE_MISS_STATS='0')),
                    ('missing transition',lambda s:s['rows'].pop()),
                    ('source identity',lambda s:s['source_after'].update({'Cargo.toml':'0'*64})),
                    ('tool identity',lambda s:s.update(tool_after='0'*64)),
                    ('toolchain identity',lambda s:s['toolchain_after'].update(extra='changed')),
                    ('input recipe',lambda s:s['rows'][0]['inputs'].update({'Cargo.toml':'0'*64})),
                    ('actual test names',lambda s:s['rows'][0]['parsed']['tests'].pop()),
                    ('environment allowlist',lambda s:s['rows'][0]['environment'].update(UNSUPPORTED_SECRET='synthetic-control')),
                    ('noise scope',lambda s:s['rows'][0]['noise'].update(contamination='clean')),
                    ('elapsed validity',lambda s:s['rows'][0].update(elapsed_seconds=-1)),
                    ('process verdict',lambda s:s['rows'][0].update(status=101)),
                    ('input unchanged',lambda s:s['rows'][0].update(input_unchanged=False)),
                    ('restoration',lambda s:s.update(unchanged=False)),
                ]
                for label,mutate in controls:
                    with self.subTest(control=label):
                        state = json.loads(original)
                        mutate(state)
                        reject([(state_path,json.dumps(state).encode())])
                # Raw corruptions also regenerate nested byte/hash receipts.
                for label,phase,needle in [('invocation witness','cold-prepare','JR_MATRIX_RUNTIME_'),('type diagnostic','type-error','E0308'),('assertion witness','assert-failure','JR_MATRIX_FAILURE_')]:
                    with self.subTest(control=label):
                        i = v.PHASES.index(phase)
                        state = json.loads(original)
                        changes = []
                        seen = False
                        for stream in ('stdout','stderr'):
                            p = root/f'runtime/on/{i:02}-{phase}.{stream}'
                            data = p.read_bytes()
                            seen |= needle.encode() in data
                            changed = data.replace(needle.encode(),b'REMOVED_CONTROL_')
                            state['rows'][i]['raw'][stream].update(bytes=len(changed),sha256=v.sha(changed))
                            changes.append((p,changed))
                        self.assertTrue(seen)
                        changes.append((state_path,json.dumps(state).encode()))
                        reject(changes)
                worker = root/'runtime/on/02-manifest-feature-add-worker-0.log'
                data = worker.read_bytes()
                for label,old,new in [('missing field',b'arrived=45207',b''),('duplicate field',b'arrived=45207',b'arrived=45207 arrived=45207'),('private field',b'arrived=45207',b'arrived=/private/name')]:
                    with self.subTest(control=label):
                        self.assertIn(old,data)
                        reject([(worker,data.replace(old,new))])
                # Coherently alter an actual counter and its derived root JSON.
                with self.subTest(control='whole actual counter independently bound'):
                    receipt = root/'root-validation.json'
                    changed = v.load(receipt)
                    changed['modes']['on'][2]['last_report']['arrived'] += 1
                    reject([(worker,data.replace(b'arrived=45207',b'arrived=45208')),(receipt,json.dumps(changed).encode())])
                # Off reports must be absent, not merely zero-valued.
                with self.subTest(control='off has report'):
                    p = root/'runtime/off/00-cold-prepare-worker-0.log'
                    report = next(line for line in data.splitlines() if line.startswith(b'ra-jit cache-miss-stats v1 '))
                    reject([(p,p.read_bytes()+b'\n'+report+b'\n')])
                with self.subTest(control='off stdout report with nested receipt resigned'):
                    p = root/'runtime/off/00-cold-prepare.stdout'
                    receipt = root/'runtime/off/state.json'
                    state = v.load(receipt)
                    changed = p.read_bytes()+b'\n'+report+b'\n'
                    state['rows'][0]['raw']['stdout'].update(bytes=len(changed),sha256=v.sha(changed))
                    reject([(p,changed),(receipt,json.dumps(state).encode())])
                c = v.load(root/'commitments.json')
                gate = next(g for g in c['gates'] if g['tests']['passed'] == 53)
                p = root/'journals'/gate['run']/'output.jsonl'
                with self.subTest(control='actual native fixture name'):
                    reject([(p,p.read_bytes().replace(b'native_usable_negative_memo_is_not_a_fresh_missing_key',b'REMOVED_NATIVE_FIXTURE'))])
                with self.subTest(control='actual 733 regression gate'):
                    p = root/'gates/v43-bench-regression.log'
                    reject([(p,p.read_bytes().replace(b'Ran 733 tests',b'Ran 732 tests'))])
                with self.subTest(control='recovery snapshot cannot be fresh assertion observation'):
                    p = root/'runtime/on/05-assert-failure-worker-0.log'
                    receipt = root/'root-validation.json'
                    changed = v.load(receipt)
                    changed['modes']['on'][5]['last_report']['transitive_failure'] += 1
                    reject([(p,p.read_bytes().replace(b'transitive_failure=29',b'transitive_failure=30')),(receipt,json.dumps(changed).encode())])
                v.seal(root)
                self.assertEqual(v.validate(root)['new_native_calls'],0)
        self.assertEqual(count,25)
        print(json.dumps(dict(relocated=True,empty_home_path=True,forbid_original_checkout=True,forbid_native_network=True,coherently_resealed_negatives=count)))


if __name__ == '__main__':
    unittest.main()
