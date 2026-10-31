#!/usr/bin/env python3
"""Stdlib offline replay tests. Mutations are resealed before semantic rejection."""
import copy
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT=Path(__file__).resolve().parent
spec=importlib.util.spec_from_file_location('planning_validator',ROOT/'validate.py')
v=importlib.util.module_from_spec(spec); spec.loader.exec_module(v)

def seal(root):
    entries={p.relative_to(root).as_posix():{'sha256':v.sha(p.read_bytes()),'bytes':p.stat().st_size} for p in sorted(root.rglob('*')) if p.is_file() and p.name!='SHA256.json'}
    (root/'SHA256.json').write_text(json.dumps(entries,sort_keys=True,indent=2)+'\n')

def change(root,name,fn):
    p=root/name; d=json.loads(p.read_text()); fn(d); p.write_text(json.dumps(d,sort_keys=True,indent=2)+'\n')

def row(fn): return lambda root:change(root,'observations.json',lambda d:fn(d['rows'][0]))
def obs(fn): return lambda root:change(root,'observations.json',fn)
def setkey(name,value): return lambda d:d.__setitem__(name,value)
def source(fn): return lambda root:change(root,'inventories/pinned-source.json',fn)

CASES={
 'unknown_observation_field':obs(setkey('ambient','not-allowed')),
 'unknown_runtime_field':row(setkey('ambient','not-allowed')),
 'unknown_stop_field':obs(lambda d:d['stop'].__setitem__('ambient','not-allowed')),
 'unknown_trace_field':row(lambda r:r['events'][0].__setitem__('args',{'ambient':'not-allowed'})),
 'missing_row':obs(lambda d:d['rows'].pop()),
 'extra_row':obs(lambda d:d['rows'].append(copy.deepcopy(d['rows'][-1]))),
 'wrong_order':obs(lambda d:d['rows'].reverse()),
 'edit_as_noop':row(setkey('kind','noop')),
 'wrong_replicate':row(setkey('replicate',1)),
 'runtime_failed':row(setkey('status',101)),
 'runtime_timeout':row(setkey('timed_out',True)),
 'runtime_launch_error':row(setkey('error','failed')),
 'runtime_negative_time':row(setkey('elapsed_seconds',-1)),
 'runtime_nan_time':row(setkey('elapsed_seconds',float('nan'))),
 'nonce_wrong':row(setkey('marker','JR_RUNTIME_EXECUTED_7999')),
 'edited_source_wrong':row(setkey('source_sha256','0'*64)),
 'trace_hash_wrong':row(setkey('trace_sha256','0'*64)),
 'missing_label':row(lambda r:r['events'].pop()),
 'duplicate_label':row(lambda r:r['events'].__setitem__(0,copy.deepcopy(r['events'][1]))),
 'wrong_trace_kind':row(lambda r:r['events'][0].__setitem__('ph','B')),
 'zero_duration':row(lambda r:r['events'][0].__setitem__('dur',0)),
 'negative_duration':row(lambda r:r['events'][0].__setitem__('dur',-1)),
 'wrong_client_pid':row(lambda r:r['events'][0].__setitem__('pid',1)),
 'wrong_client_tid':row(lambda r:r['events'][0].__setitem__('tid',9)),
 'outside_parent':row(lambda r:r['events'][0].__setitem__('ts',1)),
 'pid_count_wrong':row(setkey('public_pid_count',2)),
 'quiet_noise':row(setkey('noise_contamination','quiet')),
 'speed_claim':obs(lambda d:d['contract'].__setitem__('speed_claim',True)),
 'comparison_claim':obs(lambda d:d['contract'].__setitem__('comparison',True)),
 'promotion_claim':obs(lambda d:d['contract'].__setitem__('default_promotion',True)),
 'unbounded_noise':obs(lambda d:d['contract'].__setitem__('noise','quiet')),
 'source_changed':source(lambda d:d['after'].__setitem__('src/summary.rs','0'*64)),
 'source_missing':source(lambda d:(d['before'].pop('src/summary.rs'),d['after'].pop('src/summary.rs'))),
 'tool_changed':obs(setkey('tool_after','0'*64)),
 'stop_failed':obs(lambda d:d['stop'].__setitem__('status',1)),
 'stop_timeout':obs(lambda d:d['stop'].__setitem__('timed_out',True)),
 'stop_launch_error':obs(lambda d:d['stop'].__setitem__('error','failed')),
 'prepare_failed':obs(lambda d:d['prepare'].__setitem__('status',1)),
 'gate_failed':obs(lambda d:d['gates'].__setitem__('candidate-build-v79/check.exit','1')),
 'gate_missing':obs(lambda d:d['gates'].pop('candidate-unit-v79/exit')),
 'wrong_authority':obs(setkey('authority','live-v80')),
 'wrong_harness':obs(setkey('harness_count',760)),
 'wrong_range':obs(lambda d:d['ranges_us']['client planning'].__setitem__(0,1)),
 'bad_path':row(setkey('stdout','../outside')),
 'native_count':lambda r:change(r,'gates/recorder.json',lambda d:d[2]['tests'].__setitem__('passed',52)),
 'native_failed':lambda r:change(r,'gates/recorder.json',lambda d:d[3].__setitem__('exit',1)),
 'native_arguments':lambda r:change(r,'gates/recorder.json',lambda d:d[3]['args'].append('--ignored')),
 'tc_removed':lambda r:change(r,'inventories/toolchain.json',lambda d:d.pop('bin/rustc')),
 'native_source_removed':lambda r:change(r,'inventories/native-source.json',lambda d:d.pop('crates/ra-jit/src/cargo/metadata.rs')),
 'native_after_bad':lambda r:change(r,'inventories/joins.json',lambda d:d.__setitem__('native_after_ok',3756)),
 'tc_after_bad':lambda r:change(r,'inventories/joins.json',lambda d:d.__setitem__('toolchain_after_sha256','0'*64)),
}

