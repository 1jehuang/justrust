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
        scratch=os.environ.get('JCODE_SCRATCH_DIR')
        self.temp=tempfile.TemporaryDirectory(prefix='v74-evidence-',dir=scratch)
        self.root=Path(self.temp.name)/'relocated-bundle'
        shutil.copytree(ROOT,self.root,ignore=shutil.ignore_patterns('__pycache__'))
        self.data=json.loads((self.root/'observations.json').read_text())

    def tearDown(self):
        self.temp.cleanup()

    def reject(self,message=None):
        reseal(self.root,self.data)
        with self.assertRaisesRegex(ValueError,message or '.'):
            validator.validate_package(self.root,True)

    def test_real_bundle_all_semantic_gates(self):
        result=validator.validate_package(self.root,True)
        self.assertEqual(result['correctness'],{'strict':28,'targeted':30,'changed_manifest':9,'native_aba':6})
        self.assertEqual(result['native'],[53,369])
        self.assertEqual(result['timing']['runtime_nonces'],24)
        self.assertGreater(min(result['timing']['first_on_seconds']),10.609115069994004)

    def test_original_tuple_oracle_not_list_weakened(self):
        scopes=validator.original_oracles(self.root)
        row=self.data['strict']['on']['rows'][0]
        parsed=scopes['blaze_runtime_compare']['test_results'](validator.text(row))
        self.assertIsInstance(parsed['tests'][0],tuple)
        matrix=scopes['resident_state_matrix']
        expected=matrix['contract'](parsed,row['phase'])
        self.assertEqual(matrix['oracle'](dict(row,parsed=parsed),validator.text(row),expected,row['phase'],row['nonce'],0),[])
        self.assertNotEqual(matrix['oracle'](row,validator.text(row),expected,row['phase'],row['nonce'],0),[])

    def test_coherent_raw_parsed_wrong_name_rejected_by_original_oracle(self):
        row=self.data['strict']['on']['rows'][0]
        old=validator.NAMES[0]; new='summary::tests::invented_test'
        raw_stdout(row,row['stdout'].replace(old,new))
        row['parsed']['tests'][0][0]=new
        self.reject('original cold contract')

    def test_coherent_strict_wrong_filtered_count_rejected(self):
        row=self.data['strict']['on']['rows'][1]
        raw_stdout(row,row['stdout'].replace('144 filtered','143 filtered'))
        row['parsed']['filtered']=143
        self.reject('exact original strict oracle')

    def test_strict_input_recipe_cannot_be_replaced_by_equal_pair(self):
        self.data['strict']['on']['rows'][2]['inputs']['Cargo.toml']='0'*64
        self.reject('independently derived strict input recipe')

    def test_strict_nonce_is_fresh_runtime_challenge(self):
        row=self.data['strict']['on']['rows'][1]
        raw_stdout(row,row['stdout'].replace(row['nonce'],'0'*32))
        self.reject('exact original strict oracle')

    def test_strict_type_error_must_not_run(self):
        row=self.data['strict']['on']['rows'][3]
        raw_stdout(row,'JR_MATRIX_RUNTIME_'+row['nonce']+'\n')
        self.reject('exact original strict oracle')

    def test_pinned_source_original_changed(self):
        (self.root/'pinned/source.original').write_bytes(b'fn shares_overlapping_time() {}')
        self.reject('original pinned source')

    def test_overlay_cannot_use_compile_time_env(self):
        f=self.root/'pinned/source.fixture'
        f.write_bytes(f.read_bytes().replace(b'std::env::var(',b'env!('))
        self.reject('runtime-reachable immutable nonce overlay')

    def test_pinned_raw_names_not_parser_bypass(self):
        row=self.data['pinned']['records'][13]
        raw_stdout(row,row['stdout'].replace(validator.NAMES[0],'summary::tests::invented_test'))
        self.reject('original named libtest parity')

    def test_pinned_missing_runtime_witness(self):
        row=self.data['pinned']['records'][13]
        raw_stdout(row,row['stdout'].replace('JR_MANIFEST_RETOUCH_'+row['nonce']+'\n',''))
        self.reject('exact reachable runtime witness')

    def test_pinned_nonce_duplicate(self):
        rows=self.data['pinned']['records']; row=rows[14]; old=row['nonce']; new=rows[13]['nonce']
        row['nonce']=new; row['nonce_environment']={'JR_MANIFEST_RETOUCH_NONCE':new}
        raw_stdout(row,row['stdout'].replace(old,new))
        self.reject('fresh measured runtime nonce')

    def test_coherently_new_runtime_nonce_reseal_not_authenticity(self):
        row=self.data['pinned']['records'][13]; old=row['nonce']; new='f'*32
        row['nonce']=new; row['nonce_environment']={'JR_MANIFEST_RETOUCH_NONCE':new}
        raw_stdout(row,row['stdout'].replace(old,new))
        self.reject('fixed historical observation commitment')

    def test_retouch_bytes_changed_even_equal_before_after_hashes(self):
        t=self.data['pinned']['records'][13]['touches'][0]
        t['before']['sha256']=t['after']['sha256']='0'*64
        self.reject('same-byte hash/size proof')

    def test_retouch_requires_actual_mtime_change(self):
        t=self.data['pinned']['records'][13]['touches'][0]
        t['after']['mtime_ns']=t['before']['mtime_ns']
        self.reject('observed strictly changed mtime')

    def test_retouch_set_cannot_narrow_to_manifest(self):
        self.data['pinned']['records'][13]['touches'].pop()
        self.reject('same original combined watched fileset')

    def test_status_pid_cannot_disagree_with_proc(self):
        self.data['pinned']['records'][25]['status_worker_lines']=['worker: pid 7']
        self.reject('status and proc PID exact')

    def test_same_pid_with_changed_starttick_not_reuse(self):
        row=self.data['pinned']['records'][25]
        w=row['worker']; head,tail=w['stat'].rsplit(') ',1); fields=tail.split(); fields[19]=str(int(fields[19])+1); w['stat']=head+') '+' '.join(fields)+'\n'
        self.reject('samePID plus starttick ON')

    def test_coherent_all_segment_pid_reseal_not_authenticity(self):
        for row in self.data['pinned']['records']:
            if row['group']=='segment-1' and 'worker' in row:
                w=row['worker']; old=w['pid']; w['pid']=old+10000000
                w['stat']=str(w['pid'])+w['stat'][len(str(old)):]
                row['status_worker_lines']=['worker: pid '+str(w['pid'])]
        self.reject('fixed historical observation commitment')

    def test_worker_executable_hash(self):
        self.data['pinned']['records'][25]['worker']['exe_sha256']='0'*64
        self.reject('candidate worker executable')

    def test_worker_socket_cannot_cross_private_groups(self):
        row=self.data['pinned']['records'][25]
        row['worker']['argv']=[v.replace('<PRIVATE_segment-1_HOME>','<PRIVATE_segment-0_HOME>') for v in row['worker']['argv']]
        self.reject('private worker socket role')

    def test_measurements_median_recomputed(self):
        self.data['pinned']['on']['measurements']['on']['median_seconds']=1
        self.reject('recomputed all20 samples/medians')

    def test_coherent_all_timing_reseal_not_authenticity(self):
        p=self.data['pinned']
        for row in p['records']:
            if row['phase']=='retouch': row['elapsed_seconds']*=2
        for arm in ('off','on'):
            for measurement in p[arm]['measurements'].values():
                measurement['samples_seconds']=[v*2 for v in measurement['samples_seconds']]
                measurement['median_seconds']*=2
        old=p['off']['measurements']['off']['median_seconds']; new=p['off']['measurements']['on']['median_seconds']
        lines=p['printed_compare'].splitlines()
        lines[2]=f'OFF median: {old:.6f}s -> ON median: {new:.6f}s, delta {new-old:+.6f}s ({100*(new/old-1):+.2f}%).'
        p['printed_compare']='\n'.join(lines)
        self.reject('fixed historical observation commitment')

    def test_first_on_cost_cannot_be_dropped(self):
        self.data['pinned']['records'].pop(25)
        self.reject('all54 exact ordered real calls')

    def test_native_runtime_and_expected_coherent_tamper(self):
        row=self.data['native_aba']['env']['rows'][1]
        row['expected']=44
        raw_stdout(row,row['stdout'].replace('_22\n','_44\n'))
        self.reject('independently derived native runtime result')

    def test_file_aba_source_hash_must_match_C(self):
        self.data['native_aba']['file']['rows'][1]['source_at_call']['a.c']='0'*64
        self.reject('actual file ABA source inventory')

    def test_changed_manifest_error_cannot_be_successful(self):
        row=self.data['changed_manifest']['rows'][7]
        row['status']=0
        raw_stdout(row,'GEN_OK_'+row['nonce']+'\n')
        self.reject('changed manifest activates unused E0308')

    def test_native_named_journal_and_resealed_manifest_tamper(self):
        f=self.root/'native/resident-results.json'; lines=json.loads(f.read_text())
        lines[0]='test invented::name ... ok <0.000s>'
        write_json(f,lines)
        self.reject('fixed historical observation commitment')

    def test_frozen_candidate_source_tamper(self):
        f=self.root/'frozen/manifest-content-generation-v74/whole.candidate.rs'
        f.write_bytes(f.read_bytes()+b'\n// forged\n')
        self.reject('immutable original/candidate whole source')

    def test_original_oracle_tamper_not_executed(self):
        f=self.root/'oracles/resident_state_matrix.py.txt'
        f.write_text('raise RuntimeError("must never execute")\n')
        self.reject('fixed original oracle')

    def test_noise_unknown_cannot_be_promoted(self):
        self.data['noise']='controlled'
        self.reject('strictly scoped default-OFF acceptance')

    def test_default_promotion_rejected(self):
        self.data['default_promotion']=True
        self.reject('strictly scoped default-OFF acceptance')

    def test_general_or_independent_attribution_rejected(self):
        for key in ('general_speed_claim','causal_claim','independent_manifest_lock_claim'):
            with self.subTest(key=key):
                self.data[key]=True
                self.reject('strictly scoped default-OFF acceptance')
                self.data[key]=False

    def test_bad_native_gate_exit(self):
        self.data['native']['full-native']['exit']=101
        self.reject('native exit gate')

    def test_incomplete_regression_receipt(self):
        self.data['python_regression']['tests']=745
        self.reject('full746 actual regression journal')

    def test_private_path_or_ambient_env_cannot_be_added(self):
        self.data['private_dump']='/home/private/secret'
        self.reject('no private environment/proc path dump')

    def test_manifest_path_traversal(self):
        self.data['artifacts']['../escape']='0'*64
        self.reject('relative artifact path')

    def test_empty_home_path_relocation_audit(self):
        reseal(self.root)
        home=Path(self.temp.name)/'empty-home'; home.mkdir()
        env={'HOME':str(home),'PATH':'','PYTHONDONTWRITEBYTECODE':'1','LC_ALL':'C'}
        result=subprocess.run([sys.executable,'-I','-S','-B','-c',BOOTSTRAP,str(self.root),''],env=env,cwd=home,capture_output=True,text=True,timeout=30)
        self.assertEqual(result.returncode,0,result.stdout+result.stderr)
        self.assertEqual(json.loads(result.stdout)['native_jobs_executed'],0)
        self.assertEqual(list(home.iterdir()),[])

    def test_audit_rejects_external_read_network_native_and_write(self):
        home=Path(self.temp.name)/'empty-home'; home.mkdir()
        payloads=("open('/etc/passwd').read()", "socket.socket()", "subprocess.run(['/bin/true'])", "ctypes.CDLL(None)", "open(root/'forbidden-write','w')")
        for payload in payloads:
            with self.subTest(payload=payload):
                result=subprocess.run([sys.executable,'-I','-S','-B','-c',BOOTSTRAP,str(self.root),payload],env={'HOME':str(home),'PATH':''},cwd=home,capture_output=True,text=True,timeout=30)
                self.assertNotEqual(result.returncode,0)
                self.assertIn('AUDIT',result.stderr)


if __name__=='__main__':
    unittest.main(verbosity=2)
