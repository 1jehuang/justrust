#!/usr/bin/env python3
"""Offline evidence validation, coherent-reseal negatives, isolated audit replay.
These tests never invoke a compiler, engine, git, cargo, or network operation.
The only child executable is this same Python interpreter in isolated mode.
"""
import copy
import hashlib
import importlib.util
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT=Path(__file__).resolve().parent
spec=importlib.util.spec_from_file_location('manifest_evidence_validator',ROOT/'validate.py')
validator=importlib.util.module_from_spec(spec)
spec.loader.exec_module(validator)


def write_json(path,value):
    path.write_text(json.dumps(value,sort_keys=True,indent=2)+'\n')


def reseal(root,data=None):
    if data is not None:
        for name in data['artifacts']:
            if not Path(name).is_absolute() and '..' not in Path(name).parts and (root/name).is_file():
                data['artifacts'][name]=validator.sha((root/name).read_bytes())
        write_json(root/'observations.json',data)
    write_json(root/'SHA256.json',{str(p.relative_to(root)):validator.sha(p.read_bytes()) for p in root.rglob('*') if p.is_file() and p.name!='SHA256.json' and '__pycache__' not in p.parts})


def raw_stdout(row,new):
    row['stdout']=new
    row['raw_original']['stdout']={'sha256':validator.sha(new.encode()),'bytes':len(new.encode())}


# Preload stdlib before the audit hook. Afterwards *every* file open and directory
# traversal must stay in the relocated package. Even Python subprocesses, sockets,
# dynamic native library loading, and write opens are forbidden in the validator.
BOOTSTRAP=r'''
import argparse, ast, hashlib, json, math, pathlib, re, statistics, types, runpy, sys, os
import subprocess, socket, ctypes, shutil
argparse.ArgumentParser().parse_args([])
root=pathlib.Path(sys.argv[1]).resolve()
payload=sys.argv[2]
def guard(event,args):
    if event=='open':
        name=args[0]
        if not isinstance(name,(str,bytes,os.PathLike)):
            raise PermissionError('AUDIT external descriptor')
        p=pathlib.Path(os.fsdecode(name)).resolve()
        if not p.is_relative_to(root):
            raise PermissionError('AUDIT external read')
        mode=args[1]
        if isinstance(mode,str) and any(c in mode for c in 'wax+'):
            raise PermissionError('AUDIT write')
        if len(args)>2 and isinstance(args[2],int) and args[2] & (os.O_WRONLY|os.O_RDWR|os.O_CREAT|os.O_TRUNC):
            raise PermissionError('AUDIT write flags')
    if event in ('os.listdir','os.scandir'):
        if not pathlib.Path(os.fsdecode(args[0])).resolve().is_relative_to(root):
            raise PermissionError('AUDIT external directory')
    if event.startswith(('subprocess.','socket.','ctypes.','os.exec','os.spawn','os.posix_spawn')) or event in ('os.system','os.fork','os.forkpty','os.chdir'):
        raise PermissionError('AUDIT native/network forbidden')
sys.addaudithook(guard)
if payload:
    exec(payload)
else:
    sys.argv=[str(root/'validate.py'),'--manifest']
    exec(compile((root/'validate.py').read_bytes(),str(root/'validate.py'),'exec'),{'__name__':'__main__','__file__':str(root/'validate.py')})
'''


class EvidenceTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory(prefix='v85-evidence-',dir=os.environ.get('JCODE_SCRATCH_DIR',str(Path.home()/'.jcode/scratch')))
        self.root=Path(self.temp.name)/'relocated'
        shutil.copytree(ROOT,self.root,ignore=shutil.ignore_patterns('__pycache__'))
        self.data=json.loads((self.root/'observations.json').read_text())
    def tearDown(self): self.temp.cleanup()
    def reject(self,pattern='.'):
        reseal(self.root,self.data)
        with self.assertRaisesRegex(ValueError,pattern):validator.validate_package(self.root,True)
    def test_real_all_native_strict_profile_timing(self):
        result=validator.validate_package(self.root,True)
        self.assertEqual(result['native'],[41,22,0,53,373])
        self.assertEqual(result['correctness'],{'strict':28,'profile':8})
        self.assertEqual(result['timing']['measured'],20)
        self.assertLess(abs(result['timing']['delta_percent']),10)
    def test_original_tuple_oracle_not_json_weakened(self):
        scopes=validator.original_oracles(self.root);matrix=scopes['resident_state_matrix']
        row=self.data['strict']['on']['rows'][0]
        parsed=scopes['blaze_runtime_compare']['test_results'](validator.text(row))
        self.assertIsInstance(parsed['tests'][0],tuple)
        expected=matrix['contract'](parsed,row['phase'])
        self.assertEqual(matrix['oracle'](dict(row,parsed=parsed),validator.text(row),expected,row['phase'],row['nonce'],0),[])
        self.assertNotEqual(matrix['oracle'](row,validator.text(row),expected,row['phase'],row['nonce'],0),[])
    def test_coherent_raw_parsed_wrong_name(self):
        row=self.data['strict']['on']['rows'][0];old=validator.NAMES[0];new='summary::tests::invented'
        raw_stdout(row,row['stdout'].replace(old,new));row['parsed']['tests'][0][0]=new
        self.reject('original cold contract')
    def test_coherent_raw_nonce_reseal(self):
        row=self.data['strict']['on']['rows'][1];old=row['nonce'];row['nonce']='0'*32
        raw_stdout(row,row['stdout'].replace(old,row['nonce']))
        self.reject('trusted historical observation commitment')
    def test_coherent_raw_pid_same_segment_reseal(self):
        for row in self.data['pinned']['records']:
            if row['group']=='segment-1' and 'worker' in row:
                row['worker']['pid']+=10000
                row['status_worker_lines']=['worker: pid '+str(row['worker']['pid'])]
        self.reject('trusted historical observation commitment')
    def test_raw_timing_both_arm_summaries_resealed(self):
        import statistics
        row=next(r for r in self.data['pinned']['records'] if r['phase']=='retouch')
        row['elapsed_seconds']+=0.000001
        values={a:[] for a in ['off','on']}
        for r in self.data['pinned']['records']:
            if r['phase']=='retouch': values['on' if r['group'] in ('segment-1','segment-2') else 'off'].append(r['elapsed_seconds'])
        measured={a:{'samples_seconds':v,'median_seconds':statistics.median(v)} for a,v in values.items()}
        for a in ['off','on']: self.data['pinned'][a]['measurements']=measured
        self.reject('trusted historical observation commitment')
    def test_full_candidate_source_resealed(self):
        p=self.root/'frozen/warm-registry-fetch-v85/candidate/toolchain/resident/crates/blaze-registry/src/download.rs'
        p.write_bytes(p.read_bytes()+b'\n// tamper\n')
        self.reject('exact full original/candidate native source')
    def test_named_native_result_and_its_duplicate_resealed(self):
        row=self.data['native']['registry'];row['lines'][0]=row['lines'][0].replace(' ... ok',' ... FAILED')
        write_json(self.root/'native/registry.json',row)
        self.reject('no hidden native failures')
    def test_marker_cannot_be_replaced_by_eligibility(self):
        row=next(r for r in self.data['pinned']['records'] if r['phase']=='retouch' and r['group']=='segment-1')
        row['warm_marker_lines']=[];row['eligible']=True
        self.reject('wrong parallel batch marker count')
    def test_first_touch_cannot_be_omitted(self):
        self.data['pinned']['records']=[r for r in self.data['pinned']['records'] if not(r['phase']=='retouch' and r['replicate']==0)]
        self.reject('all54 ordered actual calls')
    def test_promotion_rejected_even_resealed(self):
        self.data['default_promotion']=True
        self.reject('rejected promotion and exact nonclaims')
    def isolated(self,payload=''):
        empty=Path(self.temp.name)/'empty-home';empty.mkdir(exist_ok=True)
        return subprocess.run([sys.executable,'-I','-S','-B','-c',BOOTSTRAP,str(self.root),payload],cwd=empty,env={'HOME':str(empty),'PATH':''},capture_output=True,text=True)
    def test_relocated_empty_home_path_audited_replay(self):
        p=self.isolated();self.assertEqual(p.returncode,0,p.stdout+p.stderr)
        self.assertIn('rejected-below-10-percent-floor',p.stdout)
    def test_audit_denials_external_read_write_native_network_dlopen(self):
        payloads=["open('/etc/passwd').read()","open(str(root/'forbidden'),'w').write('x')","subprocess.run(['/bin/true'])","socket.socket()","ctypes.CDLL(None)"]
        for payload in payloads:
            with self.subTest(payload=payload):
                p=self.isolated(payload);self.assertNotEqual(p.returncode,0)
                self.assertIn('PermissionError: AUDIT',p.stderr)

if __name__=='__main__':unittest.main(verbosity=2)