class Offline(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory(dir=os.environ.get('JCODE_SCRATCH_DIR'))
        self.root=Path(self.temp.name)/'relocated'; shutil.copytree(ROOT,self.root)
    def tearDown(self): self.temp.cleanup()
    def test_valid(self): self.assertEqual(v.validate(self.root)['rows'],8)
    def test_relocated_empty_environment_audit(self):
        proc=subprocess.run([sys.executable,'-I','-S','-B',str(self.root/'offline_sandbox.py'),str(self.root)],env={'HOME':'','PATH':'','PYTHONDONTWRITEBYTECODE':'1'},capture_output=True,text=True)
        self.assertEqual(proc.returncode,0,proc.stderr)
        result=json.loads(proc.stdout); self.assertEqual(len(result['denied_probes']),6)
        self.assertEqual(result['isolated_replay']['rows'],8)
    def test_manifest_tamper(self):
        with (self.root/'gates/harness.txt').open('a') as f:f.write('bad')
        with self.assertRaisesRegex(ValueError,'sealed file'):v.validate(self.root)
    def test_unlisted_file(self):
        (self.root/'unlisted').write_text('extra')
        with self.assertRaisesRegex(ValueError,'closed archive'):v.validate(self.root)
    def test_symlink(self):
        (self.root/'outside-link').symlink_to(self.root.parent)
        with self.assertRaisesRegex(ValueError,'symlink'):v.validate(self.root)
    def test_private_path_coherent_reseal(self):
        (self.root/'private.txt').write_text('/'+'home/secret/private')
        seal(self.root)
        with self.assertRaisesRegex(ValueError,'privacy'):v.validate(self.root,historical=False)
    def test_ambient_environment_coherent_reseal(self):
        (self.root/'private.txt').write_text(json.dumps({'environment':{'TOKEN':'private'}}))
        seal(self.root)
        with self.assertRaisesRegex(ValueError,'privacy'):v.validate(self.root,historical=False)
    def test_coherent_stdout_receipt_semantic_reject(self):
        p=self.root/'runtime/0-edit.stdout'; p.write_text(p.read_text().replace('JR_RUNTIME_EXECUTED_7901','JR_RUNTIME_EXECUTED_7999'))
        change(self.root,'private-inputs.json',lambda d:d.__setitem__('planning-subphases-profile-v79/0-edit.stdout',{'sha256':v.sha(p.read_bytes()),'bytes':p.stat().st_size}))
        seal(self.root)
        with self.assertRaisesRegex(ValueError,'runtime nonce'):v.validate(self.root,historical=False)
    def test_coherent_native_name_duplication(self):
        p=self.root/'gates/codegen-native-named.txt'; lines=p.read_text().splitlines(); lines[1]=lines[0]; p.write_text('\n'.join(lines)+'\n');seal(self.root)
        with self.assertRaisesRegex(ValueError,'named native'):v.validate(self.root,historical=False)
    def test_coherent_source_pair_cannot_escape_patch(self):
        for name in ('originals.rs','candidates.rs'):
            p=self.root/'source'/name;p.write_text(p.read_text().replace('let rustc = super::state::rustc_path();','let rustc = super::state::different_path();'))
        change(self.root,'inventories/native-source.json',lambda d:d.__setitem__('crates/ra-jit/src/cargo/metadata.rs',v.sha((self.root/'source/candidates.rs').read_bytes())))
        change(self.root,'inventories/joins.json',lambda d:d.__setitem__('native_before_sha256',v.sha((self.root/'inventories/native-source.json').read_bytes())))
        seal(self.root)
        with self.assertRaisesRegex(ValueError,'patch context'):v.validate(self.root,historical=False)
    def test_coherent_timing_reseal_fixed_historical_commitment(self):
        def scale(d):
            for r in d['rows']:
                origin=min(e['ts'] for e in r['events'])
                for e in r['events']:e.update(ts=origin+2*(e['ts']-origin),dur=2*e['dur'])
            d['ranges_us']={k:[2*x for x in value] for k,value in d['ranges_us'].items()}
        change(self.root,'observations.json',scale);seal(self.root)
        self.assertEqual(v.validate(self.root,historical=False)['rows'],8)
        with self.assertRaisesRegex(ValueError,'historical observation'):v.validate(self.root)
    def test_coherent_receipt_reseal_fixed_source_commitment(self):
        change(self.root,'private-inputs.json',lambda d:d['planning-subphases-profile-v79/state.json'].__setitem__('bytes',1));seal(self.root)
        self.assertEqual(v.validate(self.root,historical=False)['rows'],8)
        with self.assertRaisesRegex(ValueError,'historical source/receipt'):v.validate(self.root)

for name,mutate in CASES.items():
    def test(self,mutate=mutate):
        mutate(self.root);seal(self.root)
        with self.assertRaises((ValueError,KeyError)):v.validate(self.root,historical=False)
    setattr(Offline,'test_resealed_'+name,test)

if __name__=='__main__':unittest.main()
